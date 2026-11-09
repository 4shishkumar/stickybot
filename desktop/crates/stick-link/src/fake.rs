//! A stick in memory: answers `proto` lines and sound messages like StickS3 firmware 2.3 does, so the
//! link, the API and the dashboard can be exercised without Bluetooth (and without taking the real
//! stick away from the Python agent, which holds its one connection).
//!
//! Knobs for tests: refuse writes ("Insufficient Resource" or any error), go silent, drop the
//! connection, be absent, inject notifications.

use crate::proto::{Line, LineDecoder};
use crate::transport::{BleError, Connection, Connector, Found, Incoming, Target};
use serde_json::{Value, json};
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
    pub fn notify_audio(&self, frame: &[u8]) {
        self.with(|s| {
            if let Some(tx) = &s.tx {
                let _ = tx.send(Incoming::Audio(frame.to_vec()));
            }
        });
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
        return;
    }
    if s.silent {
        return;
    }
    if let Some(line) = msg.get("proto").and_then(Value::as_str) {
        let id: String =
            msg.get("id").and_then(Value::as_str).unwrap_or("").chars().filter(|c| *c != '"' && *c != '\\').collect();
        let reply = run_line(&mut s.stick, line.trim());
        emit(s, &json!({"src": "agent", "evt": "proto", "id": id, "reply": reply}));
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

/// Its connection.
pub struct FakeConn {
    stick: FakeStick,
}

impl Connection for FakeConn {
    async fn write_rx(&self, chunk: Vec<u8>) -> Result<(), BleError> {
        self.stick.on_write(chunk)
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
