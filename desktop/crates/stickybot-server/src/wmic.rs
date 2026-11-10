//! The wireless Wispr mic (core.py's wispr parts + stick_mic_device.py `Holds`).
//!
//! While A holds the Wispr shortcut away from USB, the stick streams its mic over Bluetooth; we
//! play it into a *carrier* (our own Core Audio process tap, or BlackHole 2ch) that feeds the
//! input of a public aggregate device named like the Python app's ("<stick>-bt-mic", so Wispr's
//! ranking finds it) but with the Rust UID ([`RUST_UID`]: the Python agent removes its own UID's
//! devices as left-overs, and our cleanup never touches the agent's). The device exists only
//! around a hold; Wispr switches to the highest-ranked mic present whenever devices change.
//!
//! Per press (PROTOCOL.md "the mic device"):
//! 1. A down → the stick sends `prepare` (id = a press counter) and streams right away, but holds
//!    the key.
//! 2. We open the output stream, create the device if it isn't there, wait `wispr_switch_ms`
//!    (250) only if we just created it, and send `ready` with the same id.
//! 3. The stick presses the key on `ready` (or after 800 ms without one).
//! 4. Release → 250 ms tail → key up → `stop`. We keep the device `wispr_tail_ms` (800), longer
//!    while another process still reads it (≤ 4 s more: removing it mid-dictation stops Wispr
//!    with "MicDisconnected"), then destroy it. A `prepare` in that time finds it: `ready` at once.
//!
//! The stick on USB: Wispr uses the stick's USB mic, no device (`ready` with `"device": false`).

use crate::AppState;
use macos_audio::Audio;
use macos_audio::carrier::{Carrier, CheckState, PERMISSION_HINT, pick_carrier};
use macos_audio::device::{Check, Made, PROBE_AMP, PROBE_S, StickMicDevice};
use macos_audio::jitter::RATE;
use macos_audio::naming::{Names, RUST_UID, is_auto_name, name_for};
use macos_audio::player::{Emit, Gate, WisprPlayer};
use serde_json::{Map, Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};
use stick_link::{FromStick, LinkState};
use tokio::sync::{broadcast, oneshot};

/// After creating it: time for Wispr to notice the new device before the key goes down.
pub const LEAD_MS: u64 = 250;
/// After the stream ends: keep it while Wispr finishes reading.
pub const TAIL_MS: u64 = 800;
/// "Create it now" from the dashboard, for ranking it in Wispr's settings.
pub const SHOW_S: u64 = 60;

/// How the device's lifetime is watched (tests shorten it).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    /// how often to look whether Wispr reads it
    pub poll: Duration,
    /// at most this much longer than the tail while Wispr is still reading it
    pub wait_reader: Duration,
}

impl Default for Timing {
    fn default() -> Self {
        Self { poll: Duration::from_millis(50), wait_reader: Duration::from_secs(4) }
    }
}

// --- the device, on its own thread ---------------------------------------------------------------

type Job = Box<dyn FnOnce(&mut StickMicDevice) + Send>;

/// What the pages and the hold flow read without waiting for the device's thread.
#[derive(Debug, Clone)]
pub struct Snap {
    pub name: String,
    pub present: bool,
    pub error: String,
    pub create_ms: Option<f64>,
    pub destroy_ms: Option<f64>,
    pub mode: Carrier,
    pub tap_live: bool,
    pub check: Check,
}

impl Snap {
    fn of(d: &StickMicDevice) -> Self {
        Self {
            name: d.name.clone(),
            present: d.present(),
            error: d.error.clone(),
            create_ms: d.create_ms,
            destroy_ms: d.destroy_ms,
            mode: d.mode,
            tap_live: d.tap_live(),
            check: d.check.clone(),
        }
    }
}

/// The Core Audio calls go to one thread, in order (Python: a one-thread executor): a create and
/// a remove never overtake each other.
struct DeviceWorker {
    tx: Mutex<mpsc::Sender<Job>>,
    snap: Arc<Mutex<Snap>>,
    /// the tapped device exists (the player's gate; read on the audio thread)
    live: Arc<AtomicBool>,
}

impl DeviceWorker {
    fn new(dev: StickMicDevice) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let snap = Arc::new(Mutex::new(Snap::of(&dev)));
        let live = dev.live_flag();
        let s = snap.clone();
        std::thread::Builder::new()
            .name("stick-mic".into())
            .spawn(move || {
                let mut dev = dev;
                while let Ok(job) = rx.recv() {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| job(&mut dev)));
                    if r.is_err() {
                        log::error!("stick mic: a Core Audio call panicked");
                    }
                    *s.lock().unwrap() = Snap::of(&dev);
                }
            })
            .expect("the stick-mic thread");
        Self { tx: Mutex::new(tx), snap, live }
    }

    fn post(&self, f: impl FnOnce(&mut StickMicDevice) + Send + 'static) {
        let _ = self.tx.lock().unwrap().send(Box::new(f));
    }

    async fn run<R: Send + 'static>(&self, f: impl FnOnce(&mut StickMicDevice) -> R + Send + 'static) -> Option<R> {
        let (tx, rx) = oneshot::channel();
        self.post(move |d| {
            let _ = tx.send(f(d));
        });
        rx.await.ok()
    }

    /// From outside the runtime (quit, signals): waits at most `timeout`.
    fn run_blocking<R: Send + 'static>(
        &self,
        timeout: Duration,
        f: impl FnOnce(&mut StickMicDevice) -> R + Send + 'static,
    ) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.post(move |d| {
            let _ = tx.send(f(d));
        });
        rx.recv_timeout(timeout).ok()
    }

    fn snap(&self) -> Snap {
        self.snap.lock().unwrap().clone()
    }

    fn set_name(&self, name: &str) {
        self.snap.lock().unwrap().name = name.to_string();
        let n = name.to_string();
        self.post(move |d| d.name = n);
    }

    fn set_mode(&self, mode: Carrier) {
        self.snap.lock().unwrap().mode = mode;
        self.post(move |d| d.mode = mode);
    }
}

// --- the state ----------------------------------------------------------------------------------

/// One hold's timings, for the dashboard (`device_mic.last`).
#[derive(Debug, Clone)]
struct Cur {
    t: f64,
    made: &'static str,
    create_ms: u64,
    lead_ms: u64,
    ready_ms: u64,
    released_early: bool,
    wispr_open_ms: Option<u64>,
    wispr_closed_ms: Option<u64>,
    kept_ms: Option<u64>,
    ready_at: Instant,
    stop_at: Option<Instant>,
}

impl Cur {
    fn to_json(&self) -> Value {
        json!({"t": self.t, "made": self.made, "create_ms": self.create_ms, "lead_ms": self.lead_ms,
               "ready_ms": self.ready_ms, "released_early": self.released_early,
               "wispr_open_ms": self.wispr_open_ms, "wispr_closed_ms": self.wispr_closed_ms, "kept_ms": self.kept_ms})
    }
}

#[derive(Default)]
struct Holds {
    holding: bool,
    hold_id: Option<Value>,
    keep_until: Option<Instant>,
    showing_until: Option<Instant>,
    /// the task that watches the device and removes it
    life: Option<tokio::task::JoinHandle<()>>,
    /// timings of the last hold
    last: Option<Value>,
    cur: Option<Cur>,
    /// the tap check is running
    checking: bool,
}

pub struct WirelessMic {
    audio: Mutex<Audio>,
    player: Mutex<WisprPlayer>,
    dev: DeviceWorker,
    holds: Mutex<Holds>,
    /// the stick is plugged into a computer by USB (it says so; `None` = unknown / older firmware)
    stick_usb: Mutex<Option<bool>>,
    timing: Mutex<Timing>,
    /// where the player's `wispr_mic` reports go (set once the app state exists)
    sink: Arc<OnceLock<Emit>>,
}

fn ms(d: Duration) -> u64 {
    d.as_millis() as u64
}

impl WirelessMic {
    /// With the real audio system (nothing touches Core Audio until the feature is used).
    pub fn new() -> Self {
        Self::with_audio(Audio::system())
    }

    pub fn with_audio(audio: Audio) -> Self {
        let sink: Arc<OnceLock<Emit>> = Arc::new(OnceLock::new());
        let names = Names::rust(macos_audio::naming::NAME);
        let dev = StickMicDevice::new(audio.core_audio.clone(), &names, std::process::id() as i32);
        Self {
            player: Mutex::new(WisprPlayer::new(audio.outputs.clone(), forward(&sink))),
            audio: Mutex::new(audio),
            dev: DeviceWorker::new(dev),
            holds: Mutex::new(Holds::default()),
            stick_usb: Mutex::new(None),
            timing: Mutex::new(Timing::default()),
            sink,
        }
    }

    /// Another audio system (tests: the fakes). Before the feature is turned on.
    pub fn use_audio(&self, audio: Audio) {
        *self.player.lock().unwrap() = WisprPlayer::new(audio.outputs.clone(), forward(&self.sink));
        let f = audio.core_audio.clone();
        self.dev.post(move |d| d.set_factory(f));
        *self.audio.lock().unwrap() = audio;
    }

    pub fn set_timing(&self, t: Timing) {
        *self.timing.lock().unwrap() = t;
    }

    /// Between the stick's `prepare` / `start` and `stop`.
    pub fn streaming(&self) -> bool {
        self.player.lock().unwrap().active()
    }

    pub fn device(&self) -> Snap {
        self.dev.snap()
    }

    pub fn stick_usb(&self) -> Option<bool> {
        *self.stick_usb.lock().unwrap()
    }

    fn tap_supported(&self) -> bool {
        self.audio.lock().unwrap().tap_supported
    }

    /// The player, for a look (tests).
    pub fn with_player<R>(&self, f: impl FnOnce(&mut WisprPlayer) -> R) -> R {
        f(&mut self.player.lock().unwrap())
    }
}

impl Default for WirelessMic {
    fn default() -> Self {
        Self::new()
    }
}

/// Runs `f` on the runtime if there is one (settings can change from outside it: then the next
/// hold / hello catches up).
fn go(f: impl std::future::Future<Output = ()> + Send + 'static) {
    match tokio::runtime::Handle::try_current() {
        Ok(h) => {
            h.spawn(f);
        }
        Err(_) => log::debug!("wireless mic: no runtime for a background step"),
    }
}

fn forward(sink: &Arc<OnceLock<Emit>>) -> Emit {
    let sink = sink.clone();
    Arc::new(move |v| {
        if let Some(e) = sink.get() {
            e(v);
        }
    })
}

// --- settings -----------------------------------------------------------------------------------

fn cfg(st: &AppState, key: &str) -> Value {
    st.cfg.lock().unwrap().get(key).cloned().unwrap_or(Value::Null)
}

/// `(lead ms, tail ms)`: `wispr_switch_ms` (0..2000) and `wispr_tail_ms` (0..5000), defaults on
/// anything that isn't a number.
pub fn settings(st: &AppState) -> (u64, u64) {
    let get = |key: &str, default: u64, hi: u64| match cfg(st, key) {
        Value::Number(n) => n.as_f64().map(|x| (x.max(0.0) as u64).min(hi)).unwrap_or(default),
        Value::String(s) => s.trim().parse::<i64>().map(|x| (x.max(0) as u64).min(hi)).unwrap_or(default),
        _ => default,
    };
    (get("wispr_switch_ms", LEAD_MS, 2000), get("wispr_tail_ms", TAIL_MS, 5000))
}

fn carrier_pref(st: &AppState) -> String {
    let p = cfg(st, "wireless_mic_carrier");
    let p = p.as_str().unwrap_or("auto");
    if macos_audio::carrier::PREFS.contains(&p) { p.to_string() } else { "auto".to_string() }
}

/// The tap may be used (and so is worth checking): macOS 14.2+ and not set to BlackHole.
fn tap_wanted(st: &AppState) -> bool {
    st.wmic.tap_supported() && carrier_pref(st) != "blackhole"
}

/// `wispr_mic_name` if typed in, else named after the picked stick ("stickybot-bt-mic").
pub fn mic_name(st: &AppState) -> String {
    let typed = cfg(st, "wispr_mic_name");
    let typed = typed.as_str().unwrap_or("").trim();
    if !typed.is_empty() && !is_auto_name(typed) {
        return typed.to_string();
    }
    let stick = cfg(st, "stick");
    name_for(stick.get("name").and_then(Value::as_str).unwrap_or(""))
}

// --- the carrier --------------------------------------------------------------------------------

/// Picks the carrier from the setting, the macOS version, the tap check and BlackHole; a change
/// removes a device made with the old one.
pub fn apply_carrier(st: &Arc<AppState>) -> Option<Carrier> {
    let w = &st.wmic;
    let snap = w.dev.snap();
    let state = if w.holds.lock().unwrap().checking { Some(CheckState::Checking) } else { snap.check.state };
    let bh = w.player.lock().unwrap().has_blackhole();
    let (carrier, why) = pick_carrier(&carrier_pref(st), w.tap_supported(), state, bh);
    let old = snap.mode;
    if let Some(c) = carrier {
        w.dev.set_mode(c);
    }
    let gate: Option<Gate> = (carrier == Some(Carrier::Tap)).then(|| {
        let live = w.dev.live.clone();
        Arc::new(move || live.load(Ordering::SeqCst)) as Gate
    });
    w.player.lock().unwrap().set_carrier(carrier, &why, gate);
    if let Some(c) = carrier
        && c != old
        && snap.present
    {
        go(drop_device(st.clone(), format!("now carried by {}", c.title())));
    }
    carrier
}

/// Runs the tap check (inaudible; never during a hold), then picks the carrier again and tells the
/// stick whether to stream. -> the check's result, or `None` when it didn't run.
pub async fn check_tap(st: &Arc<AppState>, force: bool) -> Option<Check> {
    if !force && !tap_wanted(st) {
        return None;
    }
    if !st.wmic.tap_supported() {
        return None;
    }
    let res = check(st).await;
    if st.wmic.player.lock().unwrap().enabled {
        apply_carrier(st);
        if st.link.connected() {
            let ready = {
                let mut p = st.wmic.player.lock().unwrap();
                p.warm();
                p.ready()
            };
            st.link.send(&json!({"src": "agent", "wmic": ready})).await;
        }
    }
    st.hub.emit("wispr_mic", status(st));
    res
}

/// The device for `secs` from the dashboard (the ranking step); tap mode: our output stream plays
/// silence through it. -> "" or why not.
pub async fn show_mic(st: &Arc<AppState>, secs: u64) -> String {
    {
        let mut p = st.wmic.player.lock().unwrap();
        if p.enabled && p.carrier.is_some() {
            p.open();
        }
    }
    show(st, Duration::from_secs(secs)).await
}

fn rename_mic(st: &Arc<AppState>, why: &str) {
    let name = mic_name(st);
    if name != st.wmic.dev.snap().name {
        st.wmic.dev.set_name(&name);
        go(drop_device(st.clone(), why.to_string()));
    }
}

/// "" if the Bluetooth mic device may exist now, else why not.
pub fn unavailable(st: &AppState) -> String {
    {
        let p = st.wmic.player.lock().unwrap();
        if !p.enabled {
            return "the wireless Wispr mic is off".into();
        }
        if p.carrier.is_none() {
            return if p.unavailable.is_empty() { "nothing can carry the audio".into() } else { p.unavailable.clone() };
        }
    }
    if !st.link.connected() {
        return "the stick isn't connected by Bluetooth".into();
    }
    if st.wmic.stick_usb() == Some(true) {
        return "the stick is plugged in by USB: Wispr uses its USB mic".into();
    }
    String::new()
}

/// `/api/status` `wireless_mic` (core.py `wireless_status`).
pub fn status(st: &AppState) -> Value {
    let w = &st.wmic;
    let mut v = w.player.lock().unwrap().status();
    let snap = w.dev.snap();
    let checking = w.holds.lock().unwrap().checking;
    let carrier = w.player.lock().unwrap().carrier;
    let c = &snap.check;
    let extra = json!({
        "device_mic": holds_status(st),
        "stick_usb": w.stick_usb(),
        "unavailable": unavailable(st),
        "carrier_pref": carrier_pref(st),
        "carrier_title": carrier.map(Carrier::title),
        "uid": RUST_UID,
        "tap": {"supported": w.tap_supported(),
                "state": if checking { json!("checking") } else { json!(c.state) },
                "detail": c.detail, "at": c.at, "create_ms": c.create_ms, "first_ms": c.first_ms, "ms": c.ms,
                "hint": PERMISSION_HINT},
    });
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    v
}

fn holds_status(st: &AppState) -> Value {
    let snap = st.wmic.dev.snap();
    let h = st.wmic.holds.lock().unwrap();
    let now = Instant::now();
    let showing = h.showing_until.filter(|t| *t > now).map(|t| (t - now).as_secs_f64().round() as u64).unwrap_or(0);
    let mut check = snap.check.to_json();
    check["running"] = json!(h.checking);
    json!({"name": snap.name, "present": snap.present, "error": snap.error, "create_ms": snap.create_ms,
           "destroy_ms": snap.destroy_ms, "showing_s": showing, "holding": h.holding, "last": h.last,
           "carrier": snap.mode, "tap_live": snap.tap_live, "check": check})
}

fn report(st: &AppState) {
    st.hub.emit("stick_mic", holds_status(st));
}

// --- the hold flow (Holds) ----------------------------------------------------------------------

async fn ready(st: &AppState, hold_id: &Option<Value>, ok: bool) {
    let mut msg = json!({"src": "agent", "wispr": "ready"});
    if let Some(id) = hold_id {
        msg["id"] = id.clone();
    }
    if !ok {
        msg["device"] = json!(false);
    }
    st.link.send(&msg).await;
}

/// A went down for Wispr: make the device, give Wispr the lead time, then say "ready".
pub async fn prepare(st: Arc<AppState>, hold_id: Option<Value>) -> bool {
    let t0 = Instant::now();
    {
        let mut h = st.wmic.holds.lock().unwrap();
        h.holding = true;
        h.hold_id = hold_id.clone();
    }
    let why = unavailable(&st);
    if !why.is_empty() {
        log::info!("stick mic: not made ({why})");
        ready(&st, &hold_id, false).await;
        return false;
    }
    let made = st.wmic.dev.run(|d| d.ensure_present()).await.unwrap_or(Made::No);
    let create_ms = ms(t0.elapsed());
    let mut lead = 0;
    if made == Made::Created {
        lead = settings(&st).0;
        tokio::time::sleep(Duration::from_millis(lead)).await;
    }
    if made.ok() {
        watch(&st);
    }
    let current = {
        let h = st.wmic.holds.lock().unwrap();
        h.hold_id == hold_id && h.holding
    };
    if current {
        ready(&st, &hold_id, made.ok()).await;
    }
    let ready_ms = ms(t0.elapsed());
    let cur = Cur {
        t: crate::hub::now(),
        made: made.as_str(),
        create_ms: if made == Made::Created { create_ms } else { 0 },
        lead_ms: lead,
        ready_ms,
        released_early: !current,
        wispr_open_ms: None,
        wispr_closed_ms: None,
        kept_ms: None,
        ready_at: Instant::now(),
        stop_at: if current { None } else { Some(Instant::now()) },
    };
    log::info!(
        "stick mic: {} {ready_ms} ms after prepare (device {}, create {} ms, lead {lead} ms)",
        if current { "ready" } else { "released before ready," },
        made.as_str(),
        cur.create_ms
    );
    {
        let mut h = st.wmic.holds.lock().unwrap();
        if !made.ok() {
            h.last = Some(cur.to_json());
        }
        h.cur = Some(cur);
    }
    report(&st);
    made.ok()
}

/// The stream ended (key up): keep the device for the tail, then remove it.
pub fn stop_hold(st: &AppState) {
    let tail = Duration::from_millis(settings(st).1);
    let mut h = st.wmic.holds.lock().unwrap();
    if !h.holding {
        return;
    }
    h.holding = false;
    let until = Instant::now() + tail;
    h.keep_until = Some(h.keep_until.map_or(until, |k| k.max(until)));
    if let Some(cur) = h.cur.as_mut()
        && cur.stop_at.is_none()
    {
        cur.stop_at = Some(Instant::now());
    }
}

/// The device for `secs`, to rank it in Wispr once. -> "" or why not.
pub async fn show(st: &Arc<AppState>, secs: Duration) -> String {
    let why = unavailable(st);
    if !why.is_empty() {
        return why;
    }
    if !st.wmic.dev.run(|d| d.ensure_present()).await.unwrap_or(Made::No).ok() {
        let e = st.wmic.dev.snap().error;
        return if e.is_empty() { "couldn't create it".into() } else { e };
    }
    {
        let mut h = st.wmic.holds.lock().unwrap();
        let until = Instant::now() + secs;
        h.showing_until = Some(until);
        h.keep_until = Some(h.keep_until.map_or(until, |k| k.max(until)));
    }
    watch(st);
    report(st);
    String::new()
}

fn watch(st: &Arc<AppState>) {
    let mut h = st.wmic.holds.lock().unwrap();
    if h.life.as_ref().is_none_or(|l| l.is_finished()) {
        h.life = Some(tokio::spawn(lifetime(st.clone())));
    }
}

async fn lifetime(st: Arc<AppState>) {
    let mut reader_since: Option<Instant> = None;
    let mut was_reading = false;
    loop {
        let timing = *st.wmic.timing.lock().unwrap();
        tokio::time::sleep(timing.poll).await;
        if !st.wmic.dev.snap().present {
            break;
        }
        let reading = st.wmic.dev.run(|d| d.recording()).await.flatten() == Some(true);
        let now = Instant::now();
        let keep = {
            let mut h = st.wmic.holds.lock().unwrap();
            if let Some(cur) = h.cur.as_mut() {
                if reading && !was_reading && cur.wispr_open_ms.is_none() {
                    cur.wispr_open_ms = Some(ms(now - cur.ready_at));
                    log::info!("stick mic: Wispr opened it {} ms after ready", ms(now - cur.ready_at));
                }
                if was_reading
                    && !reading
                    && let Some(stop) = cur.stop_at
                {
                    cur.wispr_closed_ms = Some(ms(now.saturating_duration_since(stop)));
                    log::info!("stick mic: Wispr closed it {} ms after the stream ended", ms(now - stop));
                }
            }
            h.holding || h.keep_until.is_some_and(|k| now < k)
        };
        was_reading = reading;
        if keep {
            reader_since = None;
            continue;
        }
        if reading {
            // still dictating: removing it now would stop Wispr with "MicDisconnected"
            let since = *reader_since.get_or_insert(now);
            if now - since < timing.wait_reader {
                continue;
            }
            log::warn!(
                "stick mic: Wispr still reads it {:.1} s after the tail: removing it anyway",
                timing.wait_reader.as_secs_f64()
            );
        }
        st.wmic.dev.run(|d| d.remove()).await;
        st.wmic.player.lock().unwrap().release();
        let mut h = st.wmic.holds.lock().unwrap();
        if let Some(cur) = h.cur.as_mut()
            && let Some(stop) = cur.stop_at
        {
            cur.kept_ms = Some(ms(stop.elapsed()));
            log::info!("stick mic: removed {} ms after the stream ended", ms(stop.elapsed()));
        }
        break;
    }
    {
        let mut h = st.wmic.holds.lock().unwrap();
        h.showing_until = None;
        if let Some(cur) = &h.cur {
            h.last = Some(cur.to_json());
        }
    }
    report(&st);
}

/// Remove it now (the stick went to USB / disconnected, the feature was turned off, renamed).
pub async fn drop_device(st: Arc<AppState>, why: String) {
    {
        let mut h = st.wmic.holds.lock().unwrap();
        h.holding = false;
        h.keep_until = None;
        h.showing_until = None;
        if let Some(l) = h.life.take() {
            l.abort();
        }
    }
    // only what this app made (left-overs of a crashed run go at the next start): a link change
    // never touches Core Audio while the feature is off
    if !st.wmic.dev.snap().present {
        st.wmic.player.lock().unwrap().release();
        return;
    }
    let n = st.wmic.dev.run(|d| d.remove()).await.unwrap_or(0);
    st.wmic.player.lock().unwrap().release();
    if n > 0 {
        log::info!("stick mic: removed now ({why})");
        report(&st);
    }
}

/// The tap check, in turn with the device calls; never during a hold or while the device is there
/// (the check mutes this app for its ~0.2-1.5 s). -> the result, or `None` when it can't run now.
pub async fn check(st: &Arc<AppState>) -> Option<Check> {
    {
        let mut h = st.wmic.holds.lock().unwrap();
        let busy = h.holding || h.keep_until.is_some_and(|k| Instant::now() < k);
        if busy || st.wmic.dev.snap().present {
            return None;
        }
        h.checking = true;
    }
    report(st);
    let res = st.wmic.dev.run(|d| d.probe(PROBE_S, PROBE_AMP)).await;
    st.wmic.holds.lock().unwrap().checking = false;
    report(st);
    res
}

/// Quit / Ctrl-C (synchronous, any thread outside the runtime's workers): never leave the device
/// behind (it outlives us otherwise). Only the Rust UID's objects.
pub fn shutdown(st: &AppState) {
    st.wmic.holds.lock().unwrap().holding = false;
    if let Ok(mut p) = st.wmic.player.lock() {
        p.stop();
        p.close();
    }
    if st.wmic.dev.run_blocking(Duration::from_secs(3), |d| d.cleanup_stale()).is_none() {
        log::warn!("stick mic: cleanup on quit didn't finish in 3 s");
    }
}

/// After a crash: only the device / tap removal (our UID), tolerating poisoned locks; waits at
/// most `timeout` for the device's thread.
pub fn crash_cleanup(st: &AppState, timeout: Duration) -> Option<usize> {
    let (tx, rx) = std::sync::mpsc::channel();
    let job: Job = Box::new(move |d| {
        let _ = tx.send(d.cleanup_stale());
    });
    let sent = match st.wmic.dev.tx.lock() {
        Ok(q) => q.send(job).is_ok(),
        Err(poisoned) => poisoned.into_inner().send(job).is_ok(),
    };
    if !sent {
        return None;
    }
    rx.recv_timeout(timeout).ok()
}

/// A panic that ends the app (one on the main thread: the Tauri event loop, the headless
/// `main`) removes the device first; panics in tasks and the device's own thread don't end it
/// and are left alone. SIGTERM / SIGHUP (logout, `kill`) clean up and exit. `kill -9` can't be
/// caught: the next start removes the left-overs ([`startup`]).
pub fn install_crash_cleanup(st: &Arc<AppState>) {
    let weak = Arc::downgrade(st);
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        prev(info);
        if std::thread::current().name() != Some("main") {
            return;
        }
        if let Some(st) = weak.upgrade() {
            match crash_cleanup(&st, Duration::from_secs(2)) {
                Some(n) => log::error!("crashed: removed the stick mic ({n} device(s))"),
                None => log::error!("crashed: couldn't remove the stick mic (the next start will)"),
            }
        }
    }));
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let weak = Arc::downgrade(st);
        tokio::spawn(async move {
            let (Ok(mut term), Ok(mut hup)) = (signal(SignalKind::terminate()), signal(SignalKind::hangup())) else {
                return;
            };
            let what = tokio::select! {
                _ = term.recv() => "SIGTERM",
                _ = hup.recv() => "SIGHUP",
            };
            log::info!("{what}: removing the stick mic, then quitting");
            if let Some(st) = weak.upgrade() {
                let _ = tokio::task::spawn_blocking(move || st.shutdown()).await;
            }
            std::process::exit(0);
        });
    }
}

// --- the stick's messages ------------------------------------------------------------------------

/// `wmic` / `wready` for the hello: may the stick stream (on, with a carrier and somewhere to
/// play), and we answer `prepare` with `ready`.
pub fn hello_extra(st: &AppState) -> Map<String, Value> {
    let ready = st.wmic.player.lock().unwrap().ready();
    let mut m = Map::new();
    m.insert("wmic".into(), json!(ready));
    m.insert("wready".into(), json!(true));
    m
}

/// core.py `__init__`: the player follows the setting, the device the stick's name.
pub fn init(st: &Arc<AppState>) {
    let weak = Arc::downgrade(st);
    let _ = st.wmic.sink.set(Arc::new(move |v| {
        if let Some(st) = weak.upgrade() {
            st.hub.emit("wispr_mic", v);
        }
    }));
    let on = cfg(st, "wireless_mic").as_bool().unwrap_or(false);
    st.wmic.player.lock().unwrap().set_enabled(on);
    st.wmic.dev.set_name(&mic_name(st));
    if on {
        apply_carrier(st);
    }
    let weak = Arc::downgrade(st);
    // the whole hello extra (core.py `_hello_extra`): home, world, wx (home.rs) + wmic, wready
    st.link.set_hello_extra(move || {
        weak.upgrade()
            .map(|st| {
                let mut m = crate::home::hello_extra(&st);
                m.extend(hello_extra(&st));
                // we read the last reply aloud when the stick asks ({"evt":"speak"}, firmware 2.6.0)
                m.insert("speak".into(), json!(true));
                m
            })
            .unwrap_or_default()
    });
}

/// At start: what a crashed run left behind goes (our UID only), then the tap check (inaudible)
/// if it may be used.
pub async fn startup(st: Arc<AppState>) {
    let n = st.wmic.dev.run(|d| d.cleanup_stale()).await.unwrap_or(0);
    if n > 0 {
        log::info!("stick mic: removed {n} left-over device(s)");
    }
    if st.wmic.player.lock().unwrap().enabled && tap_wanted(&st) {
        check_tap(&st, false).await;
    }
}

/// Settings changed (core.py `update_config`).
pub fn config_changed(st: &Arc<AppState>, changes: &Map<String, Value>) {
    if changes.contains_key("stick") || changes.contains_key("wispr_mic_name") {
        rename_mic(st, "renamed: made again under the new name next time");
    }
    if changes.contains_key("wireless_mic") || changes.contains_key("wireless_mic_carrier") {
        let on = cfg(st, "wireless_mic").as_bool().unwrap_or(false);
        st.wmic.player.lock().unwrap().set_enabled(on);
        if !on {
            go(drop_device(st.clone(), "turned off".into()));
        } else {
            apply_carrier(st);
            if tap_wanted(st) && st.wmic.dev.snap().check.state != Some(CheckState::Ok) {
                // the first one makes macOS ask for the permission
                let st2 = st.clone();
                go(async move {
                    check_tap(&st2, false).await;
                });
            }
        }
        if st.link.connected() {
            let ready = {
                let mut p = st.wmic.player.lock().unwrap();
                p.warm();
                p.ready()
            };
            let st2 = st.clone();
            go(async move {
                st2.link.send(&json!({"src": "agent", "wmic": ready})).await;
            });
        }
    }
}

/// A `@state` reply (core.py `_note_name`): the stick may have been renamed on the stick itself;
/// remember its name (the mic is named after it) without re-targeting the link.
pub fn note_name(st: &Arc<AppState>, reply: &Value) {
    let Some(name) = reply.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()) else { return };
    if reply.get("board").is_none_or(Value::is_null) {
        return;
    }
    let snapshot = {
        let mut cfg = st.cfg.lock().unwrap();
        let Some(mut stick) = cfg.get("stick").and_then(Value::as_object).cloned() else { return };
        if stick.get("name").and_then(Value::as_str) == Some(name) {
            return;
        }
        let was = stick.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        log::info!("the stick is now called {name:?} (was {was:?})");
        stick.insert("name".into(), json!(name));
        cfg.insert("stick".into(), Value::Object(stick));
        if let Err(e) = crate::config::save(&st.paths, &cfg) {
            log::warn!("couldn't save the stick's new name: {e}");
        }
        cfg.clone()
    };
    st.hub.emit("config", json!({"config": snapshot}));
    rename_mic(st, "the stick was renamed");
}

/// A message from the stick (core.py `_on_message`, the wispr / usb parts).
pub fn on_message(st: &Arc<AppState>, msg: &Value) {
    if msg.get("src").and_then(Value::as_str) != Some("agent") {
        return;
    }
    if msg.get("evt").and_then(Value::as_str) == Some("proto")
        && let Some(reply) = msg.get("reply").filter(|r| r.is_object())
    {
        note_name(st, reply); // any @state reply (ours on connect, the dashboard's)
        return;
    }
    let rate = msg.get("rate").and_then(Value::as_u64).filter(|r| *r > 0).unwrap_or(RATE as u64) as u32;
    match (msg.get("evt").and_then(Value::as_str), msg.get("state").and_then(Value::as_str)) {
        // 1.7.1+: the key waits for our "ready"; the output stream is open before it
        (Some("wispr"), Some("prepare")) => {
            st.wmic.player.lock().unwrap().start(rate);
            tokio::spawn(prepare(st.clone(), msg.get("id").cloned()));
        }
        // 1.7.0: the key is already down, too late for the device
        (Some("wispr"), Some("start")) => {
            st.wmic.player.lock().unwrap().start(rate);
        }
        (Some("wispr"), Some("stop")) => {
            st.wmic.player.lock().unwrap().stop();
            stop_hold(st);
        }
        (Some("usb"), _) => {
            // plugged into / out of a computer by USB
            let usb = msg.get("mounted").and_then(Value::as_bool).unwrap_or(false);
            let was = st.wmic.stick_usb.lock().unwrap().replace(usb);
            if was != Some(usb) {
                log::info!("the stick is {} USB", if usb { "on" } else { "off" });
            }
            if usb {
                tokio::spawn(drop_device(st.clone(), "the stick is on USB".into()));
            }
            st.hub.emit("wispr_mic", status(st));
        }
        _ => {}
    }
}

/// An audio frame: the Wispr stream's while one is on (else it's a talk's).
pub fn on_audio(st: &AppState, frame: &[u8]) {
    let mut p = st.wmic.player.lock().unwrap();
    if p.active()
        && frame.len() > stick_link::adpcm::HEADER
        && let Some((seq, samples)) = stick_link::adpcm::decode_frame(frame)
    {
        p.add(seq, samples);
    }
}

/// The link changed (core.py `_on_link`).
pub fn on_link(st: &Arc<AppState>, state: LinkState) {
    if state == LinkState::Connected {
        st.wmic.player.lock().unwrap().warm(); // ready before the first key-down
        // its name now (renamed on the stick?): the reply comes back through on_message -> note_name
        let st2 = st.clone();
        tokio::spawn(async move {
            st2.link.request("@state").await;
        });
    } else {
        {
            let mut p = st.wmic.player.lock().unwrap();
            p.stop();
            p.close();
        }
        *st.wmic.stick_usb.lock().unwrap() = None;
        tokio::spawn(drop_device(st.clone(), "the stick disconnected".into()));
    }
}

/// Listens to the link for Wispr holds, USB changes and the link's state. Runs for the life of the app.
pub fn spawn(st: Arc<AppState>) -> tokio::task::JoinHandle<()> {
    let mut rx = st.link.incoming();
    let mut w = st.link.watch_state();
    tokio::spawn(async move {
        let mut last = *w.borrow_and_update();
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Ok(FromStick::Audio(frame)) => on_audio(&st, &frame),
                    Ok(FromStick::Message(msg)) => on_message(&st, &msg),
                    Err(broadcast::error::RecvError::Lagged(n)) => log::warn!("wireless mic: fell behind the stick by {n}"),
                    Err(_) => return,
                },
                r = w.changed() => {
                    if r.is_err() {
                        return;
                    }
                    let s = *w.borrow_and_update();
                    if s != last {
                        // connected <-> anything else matters; searching -> starting doesn't
                        if s == LinkState::Connected || last == LinkState::Connected {
                            on_link(&st, s);
                        }
                        last = s;
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    //! test_stick_mic_device.py, test_wispr_mic.py (the agent's side) and test_tap_mic.py (the agent
    //! in tap mode), against the fake stick and the fake Core Audio / outputs. No real audio.
    use super::*;
    use crate::Options;
    use macos_audio::fake::{BLACKHOLE_ID, FakeCoreAudio, FakeOutputs};
    use macos_audio::naming::RUST_UID;
    use stick_link::fake::FakeStick;
    use stick_link::{Backend, adpcm};

    struct T {
        st: Arc<AppState>,
        fake: FakeStick,
        ca: FakeCoreAudio,
        out: FakeOutputs,
    }

    impl T {
        fn ours(&self) -> Vec<u32> {
            self.ca.ours(RUST_UID)
        }
        fn readies(&self) -> Vec<Value> {
            self.fake.wispr_readies()
        }
        fn set(&self, v: Value) {
            self.st.update_config(v.as_object().unwrap().clone()).unwrap();
        }
        fn last(&self) -> Value {
            self.st.wmic.holds.lock().unwrap().last.clone().unwrap_or(Value::Null)
        }
        fn cur(&self) -> Option<Cur> {
            self.st.wmic.holds.lock().unwrap().cur.clone()
        }
    }

    async fn wait_for(mut cond: impl FnMut() -> bool) -> bool {
        for _ in 0..400 {
            if cond() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        cond()
    }

    /// The app with the fake stick connected; `outs`: the output devices (the first is the default
    /// output); `tap`: this Mac has process taps.
    async fn app(name: &str, outs: Vec<macos_audio::player::OutDevice>, tap: bool) -> T {
        let home = std::env::temp_dir().join(format!("stickybot-rs-wmic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let fake = FakeStick::new();
        fake.set_name("StickyBot"); // as picked below (else the @state on connect renames it)
        let st = AppState::new(&Options { port: 8670, home, fake_stick: true, import_from: None }, Backend::Fake(fake.clone())).unwrap();
        let ca = FakeCoreAudio::new();
        let out = FakeOutputs::new(outs);
        st.wmic.use_audio(Audio::fake(&ca, &out, tap));
        st.wmic.set_timing(Timing { poll: Duration::from_millis(5), wait_reader: Duration::from_millis(200) });
        st.link.spawn();
        spawn(st.clone());
        let t = T { st, fake, ca, out };
        t.set(json!({"stick": {"address": "FAKE", "name": "StickyBot"}, "wispr_switch_ms": 10, "wispr_tail_ms": 30}));
        let mut w = t.st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected))
            .await
            .unwrap()
            .unwrap();
        t
    }

    /// On, carried by BlackHole (test_stick_mic_device.py `wired`).
    async fn wired(name: &str) -> T {
        let t = app(name, vec![FakeOutputs::speakers(), FakeOutputs::blackhole()], false).await;
        t.set(json!({"wireless_mic": true}));
        t
    }

    /// A Mac with process taps, default output the speakers, no BlackHole (test_tap_mic.py `tap_agent`).
    async fn tap_agent(name: &str) -> T {
        app(name, vec![FakeOutputs::speakers()], true).await
    }

    fn tone(secs: f64) -> Vec<i16> {
        (0..(16000.0 * secs) as usize)
            .map(|i| (8000.0 * (std::f64::consts::TAU * 440.0 * i as f64 / 16000.0).sin()) as i16)
            .collect()
    }

    fn prepare_msg(id: u64) -> Value {
        json!({"src": "agent", "evt": "wispr", "state": "prepare", "id": id, "rate": 16000, "codec": "ima-adpcm"})
    }

    fn stop_msg() -> Value {
        json!({"src": "agent", "evt": "wispr", "state": "stop"})
    }

    // --- the hold flow ---

    #[tokio::test]
    async fn prepare_creates_waits_the_lead_then_says_ready() {
        let t = wired("lead").await;
        t.set(json!({"wispr_switch_ms": 60}));
        let t0 = Instant::now();
        assert!(prepare(t.st.clone(), Some(json!(7))).await);
        assert!(t0.elapsed() >= Duration::from_millis(60));
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 7})]);
        assert_eq!(t.ours().len(), 1);
        let cur = t.cur().unwrap();
        assert_eq!((cur.lead_ms, cur.made), (60, "created"));
        assert!(t.last().is_null());
        drop_device(t.st.clone(), "test".into()).await;
    }

    #[tokio::test]
    async fn device_stays_for_the_tail_then_goes() {
        let t = wired("tail").await;
        t.set(json!({"wispr_switch_ms": 0, "wispr_tail_ms": 80}));
        prepare(t.st.clone(), Some(json!(1))).await;
        stop_hold(&t.st);
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!t.ours().is_empty()); // still there: Wispr is finishing
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert!(wait_for(|| !t.last().is_null()).await);
        assert!(t.last()["kept_ms"].as_u64().unwrap() >= 80);
    }

    #[tokio::test]
    async fn a_hold_during_the_tail_keeps_it_and_is_ready_at_once() {
        let t = wired("again").await;
        t.set(json!({"wispr_switch_ms": 200, "wispr_tail_ms": 60}));
        prepare(t.st.clone(), Some(json!(1))).await;
        stop_hold(&t.st);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let t0 = Instant::now();
        prepare(t.st.clone(), Some(json!(2))).await; // already there: Wispr knows it, no lead time
        assert!(t0.elapsed() < Duration::from_millis(100));
        assert_eq!(t.readies().last().unwrap(), &json!({"src": "agent", "wispr": "ready", "id": 2}));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!((t.ours().len(), t.ca.created().len()), (1, 1)); // held: never removed in between
        stop_hold(&t.st);
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert_eq!(t.ca.created().len(), 1);
    }

    #[tokio::test]
    async fn removal_waits_while_wispr_still_reads_it() {
        let t = wired("reader").await;
        t.set(json!({"wispr_switch_ms": 0, "wispr_tail_ms": 20}));
        prepare(t.st.clone(), Some(json!(1))).await;
        t.ca.set(|s| s.readers.insert(BLACKHOLE_ID, vec![4242])); // Wispr dictating
        tokio::time::sleep(Duration::from_millis(30)).await;
        stop_hold(&t.st);
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!t.ours().is_empty()); // tail over, Wispr still reading: not "MicDisconnected"
        t.ca.set(|s| s.readers.insert(BLACKHOLE_ID, vec![]));
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert!(wait_for(|| !t.last().is_null()).await);
        assert!(t.last()["wispr_open_ms"].is_u64() && t.last()["wispr_closed_ms"].is_u64(), "{}", t.last());
    }

    #[tokio::test]
    async fn removal_gives_up_waiting_eventually() {
        let t = wired("giveup").await;
        t.set(json!({"wispr_switch_ms": 0, "wispr_tail_ms": 0}));
        prepare(t.st.clone(), Some(json!(1))).await;
        t.ca.set(|s| s.readers.insert(BLACKHOLE_ID, vec![4242])); // a reader that never stops
        stop_hold(&t.st);
        assert!(wait_for(|| t.ours().is_empty()).await);
    }

    #[tokio::test]
    async fn released_before_ready_sends_nothing_and_still_removes_it() {
        let t = wired("early").await;
        t.set(json!({"wispr_switch_ms": 80, "wispr_tail_ms": 20}));
        let task = tokio::spawn(prepare(t.st.clone(), Some(json!(3))));
        tokio::time::sleep(Duration::from_millis(20)).await;
        stop_hold(&t.st); // the stick cancelled: A released before ready
        task.await.unwrap();
        assert!(t.readies().is_empty());
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert!(wait_for(|| t.last()["released_early"] == true).await, "{}", t.last());
    }

    #[tokio::test]
    async fn a_stale_ready_is_not_sent_for_a_newer_hold() {
        let t = wired("stale").await;
        t.set(json!({"wispr_switch_ms": 80, "wispr_tail_ms": 500}));
        let first = tokio::spawn(prepare(t.st.clone(), Some(json!(1))));
        tokio::time::sleep(Duration::from_millis(10)).await;
        stop_hold(&t.st);
        prepare(t.st.clone(), Some(json!(2))).await; // created by the first: no lead
        first.await.unwrap();
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 2})]);
        drop_device(t.st.clone(), "test".into()).await;
    }

    #[tokio::test]
    async fn not_allowed_answers_at_once_without_a_device() {
        let t = wired("usbnow").await;
        *t.st.wmic.stick_usb.lock().unwrap() = Some(true);
        assert!(!prepare(t.st.clone(), Some(json!(5))).await);
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 5, "device": false})]);
        assert!(t.ca.created().is_empty());
    }

    #[tokio::test]
    async fn no_blackhole_answers_ready_without_a_device() {
        let t = wired("nobh").await;
        t.ca.remove_device(BLACKHOLE_ID); // the player still finds an output, Core Audio no BlackHole
        assert!(!prepare(t.st.clone(), Some(json!(5))).await);
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 5, "device": false})]);
        assert_eq!(t.last()["made"], "failed");
    }

    #[tokio::test]
    async fn show_keeps_it_for_the_ranking_step() {
        let t = wired("show").await;
        t.set(json!({"wispr_tail_ms": 0}));
        assert_eq!(show(&t.st, Duration::from_millis(150)).await, "");
        assert!(holds_status(&t.st)["present"] == true && !t.ours().is_empty());
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!t.ours().is_empty());
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert_eq!(holds_status(&t.st)["showing_s"], 0);
    }

    #[tokio::test]
    async fn show_refused_when_not_allowed() {
        let t = app("shownot", vec![FakeOutputs::blackhole()], false).await; // the feature is off
        assert!(show(&t.st, Duration::from_secs(60)).await.contains("off"));
        t.set(json!({"wireless_mic": true}));
        t.st.link.set_target(None);
        assert!(wait_for(|| !t.st.link.connected()).await);
        assert!(show(&t.st, Duration::from_secs(60)).await.contains("connected"));
        assert!(t.ca.created().is_empty());
    }

    #[tokio::test]
    async fn drop_removes_it_now() {
        let t = wired("drop").await;
        t.set(json!({"wispr_tail_ms": 5000}));
        prepare(t.st.clone(), Some(json!(1))).await;
        stop_hold(&t.st);
        drop_device(t.st.clone(), "the stick is on USB".into()).await;
        assert!(t.ours().is_empty() && holds_status(&t.st)["present"] == false);
    }

    #[tokio::test]
    async fn shutdown_is_synchronous_and_removes_leftovers() {
        let t = wired("shutdown").await;
        t.ca.add_device(250, "stickybot-bt-mic", RUST_UID);
        t.ca.add_device(251, "stickybot-bt-mic", macos_audio::naming::UID); // the Python agent's
        let st = t.st.clone();
        tokio::task::spawn_blocking(move || st.shutdown()).await.unwrap();
        assert!(t.ours().is_empty() && t.ca.has_device(251));
    }

    #[tokio::test]
    async fn startup_removes_only_our_leftovers() {
        let t = wired("startup").await;
        t.ca.add_device(250, "stickybot-bt-mic", RUST_UID);
        t.ca.add_device(251, "stickybot-bt-mic", macos_audio::naming::UID);
        t.ca.add_tap(60, &format!("{RUST_UID}.tap"));
        t.ca.add_tap(61, "com.stickkey.wireless-mic.tap");
        startup(t.st.clone()).await;
        assert!(t.ours().is_empty() && t.ca.has_device(251));
        assert_eq!(t.ca.taps_now().iter().map(|x| x.id).collect::<Vec<_>>(), vec![61]);
    }

    // --- in the app ---

    #[tokio::test]
    async fn config_defaults() {
        let d = crate::config::defaults();
        assert_eq!((d["wireless_mic"].clone(), d["wireless_mic_carrier"].clone()), (json!(false), json!("auto")));
        assert_eq!((d["wispr_switch_ms"].clone(), d["wispr_tail_ms"].clone()), (json!(250), json!(800)));
        assert_eq!(d["wispr_mic_name"], "");
    }

    #[tokio::test]
    async fn hello_says_whether_the_stick_should_stream_and_that_we_answer_prepare() {
        let t = app("hello", vec![FakeOutputs::speakers()], false).await;
        assert_eq!(hello_extra(&t.st)["wmic"], false); // off
        assert_eq!(hello_extra(&t.st)["wready"], true);
        t.set(json!({"wireless_mic": true})); // on, but nothing carries it
        assert_eq!(hello_extra(&t.st)["wmic"], false);
        let s = status(&t.st);
        assert!(s["enabled"] == true && s["device"].is_null() && s["error"].as_str().unwrap().contains("BlackHole"));
        t.out.set_devices(vec![FakeOutputs::speakers(), FakeOutputs::blackhole()]);
        t.set(json!({"wireless_mic_carrier": "auto"})); // looked at again on a change
        assert_eq!(hello_extra(&t.st)["wmic"], true);
        assert_eq!(status(&t.st)["device"], "BlackHole 2ch");
        // and the hello on the wire says so
        assert!(wait_for(|| t.fake.last_hello().is_some_and(|h| h["wready"] == true)).await);
    }

    #[tokio::test]
    async fn turning_it_on_tells_the_stick_right_away() {
        let t = app("tell", vec![FakeOutputs::blackhole()], false).await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.fake.wmic() == Some(true)).await);
        t.set(json!({"wireless_mic": false}));
        assert!(wait_for(|| t.fake.wmic() == Some(false)).await);
    }

    #[tokio::test]
    async fn prepare_stream_stop_through_the_app() {
        let t = wired("flow").await;
        assert!(t.out.last_stream().is_some()); // BlackHole: warm while connected
        t.fake.notify(&prepare_msg(3));
        assert!(wait_for(|| t.st.wmic.streaming()).await);
        assert!(wait_for(|| !t.readies().is_empty()).await);
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 3})]);
        assert!(!t.ours().is_empty());
        for f in adpcm::encode(&tone(0.3), 320) {
            t.fake.notify_audio(&f);
        }
        t.fake.notify(&stop_msg());
        assert!(wait_for(|| !t.st.wmic.streaming()).await);
        assert!(wait_for(|| t.ours().is_empty()).await);
        let s = status(&t.st);
        assert_eq!(
            (s["device_mic"]["name"].as_str(), s["device_mic"]["present"].as_bool()),
            (Some("stickybot-bt-mic"), Some(false))
        );
        assert_eq!(s["last"]["frames"], 15);
        assert_eq!(s["uid"], RUST_UID);
    }

    #[tokio::test]
    async fn old_firmware_start_makes_no_device() {
        let t = wired("start").await;
        t.fake.notify(&json!({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000}));
        assert!(wait_for(|| t.st.wmic.streaming()).await);
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(t.ca.created().is_empty() && t.readies().is_empty());
    }

    #[tokio::test]
    async fn stick_on_usb_refuses_and_removes() {
        let t = wired("usb").await;
        t.fake.notify(&prepare_msg(1));
        assert!(wait_for(|| !t.readies().is_empty()).await && !t.ours().is_empty());
        t.fake.notify(&json!({"src": "agent", "evt": "usb", "mounted": true}));
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert!(unavailable(&t.st).starts_with("the stick is plugged in by USB"));
        t.fake.notify(&prepare_msg(2));
        assert!(wait_for(|| t.readies().len() == 2).await);
        assert_eq!(t.readies()[1], json!({"src": "agent", "wispr": "ready", "id": 2, "device": false}));
        assert!(t.ours().is_empty());
        t.fake.notify(&json!({"src": "agent", "evt": "usb", "mounted": false}));
        assert!(wait_for(|| unavailable(&t.st).is_empty()).await);
    }

    #[tokio::test]
    async fn disconnect_removes_it_at_once() {
        let t = wired("disc").await;
        t.set(json!({"wispr_tail_ms": 5000}));
        t.fake.notify(&prepare_msg(1));
        assert!(wait_for(|| !t.readies().is_empty()).await);
        t.fake.notify(&stop_msg());
        t.fake.notify(&json!({"src": "agent", "evt": "usb", "mounted": false}));
        assert!(wait_for(|| t.st.wmic.stick_usb() == Some(false)).await);
        t.fake.drop_connection();
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert!(wait_for(|| t.st.wmic.stick_usb().is_none()).await);
        assert!(t.out.stream(0).closed()); // stopped and closed
    }

    #[tokio::test]
    async fn turning_it_off_removes_it() {
        let t = wired("off").await;
        assert_eq!(show(&t.st, Duration::from_secs(60)).await, "");
        t.set(json!({"wireless_mic": false}));
        assert!(wait_for(|| t.ours().is_empty()).await);
        assert_eq!(t.out.open_count(), 0);
    }

    #[tokio::test]
    async fn rename_from_the_settings() {
        let t = wired("rename").await;
        show(&t.st, Duration::from_secs(60)).await;
        t.set(json!({"wispr_mic_name": "stick-mic-bt"}));
        assert!(wait_for(|| t.ours().is_empty()).await);
        show(&t.st, Duration::from_secs(60)).await;
        assert_eq!(t.ca.names_of(RUST_UID), vec!["stick-mic-bt".to_string()]);
        drop_device(t.st.clone(), "test".into()).await;
    }

    #[tokio::test]
    async fn the_mic_is_named_after_the_picked_stick() {
        let t = wired("named").await;
        t.set(json!({"stick": {"address": "FAKE", "name": "Arbaaz StickS3"}}));
        for old in ["stick-mic-bluetooth", "arbaaz-sticks3-mic-bluetooth", "stick-key-mic-bluetooth"] {
            t.st.cfg.lock().unwrap().insert("wispr_mic_name".into(), json!(old)); // older automatic names
            assert_eq!(mic_name(&t.st), "arbaaz-sticks3-bt-mic");
        }
        t.st.cfg.lock().unwrap().insert("wispr_mic_name".into(), json!("my mic"));
        assert_eq!(mic_name(&t.st), "my mic"); // typed in: it wins
        t.set(json!({"stick": {"address": "FAKE", "name": "StickyBot"}, "wispr_mic_name": ""}));
        assert_eq!(mic_name(&t.st), "stickybot-bt-mic");
        assert_eq!(t.st.wmic.device().name, "stickybot-bt-mic");
    }

    #[tokio::test]
    async fn a_stick_renamed_on_the_stick_renames_its_mic() {
        let t = wired("renamed").await;
        assert!(wait_for(|| t.fake.writes().iter().any(|w| String::from_utf8_lossy(w).contains("@state"))).await);
        show(&t.st, Duration::from_secs(60)).await;
        assert_eq!(t.ca.names_of(RUST_UID), vec!["stickybot-bt-mic".to_string()]);
        t.fake.set_name("Desk Stick");
        t.st.link.request("@state").await.unwrap(); // the dashboard's, or ours on the next connect
        assert!(wait_for(|| t.st.stick().is_some_and(|s| s.name == "Desk Stick")).await);
        assert_eq!(t.st.stick().unwrap().address, "FAKE");
        assert_eq!(crate::config::load(&t.st.paths)["stick"]["name"], "Desk Stick");
        assert!(wait_for(|| t.ours().is_empty()).await); // the old one goes
        show(&t.st, Duration::from_secs(60)).await;
        assert_eq!(t.ca.names_of(RUST_UID), vec!["desk-stick-bt-mic".to_string()]);
        drop_device(t.st.clone(), "test".into()).await;
        // a proto reply without a board (not a @state) changes nothing
        on_message(&t.st, &json!({"src": "agent", "evt": "proto", "id": "x", "reply": {"name": "ignored"}}));
        assert_eq!(t.st.stick().unwrap().name, "Desk Stick");
    }

    #[tokio::test]
    async fn crash_cleanup_removes_the_device() {
        let t = wired("crash").await;
        show(&t.st, Duration::from_secs(60)).await;
        assert_eq!(t.ours().len(), 1);
        let st = t.st.clone();
        let n = tokio::task::spawn_blocking(move || crash_cleanup(&st, Duration::from_secs(2))).await.unwrap();
        assert_eq!(n, Some(1));
        assert!(t.ours().is_empty());
    }

    #[tokio::test]
    async fn bad_timings_in_the_config_are_clamped() {
        let t = wired("clamp").await;
        t.st.cfg.lock().unwrap().insert("wispr_switch_ms".into(), json!("x"));
        t.st.cfg.lock().unwrap().insert("wispr_tail_ms".into(), json!(99999));
        assert_eq!(settings(&t.st), (250, 5000));
    }

    #[tokio::test]
    async fn a_wispr_stream_is_not_a_talk() {
        let t = wired("nottalk").await;
        crate::talk::spawn(t.st.clone());
        t.fake.notify(&json!({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000, "codec": "ima-adpcm"}));
        for f in adpcm::encode(&tone(0.3), 320) {
            t.fake.notify_audio(&f);
        }
        t.fake.notify(&stop_msg());
        assert!(wait_for(|| status(&t.st)["last"]["frames"] == 15).await, "{}", status(&t.st));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(t.fake.screens().is_empty()); // no transcription, nothing on the stick
        assert_eq!(t.st.agent_state(), "ready");
    }

    #[tokio::test]
    async fn talk_frames_still_go_to_the_talk() {
        let t = wired("talk").await;
        t.fake.notify(&json!({"src": "agent", "evt": "talk", "state": "start"}));
        for f in adpcm::encode(&tone(0.2), 320) {
            t.fake.notify_audio(&f);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!t.st.wmic.streaming());
        assert!(t.out.stream(0).play(4800).iter().all(|x| *x == 0.0)); // nothing of it in the mic
    }

    #[tokio::test]
    async fn disconnect_stops_and_closes() {
        let t = wired("stopclose").await;
        t.fake.notify(&json!({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000}));
        assert!(wait_for(|| t.st.wmic.streaming()).await);
        t.fake.drop_connection();
        assert!(wait_for(|| !t.st.wmic.streaming()).await);
        assert!(wait_for(|| t.out.stream(0).closed()).await);
    }

    // --- tap mode (test_tap_mic.py) ---

    #[tokio::test]
    async fn turning_it_on_checks_the_tap_then_tells_the_stick() {
        let t = tap_agent("tapon").await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.st.wmic.with_player(|p| p.carrier) == Some(Carrier::Tap)).await);
        assert!(wait_for(|| t.fake.wmic() == Some(true)).await);
        assert_eq!(t.ca.measured(), 1);
        assert!(t.ca.all_devices().iter().all(|d| !d.uid.starts_with(RUST_UID)));
        let s = status(&t.st);
        assert_eq!((s["carrier"].as_str(), s["carrier_title"].as_str()), (Some("tap"), Some("Apple audio tap")));
        assert_eq!(s["tap"]["state"], "ok");
        assert_eq!((s["unavailable"].as_str(), s["error"].as_str()), (Some(""), Some("")));
        assert_eq!(s["device"], "MacBook Pro Speakers");
    }

    #[tokio::test]
    async fn a_hold_in_tap_mode() {
        let t = tap_agent("taphold").await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.fake.wmic() == Some(true)).await);
        assert!(t.out.streams().is_empty()); // the real output device: not held open between holds
        t.fake.notify(&prepare_msg(5));
        assert!(wait_for(|| !t.out.streams().is_empty()).await);
        let stream = t.out.stream(0);
        assert_eq!(stream.device().uid, "Speakers_UID"); // the default output, open before "ready"
        assert!(wait_for(|| !t.readies().is_empty()).await);
        assert_eq!(t.readies(), vec![json!({"src": "agent", "wispr": "ready", "id": 5})]);
        assert!(t.st.wmic.device().tap_live && !t.ours().is_empty() && !t.ca.taps_now().is_empty());
        for f in adpcm::encode(&tone(0.5)[..8000], 320) {
            t.fake.notify_audio(&f);
        }
        assert!(wait_for(|| t.st.wmic.with_player(|p| p.jitter().lock().unwrap().queued()) >= 3840).await);
        let out = stream.play(48000 / 4);
        assert!(out.iter().fold(0.0f32, |a, x| a.max(x.abs())) > 0.05); // the stick's audio goes out: into the tap
        t.fake.notify(&stop_msg());
        // the device goes first, then the tap (it lists the tap)
        assert!(wait_for(|| t.ours().is_empty() && t.ca.taps_now().is_empty() && !t.st.wmic.device().tap_live).await);
        // the check, then the hold: each tap made before its device and destroyed after it
        assert_eq!(t.ca.order(), ["tap+", "dev+", "dev-", "tap-", "tap+", "dev+", "dev-", "tap-"]);
        assert!(wait_for(|| stream.closed()).await);
        assert!(!t.st.wmic.with_player(|p| p.is_open()));
    }

    #[tokio::test]
    async fn without_the_tap_nothing_reaches_the_speakers() {
        let t = tap_agent("tapgate").await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.st.wmic.with_player(|p| p.carrier) == Some(Carrier::Tap)).await);
        t.fake.notify(&json!({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000})); // no device, no tap
        assert!(wait_for(|| !t.out.streams().is_empty()).await);
        let stream = t.out.stream(0);
        for f in adpcm::encode(&tone(0.5), 320) {
            t.fake.notify_audio(&f);
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!t.st.wmic.device().tap_live);
        assert!(stream.play(48000 / 4).iter().all(|x| *x == 0.0)); // silence, not the tone
        assert!(t.st.wmic.with_player(|p| p.gated()) > 0);
    }

    #[tokio::test]
    async fn tap_not_allowed_and_no_blackhole() {
        let t = tap_agent("tapdenied").await;
        t.ca.set(|s| s.tap_hears = false);
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.st.wmic.device().check.state == Some(CheckState::Failed)).await);
        assert!(wait_for(|| t.fake.wmic() == Some(false)).await);
        let s = status(&t.st);
        assert!(s["carrier"].is_null() && s["unavailable"].as_str().unwrap().contains("System Audio Recording"));
        assert_eq!(hello_extra(&t.st)["wmic"], false);
        t.fake.notify(&prepare_msg(1));
        assert!(wait_for(|| !t.readies().is_empty()).await);
        assert_eq!(t.readies()[0]["device"], false);
        assert!(t.ours().is_empty());
    }

    #[tokio::test]
    async fn tap_not_allowed_falls_back_to_blackhole_and_check_again_switches() {
        let t = app("tapfallback", vec![FakeOutputs::speakers(), FakeOutputs::blackhole()], true).await;
        t.ca.set(|s| s.tap_hears = false);
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.st.wmic.device().check.state == Some(CheckState::Failed)).await);
        assert!(wait_for(|| t.st.wmic.with_player(|p| p.carrier) == Some(Carrier::BlackHole)).await);
        assert_eq!(t.st.wmic.device().mode, Carrier::BlackHole);
        assert_eq!(hello_extra(&t.st)["wmic"], true);
        t.ca.set(|s| s.tap_hears = true); // allowed later: "Check again"
        assert_eq!(check_tap(&t.st, true).await.unwrap().state, Some(CheckState::Ok));
        assert_eq!(t.st.wmic.with_player(|p| p.carrier), Some(Carrier::Tap));
        assert_eq!(t.st.wmic.device().mode, Carrier::Tap);
    }

    #[tokio::test]
    async fn set_to_blackhole_never_checks_the_tap() {
        let t = app("tapbh", vec![FakeOutputs::speakers(), FakeOutputs::blackhole()], true).await;
        t.set(json!({"wireless_mic": true, "wireless_mic_carrier": "blackhole"}));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(t.ca.measured(), 0);
        assert_eq!(t.st.wmic.with_player(|p| p.carrier), Some(Carrier::BlackHole));
    }

    #[tokio::test]
    async fn switching_carrier_in_the_settings() {
        let t = app("tapswitch", vec![FakeOutputs::speakers(), FakeOutputs::blackhole()], true).await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.st.wmic.with_player(|p| p.carrier) == Some(Carrier::Tap)).await);
        t.set(json!({"wireless_mic_carrier": "blackhole"}));
        assert_eq!(t.st.wmic.with_player(|p| p.carrier), Some(Carrier::BlackHole));
        assert_eq!(t.st.wmic.with_player(|p| p.device(false)).unwrap().name, "BlackHole 2ch");
        t.set(json!({"wireless_mic_carrier": "auto"}));
        assert_eq!(t.st.wmic.with_player(|p| p.carrier), Some(Carrier::Tap)); // already proven
        assert_eq!(t.ca.measured(), 1);
    }

    #[tokio::test]
    async fn older_macos_is_blackhole_as_before() {
        let t = wired("oldmac").await;
        assert_eq!(t.st.wmic.with_player(|p| p.carrier), Some(Carrier::BlackHole));
        assert_eq!(t.ca.measured(), 0);
        assert_eq!(status(&t.st)["tap"]["supported"], false);
    }

    #[tokio::test]
    async fn no_check_during_a_hold() {
        let t = tap_agent("tapnocheck").await;
        t.set(json!({"wireless_mic": true}));
        assert!(wait_for(|| t.fake.wmic() == Some(true)).await);
        prepare(t.st.clone(), Some(json!(1))).await;
        assert!(check(&t.st).await.is_none());
        assert_eq!(t.ca.measured(), 1); // just the first one
        stop_hold(&t.st);
        assert!(wait_for(|| !t.st.wmic.device().present).await);
        assert!(wait_for(|| !t.st.wmic.holds.lock().unwrap().keep_until.is_some_and(|k| Instant::now() < k)).await);
        assert_eq!(check(&t.st).await.unwrap().state, Some(CheckState::Ok));
        assert!(holds_status(&t.st)["check"]["running"] == false);
    }

    #[tokio::test]
    async fn the_fake_sticks_wispr_hold_gets_ready_and_the_frames_are_played() {
        let t = wired("fakehold").await;
        t.set(json!({"wispr_switch_ms": 100}));
        let frames = adpcm::encode(&tone(0.4), 320);
        let hold = t.fake.wispr_hold(9, &frames, Duration::from_millis(20)).await; // real time
        assert_eq!(hold.ready, Some(json!({"src": "agent", "wispr": "ready", "id": 9})));
        assert!(hold.pressed_after.unwrap() >= Duration::from_millis(100));
        assert!(wait_for(|| status(&t.st)["last"]["frames"].as_u64() == Some(20 + 12)).await, "{}", status(&t.st));
        assert_eq!(status(&t.st)["last"]["lost"], 0);
        assert!(wait_for(|| t.ours().is_empty()).await);
    }
}
