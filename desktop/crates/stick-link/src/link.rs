//! Keeps a connection to the picked stick (link.py `Link`): connect, hello every 8 s, read JSON
//! lines, answer `proto` requests by id, reconnect when it drops, stop when paused or unpicked.

use crate::proto::{self, Line, LineDecoder};
use crate::transport::{BleError, Connection, Connector, Found, Incoming, Target};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot, watch};

/// What the link is doing (the dashboard's `link`): link.py's states plus `starting`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Starting,
    NoStick,
    Searching,
    Connected,
    Paused,
}

impl LinkState {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkState::Starting => "starting",
            LinkState::NoStick => "no-stick",
            LinkState::Searching => "searching",
            LinkState::Connected => "connected",
            LinkState::Paused => "paused",
        }
    }
}

/// Timings; the defaults are link.py's.
#[derive(Debug, Clone)]
pub struct Options {
    /// `None`: no hellos (tests).
    pub hello_every: Option<Duration>,
    /// after a failed connect
    pub retry_after: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self { hello_every: Some(proto::HELLO_EVERY), retry_after: Duration::from_secs(3) }
    }
}

type HelloExtra = Box<dyn Fn() -> Map<String, Value> + Send + Sync>;

/// Asked before every connect: `Some(why)` = not now (e.g. another app holds the stick).
pub type Gate = Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send>> + Send + Sync>;

#[derive(Debug, Clone, Default, PartialEq)]
struct Control {
    target: Option<Target>,
    paused: bool,
}

struct Inner<C: Connector> {
    connector: C,
    opts: Options,
    current: Mutex<Option<Arc<C::Conn>>>,
    write_lock: tokio::sync::Mutex<()>,
    ctl: watch::Sender<Control>,
    state: watch::Sender<LinkState>,
    messages: broadcast::Sender<Value>,
    audio: broadcast::Sender<Vec<u8>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Option<Value>>>>,
    next_id: AtomicU64,
    mtu: AtomicUsize,
    hello_extra: Mutex<Option<HelloExtra>>,
    gate: Mutex<Option<Gate>>,
    held_back: Mutex<Option<String>>,
}

/// The link. Cheap to clone; every clone is the same link.
pub struct Link<C: Connector> {
    inner: Arc<Inner<C>>,
}

impl<C: Connector> Clone for Link<C> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<C: Connector> Link<C> {
    pub fn new(connector: C) -> Self {
        Self::with_options(connector, Options::default())
    }

    pub fn with_options(connector: C, opts: Options) -> Self {
        let (ctl, _) = watch::channel(Control::default());
        let (state, _) = watch::channel(LinkState::Starting);
        Self {
            inner: Arc::new(Inner {
                connector,
                opts,
                current: Mutex::new(None),
                write_lock: tokio::sync::Mutex::new(()),
                ctl,
                state,
                messages: broadcast::channel(256).0,
                audio: broadcast::channel(512).0,
                pending: Mutex::new(HashMap::new()),
                next_id: AtomicU64::new(1),
                mtu: AtomicUsize::new(0),
                hello_extra: Mutex::new(None),
                gate: Mutex::new(None),
                held_back: Mutex::new(None),
            }),
        }
    }

    pub fn connector(&self) -> &C {
        &self.inner.connector
    }

    /// Starts the reconnect loop on the current tokio runtime.
    pub fn spawn(&self) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move { me.run().await })
    }

    /// The stick to stay connected to (`None`: none picked; the link waits).
    pub fn set_target(&self, target: Option<Target>) {
        self.inner.ctl.send_modify(|c| c.target = target);
    }

    pub fn target(&self) -> Option<Target> {
        self.inner.ctl.borrow().target.clone()
    }

    /// Paused: disconnected and not trying (the dashboard's Pause).
    pub fn pause(&self, paused: bool) {
        self.inner.ctl.send_modify(|c| c.paused = paused);
    }

    pub fn paused(&self) -> bool {
        self.inner.ctl.borrow().paused
    }

    /// Merged into every hello (later: the home card, weather, `wmic`...).
    pub fn set_hello_extra(&self, f: impl Fn() -> Map<String, Value> + Send + Sync + 'static) {
        *self.inner.hello_extra.lock().unwrap() = Some(Box::new(f));
    }

    /// Checked before each connect attempt; while it says why not, the link waits (state
    /// `searching`) and asks again after `retry_after`.
    pub fn set_gate(&self, gate: Gate) {
        *self.inner.gate.lock().unwrap() = Some(gate);
    }

    /// Why the link isn't connecting right now (the gate's answer), if it's held back.
    pub fn held_back(&self) -> Option<String> {
        self.inner.held_back.lock().unwrap().clone()
    }

    pub fn state(&self) -> LinkState {
        *self.inner.state.borrow()
    }

    pub fn watch_state(&self) -> watch::Receiver<LinkState> {
        self.inner.state.subscribe()
    }

    pub fn connected(&self) -> bool {
        self.inner.current.lock().unwrap().is_some()
    }

    pub fn mtu(&self) -> usize {
        self.inner.mtu.load(Ordering::Relaxed)
    }

    /// Every JSON message from the stick.
    pub fn messages(&self) -> broadcast::Receiver<Value> {
        self.inner.messages.subscribe()
    }

    /// Every audio frame from the stick (talk / Wispr; used in Phase 2).
    pub fn audio(&self) -> broadcast::Receiver<Vec<u8>> {
        self.inner.audio.subscribe()
    }

    /// Nearby sticks (never connects).
    pub async fn scan(&self, timeout: Duration) -> Result<Vec<Found>, BleError> {
        self.inner.connector.scan(timeout).await
    }

    /// Sends one message (link.py `send`): false if not connected or the write failed.
    pub async fn send(&self, msg: &Value) -> bool {
        let Some(conn) = self.inner.current.lock().unwrap().clone() else {
            return false;
        };
        let data = proto::encode(msg);
        let _guard = self.inner.write_lock.lock().await;
        for part in proto::chunks(&data) {
            if let Err(e) = write_chunk(&*conn, part).await {
                let first = msg.as_object().and_then(|o| o.keys().next().cloned()).unwrap_or_else(|| "?".into());
                log::warn!("send failed ({first}, {} bytes): {e}", data.len());
                return false;
            }
        }
        true
    }

    /// Runs a settings line on the stick (`@state`, `@brightness pct=60`) and returns its reply;
    /// `None` if not connected, the write failed, or no answer within 5 s.
    pub async fn request(&self, line: &str) -> Option<Value> {
        self.request_timeout(line, proto::PROTO_TIMEOUT).await
    }

    pub async fn request_timeout(&self, line: &str, timeout: Duration) -> Option<Value> {
        if !self.connected() {
            return None;
        }
        let id = format!("r{}", self.inner.next_id.fetch_add(1, Ordering::Relaxed));
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().unwrap().insert(id.clone(), tx);
        let _forget = Forget { link: &self.inner.pending, id: &id };
        if !self.send(&proto::proto_request(line, &id)).await {
            return None;
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(reply)) => reply,
            _ => None,
        }
    }

    fn set_state(&self, s: LinkState) {
        self.inner.state.send_if_modified(|cur| {
            let changed = *cur != s;
            *cur = s;
            changed
        });
    }

    async fn run(self) {
        let mut ctl = self.inner.ctl.subscribe();
        loop {
            let c = ctl.borrow_and_update().clone();
            let target = match (&c.target, c.paused) {
                (_, true) => {
                    self.set_state(LinkState::Paused);
                    None
                }
                (None, false) => {
                    self.set_state(LinkState::NoStick);
                    None
                }
                (Some(t), false) => Some(t.clone()),
            };
            let Some(target) = target else {
                if ctl.changed().await.is_err() {
                    return;
                }
                continue;
            };
            self.set_state(LinkState::Searching);
            let gate = self.inner.gate.lock().unwrap().clone();
            let why = match gate {
                Some(g) => g().await,
                None => None,
            };
            let changed = *self.inner.held_back.lock().unwrap() != why;
            if changed && let Some(w) = &why {
                log::warn!("not connecting: {w}");
            }
            *self.inner.held_back.lock().unwrap() = why.clone();
            if why.is_some() {
                tokio::select! {
                    _ = tokio::time::sleep(self.inner.opts.retry_after) => {}
                    r = ctl.changed() => if r.is_err() { return; }
                }
                continue;
            }
            let attempt = tokio::select! {
                r = self.inner.connector.connect(&target) => Some(r),
                r = ctl.changed() => { if r.is_err() { return; } None }  // re-picked / paused meanwhile
            };
            match attempt {
                None => continue,
                Some(Err(e)) => {
                    if !e.not_found {
                        log::info!("link: {e}");
                    }
                    tokio::select! {
                        _ = tokio::time::sleep(self.inner.opts.retry_after) => {}
                        r = ctl.changed() => if r.is_err() { return; }
                    }
                }
                Some(Ok((conn, rx))) => {
                    log::info!("connected to {} (MTU {})", target.name, conn.mtu());
                    self.serve(Arc::new(conn), rx, &target, &mut ctl).await;
                    if !self.paused() {
                        log::info!("stick disconnected, retrying");
                    }
                }
            }
        }
    }

    async fn serve(
        &self,
        conn: Arc<C::Conn>,
        mut rx: mpsc::UnboundedReceiver<Incoming>,
        target: &Target,
        ctl: &mut watch::Receiver<Control>,
    ) {
        self.inner.mtu.store(conn.mtu(), Ordering::Relaxed);
        *self.inner.current.lock().unwrap() = Some(conn.clone());
        self.set_state(LinkState::Connected);
        let hello = self.inner.opts.hello_every.map(|every| {
            let me = self.clone();
            tokio::spawn(async move {
                loop {
                    let extra = me.inner.hello_extra.lock().unwrap().as_ref().map(|f| f());
                    let msg = proto::hello(chrono::Utc::now().timestamp(), proto::local_tz_offset(), extra.as_ref());
                    me.send(&msg).await;
                    tokio::time::sleep(every).await;
                }
            })
        });
        let mut lines = LineDecoder::new();
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    None => break,  // disconnected
                    Some(Incoming::Tx(data)) => {
                        for line in lines.push(&data) {
                            match line {
                                Line::Json(v) => self.dispatch(v),
                                Line::Bad(b) => log::warn!(
                                    "unreadable line from the stick ({} bytes): {:?}",
                                    b.len(),
                                    String::from_utf8_lossy(&b[..b.len().min(80)])
                                ),
                            }
                        }
                    }
                    Some(Incoming::Audio(frame)) => { let _ = self.inner.audio.send(frame); }
                },
                r = ctl.changed() => {
                    let c = ctl.borrow_and_update().clone();
                    if r.is_err() || c.paused || c.target.as_ref() != Some(target) {
                        break;
                    }
                }
            }
        }
        if let Some(h) = hello {
            h.abort();
        }
        *self.inner.current.lock().unwrap() = None;
        self.inner.pending.lock().unwrap().clear(); // waiting requests get None now
        self.inner.mtu.store(0, Ordering::Relaxed);
        conn.disconnect().await;
    }

    fn dispatch(&self, msg: Value) {
        if let Some((id, reply)) = proto::proto_reply(&msg)
            && let Some(tx) = self.inner.pending.lock().unwrap().remove(id)
        {
            let _ = tx.send(reply.cloned());
        }
        let _ = self.inner.messages.send(msg);
    }
}

/// One chunk, tried again while the stick says "Insufficient Resource" (link.py `_write`).
async fn write_chunk<T: Connection>(conn: &T, part: &[u8]) -> Result<(), BleError> {
    let mut n = 0;
    loop {
        match conn.write_rx(part.to_vec()).await {
            Ok(()) => return Ok(()),
            Err(e) if e.is_insufficient_resource() && n + 1 < proto::WRITE_TRIES => {
                tokio::time::sleep(proto::retry_delay(n)).await;
                n += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Removes a request's waiter however the request ends.
struct Forget<'a> {
    link: &'a Mutex<HashMap<String, oneshot::Sender<Option<Value>>>>,
    id: &'a str,
}

impl Drop for Forget<'_> {
    fn drop(&mut self) {
        self.link.lock().unwrap().remove(self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeStick;
    use serde_json::json;

    fn quiet() -> Options {
        Options { hello_every: None, retry_after: Duration::from_millis(50) }
    }

    async fn connected(fake: &FakeStick, opts: Options) -> Link<FakeStick> {
        let link = Link::with_options(fake.clone(), opts);
        link.spawn();
        link.set_target(Some(Target { address: "FAKE".into(), name: "StickyBot".into() }));
        wait_for(&link, LinkState::Connected).await;
        link
    }

    async fn wait_for(link: &Link<FakeStick>, s: LinkState) {
        let mut w = link.watch_state();
        tokio::time::timeout(Duration::from_secs(30), w.wait_for(|x| *x == s)).await.expect("state").unwrap();
    }

    // --- test_link_send.py ---------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn a_busy_stick_gets_the_chunk_again() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_busy(2, "(17, 'GATT Protocol Error: Insufficient Resource')");
        assert!(link.send(&json!({"src": "agent", "x": "y".repeat(400)})).await);
        let parts = fake.writes();
        assert_eq!(parts.len(), 3);
        assert!(parts.concat().ends_with(b"}\n"));
        assert_eq!(fake.write_attempts(), 5); // 2 refused + 3 accepted
    }

    #[tokio::test(start_paused = true)]
    async fn it_gives_up_after_a_few_tries() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_busy(10, "Insufficient Resource");
        assert!(!link.send(&json!({"src": "agent"})).await);
        assert_eq!(fake.write_attempts(), proto::WRITE_TRIES as usize);
        assert!(fake.writes().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn other_errors_are_not_retried() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_busy(1, "disconnected");
        assert!(!link.send(&json!({"src": "agent"})).await);
        assert_eq!(fake.busy_left(), 0);
        assert!(fake.writes().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn the_att_code_alone_is_enough_to_retry() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_busy_code(1, proto::ATT_INSUFFICIENT_RESOURCES, "Resources are insufficient.");
        assert!(link.send(&json!({"src": "agent"})).await);
        assert_eq!(fake.writes().len(), 1);
    }

    // --- the link itself -------------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn nothing_is_sent_without_a_connection() {
        let link = Link::with_options(FakeStick::new(), quiet());
        link.spawn();
        assert!(!link.send(&json!({"src": "agent"})).await);
        assert_eq!(link.request("@state").await, None);
        wait_for(&link, LinkState::NoStick).await;
    }

    #[tokio::test(start_paused = true)]
    async fn proto_requests_get_their_reply_by_id() {
        let fake = FakeStick::new();
        fake.set_notify_size(20); // the reply arrives in many small pieces
        let link = connected(&fake, quiet()).await;
        let st = link.request("@state").await.expect("a reply");
        assert_eq!(st["ok"], true);
        assert_eq!(st["board"], "m5stick-s3");
        let sent = String::from_utf8(fake.writes().concat()).unwrap();
        assert!(sent.contains("\"proto\":\"@state\"") && sent.contains("\"id\":\"r"));
        // two at once: each gets its own
        let (a, b) = tokio::join!(link.request("@brightness pct=60"), link.request("@state"));
        assert_eq!(a.unwrap()["ok"], true);
        assert_eq!(b.unwrap()["brightness"], 60);
    }

    #[tokio::test(start_paused = true)]
    async fn a_request_without_an_answer_times_out() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_silent(true);
        assert_eq!(link.request("@state").await, None);
        assert!(link.inner.pending.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn messages_reach_every_subscriber() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        let mut a = link.messages();
        let mut b = link.messages();
        fake.notify_raw(b"{\"src\":\"agent\",\"evt\":\"talk\",");
        fake.notify_raw(
            b"\"state\":\"start\"}\nnot json\n{\"cmd\":\"permission\",\"id\":\"x\",\"decision\":\"once\"}\n",
        );
        for rx in [&mut a, &mut b] {
            assert_eq!(rx.recv().await.unwrap(), json!({"src": "agent", "evt": "talk", "state": "start"}));
            assert_eq!(rx.recv().await.unwrap()["decision"], "once");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn audio_frames_are_passed_on() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        let mut au = link.audio();
        fake.notify_audio(&[0, 0, 1, 2, 3]);
        assert_eq!(au.recv().await.unwrap(), vec![0, 0, 1, 2, 3]);
    }

    #[tokio::test(start_paused = true)]
    async fn it_reconnects_after_a_drop() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        assert_eq!(fake.connects(), 1);
        let mut w = link.watch_state();
        fake.drop_connection();
        w.wait_for(|s| *s != LinkState::Connected).await.unwrap();
        wait_for(&link, LinkState::Connected).await;
        assert_eq!(fake.connects(), 2);
        assert!(link.request("@state").await.is_some());
    }

    #[tokio::test(start_paused = true)]
    async fn a_stick_that_isnt_there_is_retried() {
        let fake = FakeStick::new();
        fake.set_absent(true);
        let link = Link::with_options(fake.clone(), quiet());
        link.spawn();
        link.set_target(Some(Target { address: "FAKE".into(), name: "x".into() }));
        wait_for(&link, LinkState::Searching).await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(fake.connect_attempts() >= 3);
        fake.set_absent(false);
        wait_for(&link, LinkState::Connected).await;
    }

    #[tokio::test(start_paused = true)]
    async fn pausing_disconnects_and_resuming_connects_again() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        link.pause(true);
        wait_for(&link, LinkState::Paused).await;
        tokio::task::yield_now().await;
        assert!(!link.connected());
        assert_eq!(fake.disconnects(), 1);
        link.pause(false);
        wait_for(&link, LinkState::Connected).await;
        link.set_target(None);
        wait_for(&link, LinkState::NoStick).await;
        assert_eq!(fake.disconnects(), 2);
    }

    #[tokio::test(start_paused = true)]
    async fn waiting_requests_end_when_the_link_drops() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        fake.set_silent(true);
        let l2 = link.clone();
        let req = tokio::spawn(async move { l2.request_timeout("@state", Duration::from_secs(60)).await });
        tokio::time::sleep(Duration::from_millis(10)).await;
        fake.drop_connection();
        assert_eq!(req.await.unwrap(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn the_hello_goes_out_on_connect_and_every_8_s() {
        let fake = FakeStick::new();
        let link = connected(&fake, Options::default()).await;
        let mut extra = Map::new();
        extra.insert("wmic".into(), json!(false));
        link.set_hello_extra(move || extra.clone());
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(fake.hellos(), 1);
        tokio::time::sleep(Duration::from_secs(8)).await;
        assert_eq!(fake.hellos(), 2);
        let last = fake.last_hello().unwrap();
        assert_eq!(last["src"], "agent");
        assert_eq!(last["v"], 1);
        assert!(last["t"].as_i64().unwrap() > 1_600_000_000);
        assert!(last["tz"].is_i64());
        assert_eq!(last["wmic"], false);
    }

    #[tokio::test(start_paused = true)]
    async fn the_gate_holds_the_link_back() {
        let fake = FakeStick::new();
        let link = Link::with_options(fake.clone(), quiet());
        let busy = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let b = busy.clone();
        link.set_gate(Arc::new(move || {
            let held = b.load(Ordering::Relaxed);
            Box::pin(async move { held.then(|| "the Python agent holds the stick".to_string()) })
        }));
        link.spawn();
        link.set_target(Some(Target { address: "FAKE".into(), name: "x".into() }));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(fake.connect_attempts(), 0);
        assert_eq!(link.state(), LinkState::Searching);
        assert_eq!(link.held_back().as_deref(), Some("the Python agent holds the stick"));
        busy.store(false, Ordering::Relaxed);
        wait_for(&link, LinkState::Connected).await;
        assert_eq!(link.held_back(), None);
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_target_means_a_new_connection() {
        let fake = FakeStick::new();
        let link = connected(&fake, quiet()).await;
        let mut w = link.watch_state();
        link.set_target(Some(Target { address: "OTHER".into(), name: "y".into() }));
        w.wait_for(|s| *s == LinkState::Searching).await.unwrap();
        wait_for(&link, LinkState::Connected).await;
        assert_eq!(fake.connects(), 2);
        assert_eq!(fake.last_target().unwrap().address, "OTHER");
    }
}
