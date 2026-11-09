//! The Mac link's wire format, with no I/O (PROTOCOL.md section 5, `mac-agent/stick_agent/link.py`).
//!
//! - Mac -> stick (RX): one JSON object per line, `\n`-terminated, compact (`{"a":1}`), non-ASCII
//!   escaped as `\uXXXX` (what Python's `json.dumps` does), written in pieces of at most
//!   [`CHUNK`] bytes, each one a GATT write *with* response.
//! - stick -> Mac (TX): notifications that are pieces of `\n`-terminated JSON lines; a line can span
//!   several notifications and a notification can carry several lines.
//! - `{"src":"agent","proto":"@state","id":"p1"}` asks the stick to run a settings line; it answers
//!   `{"src":"agent","evt":"proto","id":"p1","reply":{...}}` (`reply` is `null` if it had nothing).

use serde_json::{Map, Value, json};
use std::time::Duration;

/// The stick's service (Nordic UART shape) and its characteristics.
pub const NUS: &str = "6e400001-b5a3-f393-e0a9-e50e24dcca9e";
/// Mac -> stick, JSON lines (write with response)
pub const RX: &str = "6e400002-b5a3-f393-e0a9-e50e24dcca9e";
/// stick -> Mac, JSON lines (notify)
pub const TX: &str = "6e400003-b5a3-f393-e0a9-e50e24dcca9e";
/// stick -> Mac, agent audio frames (notify)
pub const AUDIO: &str = "6e400004-b5a3-f393-e0a9-e50e24dcca9e";
/// Mac -> stick, sound clips (Phase 2)
pub const AUDIO_IN: &str = "6e400005-b5a3-f393-e0a9-e50e24dcca9e";
/// Mac -> stick, firmware image packets (Phase 2, stick 1.8.0+)
pub const OTA_DATA: &str = "6e400006-b5a3-f393-e0a9-e50e24dcca9e";

/// Largest RX write (link.py `CHUNK`): fits the stick's MTU with room to spare.
pub const CHUNK: usize = 180;
/// The agent's hello goes out this often (link.py `HELLO_EVERY_S`); the stick shows "agent ready" for 25 s.
pub const HELLO_EVERY: Duration = Duration::from_secs(8);
/// How long a `proto` request waits for its reply (firmware_api.py `PROTO_TIMEOUT_S`).
pub const PROTO_TIMEOUT: Duration = Duration::from_secs(5);
/// Tries per chunk when the stick says "Insufficient Resource" (link.py `_write(tries=4)`).
pub const WRITE_TRIES: u32 = 4;
/// CoreBluetooth's `CBATTErrorInsufficientResources` (ATT error 0x11).
pub const ATT_INSUFFICIENT_RESOURCES: i64 = 0x11;

/// A message as the stick gets it: compact JSON, ASCII only, plus the `\n`.
pub fn encode(msg: &Value) -> Vec<u8> {
    let mut out = ascii_json(msg);
    out.push('\n');
    out.into_bytes()
}

/// Compact JSON with every non-ASCII character escaped (`json.dumps(..., separators=(",", ":"))`).
/// Non-ASCII can only occur inside strings, so escaping it after serialising is safe.
pub fn ascii_json(msg: &Value) -> String {
    let raw = msg.to_string();
    if raw.is_ascii() {
        return raw;
    }
    let mut out = String::with_capacity(raw.len() + 16);
    for c in raw.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let mut units = [0u16; 2];
            for u in c.encode_utf16(&mut units) {
                out.push_str(&format!("\\u{u:04x}"));
            }
        }
    }
    out
}

/// The pieces one message is written in, in order.
pub fn chunks(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.chunks(CHUNK)
}

/// Should a failed write be tried again? "Insufficient Resource" means the stick's Bluetooth
/// buffers are busy for a moment (a long reply going out). Matches CoreBluetooth's ATT error code
/// and the text bleak puts in its exceptions (`GATT Protocol Error: Insufficient Resource`).
pub fn is_insufficient_resource(code: Option<i64>, message: &str) -> bool {
    code == Some(ATT_INSUFFICIENT_RESOURCES) || message.to_ascii_lowercase().contains("insufficient resource")
}

/// Wait before try `n + 1` (n counts from 0): 0.1 s, 0.2 s, 0.3 s (link.py).
pub fn retry_delay(n: u32) -> Duration {
    Duration::from_millis(100 * (u64::from(n) + 1))
}

/// Splits TX notifications back into JSON lines.
#[derive(Debug, Default)]
pub struct LineDecoder {
    buf: Vec<u8>,
}

/// One line from the stick: a JSON value, or the bytes that weren't JSON (logged and skipped).
#[derive(Debug, PartialEq)]
pub enum Line {
    Json(Value),
    Bad(Vec<u8>),
}

impl LineDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one notification; returns every line it completed.
    pub fn push(&mut self, data: &[u8]) -> Vec<Line> {
        self.buf.extend_from_slice(data);
        let mut out = Vec::new();
        while let Some(i) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=i).take(i).collect();
            match serde_json::from_slice::<Value>(&line) {
                Ok(v) => out.push(Line::Json(v)),
                Err(_) => out.push(Line::Bad(line)),
            }
        }
        out
    }

    /// Bytes waiting for their `\n`.
    pub fn pending(&self) -> usize {
        self.buf.len()
    }

    /// A new connection starts clean.
    pub fn reset(&mut self) {
        self.buf.clear();
    }
}

/// The hello (every [`HELLO_EVERY`]): `{"src":"agent","hello":1,"v":1,"t":<unix s>,"tz":<s east of UTC>}`
/// plus `extra` (the home card, weather, `wmic`... in later phases).
pub fn hello(t: i64, tz: i32, extra: Option<&Map<String, Value>>) -> Value {
    let mut m = json!({"src": "agent", "hello": 1, "v": 1, "t": t, "tz": tz});
    if let (Some(obj), Some(extra)) = (m.as_object_mut(), extra) {
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
    }
    m
}

/// This Mac's offset from UTC now, in seconds east (link.py's `tz`).
pub fn local_tz_offset() -> i32 {
    chrono::Local::now().offset().local_minus_utc()
}

/// `{"src":"agent","proto":"@state","id":"p1"}`
pub fn proto_request(line: &str, id: &str) -> Value {
    json!({"src": "agent", "proto": line.trim(), "id": id})
}

/// Checks a settings line the way firmware_api.py does: `@action [k=v&k=v]`.
pub fn check_proto_line(line: &str) -> Result<&str, &'static str> {
    let line = line.trim();
    if line.starts_with('@') { Ok(line) } else { Err("expected a line like @state") }
}

/// `@action k=v&k=v`, values URL-encoded like the dashboard's `encodeURIComponent`.
pub fn proto_line(action: &str, args: &[(&str, &str)]) -> String {
    let mut line = format!("@{action}");
    if !args.is_empty() {
        let q: Vec<String> = args.iter().map(|(k, v)| format!("{k}={}", uri_component(v))).collect();
        line.push(' ');
        line.push_str(&q.join("&"));
    }
    line
}

fn uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A reply to a `proto` request: `(id, reply)`. `reply` is `None` for `"reply":null`.
pub fn proto_reply(msg: &Value) -> Option<(&str, Option<&Value>)> {
    if msg.get("src")?.as_str()? != "agent" || msg.get("evt")?.as_str()? != "proto" {
        return None;
    }
    let id = msg.get("id")?.as_str()?;
    Some((id, msg.get("reply").filter(|r| !r.is_null())))
}

/// `{"src":"agent","sound":<payload>}`: get / set / play / on / volume (PROTOCOL.md "Sounds").
pub fn sound(payload: Value) -> Value {
    json!({"src": "agent", "sound": payload})
}

/// The stick's answer to a sound message: `{"src":"agent","evt":"sounds","sounds":{...}}`.
pub fn sounds_report(msg: &Value) -> Option<&Value> {
    if msg.get("src")?.as_str()? == "agent" && msg.get("evt")?.as_str()? == "sounds" { msg.get("sounds") } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_is_compact_json_and_a_newline() {
        let b = encode(&json!({"src": "agent", "hello": 1}));
        // (key order doesn't matter to the stick; serde_json sorts them, Python keeps insertion order)
        assert_eq!(b, b"{\"hello\":1,\"src\":\"agent\"}\n");
        let s = String::from_utf8(encode(&json!({"src": "agent", "x": [1, 2], "y": {"z": "a b"}}))).unwrap();
        assert_eq!(s, "{\"src\":\"agent\",\"x\":[1,2],\"y\":{\"z\":\"a b\"}}\n");
        let s = String::from_utf8(b).unwrap();
        assert!(s.ends_with("}\n") && !s.contains(' ') && s.matches('\n').count() == 1);
    }

    #[test]
    fn non_ascii_is_escaped_like_python() {
        // json.dumps({"t": "café ☕ 😀"}, separators=(",", ":"))
        let want = [r#"{"t":"caf"#, "\\u00e9 \\u2615 \\ud83d\\ude00", r#""}"#].concat();
        assert_eq!(ascii_json(&json!({"t": "caf\u{e9} \u{2615} \u{1f600}"})), want);
        assert!(encode(&json!({"t": "é"})).is_ascii());
    }

    #[test]
    fn long_messages_are_split_into_180_byte_writes() {
        // test_link_send: a 400-character message goes out in 3 writes and ends with "}\n"
        let data = encode(&json!({"src": "agent", "x": "y".repeat(400)}));
        let parts: Vec<&[u8]> = chunks(&data).collect();
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.len() <= CHUNK));
        assert_eq!(parts[0].len(), CHUNK);
        assert!(parts.concat().ends_with(b"}\n"));
        assert_eq!(parts.concat(), data);
    }

    #[test]
    fn a_short_message_is_one_write() {
        let data = encode(&json!({"src": "agent"}));
        assert_eq!(chunks(&data).count(), 1);
    }

    #[test]
    fn insufficient_resource_is_retried_other_errors_are_not() {
        // the text bleak gives (test_link_send) and CoreBluetooth's ATT code
        assert!(is_insufficient_resource(None, "(17, 'GATT Protocol Error: Insufficient Resource')"));
        assert!(is_insufficient_resource(Some(0x11), "Resources are insufficient."));
        assert!(!is_insufficient_resource(None, "disconnected"));
        assert!(!is_insufficient_resource(Some(14), "Unlikely error."));
        assert_eq!(retry_delay(0), Duration::from_millis(100));
        assert_eq!(retry_delay(2), Duration::from_millis(300));
    }

    #[test]
    fn lines_can_span_notifications() {
        let mut d = LineDecoder::new();
        assert!(d.push(b"{\"src\":\"agent\",\"ev").is_empty());
        assert_eq!(d.pending(), 18);
        let got = d.push(b"t\":\"talk\",\"state\":\"start\"}\n");
        assert_eq!(got, vec![Line::Json(json!({"src": "agent", "evt": "talk", "state": "start"}))]);
        assert_eq!(d.pending(), 0);
    }

    #[test]
    fn one_notification_can_carry_several_lines() {
        let mut d = LineDecoder::new();
        let got = d.push(b"{\"a\":1}\n{\"b\":2}\n{\"c\"");
        assert_eq!(got, vec![Line::Json(json!({"a": 1})), Line::Json(json!({"b": 2}))]);
        assert_eq!(d.push(b":3}\n"), vec![Line::Json(json!({"c": 3}))]);
    }

    #[test]
    fn unreadable_lines_are_reported_and_skipped() {
        let mut d = LineDecoder::new();
        let got = d.push(b"garbage\n{\"ok\":true}\r\n\n");
        assert_eq!(got.len(), 3);
        assert_eq!(got[0], Line::Bad(b"garbage".to_vec()));
        assert_eq!(got[1], Line::Json(json!({"ok": true}))); // a \r before the \n is whitespace
        assert_eq!(got[2], Line::Bad(Vec::new())); // an empty line isn't JSON (json.loads fails too)
        d.push(b"{\"half\":");
        d.reset();
        assert_eq!(d.pending(), 0);
    }

    #[test]
    fn the_hello_has_the_time_and_extras() {
        let mut extra = Map::new();
        extra.insert("wmic".into(), json!(false));
        let h = hello(1_700_000_000, 19800, Some(&extra));
        assert_eq!(h, json!({"src": "agent", "hello": 1, "v": 1, "t": 1_700_000_000i64, "tz": 19800, "wmic": false}));
        assert_eq!(hello(5, -3600, None)["tz"], json!(-3600));
    }

    #[test]
    fn proto_requests_and_replies() {
        assert_eq!(proto_request(" @state ", "p1"), json!({"src": "agent", "proto": "@state", "id": "p1"}));
        let msg = json!({"src": "agent", "evt": "proto", "id": "p1", "reply": {"ok": true, "fw": "2.3.0"}});
        let (id, reply) = proto_reply(&msg).unwrap();
        assert_eq!(id, "p1");
        assert_eq!(reply.unwrap()["fw"], "2.3.0");
        let none = json!({"src": "agent", "evt": "proto", "id": "p2", "reply": null});
        assert_eq!(proto_reply(&none), Some(("p2", None)));
        assert_eq!(proto_reply(&json!({"src": "agent", "evt": "talk"})), None);
        assert_eq!(proto_reply(&json!({"src": "cli", "evt": "proto", "id": "x"})), None);
    }

    #[test]
    fn proto_lines_are_checked_and_built_like_the_dashboard() {
        assert_eq!(check_proto_line("  @state "), Ok("@state"));
        assert!(check_proto_line("state").is_err());
        assert_eq!(proto_line("state", &[]), "@state");
        assert_eq!(proto_line("combo", &[("text", "cmd+opt")]), "@combo text=cmd%2Bopt");
        assert_eq!(proto_line("name", &[("name", "Arbaaz's stick")]), "@name name=Arbaaz's%20stick");
        assert_eq!(proto_line("motion", &[("lift", "1"), ("face", "0")]), "@motion lift=1&face=0");
    }

    #[test]
    fn sound_messages() {
        assert_eq!(sound(json!({"get": 1})), json!({"src": "agent", "sound": {"get": 1}}));
        let rep = json!({"src": "agent", "evt": "sounds", "sounds": {"on": true, "volume": 128}});
        assert_eq!(sounds_report(&rep).unwrap()["volume"], 128);
        assert!(sounds_report(&json!({"src": "agent", "evt": "talk"})).is_none());
    }
}
