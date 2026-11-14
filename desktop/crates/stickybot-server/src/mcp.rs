//! The stick tools: an MCP server inside the app, so any Claude session can page you on the stick
//! (a port of `mcp_server.py`).
//!
//! `POST http://127.0.0.1:<port>/mcp` (MCP "Streamable HTTP", JSON-RPC 2.0, one message per POST)
//!
//! Who uses it: this app's own voice session (`brain.rs` passes `--mcp-config`) and, after
//! `POST /api/mcp/terminal/install`, every Claude Code session in a terminal (`claude mcp add
//! --scope user`). Guarded like the rest of the API (`Host`, the token as `Authorization: Bearer` or
//! `X-Stick-Token`, even for GET), plus: an `Origin` header must be the dashboard's own; off in
//! Settings (`mcp_server: false`) -> 503.
//!
//! Protocol (both eras, as mcp_server.py documents them): legacy clients (`initialize`, then
//! `tools/list` / `tools/call` / `ping`, no session ids); modern 2026-07-28 clients (no handshake,
//! `params._meta["io.modelcontextprotocol/protocolVersion"]` mirrored in `MCP-Protocol-Version`,
//! `Mcp-Method`, `Mcp-Name`; `server/discover`; `"resultType": "complete"`). `ask` can wait a minute:
//! with `Accept: text/event-stream` it answers as an SSE stream with keep-alives, and the client
//! closing it clears the question on the stick. Bad arguments are tool errors (`isError`), an unknown
//! tool is a JSON-RPC error. Everything fails closed without the stick, except `stick_status`.
//! Alerts are rate-limited.

use crate::AppState;
use crate::talk::stick_text;
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// The server's name: Claude Code calls the tools `mcp__stick__<tool>`.
pub const SERVER_NAME: &str = "stick";
pub const SERVER_VERSION: &str = "1.0.0";
pub const MODERN: &[&str] = &["2026-07-28"];
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const META: &str = "io.modelcontextprotocol/";

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL: i64 = -32603;
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_VERSION: i64 = -32022;

/// notify / celebrate / speak: at least this far apart (all sessions together), at most 20 a minute.
pub const ALERT_GAP: Duration = Duration::from_secs(1);
pub const ALERT_PER_MIN: usize = 20;
pub const SSE_KEEPALIVE: Duration = Duration::from_secs(15);
/// progress / pin_home: the card goes to the stick at most this often.
pub const CARD_PUSH_GAP: Duration = Duration::from_millis(500);
/// A progress line stays this long after its last update, or this long once done == total.
pub const PROGRESS_S: f64 = 600.0;
pub const PROGRESS_DONE_S: f64 = 120.0;
pub const NOT_CONNECTED: &str = "stick not connected";

/// The tools that only give feedback (pre-allowed); `open_url`, `open_app`, `quick_command` change
/// the Mac, so Claude Code asks first.
/// (`ir_list` / `ir_send` too: the user asked for it by voice)
/// (`calendar_next` / `calendar_today` only read the Calendar the person connected, and ask for it
/// by voice; `calendar_join` opens a meeting link, so it asks first)
pub const FEEDBACK_TOOLS: &[&str] = &[
    "notify",
    "ask",
    "celebrate",
    "progress",
    "pin_home",
    "stick_status",
    "speak",
    "ir_list",
    "ir_send",
    "calendar_next",
    "calendar_today",
];
pub const MAC_TOOLS: &[&str] = &["open_url", "open_app", "quick_command", "calendar_join"];
/// The calendar tools read the Mac's Calendar, not the stick: they work with no stick connected.
pub const CALENDAR_TOOLS: &[&str] = &["calendar_next", "calendar_today", "calendar_join"];

pub const INSTRUCTIONS: &str = "Tools for the user's M5StickS3, a small device on their desk with a 135x240 screen, two buttons (A, B) and a speaker. Use it like a pager: notify() when a long task finishes or needs them, ask() for a quick A/B decision instead of waiting in the terminal, progress() for long jobs, celebrate() for real wins. Keep texts short (a few words). Don't notify for every small step. stick_status() tells whether they're at their desk.";

/// The feedback tools as Claude Code names them (for `--allowedTools`).
pub fn allowed_tool_names() -> Vec<String> {
    FEEDBACK_TOOLS.iter().map(|t| format!("mcp__{SERVER_NAME}__{t}")).collect()
}

fn text(maxlen: u64, desc: &str) -> Value {
    json!({"type": "string", "maxLength": maxlen, "description": desc})
}

/// The tools, in mcp_server.py's order and words.
pub fn tools() -> &'static [Value] {
    static TOOLS: OnceLock<Vec<Value>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        vec![
            json!({"name": "notify", "title": "Notify on the stick",
                "description": "Show a short message on the user's stick (a toast, with a sound). Use it when a long task finishes, fails, or needs the user's attention — e.g. 'Tests passed', 'Build failed: 3 errors'. level: info (quiet toast), nice (toast + chime), celebrate (full-screen confetti), heads_up (orange, stays until a button press). At most one per second. Returns 'shown' or why not.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                    "text": text(130, "What to say, a few words (the first ~40 characters are the title)."),
                    "level": {"type": "string", "enum": ["info", "nice", "celebrate", "heads_up"], "default": "info"},
                    "sound": {"type": "boolean", "description": "Play a sound (default: info no, the others yes)."}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}}),
            json!({"name": "ask", "title": "Ask on the stick",
                "description": "Ask the user a quick question on the stick and wait for the answer: button A picks the first option, B the second (default options: Yes / No). Shaking the stick dismisses it. Returns the chosen option's label, or 'timeout', 'dismissed', 'busy: …' (another prompt or a voice request is in progress — try again later) or 'stick not connected'. Use for small decisions; keep the question under ~100 characters.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["question"], "properties": {
                    "question": text(160, "The question, short."),
                    "options": {"type": "array", "items": {"type": "string", "maxLength": 16}, "minItems": 1, "maxItems": 2,
                                "description": "1-2 short labels (<= 16 characters): A, then B."},
                    "timeout_s": {"type": "integer", "minimum": 5, "maximum": 120, "default": 60}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}}),
            json!({"name": "celebrate", "title": "Celebrate on the stick",
                "description": "Full-screen confetti and a fanfare on the stick, for real wins (a release, a green CI after a long fight). At most one per second.",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {
                    "text": text(130, "What happened (default 'Done!').")}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}}),
            json!({"name": "progress", "title": "Progress on the stick",
                "description": "Show progress of a long job on the stick's home screen card, e.g. text='Running tests', done=40, total=120. Call again to update (replaces the line); it disappears 10 minutes after the last update, 2 minutes after done == total. Silent — no sound.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                    "text": text(140, "What's happening, short."),
                    "done": {"type": "integer", "minimum": 0},
                    "total": {"type": "integer", "minimum": 1}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}}),
            json!({"name": "pin_home", "title": "Pin a line on the stick",
                "description": "Pin a line on the stick's home screen card (instead of the facts/news rotation) for `minutes` (default 30), e.g. 'PR #123 waiting for review'. A new pin replaces the old one; empty text removes it.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                    "text": text(140, "The line to show ('' removes the pin)."),
                    "minutes": {"type": "integer", "minimum": 1, "maximum": 480, "default": 30}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}}),
            json!({"name": "open_url", "title": "Open a URL on the Mac",
                "description": "Open an http(s) URL in the user's browser on their Mac. Needs the user's approval.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["url"], "properties": {
                    "url": {"type": "string", "maxLength": 2000, "description": "http:// or https:// only."}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}}),
            json!({"name": "open_app", "title": "Open an app on the Mac",
                "description": "Open (or bring to the front) an installed Mac app by name, e.g. 'Slack', 'VS Code'. Needs the user's approval.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["name"], "properties": {
                    "name": {"type": "string", "maxLength": 60, "description": "The app's name."}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}}),
            json!({"name": "quick_command", "title": "Run a quick command",
                "description": "Run one of the user's quick commands (the phrases they set up for the stick, e.g. 'PR 1234', 'open slack'). Needs the user's approval. Returns the command's result, or 'no quick command matches'.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["phrase"], "properties": {
                    "phrase": {"type": "string", "maxLength": 120, "description": "The phrase, as the user would say it."}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}}),
            json!({"name": "stick_status", "title": "Stick status",
                "description": "The stick's state: connected, battery, on USB, what its screen shows, when a button was last pressed, and a hint whether the user is at their desk. Read-only.",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}},
                "annotations": {"readOnlyHint": true, "openWorldHint": false}}),
            json!({"name": "speak", "title": "Speak on the stick",
                "description": "Say a short sentence out loud (the stick's speaker or the Mac's, as the user set it). Use sparingly — only when a spoken word is worth it. Silent in quiet hours. At most one per second.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["text"], "properties": {
                    "text": text(300, "What to say, one or two short sentences.")}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": false}}),
            json!({"name": "ir_list", "title": "List the IR remotes",
                "description": "The infrared remotes the stick can play (TV, AC, soundbar...) and their buttons, as {device: [button, ...]}: buttons learned on the stick plus built-in TV code sets. Read-only. Call it before ir_send to pick the button whose name fits the request (e.g. 'set the AC to 24' -> device 'ac', button 'cool 24').",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}},
                "annotations": {"readOnlyHint": true, "openWorldHint": false}}),
            json!({"name": "ir_send", "title": "Press a remote button",
                "description": "Send an infrared remote button from the stick (it must point roughly at the device), e.g. device='tv', button='power'. Names as ir_list() gives them. repeat: extra presses (volume steps). Returns 'tv: power sent' or why not.",
                "inputSchema": {"type": "object", "additionalProperties": false, "required": ["device", "button"],
                    "properties": {
                        "device": {"type": "string", "maxLength": 32, "description": "The remote, e.g. 'tv'."},
                        "button": {"type": "string", "maxLength": 32, "description": "The button, e.g. 'volume up'."},
                        "repeat": {"type": "integer", "minimum": 0, "maximum": 10, "default": 0}}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}}),
            json!({"name": "calendar_next", "title": "Next meeting",
                "description": "The user's next meeting from their Mac's Calendar (the Calendar connection in the dashboard must be on): what is on now and what starts next today, with times. Read-only. Works without the stick.",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}},
                "annotations": {"readOnlyHint": true, "openWorldHint": false}}),
            json!({"name": "calendar_today", "title": "The day's meetings",
                "description": "The user's meetings for today or tomorrow from their Mac's Calendar (the Calendar connection must be on): one line each with the times, title and where (a meeting service or the location), at most 12 lines. Read-only. Works without the stick.",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {
                    "day": {"type": "string", "enum": ["today", "tomorrow"], "default": "today"}}},
                "annotations": {"readOnlyHint": true, "openWorldHint": false}}),
            json!({"name": "calendar_join", "title": "Join the meeting",
                "description": "Open the video-call link (Google Meet, Zoom, Microsoft Teams or Webex) of the meeting that is on now, else of the next one today that has a link, in the user's browser or app. Needs the user's approval. Returns what it opened, or 'No meeting link found.'",
                "inputSchema": {"type": "object", "additionalProperties": false, "properties": {}},
                "annotations": {"readOnlyHint": false, "destructiveHint": false, "openWorldHint": true}}),
        ]
    })
}

pub fn tool(name: &str) -> Option<&'static Value> {
    tools().iter().find(|t| t["name"] == name)
}

/// The same tools as OpenAI-style function definitions (the OpenRouter brain).
pub fn openai_tools() -> Vec<Value> {
    tools()
        .iter()
        .map(|t| {
            json!({"type": "function", "function": {"name": t["name"], "description": t["description"],
                                                          "parameters": t["inputSchema"]}})
        })
        .collect()
}

/// What the approval prompt on the stick shows for a Mac tool.
pub fn approval_hint(name: &str, args: &Value) -> String {
    let key = match name {
        "open_url" => "url",
        "open_app" => "name",
        "quick_command" => "phrase",
        "calendar_join" => return "the link of your meeting".into(),
        _ => return summarize(args, 60),
    };
    match args.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => String::new(),
    }
}

/// `k='v', n=3` for logs and the activity feed (values cut to `n`).
pub fn summarize(args: &Value, n: usize) -> String {
    let Some(o) = args.as_object() else { return String::new() };
    o.iter()
        .map(|(k, v)| {
            let s = match v {
                Value::String(s) => format!("'{s}'"),
                v => v.to_string(),
            };
            let cut: String = s.chars().take(n).collect();
            format!("{k}={}", if cut.len() < s.len() { format!("{cut}…") } else { s })
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A small JSON Schema check for our own schemas (mcp_server.py `check_args`): too-long strings are
/// cut, not refused; defaults are filled in. `Err` is the tool error's text.
pub fn check_args(schema: &Value, args: &Value) -> Result<Map<String, Value>, String> {
    let empty = Map::new();
    let args = match args {
        Value::Null => &empty,
        Value::Object(o) => o,
        _ => return Err("arguments must be an object".into()),
    };
    let props = schema["properties"].as_object().cloned().unwrap_or_default();
    let mut extra: Vec<&String> = args.keys().filter(|k| !props.contains_key(*k)).collect();
    if !extra.is_empty() {
        extra.sort();
        let names: Vec<&str> = props.keys().map(String::as_str).collect();
        return Err(format!(
            "unknown argument(s): {} (expected: {})",
            extra.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "),
            if names.is_empty() { "none".to_string() } else { names.join(", ") }
        ));
    }
    for r in schema["required"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        if args.get(r).is_none_or(Value::is_null) {
            return Err(format!("missing argument: {r}"));
        }
    }
    let mut out = Map::new();
    for (name, p) in &props {
        let v = match args.get(name) {
            None | Some(Value::Null) => {
                if let Some(d) = p.get("default") {
                    out.insert(name.clone(), d.clone());
                }
                continue;
            }
            Some(v) => v,
        };
        let v = match p["type"].as_str() {
            Some("string") => {
                let s = v.as_str().ok_or_else(|| format!("{name} must be a string"))?;
                match p["maxLength"].as_u64() {
                    Some(n) => json!(s.chars().take(n as usize).collect::<String>()),
                    None => json!(s),
                }
            }
            Some("integer") => {
                let n = match v {
                    Value::Number(n) if n.is_i64() => n.as_i64(),
                    Value::Number(n) => n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64),
                    _ => None,
                }
                .ok_or_else(|| format!("{name} must be an integer"))?;
                let (lo, hi) = (p["minimum"].as_i64(), p["maximum"].as_i64());
                if lo.is_some_and(|lo| n < lo) || hi.is_some_and(|hi| n > hi) {
                    let f = |x: Option<i64>, d: &str| x.map(|x| x.to_string()).unwrap_or_else(|| d.into());
                    return Err(format!("{name} must be between {} and {}", f(lo, "-inf"), f(hi, "inf")));
                }
                json!(n)
            }
            Some("boolean") => {
                if !v.is_boolean() {
                    return Err(format!("{name} must be true or false"));
                }
                v.clone()
            }
            Some("array") => {
                let a = v
                    .as_array()
                    .filter(|a| a.iter().all(Value::is_string))
                    .ok_or_else(|| format!("{name} must be a list of strings"))?;
                let (min, max) = (p["minItems"].as_u64().unwrap_or(0) as usize, p["maxItems"].as_u64());
                if a.len() < min || max.is_some_and(|m| a.len() > m as usize) {
                    return Err(format!(
                        "{name} takes {min}-{} items",
                        max.map(|m| m.to_string()).unwrap_or_else(|| "None".into())
                    ));
                }
                let cap = p["items"]["maxLength"].as_u64();
                json!(
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(|s| match cap {
                            Some(c) => s.chars().take(c as usize).collect::<String>(),
                            None => s.to_string(),
                        })
                        .collect::<Vec<_>>()
                )
            }
            _ => v.clone(),
        };
        if let Some(e) = p["enum"].as_array()
            && !e.contains(&v)
        {
            let opts: Vec<&str> = e.iter().filter_map(Value::as_str).collect();
            return Err(format!("{name} must be one of: {}", opts.join(", ")));
        }
        out.insert(name.clone(), v);
    }
    Ok(out)
}

/// At least `gap` between, at most `per_min` a minute (times in seconds, any origin).
#[derive(Debug)]
pub struct RateLimit {
    pub gap: f64,
    pub per_min: usize,
    times: Vec<f64>,
}

impl RateLimit {
    pub fn new(gap: f64, per_min: usize) -> Self {
        Self { gap, per_min, times: Vec::new() }
    }

    /// "" if allowed now (and counts it), else why not.
    pub fn take(&mut self, now: f64) -> String {
        self.times.retain(|t| now - t < 60.0);
        if let Some(last) = self.times.last()
            && now - last < self.gap
        {
            return format!("rate limited: wait {:.1} s (at most one alert a second)", self.gap - (now - last));
        }
        if self.times.len() >= self.per_min {
            return format!("rate limited: {} alerts in the last minute — wait a bit", self.per_min);
        }
        self.times.push(now);
        String::new()
    }
}

/// "Tests passed: 120 of 120, 3 skipped" -> a title that fits the stick (40) and the rest.
pub fn split_title(text: &str, width: usize) -> (String, String) {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return (text, String::new());
    }
    let head: String = chars[..width].iter().collect();
    let cut = head.rfind(' ').map(|i| head[..i].chars().count()).filter(|&c| c > width / 2).unwrap_or(width);
    let title: String = chars[..cut].iter().collect();
    let rest: String = chars[cut..].iter().collect();
    (title.trim_end_matches([' ', ',', ';', ':', '-']).to_string(), rest.trim().to_string())
}

/// Who's calling: the voice session sends `X-Stick-Client: voice`; terminal installs send
/// `X-Stick-Cwd: ${PWD}` (expanded by Claude Code).
pub fn caller(headers: &HeaderMap) -> String {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).unwrap_or("");
    if h("x-stick-client") == "voice" {
        return "Agent".into();
    }
    let cwd = h("x-stick-cwd");
    if !cwd.is_empty() && !cwd.contains("${") {
        let cwd = percent_decode(cwd);
        if let Some(name) = cwd.trim_end_matches('/').rsplit('/').next().filter(|n| !n.is_empty()) {
            return stick_text(name).chars().take(24).collect();
        }
    }
    "Claude Code".into()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The tools' shared state (in [`AppState`]).
pub struct McpState {
    pub alerts: Mutex<RateLimit>,
    pub calls: Mutex<u64>,
    pub last_call: Mutex<Value>,
    /// legacy clients cancel with `notifications/cancelled`: (caller, request id) -> the call
    inflight: Mutex<HashMap<(String, String), tokio::task::AbortHandle>>,
    /// the last home-card push, and a later one already planned
    card_at: Mutex<f64>,
    card_later: std::sync::atomic::AtomicBool,
}

impl Default for McpState {
    fn default() -> Self {
        Self {
            alerts: Mutex::new(RateLimit::new(ALERT_GAP.as_secs_f64(), ALERT_PER_MIN)),
            calls: Mutex::new(0),
            last_call: Mutex::new(Value::Null),
            inflight: Mutex::new(HashMap::new()),
            card_at: Mutex::new(f64::MIN),
            card_later: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

fn mono() -> f64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

/// A tool's outcome: `(text, is_error, structured)`.
pub type Outcome = (String, bool, Option<Value>);

/// Runs a tool (mcp_server.py `StickTools.call`).
pub async fn call(st: &Arc<AppState>, name: &str, args: &Value, who: &str) -> Outcome {
    let started = Instant::now();
    let out = match tool(name) {
        None => Err(format!("unknown tool: {name}")),
        Some(t) => match check_args(&t["inputSchema"], args) {
            Err(e) => Err(e),
            Ok(_) if name != "stick_status" && !CALENDAR_TOOLS.contains(&name) && !st.link.connected() => {
                Err(NOT_CONNECTED.to_string())
            }
            Ok(a) => run_tool(st, name, &a, who).await,
        },
    };
    let (text, err, structured) = match out {
        Ok((t, s)) => (t, false, s),
        Err(e) => (e, true, None),
    };
    let ms = started.elapsed().as_millis() as u64;
    *st.mcp.calls.lock().unwrap() += 1;
    *st.mcp.last_call.lock().unwrap() = json!({"t": crate::hub::now(), "tool": name, "from": who});
    // what the Calendar tools answer is the person's day: the log and the activity feed keep that it
    // was asked, not the meetings
    let shown: String = if CALENDAR_TOOLS.contains(&name) { "(from your calendar)".into() } else { text.clone() };
    let short: String = shown.chars().take(80).collect();
    log::info!("{who}: {name}({}) -> {}{short} ({ms} ms)", summarize(args, 40), if err { "error: " } else { "" });
    st.hub.emit(
        "mcp",
        json!({"tool": name, "origin": who, "args": summarize(args, 60),
               "result": shown.chars().take(200).collect::<String>(), "ok": !err, "ms": ms}),
    );
    (text, err, structured)
}

/// [`call`] for a brain without Claude Code's permission prompt (OpenRouter): the Mac tools ask on
/// the stick first (A / B), the same prompt Claude Code's hook shows.
pub async fn call_approved(st: &Arc<AppState>, name: &str, args: &Value, who: &str) -> Outcome {
    if MAC_TOOLS.contains(&name) {
        let hint: String = approval_hint(name, args).chars().take(300).collect();
        let decision = crate::approvals::ask_permission(st, name, &hint, who, crate::approvals::WAIT).await;
        if decision.as_deref() != Some("allow") {
            let off = !st.cfg.lock().unwrap().get("stick_approvals").and_then(Value::as_bool).unwrap_or(true);
            let why = match decision.as_deref() {
                None if off => "needs the user's approval, but stick approvals are turned off (dashboard → Settings)",
                Some("deny") => "the user didn't approve it",
                _ => "no answer on the stick",
            };
            log::info!("{who}: {name}({}) not run: {why}", summarize(args, 40));
            st.hub.emit(
                "mcp",
                json!({"tool": name, "origin": who, "args": summarize(args, 60), "result": why,
                                      "ok": false, "ms": 0}),
            );
            return (why.to_string(), true, None);
        }
    }
    call(st, name, args, who).await
}

type ToolResult = Result<(String, Option<Value>), String>;

async fn run_tool(st: &Arc<AppState>, name: &str, a: &Map<String, Value>, who: &str) -> ToolResult {
    let s = |k: &str| a.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match name {
        "notify" => {
            let level = a.get("level").and_then(Value::as_str).unwrap_or("info");
            let sound = a.get("sound").and_then(Value::as_bool).unwrap_or(level != "info");
            let lvl = match level {
                "celebrate" => "celebrate",
                "heads_up" => "headsup",
                _ => "nice",
            };
            let t = s("text");
            let t = if t.trim().is_empty() { "Ping".to_string() } else { t.trim().to_string() };
            event(st, lvl, &t, who, sound).await.map(|r| (r, None))
        }
        "celebrate" => {
            let t = s("text");
            let t = if t.trim().is_empty() { "Done!".to_string() } else { t.trim().to_string() };
            event(st, "celebrate", &t, who, true).await.map(|r| (r, None))
        }
        "ask" => {
            let question = s("question").split_whitespace().collect::<Vec<_>>().join(" ");
            if question.is_empty() {
                return Err("the question is empty".into());
            }
            let options: Vec<String> = match a.get("options").and_then(Value::as_array) {
                Some(o) => o
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|o| o.split_whitespace().collect::<Vec<_>>().join(" "))
                    .collect(),
                None => vec!["Yes".into(), "No".into()],
            };
            if options.iter().any(String::is_empty) {
                return Err("options can't be empty".into());
            }
            if options.len() == 2 && options[0].to_lowercase() == options[1].to_lowercase() {
                return Err("the two options must differ".into());
            }
            let secs = a.get("timeout_s").and_then(Value::as_u64).unwrap_or(60);
            let answer = crate::approvals::ask_user(st, &question, &options, Duration::from_secs(secs), who).await;
            if answer == NOT_CONNECTED {
                return Err(answer);
            }
            Ok((answer.clone(), Some(json!({"answer": answer}))))
        }
        "progress" => {
            let (done, total) = (a.get("done").and_then(Value::as_u64), a.get("total").and_then(Value::as_u64));
            if let (Some(d), Some(t)) = (done, total)
                && d > t
            {
                return Err("done can't be more than total".into());
            }
            let head = match (done, total) {
                (Some(d), Some(t)) => format!("{who}: {d}/{t} ({}%)", d * 100 / t),
                (Some(d), None) => format!("{who}: {d} done"),
                _ => format!("{who}: working"),
            };
            let finished = matches!((done, total), (Some(d), Some(t)) if d >= t);
            let secs = if finished { PROGRESS_DONE_S } else { PROGRESS_S };
            card(st, "progress", &head, stick_text(&s("text")).trim(), secs).await;
            Ok(("shown on the home screen".into(), None))
        }
        "pin_home" => {
            let text = stick_text(&s("text")).trim().to_string();
            if text.is_empty() {
                let had = crate::home::clear(st, "pin");
                crate::home::push_home(st).await;
                return Ok((if had { "pin removed" } else { "nothing was pinned" }.into(), None));
            }
            let mins = a.get("minutes").and_then(Value::as_u64).unwrap_or(30);
            card(st, "pin", &format!("Pinned · {who}"), &text, mins as f64 * 60.0).await;
            Ok((format!("pinned for {mins} min"), None))
        }
        "speak" => {
            let text = s("text").split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() {
                return Err("nothing to say".into());
            }
            let cfg = st.cfg.lock().unwrap().clone();
            if crate::events::quiet_now(&cfg) {
                return Ok(("quiet hours: not spoken".into(), None));
            }
            let why = st.mcp.alerts.lock().unwrap().take(mono());
            if !why.is_empty() {
                return Err(why);
            }
            let out = crate::speak::output(st);
            crate::speak::say(st, &text, true).await;
            Ok((format!("speaking on the {out}"), None))
        }
        "stick_status" => Ok(stick_status(st).await),
        "ir_list" => {
            let devices = crate::ir::listing(st).await?;
            if devices.is_empty() {
                return Ok((
                    "no remotes yet: learn one on the stick (menu: Remote) or in the dashboard".into(),
                    Some(json!({"devices": {}})),
                ));
            }
            let text = devices
                .iter()
                .map(|(d, b)| format!("{d}: {}", if b.is_empty() { "(no buttons)".to_string() } else { b.join(", ") }))
                .collect::<Vec<_>>()
                .join("; ");
            let map: Map<String, Value> = devices.into_iter().map(|(d, b)| (d, json!(b))).collect();
            Ok((text, Some(json!({"devices": map}))))
        }
        "ir_send" => {
            let repeat = a.get("repeat").and_then(Value::as_i64).unwrap_or(0);
            crate::ir::send(st, &s("device"), &s("button"), repeat).await.map(|t| (t, None))
        }
        "calendar_next" => crate::connections::calendar::answer_next(st).map(|t| (t, None)),
        "calendar_today" => {
            let tomorrow = a.get("day").and_then(Value::as_str) == Some("tomorrow");
            crate::connections::calendar::day_lines(st, tomorrow).map(|t| (t, None))
        }
        "calendar_join" => crate::connections::calendar::open_join(st).await.map(|t| (t, None)),
        "open_url" => {
            let url = s("url").trim().to_string();
            let host = url
                .strip_prefix("https://")
                .or_else(|| url.strip_prefix("http://"))
                .map(|r| r.split(['/', '?', '#']).next().unwrap_or(""))
                .unwrap_or("");
            if host.is_empty() || url.chars().any(|c| (c as u32) <= 0x20 || c as u32 == 0x7f) {
                return Err("only http:// or https:// URLs (no spaces or control characters)".into());
            }
            let host = host.to_string();
            run(st, crate::commands::Action::Open(vec![url])).await;
            Ok((format!("opened {host}"), None))
        }
        "open_app" => {
            let n = s("name").trim().to_string();
            let ok = !n.is_empty() && n.chars().all(|c| c.is_alphanumeric() || " _.&+'()-".contains(c));
            if !ok {
                return Err("not an app name".into());
            }
            let apps = st.apps.clone();
            let n2 = n.clone();
            let app = tokio::task::spawn_blocking(move || apps.find(&n2)).await.ok().flatten();
            let Some(app) = app else { return Err(format!("no app called '{n}' in /Applications")) };
            run(st, crate::commands::Action::Open(vec!["-a".into(), app.clone()])).await;
            Ok((format!("opened {app}"), None))
        }
        "quick_command" => {
            let phrase = s("phrase").split_whitespace().collect::<Vec<_>>().join(" ");
            let Some((cmd, slots)) = st.commands.find(&phrase) else {
                return Err("no quick command matches that phrase".into());
            };
            let (apps, runner) = (st.apps.clone(), st.runner());
            let c2 = cmd.clone();
            let result = tokio::task::spawn_blocking(move || crate::commands::run(&c2, &slots, true, &apps, &runner))
                .await
                .ok()
                .flatten();
            match result {
                Some(r) => Ok((stick_text(&r).chars().take(200).collect(), None)),
                None => Err(format!(
                    "{} doesn't apply here",
                    cmd.get("name").and_then(Value::as_str).unwrap_or("that command")
                )),
            }
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

async fn run(st: &Arc<AppState>, a: crate::commands::Action) {
    let runner = st.runner();
    let _ = tokio::task::spawn_blocking(move || runner(&a)).await;
}

/// notify / celebrate: an event on the stick (events.py `notify`, kind `claude_tool`).
async fn event(st: &Arc<AppState>, level: &str, text: &str, who: &str, sound: bool) -> Result<String, String> {
    let cfg = st.cfg.lock().unwrap().clone();
    if cfg.get("event_rules").and_then(|r| r.get("claude_tool")).and_then(Value::as_str) == Some("off") {
        return Err("the user turned notifications from Claude sessions off (dashboard → Events)".into());
    }
    let why = st.mcp.alerts.lock().unwrap().take(mono());
    if !why.is_empty() {
        return Err(why);
    }
    let (title, rest) = split_title(&stick_text(text), 40);
    let sub = if rest.is_empty() { format!("from {who}") } else { rest };
    let ev = crate::events::notify(st, "claude_tool", &title, &sub, Some("claude"), Some(level), (!sound).then_some("off"))
        .await
        .ok_or_else(|| NOT_CONNECTED.to_string())?;
    let quiet = ev["sound"] == "off" && sound;
    Ok(format!("shown{}", if quiet { " (quiet hours: no sound)" } else { "" }))
}

/// progress / pin_home: the home card's slot, pushed to the stick at most every 0.5 s (the newest
/// line a little later, coalesced; the rest rides on the next hello).
async fn card(st: &Arc<AppState>, slot: &str, a: &str, b: &str, secs: f64) {
    crate::home::put(st, slot, a, b, secs);
    let now = mono();
    let push_now = {
        let mut at = st.mcp.card_at.lock().unwrap();
        if now - *at >= CARD_PUSH_GAP.as_secs_f64() {
            *at = now;
            true
        } else {
            false
        }
    };
    if push_now {
        crate::home::push_home(st).await;
    } else if !st.mcp.card_later.swap(true, std::sync::atomic::Ordering::Relaxed) {
        let st = st.clone();
        tokio::spawn(async move {
            tokio::time::sleep(CARD_PUSH_GAP).await;
            st.mcp.card_later.store(false, std::sync::atomic::Ordering::Relaxed);
            *st.mcp.card_at.lock().unwrap() = mono();
            crate::home::push_home(&st).await;
        });
    }
}

/// The stick's status (core.py `stick_report`): `{"cmd":"status"}` -> `{"ack":"status","data"}`.
pub async fn stick_report(st: &Arc<AppState>, wait: Duration) -> Option<Value> {
    let mut rx = st.link.messages();
    if !st.link.send(&json!({"cmd": "status"})).await {
        return None;
    }
    tokio::time::timeout(wait, async {
        loop {
            match rx.recv().await {
                Ok(m) if m.get("ack").and_then(Value::as_str) == Some("status") => {
                    return m.get("data").cloned().unwrap_or(json!({}));
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return json!({}),
            }
        }
    })
    .await
    .ok()
}

fn ago(s: u64) -> String {
    if s < 90 {
        format!("{s} s")
    } else if s < 5400 {
        format!("{} min", s / 60)
    } else {
        format!("{} h", s / 3600)
    }
}

async fn stick_status(st: &Arc<AppState>) -> (String, Option<Value>) {
    let connected = st.link.connected();
    let mut info = json!({"connected": connected});
    let usb = st.wmic.stick_usb().unwrap_or(false);
    if connected {
        let rep = stick_report(st, Duration::from_secs(2)).await.unwrap_or(json!({}));
        info["battery_pct"] = rep["bat"]["pct"].clone();
        info["usb_power"] = rep["bat"]["usb"].clone();
        info["plugged_into_computer"] = json!(st.wmic.stick_usb());
        info["uptime_s"] = rep["sys"]["up"].clone();
    }
    let ago_s = st.last_input().map(|t| (crate::hub::now() - t).max(0.0) as u64);
    info["last_button_press_s_ago"] = json!(ago_s);
    let state = st.agent_state();
    let screen = if st.approvals.pending() {
        "an approval prompt".to_string()
    } else if st.approvals.asking() {
        "a question".to_string()
    } else if matches!(state.as_str(), "listening" | "transcribing" | "thinking" | "working") {
        format!("the agent ({state})")
    } else {
        "the home screen (or asleep)".to_string()
    };
    info["screen"] = if connected { json!(screen) } else { Value::Null };
    let desk = if !connected {
        "unknown (the stick isn't connected — it may be off, asleep or out of range)"
    } else if usb || ago_s.is_some_and(|a| a < 300) {
        "likely"
    } else if ago_s.is_none_or(|a| a > 1800) {
        "maybe not (no button pressed for a while)"
    } else {
        "unknown"
    };
    info["at_desk"] = json!(desk);
    if !connected {
        return ("stick not connected (it may be off, asleep or out of range)".into(), Some(info));
    }
    let mut parts = vec!["connected".to_string()];
    if let Some(p) = info["battery_pct"].as_u64() {
        let power = if info["usb_power"] == true { " (charging / on USB power)" } else { "" };
        parts.push(format!("battery {p}%{power}"));
    }
    if usb {
        parts.push("plugged into a computer".into());
    }
    parts.push(format!("screen: {screen}"));
    parts.push(format!(
        "last button press: {}",
        ago_s.map(|a| format!("{} ago", ago(a))).unwrap_or_else(|| "not since the agent started".into())
    ));
    parts.push(format!("at the desk: {desk}"));
    (parts.join(" · "), Some(info))
}

// --- the JSON-RPC endpoint ------------------------------------------------------------------------

struct RpcError {
    code: i64,
    message: String,
    status: StatusCode,
    data: Option<Value>,
}

impl RpcError {
    fn new(code: i64, message: impl Into<String>) -> Self {
        Self { code, message: message.into(), status: StatusCode::OK, data: None }
    }
    fn status(mut self, s: StatusCode) -> Self {
        self.status = s;
        self
    }
    fn data(mut self, d: Value) -> Self {
        self.data = Some(d);
        self
    }
}

fn rpc_error(id: Option<&Value>, e: RpcError) -> Response {
    let mut err = json!({"code": e.code, "message": e.message});
    if let Some(d) = e.data {
        err["data"] = d;
    }
    let mut body = json!({"jsonrpc": "2.0", "error": err});
    if let Some(id) = id {
        body["id"] = id.clone();
    }
    (e.status, axum::Json(body)).into_response()
}

fn server_info() -> Value {
    json!({"name": SERVER_NAME, "title": "stick (M5StickS3 pager)", "version": SERVER_VERSION})
}

fn supported() -> Vec<&'static str> {
    MODERN.iter().chain(LEGACY).copied().collect()
}

fn decode_header(v: &str) -> Option<String> {
    if let Some(b) = v.strip_prefix("=?base64?").and_then(|r| r.strip_suffix("?=")) {
        return base64::engine::general_purpose::STANDARD.decode(b).ok().and_then(|b| String::from_utf8(b).ok());
    }
    Some(v.to_string())
}

/// True for a modern (2026-07-28) request, false for a legacy one.
fn era(headers: &HeaderMap, method: &str, params: &Map<String, Value>) -> Result<bool, RpcError> {
    let h = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let version = params.get("_meta").and_then(|m| m.get(format!("{META}protocolVersion"))).and_then(Value::as_str);
    let header = h("mcp-protocol-version");
    let unsupported = |req: &str| {
        RpcError::new(UNSUPPORTED_VERSION, "Unsupported protocol version")
            .status(StatusCode::BAD_REQUEST)
            .data(json!({"supported": supported(), "requested": req}))
    };
    let Some(version) = version else {
        if let Some(hd) = &header {
            if MODERN.contains(&hd.as_str()) {
                return Err(RpcError::new(
                    HEADER_MISMATCH,
                    format!("MCP-Protocol-Version {hd} but no _meta protocol version"),
                )
                .status(StatusCode::BAD_REQUEST));
            }
            if !LEGACY.contains(&hd.as_str()) {
                return Err(unsupported(hd));
            }
        }
        return Ok(false);
    };
    if !MODERN.contains(&version) {
        return Err(unsupported(version));
    }
    let mismatch = |m: String| RpcError::new(HEADER_MISMATCH, m).status(StatusCode::BAD_REQUEST);
    if header.as_deref() != Some(version) {
        return Err(mismatch(format!("Header mismatch: MCP-Protocol-Version {header:?} vs body {version:?}")));
    }
    if h("mcp-method").as_deref() != Some(method) {
        return Err(mismatch(format!("Header mismatch: Mcp-Method {:?} vs body {method:?}", h("mcp-method"))));
    }
    if method == "tools/call" {
        let want = params.get("name").and_then(Value::as_str);
        let got = h("mcp-name").and_then(|v| decode_header(&v));
        if got.as_deref() != want {
            return Err(mismatch(format!("Header mismatch: Mcp-Name {got:?} vs body {want:?}")));
        }
    }
    Ok(true)
}

fn tool_result(text: &str, err: bool, structured: Option<Value>) -> Value {
    let mut out = json!({"content": [{"type": "text", "text": text}], "isError": err});
    if let Some(s) = structured.filter(|_| !err) {
        out["structuredContent"] = s;
    }
    out
}

/// `Origin` (browsers send one) must be the dashboard's own.
pub fn ok_origin(headers: &HeaderMap, port: u16) -> bool {
    match headers.get("origin").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(o) => o == format!("http://localhost:{port}") || o == format!("http://127.0.0.1:{port}"),
    }
}

/// `POST /mcp`.
pub async fn post(st: Arc<AppState>, headers: HeaderMap, body: &[u8]) -> Response {
    if !ok_origin(&headers, st.port) {
        return rpc_error(None, RpcError::new(INVALID_REQUEST, "bad origin").status(StatusCode::FORBIDDEN));
    }
    if !st.cfg.lock().unwrap().get("mcp_server").and_then(Value::as_bool).unwrap_or(true) {
        return rpc_error(
            None,
            RpcError::new(INTERNAL, "the stick tools are turned off in the StickyBot dashboard")
                .status(StatusCode::SERVICE_UNAVAILABLE),
        );
    }
    let bad = |m: &str, code| rpc_error(None, RpcError::new(code, m).status(StatusCode::BAD_REQUEST));
    let Ok(msg) = serde_json::from_slice::<Value>(body) else { return bad("parse error", PARSE_ERROR) };
    if msg.is_array() {
        return bad("batches aren't supported", INVALID_REQUEST);
    }
    let Some(obj) = msg.as_object().filter(|o| o.get("jsonrpc").and_then(Value::as_str) == Some("2.0")) else {
        return bad("not a JSON-RPC 2.0 message", INVALID_REQUEST);
    };
    let id = obj.get("id");
    let Some(method) = obj.get("method") else {
        if obj.contains_key("result") || obj.contains_key("error") {
            return StatusCode::ACCEPTED.into_response(); // a response from a legacy client
        }
        return rpc_error(id, RpcError::new(INVALID_REQUEST, "no method").status(StatusCode::BAD_REQUEST));
    };
    let params = match obj.get("params") {
        None | Some(Value::Null) => Some(Map::new()),
        Some(Value::Object(p)) => Some(p.clone()),
        _ => None,
    };
    let (Some(method), Some(params)) = (method.as_str(), params) else {
        return rpc_error(id, RpcError::new(INVALID_REQUEST, "bad method or params").status(StatusCode::BAD_REQUEST));
    };
    let who = caller(&headers);
    let Some(id) = id.cloned() else {
        // a notification
        if method == "notifications/cancelled" {
            let rid = match params.get("requestId") {
                Some(Value::String(s)) => s.clone(),
                Some(v) => v.to_string(),
                None => String::new(),
            };
            if let Some(h) = st.mcp.inflight.lock().unwrap().remove(&(who.clone(), rid)) {
                h.abort();
            }
        }
        return StatusCode::ACCEPTED.into_response();
    };
    let modern = match era(&headers, method, &params) {
        Ok(m) => m,
        Err(e) => return rpc_error(Some(&id), e),
    };
    let reply = move |mut result: Value| {
        if modern {
            let mut m = Map::new();
            m.insert("resultType".into(), json!("complete"));
            if let Value::Object(r) = result {
                m.extend(r);
            }
            result = Value::Object(m);
        }
        json!({"jsonrpc": "2.0", "id": id, "result": result})
    };
    let id_ref = obj.get("id");
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if LEGACY.contains(&asked) { asked } else { LEGACY[0] };
            json!({"protocolVersion": version, "capabilities": {"tools": {"listChanged": false}},
                   "serverInfo": server_info(), "instructions": INSTRUCTIONS})
        }
        "server/discover" => {
            json!({"supportedVersions": supported(), "capabilities": {"tools": {"listChanged": false}},
                                    "_meta": {format!("{META}serverInfo"): server_info()}, "instructions": INSTRUCTIONS})
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools()}),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            if tool(&name).is_none() {
                return rpc_error(id_ref, RpcError::new(INVALID_PARAMS, format!("Unknown tool: {name}")));
            }
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            let accept = headers.get("accept").and_then(|v| v.to_str().ok()).unwrap_or("");
            let key = (
                who.clone(),
                match id_ref {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => String::new(),
                },
            );
            let st2 = st.clone();
            let who2 = who.clone();
            let task = tokio::spawn(async move { call(&st2, &name, &args, &who2).await });
            st.mcp.inflight.lock().unwrap().insert(key.clone(), task.abort_handle());
            if params.get("name").and_then(Value::as_str) == Some("ask") && accept.contains("text/event-stream") {
                return streamed(st.clone(), task, key, reply).into_response();
            }
            let r = task.await;
            st.mcp.inflight.lock().unwrap().remove(&key);
            match r {
                Ok((text, err, structured)) => tool_result(&text, err, structured),
                Err(_) => return rpc_error(id_ref, RpcError::new(INTERNAL, "cancelled")),
            }
        }
        _ => {
            let status = if modern { StatusCode::NOT_FOUND } else { StatusCode::OK };
            return rpc_error(
                id_ref,
                RpcError::new(METHOD_NOT_FOUND, format!("Method not found: {method}")).status(status),
            );
        }
    };
    axum::Json(reply(result)).into_response()
}

/// Stops the call when the SSE stream goes away (the client closed it).
struct AbortOnDrop(tokio::task::AbortHandle, Arc<AppState>, (String, String));

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
        self.1.mcp.inflight.lock().unwrap().remove(&self.2);
    }
}

/// `ask` as an SSE stream: keep-alives while the user decides; the client closing it cancels.
fn streamed(
    st: Arc<AppState>,
    task: tokio::task::JoinHandle<Outcome>,
    key: (String, String),
    reply: impl FnOnce(Value) -> Value + Send + 'static,
) -> Sse<impl futures_util::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let guard = AbortOnDrop(task.abort_handle(), st, key);
    let stream = futures_util::stream::once(async move {
        let _guard = guard;
        let body = match task.await {
            Ok((text, err, structured)) => reply(tool_result(&text, err, structured)),
            Err(_) => json!({"jsonrpc": "2.0", "error": {"code": INTERNAL, "message": "cancelled"}}),
        };
        Ok(Event::default().event("message").data(body.to_string()))
    });
    Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE).text("keep-alive"))
}

// --- Claude Code's config for the voice session, and terminal sessions ----------------------------

/// The `--mcp-config` file for this app's own voice session (holds the token: 0600).
pub fn brain_config(home: &std::path::Path, port: u16, token: &str) -> Option<std::path::PathBuf> {
    let path = home.join("mcp-voice.json");
    let cfg = json!({"mcpServers": {SERVER_NAME: {"type": "http", "url": url(port),
        "headers": {"Authorization": format!("Bearer {token}"), "X-Stick-Client": "voice"}}}});
    let text = serde_json::to_string_pretty(&cfg).ok()?;
    if std::fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
        return Some(path);
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    use std::io::Write;
    o.open(&path).and_then(|mut f| f.write_all(text.as_bytes())).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(path)
}

pub fn url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/mcp")
}

/// `claude mcp add` for terminal sessions (mcp_setup.py `add_args`): Claude Code writes its own
/// config; `${PWD}` is left for Claude Code to expand.
pub fn add_args(port: u16, token: &str) -> Vec<String> {
    ["mcp", "add", "--transport", "http", "--scope", "user", SERVER_NAME]
        .iter()
        .map(|s| s.to_string())
        .chain([
            url(port),
            "--header".into(),
            format!("Authorization: Bearer {token}"),
            "--header".into(),
            "X-Stick-Cwd: ${PWD}".into(),
        ])
        .collect()
}

pub fn remove_args() -> Vec<String> {
    ["mcp", "remove", "--scope", "user", SERVER_NAME].iter().map(|s| s.to_string()).collect()
}

pub fn get_args() -> Vec<String> {
    ["mcp", "get", SERVER_NAME].iter().map(|s| s.to_string()).collect()
}

/// The command for people to read (the token replaced).
pub fn shown(args: &[String], token: &str) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|a| {
            let a = if a.starts_with("Authorization:") { a.replace(token, "<token>") } else { a.clone() };
            if a.chars().any(|c| " ${}".contains(c)) { format!("\"{a}\"") } else { a }
        })
        .collect();
    format!("claude {}", parts.join(" "))
}

/// Runs `claude …` (a list of arguments, never a shell); `(ok, stdout + stderr)`.
pub type ClaudeRunner = Arc<dyn Fn(&std::path::Path, &[String]) -> (bool, String) + Send + Sync>;

pub fn system_claude_runner() -> ClaudeRunner {
    Arc::new(|claude, args| match std::process::Command::new(claude).args(args).output() {
        Ok(o) => (
            o.status.success(),
            format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)).trim().to_string(),
        ),
        Err(e) => (false, e.to_string()),
    })
}

/// Are the stick tools in Claude Code's user config? (mcp_setup.py `status`)
pub fn terminal_status(claude: Option<&std::path::Path>, port: u16, token: &str, run: &ClaudeRunner) -> Value {
    let command = shown(&add_args(port, token), token);
    let Some(claude) = claude else {
        return json!({"installed": false, "detail": "Claude Code not found", "command": command});
    };
    let (ok, out) = run(claude, &get_args());
    let ours = out.contains(&url(port));
    let detail = if ok && ours {
        "added (user scope)".to_string()
    } else if ok {
        "a different \"stick\" server is configured".to_string()
    } else {
        "not added".to_string()
    };
    json!({"installed": ok && ours, "detail": detail, "command": command})
}

/// `claude mcp add|remove` for terminal sessions; `(ok, message, what ran)`.
pub fn terminal(
    action: &str,
    claude: Option<&std::path::Path>,
    port: u16,
    token: &str,
    run: &ClaudeRunner,
) -> (bool, String, Vec<String>) {
    let Some(claude) = claude else { return (false, "Claude Code not found".into(), Vec::new()) };
    let args = if action == "install" { add_args(port, token) } else { remove_args() };
    if action == "install" {
        let (ok, out) = run(claude, &get_args());
        if ok && out.contains(&url(port)) {
            return (true, "already added".into(), Vec::new());
        }
        if ok {
            // someone else's "stick" server: replace it with ours
            run(claude, &remove_args());
        }
    }
    let (ok, out) = run(claude, &args);
    let ran = vec![shown(&args, token)];
    let msg = if ok {
        if action == "install" { "added for all your Claude Code sessions" } else { "removed" }.to_string()
    } else {
        out.lines().last().unwrap_or("failed").to_string()
    };
    (ok, msg, ran)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_args_cuts_long_text_and_fills_defaults() {
        let schema = &tool("ask").unwrap()["inputSchema"];
        let out = check_args(schema, &json!({"question": "q".repeat(500), "options": ["x".repeat(40)]})).unwrap();
        assert_eq!(out["question"].as_str().unwrap().len(), 160);
        assert_eq!(out["options"], json!(["x".repeat(16)]));
        assert_eq!(out["timeout_s"], 60);
        assert!(
            check_args(schema, &json!({"question": "q", "timeout_s": 1})).unwrap_err().contains("between 5 and 120")
        );
        assert!(
            check_args(schema, &json!({"question": "q", "options": ["a", "b", "c"]}))
                .unwrap_err()
                .contains("1-2 items")
        );
        assert!(check_args(schema, &json!({"question": "q", "timeout_s": true})).unwrap_err().contains("integer"));
        assert_eq!(check_args(schema, &json!({"question": "q", "timeout_s": 10.0})).unwrap()["timeout_s"], 10);
        let n = &tool("notify").unwrap()["inputSchema"];
        for (args, why) in [
            (json!({}), "missing argument: text"),
            (json!({"text": 5}), "text must be a string"),
            (json!({"text": "hi", "level": "loud"}), "level must be one of"),
            (json!({"text": "hi", "sound": "yes"}), "sound must be true or false"),
            (json!({"text": "hi", "colour": "red"}), "unknown argument(s): colour"),
            (json!("x"), "arguments must be an object"),
        ] {
            assert!(check_args(n, &args).unwrap_err().contains(why), "{args}");
        }
    }

    #[test]
    fn rate_limit_window() {
        let mut r = RateLimit::new(1.0, 3);
        assert_eq!(r.take(0.0), "");
        assert!(r.take(0.5).starts_with("rate limited"));
        assert_eq!(r.take(1.0), "");
        assert_eq!(r.take(2.0), "");
        assert!(r.take(3.0).contains("3 alerts in the last minute"));
        assert_eq!(r.take(61.0), "");
    }

    #[test]
    fn titles_callers_and_quiet_hours() {
        assert_eq!(
            split_title("Build failed: 3 errors in the parser module, see the log", 40),
            ("Build failed: 3 errors in the parser".into(), "module, see the log".into())
        );
        assert_eq!(split_title("Tests  passed", 40), ("Tests passed".into(), String::new()));
        let mut h = HeaderMap::new();
        assert_eq!(caller(&h), "Claude Code");
        h.insert("x-stick-cwd", "/Users/me/src/web".parse().unwrap());
        assert_eq!(caller(&h), "web");
        h.insert("x-stick-cwd", "${PWD}".parse().unwrap());
        assert_eq!(caller(&h), "Claude Code");
        h.insert("x-stick-client", "voice".parse().unwrap());
        assert_eq!(caller(&h), "Agent");
    }

    #[test]
    fn tools_and_names() {
        let names: Vec<&str> = tools().iter().filter_map(|t| t["name"].as_str()).collect();
        assert_eq!(
            names,
            [
                "notify",
                "ask",
                "celebrate",
                "progress",
                "pin_home",
                "open_url",
                "open_app",
                "quick_command",
                "stick_status",
                "speak",
                "ir_list",
                "ir_send",
                "calendar_next",
                "calendar_today",
                "calendar_join"
            ]
        );
        assert_eq!(allowed_tool_names()[0], "mcp__stick__notify");
        assert_eq!(openai_tools()[0]["function"]["name"], "notify");
        assert_eq!(approval_hint("open_url", &json!({"url": "https://x"})), "https://x");
        assert_eq!(summarize(&json!({"text": "a"}), 60), "text='a'");
    }

    #[test]
    fn the_calendar_tools_ask_only_where_they_change_something() {
        // reading the day the person connected is pre-allowed for the voice session; opening a link asks first
        let allowed = allowed_tool_names();
        assert!(allowed.contains(&"mcp__stick__calendar_next".to_string()));
        assert!(allowed.contains(&"mcp__stick__calendar_today".to_string()));
        assert!(!allowed.contains(&"mcp__stick__calendar_join".to_string()));
        assert!(MAC_TOOLS.contains(&"calendar_join") && !MAC_TOOLS.contains(&"calendar_next"));
        assert_eq!(approval_hint("calendar_join", &json!({})), "the link of your meeting");
        // the OpenRouter brain gets the same definitions
        let names: Vec<String> =
            openai_tools().iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        for tool in CALENDAR_TOOLS {
            assert!(names.contains(&tool.to_string()), "{tool}");
        }
        let today = &tool("calendar_today").unwrap()["inputSchema"];
        assert_eq!(check_args(today, &json!({})).unwrap()["day"], "today");
        assert!(check_args(today, &json!({"day": "next week"})).unwrap_err().contains("one of: today, tomorrow"));
        assert!(check_args(&tool("calendar_next").unwrap()["inputSchema"], &json!({"day": "today"})).is_err());
    }

    // --- the endpoint on the fake stick (test_mcp.py) ---------------------------------------------

    use crate::talk::tests::{T, agent};
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    struct C {
        t: T,
        app: axum::Router,
        n: u64,
    }

    impl C {
        async fn new(name: &str) -> Self {
            let t = agent(name, false).await;
            t.st.mcp.alerts.lock().unwrap().gap = 0.0; // tests call alerts back to back
            let app = crate::api::router(t.st.clone());
            C { t, app, n: 0 }
        }

        async fn raw(&self, method: &str, body: &str, headers: &[(&str, &str)]) -> (StatusCode, Value, String) {
            let mut b = axum::http::Request::builder().method(method).uri("/mcp").header("host", "localhost:8670");
            for (k, v) in headers {
                b = b.header(*k, *v);
            }
            let resp = self.app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
            let status = resp.status();
            let ctype = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
            let text = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
            let v = if ctype.contains("event-stream") {
                let data = text.lines().filter_map(|l| l.strip_prefix("data: ")).next_back().unwrap_or("null");
                serde_json::from_str(data).unwrap()
            } else {
                serde_json::from_str(&text).unwrap_or(Value::Null)
            };
            (status, v, ctype)
        }

        async fn rpc(
            &mut self,
            method: &str,
            params: Value,
            headers: &[(&str, &str)],
            modern: bool,
        ) -> (StatusCode, Value) {
            let auth = format!("Bearer {}", self.t.st.token);
            let mut params = params;
            let mut hdrs: Vec<(String, String)> =
                vec![("authorization".into(), auth), ("accept".into(), "application/json, text/event-stream".into())];
            if modern {
                params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": MODERN[0]});
                hdrs.push(("mcp-protocol-version".into(), MODERN[0].into()));
                hdrs.push(("mcp-method".into(), method.into()));
                if method == "tools/call" {
                    hdrs.push(("mcp-name".into(), params["name"].as_str().unwrap_or("").into()));
                }
            }
            for (k, v) in headers {
                hdrs.retain(|(hk, _)| !hk.eq_ignore_ascii_case(k));
                hdrs.push((k.to_string(), v.to_string()));
            }
            self.n += 1;
            let msg = json!({"jsonrpc": "2.0", "id": self.n, "method": method, "params": params});
            let h: Vec<(&str, &str)> = hdrs.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            let (s, v, _) = self.raw("POST", &msg.to_string(), &h).await;
            (s, v)
        }

        async fn call(&mut self, name: &str, args: Value, headers: &[(&str, &str)]) -> (String, bool, Value) {
            let (s, body) = self.rpc("tools/call", json!({"name": name, "arguments": args}), headers, false).await;
            assert_eq!(s, StatusCode::OK, "{body}");
            let r = body["result"].clone();
            (r["content"][0]["text"].as_str().unwrap().to_string(), r["isError"].as_bool().unwrap(), r)
        }

        fn sent(&self) -> Vec<Value> {
            String::from_utf8(self.t.fake.writes().concat())
                .unwrap()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        }

        fn events(&self) -> Vec<Value> {
            self.sent().into_iter().filter_map(|m| m.get("event").cloned()).collect()
        }

        fn asks_on_stick(&self) -> Vec<Value> {
            self.sent().into_iter().filter(|m| m["src"] == "cli").filter_map(|m| m.get("prompt").cloned()).collect()
        }
    }

    #[tokio::test]
    async fn the_calendar_tools_over_the_endpoint() {
        use macos_calendar::fake::FakeCalendar;
        let mut c = C::new("mcp-calendar").await;
        let cal = FakeCalendar::new();
        cal.set_events(vec![macos_calendar::Event {
            id: "s".into(),
            title: "Standup".into(),
            start: 1_800_000_900,
            end: 1_800_001_800,
            url: "https://meet.google.com/abc-defg-hij".into(),
            ..Default::default()
        }]);
        c.t.st.calendar.set_source(Arc::new(cal));
        c.t.st.calendar.set_clock(1_800_000_180, 0);
        let (_, v) = c.rpc("tools/list", json!({}), &[], false).await;
        let listed: Vec<&str> = v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        for tool in ["calendar_next", "calendar_today", "calendar_join"] {
            assert!(listed.contains(&tool), "{tool}");
        }
        // off: a tool error in words, not a protocol error
        let (text, err, _) = c.call("calendar_next", json!({}), &[]).await;
        assert!(err && text == crate::connections::calendar::NOT_CONNECTED, "{text}");
        crate::connections::calendar::connect(&c.t.st).await.unwrap();
        let (text, err, r) = c.call("calendar_next", json!({}), &[]).await;
        assert_eq!((text.as_str(), err), ("Standup in 12 min, at 08:15.", false), "{r}");
        let (text, err, _) = c.call("calendar_join", json!({}), &[]).await;
        assert_eq!((text.as_str(), err), ("Opened Google Meet for Standup.", false));
        assert_eq!(*c.t.ran.lock().unwrap(), vec![crate::commands::Action::Open(vec!["https://meet.google.com/abc-defg-hij".into()])]);
        // the activity feed and the log keep that it was asked, not the answer
        assert_eq!(c.t.st.mcp.last_call.lock().unwrap()["tool"], "calendar_join");
    }

    #[tokio::test]
    async fn guards_token_origin_methods_and_switch() {
        let c = C::new("mcp-guard").await;
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let tok = c.t.st.token.clone();
        for h in [vec![], vec![("authorization", "Bearer wrong")], vec![("authorization", tok.as_str())]] {
            assert_eq!(c.raw("POST", ping, &h).await.0, StatusCode::FORBIDDEN);
        }
        assert_eq!(c.raw("GET", "", &[]).await.0, StatusCode::FORBIDDEN); // even a GET
        let (s, v, _) = c.raw("POST", ping, &[("x-stick-token", &tok)]).await;
        assert_eq!((s, v["result"].clone()), (StatusCode::OK, json!({})));
        let bearer = format!("Bearer {tok}");
        let auth = [("authorization", bearer.as_str())];
        assert_eq!(c.raw("GET", "", &auth).await.0, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(c.raw("DELETE", "", &auth).await.0, StatusCode::METHOD_NOT_ALLOWED);
        let evil = [auth[0], ("origin", "https://evil.example")];
        assert_eq!(c.raw("POST", ping, &evil).await.0, StatusCode::FORBIDDEN);
        let (s, v, _) = c.raw("POST", "{nope", &auth).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(PARSE_ERROR)));
        assert_eq!(c.raw("POST", &format!("[{ping}]"), &auth).await.0, StatusCode::BAD_REQUEST);
        let (s, v, _) = c.raw("POST", r#"{"id":1,"method":"ping"}"#, &auth).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(INVALID_REQUEST)));
        c.t.st.cfg.lock().unwrap().insert("mcp_server".into(), json!(false));
        let (s, v, _) = c.raw("POST", ping, &auth).await;
        assert_eq!(s, StatusCode::SERVICE_UNAVAILABLE);
        assert!(v["error"]["message"].as_str().unwrap().contains("turned off"));
    }

    #[tokio::test]
    async fn both_protocol_eras() {
        let mut c = C::new("mcp-eras").await;
        let (s, v) = c.rpc("initialize", json!({"protocolVersion": "2025-06-18"}), &[], false).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["result"]["protocolVersion"], "2025-06-18");
        assert!(v["result"].get("resultType").is_none());
        let (_, v) = c.rpc("initialize", json!({"protocolVersion": "1999-01-01"}), &[], false).await;
        assert_eq!(v["result"]["protocolVersion"], "2025-11-25");
        let bearer = format!("Bearer {}", c.t.st.token);
        let (s, _, _) = c
            .raw("POST", r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &[("authorization", &bearer)])
            .await;
        assert_eq!(s, StatusCode::ACCEPTED);
        let (_, v) = c.rpc("tools/list", json!({}), &[("mcp-protocol-version", "2025-11-25")], false).await;
        assert_eq!(v["result"]["tools"].as_array().unwrap().len(), 15);
        let (s, v) = c.rpc("tools/list", json!({}), &[("mcp-protocol-version", "1999-01-01")], false).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(UNSUPPORTED_VERSION)));
        let (s, v) = c.rpc("nope/nope", json!({}), &[], false).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::OK, Some(METHOD_NOT_FOUND)));
        // modern
        let (s, v) = c.rpc("server/discover", json!({}), &[], true).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["result"]["resultType"], "complete");
        assert_eq!(v["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "stick");
        let (s, v) = c.rpc("tools/call", json!({"name": "stick_status", "arguments": {}}), &[], true).await;
        assert_eq!((s, v["result"]["resultType"].as_str()), (StatusCode::OK, Some("complete")));
        for h in [("mcp-protocol-version", "2025-11-25"), ("mcp-method", "tools/call")] {
            let (s, v) = c.rpc("tools/list", json!({}), &[h], true).await;
            assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::BAD_REQUEST, Some(HEADER_MISMATCH)));
        }
        let (s, _) = c
            .rpc("tools/call", json!({"name": "stick_status", "arguments": {}}), &[("mcp-name", "notify")], true)
            .await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let enc = format!("=?base64?{}?=", base64::engine::general_purpose::STANDARD.encode("stick_status"));
        let (s, _) =
            c.rpc("tools/call", json!({"name": "stick_status", "arguments": {}}), &[("mcp-name", &enc)], true).await;
        assert_eq!(s, StatusCode::OK);
        let (s, v) = c.rpc("nope/nope", json!({}), &[], true).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::NOT_FOUND, Some(METHOD_NOT_FOUND)));
        let (s, v) = c.rpc("tools/call", json!({"name": "rm_rf", "arguments": {}}), &[], false).await;
        assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::OK, Some(INVALID_PARAMS)));
    }

    #[tokio::test]
    async fn notify_celebrate_rate_limit_and_switch_off() {
        let mut c = C::new("mcp-notify").await;
        c.call("notify", json!({"text": "Tests passed"}), &[]).await;
        c.call(
            "notify",
            json!({"text": "Build failed: 3 errors in the parser module, see the log", "level": "heads_up"}),
            &[("x-stick-cwd", "/Users/me/src/web")],
        )
        .await;
        c.call("celebrate", json!({}), &[("x-stick-client", "voice")]).await;
        c.call("notify", json!({"text": "Café ✅ done"}), &[]).await;
        let evs = c.events();
        let lv: Vec<(&str, &str)> =
            evs.iter().map(|e| (e["level"].as_str().unwrap(), e["sound"].as_str().unwrap())).collect();
        assert_eq!(lv, [("nice", "off"), ("headsup", ""), ("celebrate", ""), ("nice", "off")]);
        assert_eq!(
            (evs[0]["title"].as_str(), evs[0]["sub"].as_str()),
            (Some("Tests passed"), Some("from Claude Code"))
        );
        assert_eq!(evs[1]["title"], "Build failed: 3 errors in the parser");
        assert_eq!(evs[2]["title"], "Done!");
        assert_eq!(evs[2]["sub"], "from Agent");
        assert_eq!(evs[3]["title"], "Cafe done");
        let (text, err, _) = c.call("notify", json!({"text": "hi", "colour": "red"}), &[]).await;
        assert!(err && text.contains("unknown argument(s): colour"));
        *c.t.st.mcp.alerts.lock().unwrap() = RateLimit::new(1.0, 20);
        assert!(!c.call("notify", json!({"text": "one"}), &[]).await.1);
        let (text, err, _) = c.call("celebrate", json!({}), &[]).await;
        assert!(err && text.starts_with("rate limited"));
        c.t.st.cfg.lock().unwrap().insert("event_rules".into(), json!({"claude_tool": "off"}));
        let (text, err, _) = c.call("notify", json!({"text": "hi"}), &[]).await;
        assert!(err && text.contains("turned notifications from Claude sessions off"));
        assert_eq!(*c.t.st.mcp.calls.lock().unwrap(), 8);
    }

    #[tokio::test]
    async fn progress_and_pin_on_the_home_card() {
        let mut c = C::new("mcp-card").await;
        let homes = |c: &C| c.sent().into_iter().filter_map(|m| m.get("home").cloned()).collect::<Vec<_>>();
        let (_, err, _) =
            c.call("progress", json!({"text": "Running tests", "done": 40, "total": 120}), &[("x-stick-cwd", "/x/web")]).await;
        assert!(!err);
        assert_eq!(homes(&c).last().unwrap(), &json!({"k": "claude", "a": "web: 40/120 (33%)", "b": "Running tests"}));
        let (text, err, _) = c.call("progress", json!({"text": "x", "done": 5, "total": 2}), &[]).await;
        assert!(err && text.contains("more than total"));
        tokio::time::sleep(CARD_PUSH_GAP).await;
        let (text, _, _) = c.call("pin_home", json!({"text": "PR #123 waiting", "minutes": 5}), &[]).await;
        assert_eq!(text, "pinned for 5 min");
        let card = crate::home::home_card(&c.t.st);
        assert_eq!((card["a"].as_str(), card["b"].as_str()), (Some("Pinned · Claude Code"), Some("PR #123 waiting")));
        let (text, _, _) = c.call("pin_home", json!({"text": ""}), &[]).await;
        assert_eq!(text, "pin removed");
        assert_eq!(crate::home::home_card(&c.t.st)["b"], "Running tests"); // the progress line is back
        // pushes are coalesced: one now, the newest a little later
        tokio::time::sleep(CARD_PUSH_GAP).await;
        let before = homes(&c).len();
        for i in 0..5 {
            c.call("progress", json!({"text": format!("step {i}"), "done": i, "total": 10}), &[]).await;
        }
        assert_eq!(homes(&c).len(), before + 1);
        tokio::time::sleep(CARD_PUSH_GAP + Duration::from_millis(200)).await;
        assert_eq!(homes(&c).len(), before + 2);
        assert_eq!(homes(&c).last().unwrap()["b"], "step 4");
    }

    #[tokio::test]
    async fn fail_closed_without_the_stick_and_stick_status() {
        let mut c = C::new("mcp-status").await;
        c.t.fake.notify(&json!({"src": "agent", "evt": "cancel"})); // a press
        tokio::time::sleep(Duration::from_millis(50)).await;
        let (text, err, r) = c.call("stick_status", json!({}), &[]).await;
        let s = &r["structuredContent"];
        assert!(!err, "{text}");
        assert_eq!(s["battery_pct"], 87);
        assert_eq!(s["last_button_press_s_ago"], 0);
        assert_eq!(s["at_desk"], "likely");
        assert!(text.contains("battery 87%") && text.contains("screen: the home screen"), "{text}");
        c.t.st.link.pause(true);
        tokio::time::sleep(Duration::from_millis(100)).await;
        for (name, args) in [
            ("notify", json!({"text": "hi"})),
            ("ask", json!({"question": "ok?"})),
            ("open_url", json!({"url": "https://example.com"})),
            ("speak", json!({"text": "hi"})),
        ] {
            let (text, err, _) = c.call(name, args, &[]).await;
            assert!(err && text == NOT_CONNECTED, "{name}: {text}");
        }
        let (text, err, r) = c.call("stick_status", json!({}), &[]).await;
        assert!(!err && text.starts_with("stick not connected") && r["structuredContent"]["connected"] == false);
    }

    async fn answer_when_shown(fake: &stick_link::fake::FakeStick, decision: &str, n: usize) {
        for _ in 0..200 {
            let sent: Vec<Value> = String::from_utf8(fake.writes().concat())
                .unwrap()
                .lines()
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect();
            let asks: Vec<Value> =
                sent.into_iter().filter(|m| m["src"] == "cli").filter_map(|m| m.get("prompt").cloned()).collect();
            if asks.len() >= n {
                let id = asks.last().unwrap()["id"].as_str().unwrap().to_string();
                fake.notify(&json!({"cmd": "permission", "id": id, "decision": decision}));
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no question on the stick");
    }

    #[tokio::test]
    async fn ask_answered_on_the_stick_sse_and_json() {
        for (i, (decision, options, want)) in [
            ("once", Some(json!(["Staging", "Prod"])), "Staging"),
            ("deny", Some(json!(["Staging", "Prod"])), "Prod"),
            ("deny", None, "No"),
            ("deny", Some(json!(["OK"])), "dismissed"),
            ("dismiss", Some(json!(["A", "B"])), "dismissed"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut c = C::new(&format!("mcp-ask{i}")).await;
            let mut args = json!({"question": "Deploy where?"});
            if let Some(o) = &options {
                args["options"] = o.clone();
            }
            let accept = if i % 2 == 0 { "application/json, text/event-stream" } else { "application/json" };
            let fake = c.t.fake.clone();
            let hdrs = [("x-stick-cwd", "/w/web"), ("accept", accept)];
            let (r, ()) = tokio::join!(c.call("ask", args, &hdrs), answer_when_shown(&fake, decision, 1));
            let (text, err, res) = r;
            assert!(!err && text == want, "{decision}: {text}");
            assert_eq!(res["structuredContent"], json!({"answer": want}));
            let p = &c.asks_on_stick()[0];
            assert_eq!(
                (p["kind"].as_str(), p["q"].as_str(), p["from"].as_str()),
                (Some("ask"), Some("Deploy where?"), Some("web"))
            );
            assert!(p["hint"].as_str().unwrap().starts_with("Deploy where? - A: "));
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(c.sent().last().unwrap(), &json!({"src": "cli"})); // cleared
            assert!(!c.t.st.approvals.asking());
        }
    }

    #[tokio::test]
    async fn ask_from_the_dashboard_busy_and_timeout() {
        let mut c = C::new("mcp-ask-dash").await;
        let st = c.t.st.clone();
        let first = tokio::spawn(async move {
            crate::approvals::ask_user(&st, "one?", &["Go".into(), "Stop".into()], Duration::from_secs(5), "x").await
        });
        for _ in 0..100 {
            if c.t.st.approvals.asking() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(c.t.st.status()["asks"][0]["options"], json!(["Go", "Stop"]));
        let (text, _, _) = c.call("ask", json!({"question": "two?"}), &[]).await;
        assert_eq!(text, "busy: another question is waiting on the stick");
        let id = c.t.st.approvals.asks()[0]["id"].as_str().unwrap().to_string();
        let tok = c.t.st.token.clone();
        let req = axum::http::Request::builder()
            .method("POST")
            .uri("/api/mcp/answer")
            .header("host", "localhost:8670")
            .header("x-stick-token", tok)
            .body(Body::from(json!({"id": id, "choice": 1}).to_string()))
            .unwrap();
        assert_eq!(c.app.clone().oneshot(req).await.unwrap().status(), StatusCode::OK);
        assert_eq!(first.await.unwrap(), "Stop");
        let a = crate::approvals::ask_user(&c.t.st, "still there?", &["Yes".into()], Duration::from_millis(200), "web")
            .await;
        assert_eq!(a, "timeout");
        let (text, err, _) = c.call("ask", json!({"question": "x", "options": ["Same", "same"]}), &[]).await;
        assert!(err && text.contains("differ"));
    }

    #[tokio::test]
    async fn mac_tools_and_quick_commands() {
        let mut c = C::new("mcp-mac").await;
        let (text, err, _) = c.call("open_url", json!({"url": "https://github.com/acme/web/pull/1"}), &[]).await;
        assert!(!err && text == "opened github.com", "{text}");
        for bad in
            ["file:///etc/passwd", "javascript:alert(1)", "https://a b.com", "https://", "https://ok.com/\n--args"]
        {
            assert!(c.call("open_url", json!({"url": bad}), &[]).await.1, "{bad}");
        }
        for bad in ["Slack; rm -rf ~", "$(reboot)", "`x`"] {
            assert!(c.call("open_app", json!({"name": bad}), &[]).await.1, "{bad}");
        }
        let cmds = json!({"commands": [{"name": "PR", "phrases": ["pr {n}"], "open_url": "https://x.test/pull/{n}"}]});
        std::fs::write(c.t.st.paths.commands(), cmds.to_string()).unwrap();
        let (text, err, _) = c.call("quick_command", json!({"phrase": "PR 42"}), &[]).await;
        assert!(!err && text == "Opened PR #42", "{text}");
        let (text, err, _) = c.call("quick_command", json!({"phrase": "make coffee"}), &[]).await;
        assert!(err && text.contains("no quick command"));
        let ran = c.t.ran.lock().unwrap().clone();
        assert_eq!(ran[0], crate::commands::Action::Open(vec!["https://github.com/acme/web/pull/1".into()]));
        assert_eq!(ran.len(), 2);
    }

    #[tokio::test]
    async fn speak_and_the_dashboard_status() {
        let mut c = C::new("mcp-speak").await;
        let v = Arc::new(crate::speak::tests::FakeVoice::default());
        *c.t.st.speaker.voice.lock().unwrap() = Arc::new(v.clone());
        let (text, err, _) = c.call("speak", json!({"text": "Build  is green."}), &[]).await;
        assert!(!err && text == "speaking on the stick", "{text}");
        for _ in 0..100 {
            if !v.played.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(*v.played.lock().unwrap(), [vec!["Build is green.".to_string()]]);
        let ran: Arc<Mutex<usize>> = Arc::default();
        let r2 = ran.clone();
        *c.t.st.claude_runner.lock().unwrap() = Arc::new(move |_, _| {
            *r2.lock().unwrap() += 1;
            (false, "not added".into())
        });
        c.t.st.cfg.lock().unwrap().insert("claude_path".into(), json!("/fake/claude"));
        let get = |path: &str, method: &str, body: &str| {
            axum::http::Request::builder()
                .method(method)
                .uri(path)
                .header("host", "localhost:8670")
                .header("x-stick-token", c.t.st.token.clone())
                .body(Body::from(body.to_string()))
                .unwrap()
        };
        let resp = c.app.clone().oneshot(get("/api/mcp", "GET", "")).await.unwrap();
        let body: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(body["enabled"], true);
        assert_eq!(body["terminal"]["installed"], false);
        assert!(body["url"].as_str().unwrap().ends_with("/mcp"));
        let approval: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["approval"] == true)
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(approval, ["open_url", "open_app", "quick_command", "calendar_join"]);
        assert!(body["calls"].as_u64().unwrap() >= 1);
        let resp = c.app.clone().oneshot(get("/api/mcp/terminal/install", "POST", "{}")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let n = *ran.lock().unwrap();
        let resp =
            c.app.clone().oneshot(get("/api/mcp/terminal/install", "POST", r#"{"confirm":true}"#)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR); // the fake claude refuses
        assert!(*ran.lock().unwrap() > n);
    }

    #[test]
    fn the_voice_sessions_config_file() {
        let d = std::env::temp_dir().join(format!("stickybot-mcp-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = brain_config(&d, 8670, "TOK").unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["stick"]["url"], "http://127.0.0.1:8670/mcp");
        assert_eq!(v["mcpServers"]["stick"]["headers"]["Authorization"], "Bearer TOK");
        assert_eq!(v["mcpServers"]["stick"]["headers"]["X-Stick-Client"], "voice");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn terminal_install_runs_claude_mcp_add() {
        let ran: Arc<Mutex<Vec<Vec<String>>>> = Arc::default();
        let r2 = ran.clone();
        let runner: ClaudeRunner = Arc::new(move |_, args| {
            r2.lock().unwrap().push(args.to_vec());
            if args[1] == "get" { (false, "No MCP server found".into()) } else { (true, String::new()) }
        });
        let claude = std::path::Path::new("/fake/claude");
        let (ok, msg, shown_cmd) = terminal("install", Some(claude), 8670, "SECRET", &runner);
        assert!(ok && msg.contains("added"), "{msg}");
        let all = ran.lock().unwrap().clone();
        assert_eq!(all[1][..7], ["mcp", "add", "--transport", "http", "--scope", "user", "stick"]);
        assert!(all[1].contains(&"Authorization: Bearer SECRET".to_string()));
        assert!(all[1].contains(&"X-Stick-Cwd: ${PWD}".to_string()));
        assert!(!shown_cmd[0].contains("SECRET") && shown_cmd[0].contains("<token>"));
        let st = terminal_status(Some(claude), 8670, "SECRET", &runner);
        assert_eq!(st["installed"], false);
        assert!(terminal("install", None, 8670, "SECRET", &runner).1.contains("not found"));
    }
}
