//! Firmware update over Bluetooth, the transfer (a port of `ota.py`'s `Sender`; PROTOCOL.md
//! "Firmware update over Bluetooth").
//!
//! `begin` (JSON) -> `ready` with the offset to start at (0, or where an interrupted transfer got
//! to) -> packets `[offset u32 LE][bytes]` on OTA_DATA, written without response and paced by
//! CoreBluetooth's `canSendWriteWithoutResponse`, at most `window` bytes past the stick's last ack
//! -> `end` -> `done` -> the stick restarts. A lost packet makes the stick ask for a resend
//! (`gap`); a dropped link is resumed with a new `begin` when it comes back.

use crate::link::Link;
use crate::transport::Connector;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc};
use tokio::time::Instant;

/// `[offset u32]` in front of each packet.
pub const HEADER: usize = 4;

/// The waits; the defaults are ota.py's.
#[derive(Debug, Clone)]
pub struct Timings {
    /// no ack this long with the window full: resend from the last ack
    pub ack_wait: Duration,
    /// no progress at all this long (link up): give up
    pub stall: Duration,
    /// link down this long: give up (the stick keeps the transfer ~3 min)
    pub reconnect: Duration,
    /// `begin` / `cancel` replies
    pub reply: Duration,
    /// `end`: the stick reads the image back for the hash (~1-2 s)
    pub end_reply: Duration,
    /// a repeated `gap` for the offset just rewound to
    pub gap_debounce: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            ack_wait: Duration::from_secs(1),
            stall: Duration::from_secs(20),
            reconnect: Duration::from_secs(120),
            reply: Duration::from_secs(6),
            end_reply: Duration::from_secs(20),
            gap_debounce: Duration::from_millis(500),
        }
    }
}

/// The image to send (the bundle's app.bin).
#[derive(Debug, Clone)]
pub struct Image {
    pub data: Arc<Vec<u8>>,
    pub sha256: String,
    pub version: String,
}

impl Image {
    pub fn new(data: Vec<u8>, version: &str) -> Self {
        use sha2::{Digest, Sha256};
        let sha256 = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
        Self { data: Arc::new(data), sha256, version: version.into() }
    }
    pub fn size(&self) -> usize {
        self.data.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtaError {
    Failed(String),
    Cancelled(String),
}

impl std::fmt::Display for OtaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OtaError::Failed(m) | OtaError::Cancelled(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for OtaError {}

fn fail(m: impl Into<String>) -> OtaError {
    OtaError::Failed(m.into())
}

/// The stick's refusal / error codes in words (ota.py `REASONS`).
pub fn reason(msg: &Value) -> String {
    let code = msg.get("error").and_then(Value::as_str).unwrap_or("?");
    let why = msg.get("why").and_then(Value::as_str).filter(|w| !w.is_empty()).unwrap_or("busy");
    match code {
        "busy" => format!("the stick is busy ({why}) — try again when it's idle"),
        "battery" => "the stick's battery is below 30 % — plug it in to charge, then try again".into(),
        "pending" => PENDING.into(),
        "unsupported" => "the stick can't take a compressed image".into(),
        "size" => "the image doesn't fit the stick's update slot".into(),
        "sha" => "the image's checksum didn't match (nothing was changed on the stick)".into(),
        "downgrade" => "that's an older firmware than the stick's — use force to install it anyway".into(),
        "same" => "the stick already runs this version — use force to install it again".into(),
        "image" => "the stick refused the image (not a valid M5StickS3 firmware)".into(),
        "open" => "the stick couldn't start writing its update slot".into(),
        "write" => "writing the stick's flash failed".into(),
        "read" => "reading the image back on the stick failed".into(),
        "timeout" => "the stick gave up waiting for data".into(),
        "not_started" => "the stick had no transfer running".into(),
        other => format!("the stick said {other}"),
    }
}

pub const PENDING: &str =
    "the stick has just been updated and is still checking the new firmware — try again in half a minute";

/// What a finished transfer took.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub bytes: usize,
    pub seconds: f64,
    pub kbps: f64,
    pub resent: usize,
    pub reconnects: usize,
    pub sent: usize,
}

impl Stats {
    pub fn to_json(&self) -> Value {
        json!({"bytes": self.bytes, "seconds": self.seconds, "kbps": self.kbps, "resent": self.resent,
               "reconnects": self.reconnects, "sent": self.sent})
    }
}

pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;

/// Sends one image over a [`Link`]. `emit` gets `{"line"}` / `{"progress","stage"}` events.
pub struct Sender<C: Connector> {
    link: Link<C>,
    emit: Emit,
    cancelled: Arc<AtomicBool>,
    pub timings: Timings,
    pct: i64,
}

/// Stops a running [`Sender`] (the dashboard's Cancel).
#[derive(Clone)]
pub struct CancelHandle(Arc<AtomicBool>);

impl CancelHandle {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// The stick's `evt: ota` messages, from when the sender started listening.
struct Inbox {
    rx: mpsc::UnboundedReceiver<Value>,
    pump: tokio::task::JoinHandle<()>,
}

impl Drop for Inbox {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

impl<C: Connector> Sender<C> {
    pub fn new(link: Link<C>, emit: Emit) -> Self {
        Self { link, emit, cancelled: Arc::new(AtomicBool::new(false)), timings: Timings::default(), pct: -1 }
    }

    pub fn cancel_handle(&self) -> CancelHandle {
        CancelHandle(self.cancelled.clone())
    }

    fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }

    fn listen(&self) -> Inbox {
        let mut msgs = self.link.messages();
        let (tx, rx) = mpsc::unbounded_channel();
        let pump = tokio::spawn(async move {
            loop {
                match msgs.recv().await {
                    Ok(m) => {
                        if m.get("src").and_then(Value::as_str) == Some("agent")
                            && m.get("evt").and_then(Value::as_str) == Some("ota")
                            && tx.send(m).is_err()
                        {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => return,
                }
            }
        });
        Inbox { rx, pump }
    }

    fn progress(&mut self, offset: usize, size: usize) {
        let pct = (offset * 100 / size.max(1)).min(100) as i64;
        if pct != self.pct {
            self.pct = pct;
            (self.emit)(json!({"progress": pct, "stage": "send"}));
        }
    }

    /// The next ota message whose state is in `states` (others — late acks — are skipped).
    async fn reply(&self, inbox: &mut Inbox, states: &[&str], timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        loop {
            match tokio::time::timeout_at(deadline, inbox.rx.recv()).await {
                Ok(Some(m)) => {
                    if m.get("state").and_then(Value::as_str).is_some_and(|s| states.contains(&s)) {
                        return Some(m);
                    }
                }
                _ => return None,
            }
        }
    }

    async fn wait_link(&self, timeout: Duration) -> Result<(), OtaError> {
        let deadline = Instant::now() + timeout;
        while !self.link.connected() {
            if self.cancelled() {
                return Err(OtaError::Cancelled("cancelled".into()));
            }
            if Instant::now() > deadline {
                return Err(fail("lost the stick (Bluetooth) and it didn't come back"));
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }

    async fn begin(&self, inbox: &mut Inbox, img: &Image, force: bool) -> Result<Value, OtaError> {
        let msg = json!({"src": "agent", "ota": {"begin": {"size": img.size(), "sha256": img.sha256,
                          "version": img.version, "compressed": false, "force": force}}});
        let deadline = Instant::now() + self.timings.reconnect;
        let mut unanswered = 0;
        while unanswered < 3 {
            if !self.link.send(&msg).await {
                // the link is going down (the write failed before it noticed) or is down
                if Instant::now() > deadline {
                    return Err(fail("lost the stick (Bluetooth) and it didn't come back"));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_link(deadline.saturating_duration_since(Instant::now())).await?;
                continue;
            }
            unanswered += 1;
            let Some(r) = self.reply(inbox, &["ready", "refused", "error"], self.timings.reply).await else {
                continue;
            };
            if r["state"] != "ready" {
                return Err(fail(reason(&r)));
            }
            return Ok(r);
        }
        Err(fail("the stick didn't answer the update request (firmware older than 1.8.0?)"))
    }

    async fn abort(&self, inbox: &mut Inbox) {
        if self.link.connected() {
            self.link.send(&json!({"src": "agent", "ota": {"cancel": 1}})).await;
            self.reply(inbox, &["cancelled"], self.timings.reply).await;
        }
    }

    /// Sends `img` and waits for `done`.
    pub async fn run(&mut self, img: &Image, force: bool) -> Result<Stats, OtaError> {
        let mut inbox = self.listen();
        let size = img.size();
        let data = img.data.clone();
        let t0 = Instant::now();
        let (mut sent_bytes, mut resends, mut reconnects) = (0usize, 0usize, 0usize);
        let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_u64).map(|x| x as usize);
        (self.emit)(json!({"progress": 0, "stage": "send"}));
        loop {
            // one pass per connection (a dropped link resumes with a new begin)
            if self.cancelled() {
                return Err(OtaError::Cancelled("cancelled".into()));
            }
            let r = self.begin(&mut inbox, img, force).await?;
            let mut acked = n(&r, "offset").unwrap_or(0).min(size);
            let mut sent = acked;
            let window = n(&r, "window").filter(|w| *w > 0).unwrap_or(16384);
            let chunk = n(&r, "chunk")
                .filter(|c| *c > 0)
                .unwrap_or(180)
                .min(self.link.ota_max_write().saturating_sub(HEADER))
                .max(16);
            if r.get("resumed").and_then(Value::as_bool) == Some(true) {
                (self.emit)(
                    json!({"line": format!("resuming at {}% ({acked} of {size} bytes)", acked * 100 / size.max(1))}),
                );
            } else {
                (self.emit)(
                    json!({"line": format!("sending {} KB in {chunk}-byte packets, window {} KB", size / 1024, window / 1024)}),
                );
            }
            let mut last_progress = Instant::now();
            let mut waited_from = last_progress;
            let mut rewound: (Option<usize>, Instant) = (None, Instant::now());
            let mut dropped = false;
            while acked < size {
                if self.cancelled() {
                    self.abort(&mut inbox).await;
                    return Err(OtaError::Cancelled("cancelled".into()));
                }
                // fill the window
                while sent < size && sent - acked < window && inbox.rx.is_empty() {
                    let end = (sent + chunk).min(size);
                    let mut pkt = Vec::with_capacity(HEADER + end - sent);
                    pkt.extend_from_slice(&(sent as u32).to_le_bytes());
                    pkt.extend_from_slice(&data[sent..end]);
                    if !self.link.ota_write(pkt).await {
                        dropped = true;
                        break;
                    }
                    sent_bytes += end - sent;
                    sent = end;
                }
                if dropped {
                    break;
                }
                let full = sent - acked >= window || sent >= size;
                let wait = if full { self.timings.ack_wait } else { Duration::from_millis(50) };
                let msg = tokio::time::timeout(wait, inbox.rx.recv()).await.ok().flatten();
                let Some(msg) = msg else {
                    if !self.link.connected() {
                        dropped = true;
                        break;
                    }
                    if full && waited_from.elapsed() >= self.timings.ack_wait {
                        // nothing back: resend what isn't acknowledged
                        resends += sent - acked;
                        sent = acked;
                        waited_from = Instant::now();
                    }
                    if last_progress.elapsed() > self.timings.stall {
                        return Err(fail("the stick stopped answering during the update"));
                    }
                    continue;
                };
                match msg.get("state").and_then(Value::as_str) {
                    Some("ack") => {
                        let off = n(&msg, "offset").unwrap_or(0).min(size);
                        if off > acked {
                            acked = off;
                            last_progress = Instant::now();
                            waited_from = last_progress;
                            self.progress(acked, size);
                        }
                        if msg.get("gap").and_then(Value::as_bool) == Some(true) {
                            let again = rewound.0 == Some(off) && rewound.1.elapsed() < self.timings.gap_debounce;
                            if !again && off < sent {
                                resends += sent - off;
                                sent = off;
                                rewound = (Some(off), Instant::now());
                            }
                        }
                        sent = sent.max(acked);
                    }
                    Some("cancelled") => {
                        let by_stick = msg.get("by").and_then(Value::as_str) == Some("stick");
                        return Err(OtaError::Cancelled(
                            if by_stick { "cancelled on the stick" } else { "cancelled" }.into(),
                        ));
                    }
                    Some("error") => return Err(fail(reason(&msg))),
                    _ => {}
                }
            }
            if dropped {
                reconnects += 1;
                (self.emit)(
                    json!({"line": format!("Bluetooth dropped at {}% — waiting for the stick to resume", acked * 100 / size.max(1))}),
                );
                self.wait_link(self.timings.reconnect).await?;
                continue;
            }
            // everything acknowledged: the stick checks the hash and commits
            (self.emit)(json!({"progress": 100, "stage": "send"}));
            (self.emit)(json!({"progress": 0, "stage": "verify"}));
            if !self.link.send(&json!({"src": "agent", "ota": {"end": 1}})).await {
                reconnects += 1;
                self.wait_link(self.timings.reconnect).await?;
                continue;
            }
            let Some(r) = self.reply(&mut inbox, &["done", "error", "cancelled"], self.timings.end_reply).await else {
                if !self.link.connected() {
                    // the reply may be lost; a new begin says where it is
                    reconnects += 1;
                    self.wait_link(self.timings.reconnect).await?;
                    continue;
                }
                return Err(fail("the stick didn't confirm the image"));
            };
            match (r["state"].as_str(), r.get("error").and_then(Value::as_str)) {
                (Some("error"), Some("incomplete")) => continue, // missing bytes after all: resume
                (Some("cancelled"), _) => return Err(OtaError::Cancelled("cancelled on the stick".into())),
                (Some("error"), _) => return Err(fail(reason(&r))),
                _ => {}
            }
            (self.emit)(json!({"progress": 100, "stage": "verify"}));
            break;
        }
        let secs = t0.elapsed().as_secs_f64().max(1e-6);
        let kbps = size as f64 / 1024.0 / secs;
        let stats = Stats {
            bytes: size,
            seconds: (secs * 10.0).round() / 10.0,
            kbps: (kbps * 10.0).round() / 10.0,
            resent: resends,
            reconnects,
            sent: sent_bytes,
        };
        log::info!(
            "ota: {} KB in {secs:.1} s = {kbps:.1} KB/s ({resends} bytes resent, {reconnects} reconnects)",
            size / 1024
        );
        let resent = if resends >= 1024 { format!(", {} KB resent", resends / 1024) } else { String::new() };
        let again = if reconnects > 0 { format!(", {reconnects} reconnects") } else { String::new() };
        (self.emit)(
            json!({"line": format!("sent {} KB in {secs:.0} s ({kbps:.1} KB/s{resent}{again}) — the stick verified it", size / 1024)}),
        );
        Ok(stats)
    }
}

/// `'1.6.0'` -> `[1, 6, 0]`; anything unparsable -> `[]` (flasher.py `version_tuple`).
pub fn version_tuple(v: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut cur: Option<u64> = None;
    for c in v.chars() {
        match c.to_digit(10) {
            Some(d) => cur = Some(cur.unwrap_or(0).saturating_mul(10).saturating_add(d as u64)),
            None => {
                if let Some(n) = cur.take() {
                    out.push(n);
                }
            }
        }
    }
    out.extend(cur);
    out.truncate(3);
    out
}

/// The stick's `@state` says it takes updates over Bluetooth (firmware 1.8.0+).
pub fn supported(state: &Value) -> bool {
    state.get("ota").and_then(Value::as_i64).unwrap_or(0) >= 1
}

/// `None`, or why the update shouldn't run (older / same) without `force`.
pub fn check_versions(new: &str, current: Option<&str>, force: bool) -> Option<String> {
    let current = current.filter(|c| !c.is_empty())?;
    if force {
        return None;
    }
    let (n, c) = (version_tuple(new), version_tuple(current));
    if n < c {
        return Some(format!("the stick runs {current}, newer than {new} — use force to downgrade"));
    }
    if n == c {
        return Some(format!("the stick already runs {current} — use force to install it again"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeStick;
    use crate::{LinkState, Options, Target};
    use std::collections::HashSet;
    use std::sync::Mutex;

    fn offsets(chunk: usize, idx: &[usize]) -> HashSet<u32> {
        idx.iter().map(|i| (i * chunk) as u32).collect()
    }

    fn app_image(n: usize) -> Vec<u8> {
        let mut d: Vec<u8> = (0..n).map(|i| ((i * 7 + (i >> 8)) & 0xFF) as u8).collect();
        d[0] = 0xE9;
        d
    }

    fn quick() -> Timings {
        Timings {
            ack_wait: Duration::from_millis(50),
            stall: Duration::from_secs(1),
            reconnect: Duration::from_secs(1),
            reply: Duration::from_millis(300),
            end_reply: Duration::from_millis(300),
            gap_debounce: Duration::from_millis(500),
        }
    }

    async fn setup() -> (FakeStick, Link<FakeStick>) {
        let fake = FakeStick::new();
        let link =
            Link::with_options(fake.clone(), Options { hello_every: None, retry_after: Duration::from_millis(50) });
        link.spawn();
        link.set_target(Some(Target { address: "FAKE".into(), name: "x".into() }));
        let mut w = link.watch_state();
        tokio::time::timeout(Duration::from_secs(30), w.wait_for(|s| *s == LinkState::Connected))
            .await
            .unwrap()
            .unwrap();
        (fake, link)
    }

    fn sender(link: &Link<FakeStick>) -> (Sender<FakeStick>, Arc<Mutex<Vec<Value>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let e = events.clone();
        let mut s = Sender::new(link.clone(), Arc::new(move |v| e.lock().unwrap().push(v)));
        s.timings = quick();
        (s, events)
    }

    fn image() -> Image {
        Image::new(app_image(50_000), "1.8.1")
    }

    fn committed(fake: &FakeStick, img: &Image) -> bool {
        fake.ota(|o| o.committed && o.session.as_ref().is_some_and(|(_, _, d)| *d == *img.data))
    }

    // --- test_ota.py ---------------------------------------------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn sends_the_whole_image_and_gets_done() {
        let (fake, link) = setup().await;
        let (mut s, events) = sender(&link);
        let img = image();
        let st = s.run(&img, false).await.unwrap();
        assert!(committed(&fake, &img));
        assert_eq!((st.bytes, st.resent, st.reconnects), (50_000, 0, 0));
        let begin = fake.ota(|o| o.begins[0].clone());
        assert_eq!(
            begin,
            json!({"size": 50_000, "sha256": img.sha256, "version": "1.8.1", "compressed": false, "force": false})
        );
        let ev = events.lock().unwrap();
        let progress: Vec<i64> =
            ev.iter().filter(|e| e["stage"] == "send").map(|e| e["progress"].as_i64().unwrap()).collect();
        assert_eq!((progress[0], *progress.last().unwrap()), (0, 100));
        assert!(progress.windows(2).all(|w| w[0] <= w[1]));
        assert!(ev.iter().any(|e| e["stage"] == "verify" && e["progress"] == 100));
        assert!(ev.iter().any(|e| e["line"].as_str().is_some_and(|l| l.contains("KB/s"))));
        // packets: at most the stick's chunk (240), never more than the window in flight
        assert_eq!(fake.ota(|o| o.packets.iter().copied().max()), Some(240));
    }

    #[tokio::test(start_paused = true)]
    async fn never_more_than_the_window_in_flight() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.window = 4096);
        let (mut s, _) = sender(&link);
        s.run(&image(), false).await.unwrap();
        assert!(fake.ota(|o| o.max_in_flight) <= 4096 + 240);
    }

    #[tokio::test(start_paused = true)]
    async fn chunk_is_limited_by_the_mac_side() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.chunk = 600); // the stick would take more than the Mac can write (244 - 4)
        let (mut s, _) = sender(&link);
        s.run(&image(), false).await.unwrap();
        assert_eq!(fake.ota(|o| o.packets.iter().copied().max()), Some(240));
    }

    #[tokio::test(start_paused = true)]
    async fn dropped_packets_are_resent_after_a_gap() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.drop = offsets(240, &[10, 50, 51]));
        let (mut s, _) = sender(&link);
        let img = image();
        let st = s.run(&img, false).await.unwrap();
        assert!(committed(&fake, &img));
        assert!(st.resent > 0);
    }

    #[tokio::test(start_paused = true)]
    async fn lost_acks_time_out_and_resend() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.lose_acks = 2);
        let (mut s, _) = sender(&link);
        let img = image();
        let st = s.run(&img, false).await.unwrap();
        assert!(committed(&fake, &img));
        assert_eq!(st.bytes, 50_000);
    }

    #[tokio::test(start_paused = true)]
    async fn resumes_after_the_link_drops() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.disconnect_at = Some(20_000));
        let (mut s, events) = sender(&link);
        let img = image();
        let st = s.run(&img, false).await.unwrap();
        assert!(committed(&fake, &img));
        assert_eq!(st.reconnects, 1);
        assert_eq!(fake.ota(|o| o.begins.len()), 2); // the second begin resumed
        assert!(events.lock().unwrap().iter().any(|e| e["line"].as_str().is_some_and(|l| l.contains("resuming"))));
        assert_eq!(fake.ota(|o| o.received), 50_000); // nothing went into the image twice
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_when_the_stick_doesnt_come_back() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.disconnect_at = Some(10_000));
        let (mut s, _) = sender(&link);
        let l2 = link.clone();
        tokio::spawn(async move {
            // the stick goes away for good once the link drops
            let mut w = l2.watch_state();
            let _ = w.wait_for(|s| *s != LinkState::Connected).await;
            l2.pause(true);
        });
        let e = s.run(&image(), false).await.unwrap_err();
        assert!(e.to_string().contains("didn't come back"), "{e}");
    }

    #[tokio::test(start_paused = true)]
    async fn refused_begin_explains_why() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.refuse = Some("busy".into()));
        let (mut s, _) = sender(&link);
        let e = s.run(&image(), false).await.unwrap_err();
        assert!(e.to_string().contains("busy (dictation)"), "{e}");
        fake.ota(|o| o.refuse = Some("battery".into()));
        let e = s.run(&image(), false).await.unwrap_err();
        assert!(e.to_string().contains("30 %"), "{e}");
    }

    #[tokio::test(start_paused = true)]
    async fn no_answer_to_begin_means_old_firmware() {
        let (fake, link) = setup().await;
        fake.set_old_firmware(true);
        let (mut s, _) = sender(&link);
        let e = s.run(&image(), false).await.unwrap_err();
        assert!(e.to_string().contains("older than 1.8.0"), "{e}");
    }

    #[tokio::test(start_paused = true)]
    async fn sha_mismatch_on_the_stick_fails() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.corrupt = true);
        let (mut s, _) = sender(&link);
        let e = s.run(&image(), false).await.unwrap_err();
        assert!(e.to_string().contains("checksum"), "{e}");
        assert!(!fake.ota(|o| o.committed));
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_from_the_mac() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.window = 2048);
        let (mut s, _) = sender(&link);
        let h = s.cancel_handle();
        let task = tokio::spawn(async move { s.run(&image(), false).await });
        tokio::time::sleep(Duration::from_millis(1)).await;
        h.cancel();
        let e = task.await.unwrap().unwrap_err();
        assert!(matches!(e, OtaError::Cancelled(_)), "{e}");
        assert!(!fake.ota(|o| o.committed));
        assert!(fake.ota(|o| o.session.is_none()));
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_on_the_stick() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.cancel_at = Some(8000));
        let (mut s, _) = sender(&link);
        let e = s.run(&image(), false).await.unwrap_err();
        assert_eq!(e, OtaError::Cancelled("cancelled on the stick".into()));
    }

    #[tokio::test(start_paused = true)]
    async fn incomplete_end_resumes() {
        let (fake, link) = setup().await;
        fake.ota(|o| o.lose_tail = true);
        let (mut s, _) = sender(&link);
        let img = image();
        s.run(&img, false).await.unwrap();
        assert!(committed(&fake, &img));
        assert_eq!(fake.ota(|o| o.ends), 2);
    }

    #[test]
    fn support_and_versions() {
        assert!(supported(&json!({"fw": "1.8.0", "ota": 1})));
        assert!(!supported(&json!({"fw": "1.7.2"})));
        assert!(!supported(&Value::Null));
        assert_eq!(check_versions("1.8.1", Some("1.8.0"), false), None);
        assert!(check_versions("1.7.2", Some("1.8.0"), false).unwrap().contains("downgrade"));
        assert!(check_versions("1.8.0", Some("1.8.0"), false).unwrap().contains("already"));
        assert_eq!(check_versions("1.7.2", Some("1.8.0"), true), None);
        assert_eq!(check_versions("1.8.0", None, false), None);
        assert_eq!(version_tuple("v1.10.2-rc1"), vec![1, 10, 2]);
        assert_eq!(version_tuple("?"), Vec::<u64>::new());
    }
}
