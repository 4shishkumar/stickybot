//! A stick in memory: answers `proto` lines and sound messages like StickS3 firmware 2.3 does, so the
//! link, the API and the dashboard can be exercised without Bluetooth (and without taking the real
//! stick away from the Python agent, which holds its one connection).
//!
//! Knobs for tests: refuse writes ("Insufficient Resource" or any error), go silent, drop the
//! connection, be absent, inject notifications.
//!
//! It also plays sound clips (AUDIO_IN: start / frames / end) and takes firmware updates over
//! Bluetooth like `ota_rx.cpp` (acks every 4 KB, a `gap` for a lost packet, resume, the SHA-256
//! check), with knobs to lose packets, drop the link or cancel at an offset, corrupt the image.

use crate::proto::{Line, LineDecoder};
use crate::transport::{BleError, Char, Connection, Connector, Found, Incoming, Target};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;

#[derive(Default)]
struct State {
    stick: Value,
    sounds: Value,
    writes: Vec<Vec<u8>>,
    attempts: usize,
    busy: usize,
    busy_err: Option<BleError>,
    silent: bool,
    absent: bool,
    notify_size: usize,
    connect_attempts: usize,
    connects: usize,
    disconnects: usize,
    hellos: usize,
    last_hello: Option<Value>,
    last_target: Option<Target>,
    /// what the agent showed on the screen: `(state, heard, text)` per `{"src":"agent","state":…}`
    screens: Vec<(String, String, String)>,
    /// AUDIO_IN packets of the clip being received, and every clip played (its packets)
    clip: Vec<Vec<u8>>,
    clips: Vec<Vec<Vec<u8>>>,
    /// writes with response on AUDIO_IN (the first and last packet of a clip)
    audio_in_acked: usize,
    /// no AUDIO_IN / OTA_DATA (old firmware)
    old_firmware: bool,
    /// `{"src":"agent","wispr":"ready",…}` answers to `prepare`, and when they came
    wispr_ready: Vec<(std::time::Instant, Value)>,
    /// the last `wmic` the agent sent (in a hello or on its own)
    wmic: Option<bool>,
    /// a model of CoreBluetooth's write-without-response queue and the radio (timing tests);
    /// `None`: every write is instant
    radio: Option<crate::pacing::RadioQueue>,
    radio_ready: Arc<tokio::sync::Notify>,
    pub ota: Ota,
    pub faces: Faces,
    tx: Option<mpsc::UnboundedSender<Incoming>>,
    decoder: LineDecoder,
}

/// The fake stick; clones share it.
#[derive(Clone)]
pub struct FakeStick {
    s: Arc<Mutex<State>>,
}

impl Default for FakeStick {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeStick {
    pub fn new() -> Self {
        let st = State {
            stick: initial_state(),
            sounds: json!({"on": true, "volume": 160, "tunes": ["off", "beep", "confirmation_001", "confirmation_002", "jingle_win", "question_001"],
                           "map": {"sent": "beep", "answer": "confirmation_001", "approval": "question_001", "error": "beep"}}),
            notify_size: 180,
            ..State::default()
        };
        Self { s: Arc::new(Mutex::new(st)) }
    }

    fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
        f(&mut self.s.lock().unwrap())
    }

    /// The next `n` writes fail with `message` ("Insufficient Resource" is retried by the link).
    pub fn set_busy(&self, n: usize, message: &str) {
        self.with(|s| {
            s.busy = n;
            s.busy_err = Some(BleError::new(message));
        });
    }
    pub fn set_busy_code(&self, n: usize, code: i64, message: &str) {
        self.with(|s| {
            s.busy = n;
            s.busy_err = Some(BleError::with_code(code, message));
        });
    }
    pub fn busy_left(&self) -> usize {
        self.with(|s| s.busy)
    }
    /// Accepted writes (each at most one chunk).
    pub fn writes(&self) -> Vec<Vec<u8>> {
        self.with(|s| s.writes.clone())
    }
    /// Accepted + refused writes.
    pub fn write_attempts(&self) -> usize {
        self.with(|s| s.attempts)
    }
    /// Takes messages but never answers.
    pub fn set_silent(&self, silent: bool) {
        self.with(|s| s.silent = silent);
    }
    /// Out of range: connecting fails with "not found".
    pub fn set_absent(&self, absent: bool) {
        self.with(|s| s.absent = absent);
    }
    /// The largest notification (the real stick: 180).
    pub fn set_notify_size(&self, n: usize) {
        self.with(|s| s.notify_size = n.max(1));
    }
    pub fn connect_attempts(&self) -> usize {
        self.with(|s| s.connect_attempts)
    }
    pub fn connects(&self) -> usize {
        self.with(|s| s.connects)
    }
    pub fn disconnects(&self) -> usize {
        self.with(|s| s.disconnects)
    }
    pub fn hellos(&self) -> usize {
        self.with(|s| s.hellos)
    }
    pub fn last_hello(&self) -> Option<Value> {
        self.with(|s| s.last_hello.clone())
    }
    pub fn last_target(&self) -> Option<Target> {
        self.with(|s| s.last_target.clone())
    }
    /// The stick's settings as `@state` reports them.
    pub fn stick_state(&self) -> Value {
        self.with(|s| s.stick.clone())
    }
    /// The connection drops (out of range, restart).
    pub fn drop_connection(&self) {
        self.with(|s| s.tx = None);
    }
    /// A TX notification as is.
    pub fn notify_raw(&self, data: &[u8]) {
        self.with(|s| {
            if let Some(tx) = &s.tx {
                let _ = tx.send(Incoming::Tx(data.to_vec()));
            }
        });
    }
    /// A message from the stick, split like the firmware splits it.
    pub fn notify(&self, msg: &Value) {
        self.with(|s| emit(s, msg));
    }
    /// What the agent put on the screen so far: `(state, heard, text)`.
    pub fn screens(&self) -> Vec<(String, String, String)> {
        self.with(|s| s.screens.clone())
    }
    pub fn clear_screens(&self) {
        self.with(|s| s.screens.clear());
    }
    /// The clips the stick played, as their AUDIO_IN packets (start, frames, end).
    pub fn clips(&self) -> Vec<Vec<Vec<u8>>> {
        self.with(|s| s.clips.clone())
    }
    /// AUDIO_IN writes that waited for an answer.
    pub fn audio_in_acked(&self) -> usize {
        self.with(|s| s.audio_in_acked)
    }
    /// Firmware before 1.8: no AUDIO_IN or OTA_DATA, and `{"ota":…}` is ignored.
    pub fn set_old_firmware(&self, old: bool) {
        self.with(|s| s.old_firmware = old);
    }
    /// The firmware update side: knobs and what happened.
    pub fn ota<R>(&self, f: impl FnOnce(&mut Ota) -> R) -> R {
        self.with(|s| f(&mut s.ota))
    }
    /// Face packages (firmware 2.4.0+): knobs and what's installed. The transfer itself shares
    /// [`FakeStick::ota`]'s session and packet knobs (the same characteristic).
    pub fn faces<R>(&self, f: impl FnOnce(&mut Faces) -> R) -> R {
        self.with(|s| f(&mut s.faces))
    }
    /// Sets one field of what `@state` reports (e.g. `fw`).
    pub fn set_stick(&self, key: &str, value: Value) {
        self.with(|s| s.stick[key] = value);
    }
    /// B held while saying `frames` (ADPCM, see [`crate::adpcm::encode`]): `talk start`, the
    /// frames, `talk stop`, in the order the firmware sends them.
    pub fn talk(&self, frames: &[Vec<u8>]) {
        self.notify(&json!({"src": "agent", "evt": "talk", "state": "start", "rate": 16000, "codec": "ima-adpcm"}));
        for f in frames {
            self.notify_audio(f);
        }
        self.notify(&json!({"src": "agent", "evt": "talk", "state": "stop"}));
    }
    /// The agent's `wispr: ready` answers so far.
    pub fn wispr_readies(&self) -> Vec<Value> {
        self.with(|s| s.wispr_ready.iter().map(|(_, v)| v.clone()).collect())
    }
    /// The last `wmic` the agent sent: may the stick stream its mic for Wispr?
    pub fn wmic(&self) -> Option<bool> {
        self.with(|s| s.wmic)
    }
    /// A held for Wispr away from USB, like firmware 1.7.1+ does it with an agent that said
    /// `wready`: `prepare` (with `id`), then `frames` streamed in real time (one per `every`) from
    /// the start; the key goes down on the agent's `ready` with this id (or after 800 ms without
    /// one); on release the stream goes on for the 250 ms tail (silence), then `stop`.
    pub async fn wispr_hold(&self, id: u64, frames: &[Vec<u8>], every: Duration) -> WisprHold {
        let t0 = std::time::Instant::now();
        let seen = self.with(|s| s.wispr_ready.len());
        self.notify(&json!({"src": "agent", "evt": "wispr", "state": "prepare", "id": id, "rate": 16000,
                            "codec": "ima-adpcm"}));
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
        let mut hold = WisprHold::default();
        let check_ready = |hold: &mut WisprHold| {
            if hold.pressed_after.is_some() {
                return;
            }
            let ready = self.with(|s| {
                s.wispr_ready[seen..].iter().find(|(_, v)| v.get("id").and_then(Value::as_u64) == Some(id)).cloned()
            });
            if let Some((at, v)) = ready {
                hold.pressed_after = Some(at.duration_since(t0));
                hold.ready = Some(v);
            } else if t0.elapsed() >= READY_TIMEOUT {
                hold.pressed_after = Some(t0.elapsed()); // pressed anyway
            }
        };
        let tail = crate::adpcm::encode(&vec![0i16; 320 * (TAIL.as_millis() / 20) as usize], 320);
        let last_seq = frames.last().and_then(|f| crate::adpcm::decode_frame(f)).map(|(q, _)| q).unwrap_or(0);
        let tail: Vec<Vec<u8>> = tail
            .into_iter()
            .enumerate()
            .map(|(i, mut f)| {
                f[..2].copy_from_slice(&last_seq.wrapping_add(1 + i as u16).to_le_bytes());
                f
            })
            .collect();
        for f in frames.iter().chain(tail.iter()) {
            tick.tick().await;
            self.notify_audio(f);
            check_ready(&mut hold);
        }
        check_ready(&mut hold);
        self.notify(&json!({"src": "agent", "evt": "wispr", "state": "stop"}));
        hold.streamed = t0.elapsed();
        hold
    }
    /// Writes to AUDIO_IN / OTA_DATA take radio time like CoreBluetooth's queue (see
    /// [`crate::pacing::Radio`]): without response they wait for room, with response for the
    /// queue to drain and the stick's answer. `None`: instant (the default).
    pub fn set_radio(&self, radio: Option<crate::pacing::Radio>) {
        self.with(|s| s.radio = radio.map(crate::pacing::RadioQueue::new));
    }

    /// Paces one write through the radio model, if there is one.
    async fn radio_write(&self, response: bool) {
        if response {
            let Some(at) = self.with(|s| s.radio.as_ref().map(|q| q.ack_at())) else { return };
            tokio::time::sleep_until(at).await;
            return;
        }
        let ready = self.with(|s| s.radio.is_some().then(|| s.radio_ready.clone()));
        let Some(ready) = ready else { return };
        let _ = crate::pacing::wait_and_send(&ready, Some(crate::pacing::POLL), crate::pacing::ROOM_TIMEOUT, |late| {
            let pushed = self.with(|s| {
                let q = s.radio.as_mut()?;
                if !late && !q.can_send() {
                    return Some(None);
                }
                Some(Some(q.push().1))
            });
            match pushed {
                None => Ok(true), // the model was turned off meanwhile
                Some(None) => Ok(false),
                Some(Some(ready_at)) => {
                    if let Some(at) = ready_at {
                        let ready = ready.clone();
                        tokio::spawn(async move {
                            tokio::time::sleep_until(at).await;
                            ready.notify_waiters();
                        });
                    }
                    Ok(true)
                }
            }
        })
        .await;
    }

    pub fn notify_audio(&self, frame: &[u8]) {
        self.with(|s| {
            if let Some(tx) = &s.tx {
                let _ = tx.send(Incoming::Audio(frame.to_vec()));
            }
        });
    }

    fn on_char_write(&self, ch: Char, data: Vec<u8>, response: bool) -> Result<(), BleError> {
        let drop_now = self.with(|s| -> Result<bool, BleError> {
            s.attempts += 1;
            if s.tx.is_none() {
                return Err(BleError::new("Peripheral disconnected"));
            }
            if s.old_firmware {
                return Err(BleError::new(format!("no {ch:?} characteristic")));
            }
            if response && s.busy > 0 {
                s.busy -= 1;
                return Err(s.busy_err.clone().unwrap_or_else(|| BleError::new("busy")));
            }
            match ch {
                Char::AudioIn => {
                    if response {
                        s.audio_in_acked += 1;
                    }
                    match data.first() {
                        Some(0) => s.clip = vec![data],
                        Some(2) => {
                            s.clip.push(data);
                            let clip = std::mem::take(&mut s.clip);
                            s.clips.push(clip);
                        }
                        _ => s.clip.push(data),
                    }
                    Ok(false)
                }
                Char::OtaData => Ok(ota_packet(s, &data)),
                Char::Rx => unreachable!(),
            }
        })?;
        if drop_now {
            self.drop_connection();
        }
        Ok(())
    }

    fn on_write(&self, chunk: Vec<u8>) -> Result<(), BleError> {
        self.with(|s| {
            s.attempts += 1;
            if s.tx.is_none() {
                return Err(BleError::new("Peripheral disconnected"));
            }
            if s.busy > 0 {
                s.busy -= 1;
                return Err(s.busy_err.clone().unwrap_or_else(|| BleError::new("busy")));
            }
            s.writes.push(chunk.clone());
            for line in s.decoder.push(&chunk) {
                if let Line::Json(msg) = line {
                    handle(s, &msg);
                }
            }
            Ok(())
        })
    }
}

/// The firmware presses the key after this long without the agent's `ready` (`wispr::READY_TIMEOUT_MS`).
pub const READY_TIMEOUT: Duration = Duration::from_millis(800);
/// On release the key-up waits this long, streaming, so the audio on its way reaches Wispr.
pub const TAIL: Duration = Duration::from_millis(250);

/// What [`FakeStick::wispr_hold`] saw.
#[derive(Debug, Clone, Default)]
pub struct WisprHold {
    /// when the key went down (after `prepare`): on `ready`, or at the timeout
    pub pressed_after: Option<Duration>,
    /// the agent's `ready` for this hold (`None`: it never came)
    pub ready: Option<Value>,
    /// prepare -> stop
    pub streamed: Duration,
}

fn emit(s: &State, msg: &Value) {
    if let Some(tx) = &s.tx {
        let data = format!("{msg}\n").into_bytes();
        for part in data.chunks(s.notify_size) {
            let _ = tx.send(Incoming::Tx(part.to_vec()));
        }
    }
}

/// What the firmware does with a message from the agent (buddy.cpp), as far as Phase 1 needs.
fn handle(s: &mut State, msg: &Value) {
    if msg.get("hello").is_some() {
        s.hellos += 1;
        s.last_hello = Some(msg.clone());
    }
    if let Some(on) = msg.get("wmic").and_then(Value::as_bool) {
        s.wmic = Some(on); // hello or on its own
    }
    if msg.get("hello").is_some() {
        return;
    }
    if s.silent {
        return;
    }
    if msg.get("wispr").and_then(Value::as_str) == Some("ready") {
        s.wispr_ready.push((std::time::Instant::now(), msg.clone()));
        return;
    }
    if msg.get("wmic").is_some() {
        return;
    }
    if msg.get("cmd").and_then(Value::as_str) == Some("status") {
        // the Hardware Buddy status (PROTOCOL.md: the stick tools' stick_status)
        let name = s.stick["name"].clone();
        let (pct, usb) = (s.stick["battery"].clone(), s.stick["charging"].clone());
        emit(
            s,
            &json!({"ack": "status", "ok": true, "data": {"name": name, "sec": true,
                    "bat": {"pct": pct, "mV": 4000, "usb": usb}, "sys": {"up": 3600, "heap": 120000}}}),
        );
        return;
    }
    if msg.get("src").and_then(Value::as_str) == Some("agent")
        && let Some(state) = msg.get("state").and_then(Value::as_str)
    {
        let field = |k: &str| msg.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        s.screens.push((state.to_string(), field("heard"), field("text")));
        return;
    }
    if let Some(line) = msg.get("proto").and_then(Value::as_str) {
        let id: String =
            msg.get("id").and_then(Value::as_str).unwrap_or("").chars().filter(|c| *c != '"' && *c != '\\').collect();
        let reply = match faces_line(s, line.trim()) {
            Some(r) => r,
            None => run_line(&mut s.stick, line.trim()),
        };
        emit(s, &json!({"src": "agent", "evt": "proto", "id": id, "reply": reply}));
        return;
    }
    if let Some(o) = msg.get("ota").and_then(Value::as_object) {
        if !s.old_firmware {
            ota_message(s, o);
        }
        return;
    }
    if let Some(o) = msg.get("faces").and_then(Value::as_object) {
        if s.faces.supported && !s.faces.silent {
            faces_message(s, o);
        }
        return;
    }
    if let Some(snd) = msg.get("sound").and_then(Value::as_object) {
        if let Some(on) = snd.get("on").and_then(Value::as_bool) {
            s.sounds["on"] = json!(on);
        }
        if let Some(v) = snd.get("volume").and_then(Value::as_u64) {
            s.sounds["volume"] = json!(v.min(255));
        }
        if let Some(set) = snd.get("set").and_then(Value::as_object) {
            for (k, v) in set {
                s.sounds["map"][k] = v.clone();
            }
        }
        let rep = json!({"src": "agent", "evt": "sounds", "sounds": s.sounds.clone()});
        emit(s, &rep);
    }
}

fn args(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_string(), percent_decode(v))
        })
        .collect()
}

fn percent_decode(v: &str) -> String {
    let b = v.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(h) = v.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(h);
            i += 3;
            continue;
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `@action k=v&...` -> the reply JSON (protocol.cpp, roughly).
fn run_line(st: &mut Value, line: &str) -> Value {
    let Some(rest) = line.strip_prefix('@') else {
        return json!({"ok": false, "error": "expected @action"});
    };
    let (action, q) = rest.split_once(' ').unwrap_or((rest, ""));
    let a = args(q);
    let get = |k: &str| a.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let num = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
    let flag = |k: &str| num(k).map(|n| n != 0);
    match action {
        "state" => st.clone(),
        "keys" => json!({"ok": true, "mods": [["Ctrl", 1], ["Option", 4], ["Shift", 2], ["Cmd", 8], ["Fn", 16]],
                         "keys": [["(none)", 0], ["A", 4], ["B", 5], ["Return", 40], ["Escape", 41], ["Space", 44], ["F5", 62]]}),
        "test" => json!({"ok": true}),
        "brightness" => match num("pct") {
            Some(p) if (10..=100).contains(&p) => {
                st["brightness"] = json!(p);
                json!({"ok": true, "brightness": p})
            }
            _ => json!({"ok": false, "error": "pct must be 10-100"}),
        },
        "standby" => match num("min") {
            Some(m) if (0..=120).contains(&m) => {
                st["standby"] = json!(m);
                json!({"ok": true, "standby": m})
            }
            _ => json!({"ok": false, "error": "min must be 0-120"}),
        },
        "autorotate" | "landscape" | "devmode" | "wmic" => {
            let key = match action {
                "autorotate" => "autoRotate",
                "devmode" => "devMode",
                "wmic" => "wirelessMic",
                other => other,
            };
            st[key] = json!(flag("on").unwrap_or(false));
            json!({"ok": true})
        }
        "motion" => {
            for k in ["lift", "face", "shake"] {
                if let Some(v) = flag(k) {
                    st["motion"][k] = json!(v);
                }
            }
            json!({"ok": true})
        }
        "enter" => {
            st["enterDelay"] = json!(num("delay").unwrap_or(0));
            json!({"ok": true})
        }
        "face" => {
            if let Some(id) = get("id") {
                st["face"] = json!(id);
            }
            json!({"ok": true})
        }
        "combo" => {
            let mods = num("mods").unwrap_or(0);
            let key = num("key").unwrap_or(0);
            st["mods"] = json!(mods);
            st["key"] = json!(key);
            json!({"ok": true, "combo": st["combo"].clone()})
        }
        "macpass" => {
            st["macPassSet"] = json!(!get("pass").unwrap_or_default().is_empty());
            json!({"ok": true})
        }
        "name" => match get("name") {
            Some(n) if (1..=20).contains(&n.chars().count()) => {
                st["name"] = json!(n);
                json!({"ok": true, "restarting": true})
            }
            _ => json!({"ok": false, "error": "name must be 1-20 characters"}),
        },
        _ => json!({"ok": false, "error": format!("unknown command: {action}")}),
    }
}

fn initial_state() -> Value {
    json!({
        "ok": true, "proto": 1, "board": "m5stick-s3", "fw": "2.3.0", "name": "StickyBot (fake)",
        "combo": "Option+Cmd", "mods": 12, "key": 0, "connected": true, "bt": "connected",
        "battery": 87, "charging": false, "standby": 10, "brightness": 50, "slot": 1,
        "slots": [{"n": 1, "paired": true}, {"n": 2, "paired": false}, {"n": 3, "paired": false}],
        "web": {"on": false, "mode": "off", "ssid": "", "url": "", "homeSsid": ""},
        "autoRotate": true, "landscape": false, "devMode": false, "enterDelay": 0, "macPassSet": false,
        "motion": {"lift": true, "face": true, "shake": true},
        "wirelessMic": true, "usbMounted": false, "ota": 1, "otaPending": false,
        "face": "stickyworld",
        "faces": [
            {"id": "classic", "title": "Classic", "about": "The eyes and the time"},
            {"id": "stickyworld", "title": "StickyWorld", "about": "A city that grows with every merged PR"},
            {"id": "aquarium", "title": "Aquarium", "about": "A fish for every Claude session"},
            {"id": "bigtime", "title": "Big time", "about": "Just the time, large"}
        ]
    })
}

// --- firmware update (logic/ota_rx.cpp, roughly; test_ota.py's FakeStick) ------------------------

/// The stick's side of a Bluetooth firmware update, with test knobs.
#[derive(Debug, Clone)]
pub struct Ota {
    pub window: usize,
    pub chunk: usize,
    /// `(size, sha256, data so far)`
    pub session: Option<(usize, String, Vec<u8>)>,
    /// new bytes taken into the image
    pub received: usize,
    acked: usize,
    gap: bool,
    pub begins: Vec<Value>,
    pub ends: usize,
    pub committed: bool,
    /// the next `begin` is refused with this code (`why`: "dictation")
    pub refuse: Option<String>,
    /// packet offsets to lose once
    pub drop: HashSet<u32>,
    /// the link drops once the image reaches this many bytes
    pub disconnect_at: Option<usize>,
    /// B pressed on the stick once the image reaches this many bytes
    pub cancel_at: Option<usize>,
    /// one byte of the first packet is flipped (the SHA-256 check fails at `end`)
    pub corrupt: bool,
    /// acks that get lost on the way (count down)
    pub lose_acks: usize,
    /// the next `end` finds the last 100 bytes missing (`incomplete`)
    pub lose_tail: bool,
    /// packets that arrived (with their sizes)
    pub packets: Vec<usize>,
    /// after `done` the stick restarts (the link drops) into the new version...
    pub reboot: bool,
    /// ...or, with this, rolls back to the old one
    pub rollback: bool,
    /// the largest number of bytes past the last ack a packet reached
    pub max_in_flight: usize,
    /// whose session it is: `"ota"` (firmware) or `"faces"` (a face package); the acks carry it
    /// as their `evt`
    pub family: &'static str,
}

impl Ota {
    /// A transfer of `family` is under way (one bulk transfer at a time on OTA_DATA).
    pub fn active(&self, family: &str) -> bool {
        self.family == family && self.session.is_some() && !self.committed
    }
}

impl Default for Ota {
    fn default() -> Self {
        Self {
            window: 16384,
            chunk: 240,
            session: None,
            received: 0,
            acked: 0,
            gap: false,
            begins: Vec::new(),
            ends: 0,
            committed: false,
            refuse: None,
            drop: HashSet::new(),
            disconnect_at: None,
            cancel_at: None,
            corrupt: false,
            lose_acks: 0,
            lose_tail: false,
            packets: Vec::new(),
            max_in_flight: 0,
            family: "ota",
            reboot: true,
            rollback: false,
        }
    }
}

const OTA_ACK_EVERY: usize = 4096;

fn ota_reply(s: &mut State, mut m: Value) {
    m["src"] = json!("agent");
    m["evt"] = json!(s.ota.family);
    if m["state"] == "ack" && s.ota.lose_acks > 0 {
        s.ota.lose_acks -= 1;
        return;
    }
    emit(s, &m);
}

fn ota_message(s: &mut State, o: &serde_json::Map<String, Value>) {
    if let Some(b) = o.get("begin") {
        s.ota.begins.push(b.clone());
        if s.ota.active("faces") {
            // a face is arriving: it owns OTA_DATA
            emit(s, &json!({"src": "agent", "evt": "ota", "state": "refused", "error": "busy", "why": "faces"}));
            return;
        }
        s.ota.family = "ota";
        if let Some(code) = s.ota.refuse.clone() {
            ota_reply(s, json!({"state": "refused", "error": code, "why": "dictation"}));
            return;
        }
        let size = b["size"].as_u64().unwrap_or(0) as usize;
        let sha = b["sha256"].as_str().unwrap_or("").to_string();
        let (window, chunk) = (s.ota.window, s.ota.chunk);
        if let Some((sz, sh, data)) = &s.ota.session
            && *sz == size
            && *sh == sha
        {
            let have = data.len();
            s.ota.gap = false;
            ota_reply(
                s,
                json!({"state": "ready", "offset": have, "size": size, "resumed": true, "window": window, "chunk": chunk}),
            );
            return;
        }
        s.ota.session = Some((size, sha, Vec::new()));
        s.ota.acked = 0;
        ota_reply(
            s,
            json!({"state": "ready", "offset": 0, "size": size, "resumed": false, "window": window, "chunk": chunk}),
        );
    } else if o.contains_key("end") {
        s.ota.ends += 1;
        if s.ota.lose_tail {
            s.ota.lose_tail = false;
            if let Some((_, _, data)) = s.ota.session.as_mut() {
                let n = data.len().saturating_sub(100);
                data.truncate(n);
                s.ota.acked = n;
            }
        }
        match s.ota.session.clone() {
            None => ota_reply(s, json!({"state": "error", "error": "not_started", "offset": 0})),
            Some((size, _, data)) if data.len() != size => {
                ota_reply(s, json!({"state": "error", "error": "incomplete", "offset": data.len()}))
            }
            Some((size, sha, data)) => {
                let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
                if got != sha {
                    s.ota.session = None;
                    ota_reply(s, json!({"state": "error", "error": "sha", "offset": size}));
                } else {
                    s.ota.committed = true;
                    ota_reply(s, json!({"state": "done", "offset": size}));
                    if s.ota.reboot {
                        let v = s.ota.begins.last().and_then(|b| b["version"].as_str()).unwrap_or("?").to_string();
                        if !s.ota.rollback {
                            s.stick["fw"] = json!(v);
                            s.stick["otaPending"] = json!(false);
                        }
                        s.tx = None; // restarting
                    }
                }
            }
        }
    } else if o.contains_key("cancel") {
        s.ota.session = None;
        ota_reply(s, json!({"state": "cancelled", "by": "agent", "offset": 0}));
    } else if o.contains_key("status") {
        let (receiving, offset, size) = match &s.ota.session {
            Some((size, _, d)) => (true, d.len(), *size),
            None => (false, 0, 0),
        };
        ota_reply(
            s,
            json!({"state": "status", "receiving": receiving, "offset": offset, "size": size, "pending": false}),
        );
    }
}

/// One OTA_DATA packet. Returns true when the link should drop now (`disconnect_at`).
fn ota_packet(s: &mut State, pkt: &[u8]) -> bool {
    if pkt.len() < 4 {
        return false;
    }
    let off = u32::from_le_bytes([pkt[0], pkt[1], pkt[2], pkt[3]]);
    let mut body = pkt[4..].to_vec();
    s.ota.packets.push(body.len());
    if s.ota.drop.remove(&off) {
        return false;
    }
    let acked = s.ota.acked;
    let Some((size, _, data)) = s.ota.session.as_mut() else { return false };
    let size = *size;
    let have = data.len();
    let off = off as usize;
    s.ota.max_in_flight = s.ota.max_in_flight.max((off + body.len()).saturating_sub(acked));
    if off > have {
        if !s.ota.gap {
            s.ota.gap = true;
            ota_reply(s, json!({"state": "ack", "offset": have, "gap": true}));
        }
        return false;
    }
    if off + body.len() <= have {
        // a duplicate (a resend after a lost ack): ignored, and re-acked
        ota_reply(s, json!({"state": "ack", "offset": have}));
        return false;
    }
    if s.ota.corrupt && have == 0 && body.len() > 1 {
        body[1] ^= 1;
    }
    let new = body.len() - (have - off);
    data.extend_from_slice(&body[have - off..]);
    let have = data.len();
    s.ota.received += new;
    s.ota.gap = false;
    if have - s.ota.acked >= OTA_ACK_EVERY || have == size {
        s.ota.acked = have;
        ota_reply(s, json!({"state": "ack", "offset": have}));
    }
    if s.ota.cancel_at.is_some_and(|c| have >= c) {
        s.ota.cancel_at = None;
        s.ota.session = None;
        ota_reply(s, json!({"state": "cancelled", "by": "stick", "offset": have}));
    }
    if s.ota.disconnect_at.is_some_and(|d| have >= d) {
        s.ota.disconnect_at = None;
        return true;
    }
    false
}

// --- face packages (PROTOCOL.md "Face packages over Bluetooth"; test_face_install.py's FacesStick) ----

/// The stick's face packages (firmware 2.4.0+), with test knobs.
#[derive(Debug, Clone, Default)]
pub struct Faces {
    /// the face engine is there: `pkgFaces` in `@state`, `@faces`, `faces` messages
    pub supported: bool,
    /// id -> (version, sha256)
    pub installed: std::collections::BTreeMap<String, (String, String)>,
    /// every `put` the agent sent
    pub puts: Vec<Value>,
    pub ends: usize,
    /// the next `put` is refused with this code (`why`: "dictation")
    pub refuse: Option<String>,
    /// `end` answers `invalid` with this `why`
    pub invalid: Option<String>,
    /// `{"faces":…}` messages are taken but never answered
    pub silent: bool,
    pub removed: Vec<String>,
    /// the `@face…` protocol lines the stick got
    pub lines: Vec<String>,
}

const FACES_PAGE: usize = 6;
const FACES_MAX: usize = 12;

fn faces_reply(s: &State, mut m: Value) {
    m["src"] = json!("agent");
    m["evt"] = json!("faces");
    emit(s, &m);
}

/// `@state` (with `pkgFaces`), `@faces [page=N]`; `None` for any other line.
fn faces_line(s: &mut State, line: &str) -> Option<Value> {
    if line == "@state" {
        let mut st = s.stick.clone();
        if s.faces.supported {
            st["pkgFaces"] = json!({"n": s.faces.installed.len(), "fmt": [1]});
        }
        return Some(st);
    }
    if line.starts_with("@face") {
        s.faces.lines.push(line.to_string());
    }
    let rest = line.strip_prefix("@faces")?;
    if !s.faces.supported {
        return Some(json!({"ok": false, "error": "unknown command: faces"}));
    }
    let page = rest.trim().strip_prefix("page=").and_then(|p| p.parse::<usize>().ok()).unwrap_or(0);
    let items: Vec<Value> = s
        .faces
        .installed
        .iter()
        .map(|(id, (v, sha))| json!({"id": id, "v": v, "sz": 100, "sha": &sha[..8.min(sha.len())]}))
        .collect();
    let from = (page * FACES_PAGE).min(items.len());
    let to = ((page + 1) * FACES_PAGE).min(items.len());
    Some(json!({"ok": true, "faces": items[from..to], "free": 700, "max": FACES_MAX,
                "more": items.len() > (page + 1) * FACES_PAGE}))
}

fn faces_message(s: &mut State, o: &serde_json::Map<String, Value>) {
    if let Some(p) = o.get("put") {
        s.faces.puts.push(p.clone());
        if let Some(code) = s.faces.refuse.clone() {
            faces_reply(s, json!({"state": "refused", "error": code, "why": "dictation"}));
            return;
        }
        if s.ota.active("ota") {
            faces_reply(s, json!({"state": "refused", "error": "ota"}));
            return;
        }
        let id = p["id"].as_str().unwrap_or("").to_string();
        let size = p["size"].as_u64().unwrap_or(0) as usize;
        let sha = p["sha256"].as_str().unwrap_or("").to_string();
        let (window, chunk) = (s.ota.window, s.ota.chunk);
        if s.ota.family == "faces"
            && let Some((sz, sh, data)) = &s.ota.session
            && *sz == size
            && *sh == sha
            && !s.ota.committed
        {
            let have = data.len();
            s.ota.gap = false;
            faces_reply(
                s,
                json!({"state": "ready", "id": id, "offset": have, "size": size, "resumed": true, "window": window, "chunk": chunk}),
            );
            return;
        }
        s.ota.family = "faces";
        s.ota.committed = false;
        s.ota.session = Some((size, sha, Vec::new()));
        s.ota.acked = 0;
        faces_reply(
            s,
            json!({"state": "ready", "id": id, "offset": 0, "size": size, "resumed": false, "window": window, "chunk": chunk}),
        );
    } else if o.contains_key("end") {
        s.faces.ends += 1;
        let put = s.faces.puts.last().cloned().unwrap_or(Value::Null);
        match s.ota.session.clone().filter(|_| s.ota.family == "faces") {
            None => faces_reply(s, json!({"state": "error", "error": "not_started", "offset": 0})),
            Some((size, _, data)) if data.len() != size => {
                faces_reply(s, json!({"state": "error", "error": "incomplete", "offset": data.len()}))
            }
            Some((size, sha, data)) => {
                let got: String = Sha256::digest(&data).iter().map(|b| format!("{b:02x}")).collect();
                if got != sha {
                    s.ota.session = None;
                    faces_reply(s, json!({"state": "error", "error": "sha", "offset": size}));
                } else if let Some(why) = s.faces.invalid.clone() {
                    s.ota.session = None;
                    faces_reply(s, json!({"state": "error", "error": "invalid", "why": why, "offset": size}));
                } else {
                    s.ota.committed = true;
                    let id = put["id"].as_str().unwrap_or("").to_string();
                    let version = put["version"].as_str().unwrap_or("").to_string();
                    s.faces.installed.insert(id.clone(), (version.clone(), sha));
                    faces_reply(s, json!({"state": "done", "id": id, "version": version}));
                }
            }
        }
    } else if o.contains_key("cancel") {
        if s.ota.family == "faces" {
            s.ota.session = None;
        }
        faces_reply(s, json!({"state": "cancelled", "by": "agent", "offset": 0}));
    } else if let Some(id) = o.get("rm").and_then(Value::as_str) {
        if id == "all" {
            let n = s.faces.installed.len();
            s.faces.removed.extend(s.faces.installed.keys().cloned());
            s.faces.installed.clear();
            faces_reply(s, json!({"state": "removed", "id": "all", "n": n}));
        } else if s.faces.installed.remove(id).is_some() {
            s.faces.removed.push(id.to_string());
            let n = s.faces.installed.len();
            faces_reply(s, json!({"state": "removed", "id": id, "n": n}));
        } else {
            faces_reply(s, json!({"state": "error", "error": "notfound", "id": id}));
        }
    }
}

/// Its connection.
pub struct FakeConn {
    stick: FakeStick,
}

impl Connection for FakeConn {
    async fn write_rx(&self, chunk: Vec<u8>) -> Result<(), BleError> {
        self.stick.on_write(chunk)
    }
    async fn write(&self, ch: Char, data: Vec<u8>, response: bool) -> Result<(), BleError> {
        match ch {
            Char::Rx => self.stick.on_write(data),
            _ => {
                self.stick.radio_write(response).await;
                self.stick.on_char_write(ch, data, response)
            }
        }
    }
    fn has(&self, ch: Char) -> bool {
        ch == Char::Rx || !self.stick.with(|s| s.old_firmware)
    }
    fn max_write_without_response(&self) -> usize {
        244
    }
    fn mtu(&self) -> usize {
        247
    }
    async fn disconnect(&self) {
        self.stick.with(|s| {
            s.disconnects += 1;
            s.tx = None;
            s.decoder.reset();
        });
    }
}

impl Connector for FakeStick {
    type Conn = FakeConn;

    async fn connect(&self, target: &Target) -> Result<(FakeConn, mpsc::UnboundedReceiver<Incoming>), BleError> {
        tokio::time::sleep(Duration::from_millis(10)).await; // so state changes can be seen in tests
        self.with(|s| {
            s.connect_attempts += 1;
            if s.absent {
                return Err(BleError::not_found(format!("{} not found", target.address)));
            }
            let (tx, rx) = mpsc::unbounded_channel();
            s.tx = Some(tx);
            s.connects += 1;
            s.decoder.reset();
            s.last_target = Some(target.clone());
            Ok((FakeConn { stick: self.clone() }, rx))
        })
    }

    async fn scan(&self, _timeout: Duration) -> Result<Vec<Found>, BleError> {
        let name = self.with(|s| s.stick["name"].as_str().unwrap_or("StickyBot").to_string());
        Ok(vec![Found { address: "00000000-0000-0000-0000-00000000FA6E".into(), name, rssi: -42 }])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_lines_change_the_state() {
        let mut st = initial_state();
        assert_eq!(run_line(&mut st, "@brightness pct=70")["ok"], true);
        assert_eq!(st["brightness"], 70);
        assert_eq!(run_line(&mut st, "@brightness pct=5")["ok"], false);
        run_line(&mut st, "@motion lift=0&shake=1");
        assert_eq!(st["motion"], json!({"lift": false, "face": true, "shake": true}));
        run_line(&mut st, "@name name=Arbaaz%27s%20stick");
        assert_eq!(st["name"], "Arbaaz's stick");
        assert_eq!(run_line(&mut st, "@nope")["ok"], false);
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%2Bb%20c"), "a+b c");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(crate::proto::proto_line("x", &[("v", "é")]), "@x v=%C3%A9");
        assert_eq!(percent_decode("%C3%A9"), "é");
    }
}
