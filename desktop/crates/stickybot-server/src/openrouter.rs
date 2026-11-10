//! The OpenRouter brain (a port of `openai_brain.py`): any model on OpenRouter with the user's own
//! key (the Keychain, `keys.rs`), pay per use.
//!
//! The conversation lives in memory; each round is one streamed `POST {base}/chat/completions`
//! (SSE: `data: {chunk}` lines, `: comment` keep-alives, `data: [DONE]`). The stick tools go along as
//! OpenAI-style functions (`mcp::openai_tools`) and run through `mcp::call_approved` (the same code
//! as the MCP server; open_url / open_app / quick_command ask on the stick first), at most
//! [`MAX_ROUNDS`] tool rounds, then one last round without tools for the answer.
//!
//! OpenRouter specifics (as openai_brain.py documents them): `Authorization: Bearer`, `HTTP-Referer`
//! and `X-Title` name the app; usage incl. `cost` comes in the last chunk; an error before the
//! stream is an HTTP status with `{"error": {"code", "message"}}`, after it a chunk with a top-level
//! `error`; closing the connection cancels the request. A model that can't call tools (its
//! `supported_parameters` lack "tools", or a 404/400 "…tool use…") is asked again without them,
//! and remembered.
//!
//! HTTP sits behind [`Transport`] (ureq on a blocking thread; tests use a fake and never reach the
//! network).

use crate::AppState;
use crate::talk::{Brain, BrainEvent, plain};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub const MAX_ROUNDS: usize = 5;
pub const HISTORY_MESSAGES: usize = 24;
pub const TOOL_RESULT_CHARS: usize = 2000;
pub const CONNECT: Duration = Duration::from_secs(10);
/// Nothing from the server this long mid-stream -> give up.
pub const READ: Duration = Duration::from_secs(60);
pub const MODELS_TTL: Duration = Duration::from_secs(600);
pub const BASE_URL: &str = "https://openrouter.ai/api/v1";
pub const REFERER: &str = "https://github.com/4shishkumar/stickybot";
pub const LABEL: &str = "OpenRouter";

/// Fast, cheap models that can call tools, in order of preference (openai_brain.py `PREFERRED`).
pub const PREFERRED: &[&str] = &[
    "google/gemini-3.1-flash-lite",
    "anthropic/claude-haiku-5.5",
    "openai/gpt-5.4-nano",
    "google/gemini-2.5-flash-lite",
    "openai/gpt-4.1-nano",
    "openai/gpt-4o-mini",
];

const SYSTEM: &str = "You are the user's voice assistant on their Mac. Requests arrive by voice from a small handheld device (an M5StickS3) and were transcribed automatically, so expect transcription slips (\"get hub\" = GitHub, numbers written as words). Act on what the user most likely meant.

Your reply is shown on a 135x240 pixel screen and may be read aloud:
- One or two short sentences, at most about 200 characters. Plain text only: no markdown, no lists, no code blocks, no emoji.
- Lead with the answer or the result (\"Opened Slack.\", \"Paris is the capital of France.\").
- If you don't know something current (news, weather, prices), say so briefly instead of guessing.

You can't run commands on the Mac or browse the web. Your tools: open_url, open_app and quick_command change the Mac and need the user's approval on the device (they press a button); notify, ask (an A/B question on the device), celebrate, progress, pin_home, speak and stick_status are feedback on the device. Your reply already shows on the device, so don't notify about it. Use tools only when they help.";

pub fn system_prompt() -> String {
    format!("{SYSTEM}\n\nIt is {} for the user.", chrono::Local::now().format("%A %d %B %Y, %H:%M"))
}

/// A stick tool call as a progress line ("Opening github.com").
pub fn describe(name: &str, args: &Value) -> String {
    let s = |k: &str, n: usize| -> String {
        let v = match args.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(v) => v.to_string(),
            None => String::new(),
        };
        v.chars().take(n).collect()
    };
    match name {
        "open_url" => {
            let u = s("url", 2000);
            let u = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).unwrap_or(&u);
            format!("Opening {}", u.chars().take(80).collect::<String>())
        }
        "open_app" => format!("Opening {}", s("name", 60)),
        "quick_command" => format!("Running: {}", s("phrase", 100)),
        "ask" => format!("Asking: {}", s("question", 100)),
        "notify" => format!("Notifying: {}", s("text", 100)),
        "speak" => "Speaking".into(),
        "celebrate" => "Celebrating".into(),
        "progress" => format!("Progress: {}", s("text", 100)),
        "pin_home" => format!("Pinning: {}", s("text", 100)),
        "stick_status" => "Checking the stick".into(),
        _ => name.into(),
    }
}

// --- HTTP ------------------------------------------------------------------------------------------

/// A network failure (no answer at all, or the stream stalled).
#[derive(Debug, Clone)]
pub struct NetError {
    pub message: String,
    pub timeout: bool,
}

pub struct HttpResp {
    pub status: u16,
    pub body: Box<dyn Read + Send>,
}

/// Blocking HTTP (called on a blocking thread). The body is read as it arrives.
pub trait Transport: Send + Sync {
    fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResp, NetError>;
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResp, NetError>;
}

/// ureq: 10 s to connect, 60 s without a byte mid-stream.
pub struct Ureq {
    agent: ureq::Agent,
}

impl Default for Ureq {
    fn default() -> Self {
        Self { agent: ureq::AgentBuilder::new().timeout_connect(CONNECT).timeout_read(READ).build() }
    }
}

fn ureq_result(r: Result<ureq::Response, ureq::Error>) -> Result<HttpResp, NetError> {
    match r {
        Ok(resp) => Ok(HttpResp { status: resp.status(), body: Box::new(resp.into_reader()) }),
        Err(ureq::Error::Status(code, resp)) => Ok(HttpResp { status: code, body: Box::new(resp.into_reader()) }),
        Err(ureq::Error::Transport(t)) => {
            let message = t.to_string();
            let timeout = message.to_lowercase().contains("timed out");
            Err(NetError { message, timeout })
        }
    }
}

impl Transport for Ureq {
    fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResp, NetError> {
        let mut r = self.agent.post(url).set("Content-Type", "application/json");
        for (k, v) in headers {
            r = r.set(k, v);
        }
        ureq_result(r.send_string(body))
    }
    fn get(&self, url: &str, headers: &[(String, String)]) -> Result<HttpResp, NetError> {
        let mut r = self.agent.get(url).timeout(Duration::from_secs(20));
        for (k, v) in headers {
            r = r.set(k, v);
        }
        ureq_result(r.call())
    }
}

/// The API said no (an HTTP status, or an error chunk mid-stream).
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

#[derive(Debug)]
enum Fail {
    Api(ApiError),
    Net(NetError),
}

/// `{"error": {"message"}}` (or whatever the body says, cut).
fn error_message(body: &str) -> String {
    match serde_json::from_str::<Value>(body) {
        Ok(v) => match v.get("error") {
            Some(Value::Object(e)) => e.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
            Some(e) => e.as_str().map(str::to_string).unwrap_or_else(|| e.to_string()).chars().take(300).collect(),
            None => v.to_string().chars().take(300).collect(),
        },
        Err(_) => body.chars().take(300).collect(),
    }
}

fn read_text(mut r: Box<dyn Read + Send>) -> String {
    let mut b = Vec::new();
    let _ = r.by_ref().take(256 * 1024).read_to_end(&mut b);
    String::from_utf8_lossy(&b).into_owned()
}

enum Msg {
    Data(String),
    End,
    Status(u16, String),
    Net(NetError),
}

/// The blocking side of one streamed request: SSE `data:` payloads into `tx`. Stops (and closes the
/// connection) as soon as nobody listens any more (cancel).
fn stream(
    t: Arc<dyn Transport>,
    url: String,
    headers: Vec<(String, String)>,
    body: String,
    tx: mpsc::UnboundedSender<Msg>,
) {
    let resp = match t.post(&url, &headers, &body) {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(Msg::Net(e));
            return;
        }
    };
    if resp.status != 200 {
        let _ = tx.send(Msg::Status(resp.status, error_message(&read_text(resp.body))));
        return;
    }
    for line in BufReader::new(resp.body).lines() {
        if tx.is_closed() {
            return;
        }
        match line {
            Ok(l) => {
                if let Some(d) = l.strip_prefix("data:") {
                    let d = d.trim();
                    if d == "[DONE]" {
                        break;
                    }
                    if tx.send(Msg::Data(d.to_string())).is_err() {
                        return;
                    }
                }
            }
            Err(e) => {
                let timeout = matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock);
                let _ = tx.send(Msg::Net(NetError { message: e.to_string(), timeout }));
                return;
            }
        }
    }
    let _ = tx.send(Msg::End);
}

// --- the model list ------------------------------------------------------------------------------------

fn per_million(price: &Value) -> Option<f64> {
    let v = match price {
        Value::String(s) => s.parse::<f64>().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    } * 1_000_000.0;
    (v >= 0.0).then(|| (v * 10_000.0).round() / 10_000.0)
}

/// A `/models` entry -> what the dashboard needs (None if it can't chat).
pub fn simplify(m: &Value) -> Option<Value> {
    let id = m.get("id")?.as_str()?;
    if let Some(out) = m["architecture"]["output_modalities"].as_array()
        && !out.is_empty()
        && !out.iter().any(|x| x == "text")
    {
        return None;
    }
    let p = &m["pricing"];
    let tools = m["supported_parameters"].as_array().is_some_and(|a| a.iter().any(|x| x == "tools"));
    let name = m.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).unwrap_or(id);
    Some(json!({"id": id, "name": name, "in": per_million(&p["prompt"]), "out": per_million(&p["completion"]),
                "tools": tools, "context": m.get("context_length").cloned().unwrap_or(Value::Null)}))
}

/// The first preferred model in the list that can use tools, else the cheapest one that can.
pub fn suggest(models: &[Value]) -> Option<String> {
    for p in PREFERRED {
        if models.iter().any(|m| m["id"] == *p && m["tools"] == true) {
            return Some(p.to_string());
        }
    }
    models
        .iter()
        .filter(|m| {
            let id = m["id"].as_str().unwrap_or("");
            m["tools"] == true
                && m["in"].as_f64().is_some_and(|v| v != 0.0)
                && m["out"].is_number()
                && !id.starts_with('~')
                && !id.contains(':')
        })
        .min_by(|a, b| {
            let c = |m: &Value| m["in"].as_f64().unwrap_or(0.0) + m["out"].as_f64().unwrap_or(0.0);
            c(a).total_cmp(&c(b))
        })
        .and_then(|m| m["id"].as_str().map(str::to_string))
}

// --- the brain ------------------------------------------------------------------------------------------

/// One streamed completion: its text, tool calls (merged from their deltas), usage.
#[derive(Default)]
struct Round {
    text: String,
    calls: BTreeMap<u64, (String, String, String)>, // index -> (id, name, arguments)
    usage: Option<Value>,
}

impl Round {
    fn tool_calls(&self) -> Vec<(String, String, String)> {
        self.calls.values().filter(|c| !c.1.is_empty()).cloned().collect()
    }
    fn add_call_delta(&mut self, d: &Value) {
        let i = d.get("index").and_then(Value::as_u64).unwrap_or(self.calls.len() as u64);
        let c = self.calls.entry(i).or_default();
        if let Some(id) = d.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()) {
            c.0 = id.to_string();
        }
        c.1 += d["function"]["name"].as_str().unwrap_or("");
        c.2 += d["function"]["arguments"].as_str().unwrap_or("");
    }
}

/// The answer as it's written, to the stick ("" withdraws a preamble).
struct TextOut<'a> {
    tx: Option<&'a mpsc::UnboundedSender<BrainEvent>>,
    shown: String,
}

impl TextOut<'_> {
    fn set(&mut self, t: &str) {
        if t != self.shown {
            self.shown = t.to_string();
            if let Some(tx) = self.tx {
                let _ = tx.send(BrainEvent::Text(t.to_string()));
            }
        }
    }
}

#[derive(Default)]
struct Info {
    model: String,
    first_token_ms: Option<u64>,
    cost: Option<f64>,
    rounds: u32,
    tools: Vec<String>,
}

struct Inner {
    st: Weak<AppState>,
    transport: Mutex<Arc<dyn Transport>>,
    base_url: Mutex<String>,
    history: Mutex<Vec<Value>>,
    last_at: Mutex<Option<Instant>>,
    busy: AtomicBool,
    cancelled: AtomicBool,
    cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    last: Mutex<Value>,
    recent: Mutex<VecDeque<Value>>,
    no_tools: Mutex<HashSet<String>>,
    models: tokio::sync::Mutex<Option<(Vec<Value>, Instant)>>,
    /// the cached list for `tools_supported` (no await)
    known: Mutex<Vec<Value>>,
}

/// OpenRouter as the brain. Cheap to clone.
#[derive(Clone)]
pub struct OpenRouterBrain {
    inner: Arc<Inner>,
}

impl OpenRouterBrain {
    pub fn new(st: Weak<AppState>) -> Self {
        Self {
            inner: Arc::new(Inner {
                st,
                transport: Mutex::new(Arc::new(Ureq::default())),
                base_url: Mutex::new(BASE_URL.into()),
                history: Mutex::default(),
                last_at: Mutex::default(),
                busy: AtomicBool::new(false),
                cancelled: AtomicBool::new(false),
                cancel: Mutex::default(),
                last: Mutex::new(Value::Null),
                recent: Mutex::default(),
                no_tools: Mutex::default(),
                models: tokio::sync::Mutex::new(None),
                known: Mutex::default(),
            }),
        }
    }

    /// Another HTTP transport / server (tests).
    pub fn set_transport(&self, t: Arc<dyn Transport>, base_url: &str) {
        *self.inner.transport.lock().unwrap() = t;
        *self.inner.base_url.lock().unwrap() = base_url.into();
    }

    pub fn history(&self) -> Vec<Value> {
        self.inner.history.lock().unwrap().clone()
    }

    /// Makes the conversation look idle since `ago` (tests).
    pub fn set_idle_since(&self, ago: Duration) {
        *self.inner.last_at.lock().unwrap() = Instant::now().checked_sub(ago);
    }

    pub fn last(&self) -> Value {
        self.inner.last.lock().unwrap().clone()
    }

    fn cfg(&self) -> Map<String, Value> {
        self.inner.st.upgrade().map(|s| s.cfg.lock().unwrap().clone()).unwrap_or_else(crate::config::defaults)
    }

    pub fn model(cfg: &Map<String, Value>) -> String {
        cfg.get("openrouter_model").and_then(Value::as_str).unwrap_or("").trim().to_string()
    }

    fn max_tokens(cfg: &Map<String, Value>) -> u64 {
        cfg.get("openrouter_max_tokens").and_then(Value::as_u64).unwrap_or(400).clamp(50, 4000)
    }

    fn headers(key: &str) -> Vec<(String, String)> {
        vec![
            ("Authorization".into(), format!("Bearer {key}")),
            ("HTTP-Referer".into(), REFERER.into()),
            ("X-Title".into(), "StickyBot".into()),
        ]
    }

    fn tools_supported(&self, model: &str) -> Option<bool> {
        self.inner.known.lock().unwrap().iter().find(|m| m["id"] == model).and_then(|m| m["tools"].as_bool())
    }

    /// The key (may read the Keychain: on a blocking thread).
    async fn key(&self) -> Option<String> {
        let st = self.inner.st.upgrade()?;
        tokio::task::spawn_blocking(move || st.keys.get("openrouter")).await.ok().flatten()
    }

    fn not_ready(key: &Option<String>, model: &str) -> Option<String> {
        if key.is_none() {
            return Some("Add your OpenRouter key in Settings → Brain.".into());
        }
        if model.is_empty() {
            return Some("Pick an OpenRouter model in Settings → Brain.".into());
        }
        None
    }

    pub fn error_text(e: &ApiError) -> String {
        let m = e.message.to_lowercase();
        match e.status {
            401 => "OpenRouter rejected the key. Check it in Settings → Brain.".into(),
            402 => "Out of OpenRouter credits. Add some at openrouter.ai.".into(),
            429 => "OpenRouter is rate limiting. Try again in a moment.".into(),
            403 => "OpenRouter refused that request (moderation or permissions).".into(),
            404 => "OpenRouter doesn't know that model. Pick another in Settings → Brain.".into(),
            400 if m.contains("model") => {
                "OpenRouter doesn't know that model. Pick another in Settings → Brain.".into()
            }
            502 | 503 => "The model is unavailable right now. Try again, or pick another.".into(),
            408 => "OpenRouter timed out. Try again.".into(),
            s => {
                let p: String = plain(&e.message).chars().take(120).collect();
                format!("OpenRouter: {}", if p.is_empty() { format!("error {s}") } else { p })
            }
        }
    }

    fn net_text(e: &NetError) -> String {
        if e.timeout {
            format!("{LABEL} stopped answering. Try again.")
        } else {
            format!("Couldn't reach {LABEL}. Check the internet connection.")
        }
    }

    fn fail_text(f: &Fail) -> String {
        match f {
            Fail::Api(e) => Self::error_text(e),
            Fail::Net(e) => {
                log::warn!("openrouter: {}", e.message);
                Self::net_text(e)
            }
        }
    }

    /// Unused for `brain_idle_min`: the next request starts a new conversation.
    fn expire(&self, cfg: &Map<String, Value>) {
        let idle =
            Duration::from_secs_f64(cfg.get("brain_idle_min").and_then(Value::as_f64).unwrap_or(10.0).max(0.0) * 60.0);
        let stale = self.inner.last_at.lock().unwrap().is_none_or(|t| t.elapsed() >= idle);
        let mut h = self.inner.history.lock().unwrap();
        if stale && !h.is_empty() {
            log::info!("openrouter: idle, new conversation");
            h.clear();
        }
    }

    /// Adds a finished turn; the oldest turns go first, whole (up to the next user message).
    pub fn remember(&self, turn: Vec<Value>) {
        let mut h = self.inner.history.lock().unwrap();
        h.extend(turn);
        while h.len() > HISTORY_MESSAGES {
            let next =
                h.iter().enumerate().skip(1).find(|(_, m)| m["role"] == "user").map(|(i, _)| i).unwrap_or(h.len());
            h.drain(..next);
        }
    }

    /// One streamed completion.
    #[allow(clippy::too_many_arguments)] // one round's whole request
    async fn complete(
        &self,
        key: &str,
        model: &str,
        messages: &[Value],
        tools: bool,
        max_tokens: u64,
        mut out: Option<&mut TextOut<'_>>,
        info: &mut Info,
        started: Instant,
    ) -> Result<Round, Fail> {
        let mut body = json!({"model": model, "messages": messages, "stream": true, "max_tokens": max_tokens});
        if tools {
            body["tools"] = json!(crate::mcp::openai_tools());
        }
        info.rounds += 1;
        let (tx, mut rx) = mpsc::unbounded_channel();
        let t = self.inner.transport.lock().unwrap().clone();
        let url = format!("{}/chat/completions", self.inner.base_url.lock().unwrap());
        let headers = Self::headers(key);
        let body = body.to_string();
        tokio::task::spawn_blocking(move || stream(t, url, headers, body, tx));
        let mut r = Round::default();
        loop {
            match rx.recv().await {
                None | Some(Msg::End) => break,
                Some(Msg::Net(e)) => return Err(Fail::Net(e)),
                Some(Msg::Status(s, m)) => return Err(Fail::Api(ApiError { status: s, message: m })),
                Some(Msg::Data(d)) => {
                    let Ok(chunk) = serde_json::from_str::<Value>(&d) else { continue };
                    if let Some(err) = chunk.get("error").filter(|e| !e.is_null()) {
                        let status = err.get("code").and_then(Value::as_u64).unwrap_or(500) as u16;
                        let message = err
                            .get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| err.as_str().unwrap_or("error mid-stream").to_string());
                        return Err(Fail::Api(ApiError { status, message }));
                    }
                    if let Some(u) = chunk.get("usage").filter(|u| !u.is_null()) {
                        r.usage = Some(u.clone());
                    }
                    for ch in chunk["choices"].as_array().into_iter().flatten() {
                        let d = &ch["delta"];
                        for tc in d["tool_calls"].as_array().into_iter().flatten() {
                            r.add_call_delta(tc);
                        }
                        if let Some(piece) = d["content"].as_str().filter(|p| !p.is_empty()) {
                            if info.first_token_ms.is_none() {
                                info.first_token_ms = Some(started.elapsed().as_millis() as u64);
                            }
                            r.text += piece;
                            if r.calls.is_empty()
                                && let Some(o) = out.as_deref_mut()
                            {
                                o.set(&r.text);
                            }
                        }
                    }
                }
            }
        }
        if let Some(c) = r.usage.as_ref().and_then(|u| u["cost"].as_f64()) {
            info.cost = Some(info.cost.unwrap_or(0.0) + c);
        }
        Ok(r)
    }

    async fn run_tool(
        &self,
        name: &str,
        arguments: &str,
        tx: Option<&mpsc::UnboundedSender<BrainEvent>>,
        info: &mut Info,
    ) -> String {
        let args: Value = match serde_json::from_str(if arguments.is_empty() { "{}" } else { arguments }) {
            Ok(a) => a,
            Err(_) => return "error: the arguments weren't valid JSON".into(),
        };
        if crate::mcp::tool(name).is_none() {
            return format!("error: no tool called {name}");
        }
        info.tools.push(name.to_string());
        if let Some(tx) = tx {
            let _ = tx.send(BrainEvent::Progress(describe(name, &args)));
        }
        let Some(st) = self.inner.st.upgrade() else { return "error: the stick tools aren't available".into() };
        let (text, err, _) = crate::mcp::call_approved(&st, name, &args, "Agent").await;
        format!("{}{text}", if err { "error: " } else { "" }).chars().take(TOOL_RESULT_CHARS).collect()
    }

    async fn converse(
        &self,
        text: &str,
        cfg: &Map<String, Value>,
        tx: &mpsc::UnboundedSender<BrainEvent>,
        info: &mut Info,
        started: Instant,
    ) -> (bool, String) {
        let key = self.key().await;
        let model = info.model.clone();
        if let Some(why) = Self::not_ready(&key, &model) {
            return (false, why);
        }
        let key = key.unwrap_or_default();
        let mut turn = vec![json!({"role": "user", "content": text})];
        let mut use_tools = cfg.get("mcp_server").and_then(Value::as_bool).unwrap_or(true)
            && !self.inner.no_tools.lock().unwrap().contains(&model)
            && self.tools_supported(&model) != Some(false);
        let mut out = TextOut { tx: Some(tx), shown: String::new() };
        let max_tokens = Self::max_tokens(cfg);
        for rnd in 0..=MAX_ROUNDS {
            let mut tools = use_tools && rnd < MAX_ROUNDS;
            if rnd > 0 {
                let _ = tx.send(BrainEvent::Progress("Thinking…".into()));
            }
            out.set("");
            let mut msgs = vec![json!({"role": "system", "content": system_prompt()})];
            msgs.extend(self.inner.history.lock().unwrap().iter().cloned());
            msgs.extend(turn.iter().cloned());
            let r = match self.complete(&key, &model, &msgs, tools, max_tokens, Some(&mut out), info, started).await {
                Ok(r) => r,
                Err(Fail::Api(e))
                    if tools && matches!(e.status, 400 | 404) && e.message.to_lowercase().contains("tool") =>
                {
                    log::info!("openrouter: {model} can't use tools ({}), going on without them", e.message);
                    self.inner.no_tools.lock().unwrap().insert(model.clone());
                    use_tools = false;
                    tools = false;
                    match self.complete(&key, &model, &msgs, false, max_tokens, Some(&mut out), info, started).await {
                        Ok(r) => r,
                        Err(f) => return (false, Self::fail_text(&f)),
                    }
                }
                Err(f) => return (false, Self::fail_text(&f)),
            };
            let calls = if tools { r.tool_calls() } else { Vec::new() };
            if calls.is_empty() {
                let answer = r.text.trim().to_string();
                turn.push(json!({"role": "assistant", "content": answer}));
                self.remember(turn);
                let p = plain(&answer);
                return (true, if p.is_empty() { "(no answer)".into() } else { p });
            }
            out.set(""); // what it wrote before the tool calls wasn't the answer
            let ids: Vec<String> = calls
                .iter()
                .enumerate()
                .map(|(i, c)| if c.0.is_empty() { format!("call_{rnd}_{i}") } else { c.0.clone() })
                .collect();
            turn.push(json!({"role": "assistant", "content": if r.text.is_empty() { Value::Null } else { json!(r.text) },
                "tool_calls": calls.iter().zip(&ids).map(|(c, id)| json!({"id": id, "type": "function",
                    "function": {"name": c.1, "arguments": if c.2.is_empty() { "{}" } else { c.2.as_str() }}})).collect::<Vec<_>>()}));
            for (c, id) in calls.iter().zip(&ids) {
                let result = self.run_tool(&c.1, &c.2, Some(tx), info).await;
                turn.push(json!({"role": "tool", "tool_call_id": id, "content": result}));
            }
        }
        (false, "Too many steps, stopped.".into())
    }

    async fn ask_inner(&self, text: String, tx: mpsc::UnboundedSender<BrainEvent>) -> (bool, String) {
        let cfg = self.cfg();
        self.expire(&cfg);
        let mut info = Info { model: Self::model(&cfg), ..Default::default() };
        let started = Instant::now();
        let (ctx, crx) = tokio::sync::oneshot::channel();
        *self.inner.cancel.lock().unwrap() = Some(ctx);
        let r = tokio::select! {
            r = self.converse(&text, &cfg, &tx, &mut info, started) => r,
            _ = crx => (false, "Cancelled.".to_string()), // the stream is dropped: the request stops there
        };
        self.inner.cancel.lock().unwrap().take();
        *self.inner.last_at.lock().unwrap() = Some(Instant::now());
        let cancelled = self.inner.cancelled.load(Ordering::Relaxed);
        let total_ms = started.elapsed().as_millis() as u64;
        let last = json!({"model": info.model, "first_token_ms": info.first_token_ms, "cost": info.cost,
            "rounds": info.rounds, "tools": info.tools, "at": crate::hub::now(), "ok": r.0, "cancelled": cancelled,
            "error": if r.0 || cancelled { Value::Null } else { json!(r.1) }, "total_ms": total_ms});
        if r.0 && !cancelled {
            let mut rec = self.inner.recent.lock().unwrap();
            if rec.len() == 20 {
                rec.pop_front();
            }
            rec.push_back(last.clone());
        }
        log::info!(
            "openrouter {}: first token {}, total {:.1} s, {} round{}, cost {}{}",
            info.model,
            info.first_token_ms.map(|m| format!("{:.2} s", m as f64 / 1000.0)).unwrap_or_else(|| "-".into()),
            total_ms as f64 / 1000.0,
            info.rounds,
            if info.rounds == 1 { "" } else { "s" },
            info.cost.map(|c| format!("${c:.5}")).unwrap_or_else(|| "-".into()),
            if cancelled {
                " (cancelled)".to_string()
            } else if r.0 {
                String::new()
            } else {
                format!(" (error: {})", r.1)
            }
        );
        *self.inner.last.lock().unwrap() = last;
        r
    }

    /// One short request without history or tools (the dashboard's Test button).
    pub async fn test(&self, model: Option<String>) -> Value {
        if self.busy() {
            return json!({"ok": false, "error": "Busy with a request, try again in a moment."});
        }
        let cfg = self.cfg();
        let model = model.unwrap_or_else(|| Self::model(&cfg));
        let mut info = Info { model: model.clone(), ..Default::default() };
        let started = Instant::now();
        let key = self.key().await;
        if let Some(why) = Self::not_ready(&key, &model) {
            return json!({"ok": false, "error": why, "model": model});
        }
        let msgs = [
            json!({"role": "system", "content": system_prompt()}),
            json!({"role": "user", "content": "Say hello and tell me which model you are, in one short sentence."}),
        ];
        let r = self.complete(&key.unwrap_or_default(), &model, &msgs, false, 60, None, &mut info, started).await;
        let (ok, reply) = match r {
            Ok(r) => {
                let p = plain(&r.text);
                (true, if p.is_empty() { "(an empty reply)".into() } else { p })
            }
            Err(f) => (false, Self::fail_text(&f)),
        };
        json!({"ok": ok, "reply": if ok { json!(reply) } else { Value::Null }, "error": if ok { Value::Null } else { json!(reply) },
               "model": model, "first_token_ms": info.first_token_ms, "total_ms": started.elapsed().as_millis() as u64,
               "cost": info.cost})
    }

    /// `(models, suggested id, error)`; cached [`MODELS_TTL`], a stale list beats none when offline.
    pub async fn list_models(&self, force: bool) -> (Vec<Value>, Option<String>, Option<String>) {
        let mut cache = self.inner.models.lock().await;
        let mut err = None;
        if force || cache.as_ref().is_none_or(|(_, at)| at.elapsed() >= MODELS_TTL) {
            let t = self.inner.transport.lock().unwrap().clone();
            let url = format!("{}/models", self.inner.base_url.lock().unwrap());
            let got = tokio::task::spawn_blocking(move || -> Result<Vec<Value>, String> {
                let headers =
                    [("HTTP-Referer".to_string(), REFERER.to_string()), ("X-Title".into(), "StickyBot".into())];
                let r = t.get(&url, &headers).map_err(|e| e.message)?;
                let status = r.status;
                let text = read_text(r.body);
                if status != 200 {
                    return Err(format!("{status}: {}", error_message(&text)));
                }
                let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                let data = v.get("data").and_then(Value::as_array).ok_or("unexpected model list")?;
                let mut out: Vec<Value> = data.iter().filter_map(simplify).collect();
                out.sort_by_key(|m| m["name"].as_str().unwrap_or("").to_lowercase());
                Ok(out)
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            match got {
                Ok(list) => {
                    *self.inner.known.lock().unwrap() = list.clone();
                    *cache = Some((list, Instant::now()));
                }
                Err(e) => {
                    log::warn!("openrouter models: {e}");
                    err = Some("Couldn't load the model list from OpenRouter.".to_string());
                }
            }
        }
        let models = cache.as_ref().map(|(m, _)| m.clone()).unwrap_or_default();
        let s = suggest(&models);
        (models, s, err)
    }
}

impl Brain for OpenRouterBrain {
    fn busy(&self) -> bool {
        self.inner.busy.load(Ordering::Relaxed)
    }

    fn ask(&self, text: String, events: mpsc::UnboundedSender<BrainEvent>) -> BoxFuture<'static, (bool, String)> {
        let me = self.clone();
        Box::pin(async move {
            if me.inner.busy.swap(true, Ordering::Relaxed) {
                return (false, "Still working on the last request.".into());
            }
            me.inner.cancelled.store(false, Ordering::Relaxed);
            let r = me.ask_inner(text, events).await;
            me.inner.busy.store(false, Ordering::Relaxed);
            r
        })
    }

    fn cancel(&self) {
        if let Some(c) = self.inner.cancel.lock().unwrap().take() {
            self.inner.cancelled.store(true, Ordering::Relaxed);
            let _ = c.send(());
        }
    }

    fn cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }

    fn forget(&self) {
        self.inner.history.lock().unwrap().clear();
    }

    fn status(&self) -> Value {
        let cfg = self.cfg();
        let rec = self.inner.recent.lock().unwrap();
        let avg = |k: &str| {
            let v: Vec<u64> = rec.iter().filter_map(|r| r[k].as_u64()).collect();
            if v.is_empty() { Value::Null } else { json!(v.iter().sum::<u64>() / v.len() as u64) }
        };
        let a = if rec.is_empty() {
            json!({})
        } else {
            json!({"openrouter": {"n": rec.len(), "first_token_ms": avg("first_token_ms"), "total_ms": avg("total_ms")}})
        };
        let key = self.inner.st.upgrade().map(|s| s.keys.known_status("openrouter")).unwrap_or(Value::Null);
        json!({"provider": LABEL, "model": Self::model(&cfg), "alive": false,
               "session": !self.inner.history.lock().unwrap().is_empty(),
               "last": self.inner.last.lock().unwrap().clone(), "avg": a, "key": key})
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::talk::tests::{T, agent};
    use std::sync::atomic::AtomicUsize;

    pub const KEY: &str = concat!("sk-or-v1-", "0123456789abcdef", "0123456789abcdef", "0123456789abcdef", "0123456789abcdef");
    pub const MODEL: &str = "test/fast-model";

    pub fn text_chunk(t: &str) -> Value {
        json!({"choices": [{"index": 0, "delta": {"content": t}}]})
    }

    fn tool_chunk(i: u64, id: &str, name: &str, args: Value) -> Value {
        json!({"choices": [{"index": 0, "delta": {"tool_calls": [
            {"index": i, "id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}}]}}]})
    }

    fn usage_chunk(cost: f64, finish: &str) -> Value {
        json!({"choices": [{"index": 0, "delta": {"content": ""}, "finish_reason": finish}],
               "usage": {"prompt_tokens": 50, "completion_tokens": 8, "total_tokens": 58, "cost": cost}})
    }

    pub enum Reply {
        Stream(Vec<Value>),
        Status(u16, String),
        Hang(Vec<Value>),
    }

    pub fn answer(parts: &[&str], cost: f64) -> Reply {
        let mut c: Vec<Value> = parts.iter().map(|p| text_chunk(p)).collect();
        c.push(usage_chunk(cost, "stop"));
        Reply::Stream(c)
    }

    fn tool_call(name: &str, args: Value, preamble: &str, id: &str) -> Reply {
        let mut c = Vec::new();
        if !preamble.is_empty() {
            c.push(text_chunk(preamble));
        }
        c.push(tool_chunk(0, id, name, args));
        c.push(usage_chunk(0.0001, "tool_calls"));
        Reply::Stream(c)
    }

    /// A request the fake saw: its headers and JSON body.
    pub type Request = (Vec<(String, String)>, Value);

    /// A fake OpenRouter: chat answers with the next scripted reply, `/models` with a list.
    #[derive(Default)]
    pub struct Fake {
        pub replies: Mutex<VecDeque<Reply>>,
        pub requests: Mutex<Vec<Request>>,
        pub model_fetches: AtomicUsize,
        pub disconnected: Arc<AtomicBool>,
        pub offline: AtomicBool,
    }

    impl Fake {
        pub fn reply(&self, r: Reply) {
            self.replies.lock().unwrap().push_back(r);
        }
        pub fn body(&self, i: usize) -> Value {
            self.requests.lock().unwrap()[i].1.clone()
        }
        pub fn n(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
    }

    /// Chunks, then keep-alive comments for up to 5 s; notes when the client hangs up.
    struct Hanging {
        head: std::io::Cursor<Vec<u8>>,
        left: usize,
        gone: Arc<AtomicBool>,
    }

    impl Read for Hanging {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.head.read(buf)?;
            if n > 0 {
                return Ok(n);
            }
            if self.left == 0 {
                return Ok(0);
            }
            self.left -= 1;
            std::thread::sleep(Duration::from_millis(10));
            let k = b": OPENROUTER PROCESSING\n\n";
            buf[..k.len()].copy_from_slice(k);
            Ok(k.len())
        }
    }

    impl Drop for Hanging {
        fn drop(&mut self) {
            self.gone.store(true, Ordering::Relaxed);
        }
    }

    fn sse(chunks: &[Value], done: bool) -> Vec<u8> {
        let mut s = String::from(": OPENROUTER PROCESSING\n\n");
        for c in chunks {
            s += &format!("data: {c}\n\n");
        }
        if done {
            s += "data: [DONE]\n\n";
        }
        s.into_bytes()
    }

    impl Transport for Arc<Fake> {
        fn post(&self, url: &str, headers: &[(String, String)], body: &str) -> Result<HttpResp, NetError> {
            if self.offline.load(Ordering::Relaxed) {
                return Err(NetError { message: "connection refused".into(), timeout: false });
            }
            assert!(url.ends_with("/chat/completions"), "{url}");
            self.requests.lock().unwrap().push((headers.to_vec(), serde_json::from_str(body).unwrap()));
            let r = self.replies.lock().unwrap().pop_front().unwrap_or_else(|| answer(&["OK."], 0.0));
            Ok(match r {
                Reply::Status(s, m) => HttpResp {
                    status: s,
                    body: Box::new(std::io::Cursor::new(
                        json!({"error": {"code": s, "message": m}}).to_string().into_bytes(),
                    )),
                },
                Reply::Stream(c) => HttpResp { status: 200, body: Box::new(std::io::Cursor::new(sse(&c, true))) },
                Reply::Hang(c) => HttpResp {
                    status: 200,
                    body: Box::new(Hanging {
                        head: std::io::Cursor::new(sse(&c, false)),
                        left: 500,
                        gone: self.disconnected.clone(),
                    }),
                },
            })
        }

        fn get(&self, url: &str, _headers: &[(String, String)]) -> Result<HttpResp, NetError> {
            if self.offline.load(Ordering::Relaxed) {
                return Err(NetError { message: "connection refused".into(), timeout: false });
            }
            assert!(url.ends_with("/models"));
            self.model_fetches.fetch_add(1, Ordering::Relaxed);
            let list = json!({"data": [
                {"id": MODEL, "name": "Test: Fast", "pricing": {"prompt": "0.0000001", "completion": "0.0000004"},
                 "supported_parameters": ["tools", "max_tokens"], "architecture": {"output_modalities": ["text"]},
                 "context_length": 100000},
                {"id": "test/no-tools", "name": "Test: No tools", "pricing": {"prompt": "0.00000005", "completion": "0"},
                 "supported_parameters": ["max_tokens"], "architecture": {"output_modalities": ["text"]}},
                {"id": "test/image", "name": "Image maker", "pricing": {"prompt": "0", "completion": "0"},
                 "architecture": {"output_modalities": ["image"]}}]});
            Ok(HttpResp { status: 200, body: Box::new(std::io::Cursor::new(list.to_string().into_bytes())) })
        }
    }

    /// An agent on the fake stick whose OpenRouter brain talks to a fake server, with a key (in
    /// memory) and a model.
    pub async fn orb(name: &str) -> (T, OpenRouterBrain, Arc<Fake>) {
        let t = agent(name, false).await;
        t.st.keys.save("openrouter", KEY).unwrap();
        t.st.cfg.lock().unwrap().insert("openrouter_model".into(), json!(MODEL));
        let fake = Arc::new(Fake::default());
        let b = t.st.openrouter();
        b.set_transport(Arc::new(fake.clone()), "https://fake.test/api/v1");
        (t, b, fake)
    }

    pub struct Asked {
        pub ok: bool,
        pub reply: String,
        pub progress: Vec<String>,
        pub texts: Vec<String>,
    }

    pub async fn ask(b: &OpenRouterBrain, text: &str) -> Asked {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (ok, reply) = b.ask(text.into(), tx).await;
        let (mut progress, mut texts) = (Vec::new(), Vec::new());
        while let Ok(ev) = rx.try_recv() {
            match ev {
                BrainEvent::Progress(p) => progress.push(p),
                BrainEvent::Text(t) => texts.push(t),
            }
        }
        Asked { ok, reply, progress, texts }
    }

    fn header<'a>(h: &'a [(String, String)], k: &str) -> Option<&'a str> {
        h.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
    }

    // --- test_openrouter.py ------------------------------------------------------------------------

    #[tokio::test]
    async fn streams_the_answer_with_cost() {
        let (_t, b, fake) = orb("or-stream").await;
        fake.reply(answer(&["It's **Paris**. ", "Lovely city."], 0.00025));
        let a = ask(&b, "what's the capital of France?").await;
        assert!(a.ok && a.reply == "It's Paris. Lovely city.", "{}", a.reply);
        assert_eq!(a.texts.last().unwrap(), "It's **Paris**. Lovely city.");
        assert!(a.texts.iter().filter(|t| !t.is_empty()).count() >= 2); // as it was written
        let (h, body) = fake.requests.lock().unwrap()[0].clone();
        assert_eq!(header(&h, "Authorization"), Some(format!("Bearer {KEY}").as_str()));
        assert_eq!(header(&h, "HTTP-Referer"), Some(REFERER));
        assert_eq!(header(&h, "X-Title"), Some("StickyBot"));
        assert_eq!(
            (body["model"].as_str(), body["stream"].as_bool(), body["max_tokens"].as_u64()),
            (Some(MODEL), Some(true), Some(400))
        );
        assert_eq!(body["messages"][0]["role"], "system");
        assert!(body["messages"][0]["content"].as_str().unwrap().contains("135x240"));
        assert_eq!(
            body["messages"].as_array().unwrap().last().unwrap(),
            &json!({"role": "user", "content": "what's the capital of France?"})
        );
        assert_eq!(body["tools"].as_array().unwrap().len(), crate::mcp::tools().len());
        let last = b.last();
        assert!(
            last["ok"] == true
                && (last["cost"].as_f64().unwrap() - 0.00025).abs() < 1e-9
                && last["first_token_ms"].is_u64()
        );
        let st = b.status();
        assert_eq!((st["provider"].as_str(), st["model"].as_str()), (Some("OpenRouter"), Some(MODEL)));
        assert_eq!(st["avg"]["openrouter"]["n"], 1);
        assert!(!st.to_string().contains(KEY));
    }

    #[tokio::test]
    async fn mid_stream_error_is_a_failure() {
        let (_t, b, fake) = orb("or-midstream").await;
        fake.reply(Reply::Stream(vec![
            text_chunk("Hel"),
            json!({"error": {"code": 502, "message": "provider fell over"}, "choices": [{"delta": {}, "finish_reason": "error"}]}),
        ]));
        let a = ask(&b, "hi").await;
        assert!(!a.ok && a.reply.contains("unavailable"), "{}", a.reply);
        assert!(b.history().is_empty()); // a failed turn isn't remembered
    }

    #[tokio::test]
    async fn tool_round_trip() {
        let (_t, b, fake) = orb("or-tool").await;
        fake.reply(tool_call("stick_status", json!({}), "Let me check.", "call_1"));
        fake.reply(answer(&["Battery is at 87%."], 0.00012));
        let a = ask(&b, "how's the stick's battery?").await;
        assert!(a.ok && a.reply == "Battery is at 87%.", "{}", a.reply);
        let i = a.texts.iter().position(|t| t == "Let me check.").expect("preamble shown");
        assert_eq!(a.texts[i + 1], ""); // withdrawn
        assert_eq!(a.progress, ["Checking the stick", "Thinking…"]);
        let second = fake.body(1)["messages"].as_array().unwrap().clone();
        let n = second.len();
        assert_eq!(second[n - 2]["role"], "assistant");
        assert_eq!(second[n - 2]["tool_calls"][0]["function"]["name"], "stick_status");
        assert_eq!(
            (second[n - 1]["role"].as_str(), second[n - 1]["tool_call_id"].as_str()),
            (Some("tool"), Some("call_1"))
        );
        assert!(second[n - 1]["content"].as_str().unwrap().contains("battery 87%"), "{}", second[n - 1]);
        let last = b.last();
        assert!((last["cost"].as_f64().unwrap() - 0.00022).abs() < 1e-9);
        assert_eq!(last["tools"], json!(["stick_status"]));
        let roles: Vec<String> = b.history().iter().map(|m| m["role"].as_str().unwrap().to_string()).collect();
        assert_eq!(roles, ["user", "assistant", "tool", "assistant"]);
    }

    async fn decide_when_asked(t: &crate::talk::tests::T, allow: bool) {
        for _ in 0..300 {
            if let Some(p) = t.st.approvals.list().as_array().and_then(|a| a.first().cloned()) {
                assert_eq!(
                    (p["tool"].as_str(), p["hint"].as_str(), p["from"].as_str()),
                    (Some("open_url"), Some("https://example.com/docs"), Some("Agent"))
                );
                t.st.approvals.decide(p["id"].as_str().unwrap(), if allow { "once" } else { "deny" }, "dashboard");
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no approval prompt");
    }

    #[tokio::test]
    async fn mac_tools_ask_on_the_stick() {
        for allow in [true, false] {
            let (t, b, fake) = orb(&format!("or-mac-{allow}")).await;
            fake.reply(tool_call("open_url", json!({"url": "https://example.com/docs"}), "", "call_1"));
            fake.reply(answer(&[if allow { "Opened it." } else { "OK, I won't." }], 0.0));
            let (a, ()) = tokio::join!(ask(&b, "open the docs"), decide_when_asked(&t, allow));
            assert!(a.ok && a.progress[0] == "Opening example.com/docs", "{:?}", a.progress);
            let result =
                fake.body(1)["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap().to_string();
            let ran = t.ran.lock().unwrap().clone();
            if allow {
                assert_eq!(result, "opened example.com");
                assert_eq!(ran, [crate::commands::Action::Open(vec!["https://example.com/docs".into()])]);
            } else {
                assert_eq!(result, "error: the user didn't approve it");
                assert!(ran.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn mac_tool_without_stick_approvals_fails_closed() {
        let (t, b, fake) = orb("or-noapprove").await;
        t.st.cfg.lock().unwrap().insert("stick_approvals".into(), json!(false));
        fake.reply(tool_call("open_app", json!({"name": "Slack"}), "", "call_1"));
        fake.reply(answer(&["I can't without approval."], 0.0));
        assert!(ask(&b, "open slack").await.ok);
        let last = fake.body(1)["messages"].as_array().unwrap().last().unwrap().clone();
        assert!(last["content"].as_str().unwrap().contains("approvals are turned off"), "{last}");
        assert!(t.ran.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn tool_loop_is_capped() {
        let (_t, b, fake) = orb("or-cap").await;
        for i in 0..MAX_ROUNDS {
            fake.reply(tool_call("stick_status", json!({}), "", &format!("c{i}")));
        }
        fake.reply(answer(&["Done checking."], 0.0));
        let a = ask(&b, "check").await;
        assert!(a.ok && a.reply == "Done checking.");
        assert_eq!(fake.n(), MAX_ROUNDS + 1);
        assert!(fake.body(MAX_ROUNDS - 1).get("tools").is_some() && fake.body(MAX_ROUNDS).get("tools").is_none());
    }

    #[tokio::test]
    async fn models_without_tools() {
        // the model refuses tools: asked again without them, and remembered
        let (t, b, fake) = orb("or-notools").await;
        fake.reply(Reply::Status(404, "No endpoints found that support tool use. Try disabling \"tools\".".into()));
        fake.reply(answer(&["Hi."], 0.0));
        fake.reply(answer(&["Hi again."], 0.0));
        let a = ask(&b, "hi").await;
        assert!(a.ok && a.reply == "Hi.");
        assert!(fake.body(0).get("tools").is_some() && fake.body(1).get("tools").is_none());
        ask(&b, "hi").await;
        assert!(fake.body(2).get("tools").is_none());
        // the list says so
        t.st.cfg.lock().unwrap().insert("openrouter_model".into(), json!("test/no-tools"));
        b.list_models(false).await;
        ask(&b, "hi").await;
        assert!(fake.body(3).get("tools").is_none());
        // the stick tools turned off
        t.st.cfg.lock().unwrap().insert("openrouter_model".into(), json!(MODEL));
        t.st.cfg.lock().unwrap().insert("mcp_server".into(), json!(false));
        b.inner.no_tools.lock().unwrap().clear();
        ask(&b, "hi").await;
        assert!(fake.body(4).get("tools").is_none());
    }

    #[tokio::test]
    async fn not_set_up_and_errors() {
        let (t, b, fake) = orb("or-errors").await;
        if std::env::var("STICK_AGENT_OPENROUTER_KEY").is_err() {
            t.st.keys.remove("openrouter").unwrap();
            let a = ask(&b, "hi").await;
            assert_eq!((a.ok, a.reply.as_str()), (false, "Add your OpenRouter key in Settings → Brain."));
            assert_eq!(fake.n(), 0);
            t.st.keys.save("openrouter", KEY).unwrap();
        }
        t.st.cfg.lock().unwrap().insert("openrouter_model".into(), json!(""));
        let a = ask(&b, "hi").await;
        assert!(!a.ok && a.reply.contains("Pick an OpenRouter model") && fake.n() == 0);
        t.st.cfg.lock().unwrap().insert("openrouter_model".into(), json!(MODEL));
        for (status, words) in [
            (401, "rejected the key"),
            (402, "Out of OpenRouter credits"),
            (429, "rate limiting"),
            (503, "unavailable"),
        ] {
            fake.reply(Reply::Status(status, "nope".into()));
            let a = ask(&b, "hi").await;
            assert!(!a.ok && a.reply.contains(words), "{status}: {}", a.reply);
            assert_eq!(b.last()["error"], json!(a.reply));
        }
        fake.offline.store(true, Ordering::Relaxed);
        let a = ask(&b, "hi").await;
        assert_eq!((a.ok, a.reply.as_str()), (false, "Couldn't reach OpenRouter. Check the internet connection."));
    }

    #[tokio::test]
    async fn cancel_mid_stream_and_busy() {
        let (_t, b, fake) = orb("or-cancel").await;
        fake.reply(Reply::Hang(vec![text_chunk("Once upon a time")]));
        let (tx, mut rx) = mpsc::unbounded_channel();
        let task = tokio::spawn(b.ask("tell me a story".into(), tx));
        loop {
            match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await.unwrap() {
                Some(BrainEvent::Text(t)) if !t.is_empty() => break,
                _ => {}
            }
        }
        assert!(b.busy());
        let (tx2, _rx2) = mpsc::unbounded_channel();
        assert_eq!(b.ask("two".into(), tx2).await, (false, "Still working on the last request.".to_string()));
        let t0 = Instant::now();
        b.cancel();
        let r = tokio::time::timeout(Duration::from_secs(2), task).await.unwrap().unwrap();
        assert_eq!(r, (false, "Cancelled.".to_string()));
        assert!(b.cancelled() && !b.busy() && t0.elapsed() < Duration::from_secs(1));
        for _ in 0..300 {
            if fake.disconnected.load(Ordering::Relaxed) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(fake.disconnected.load(Ordering::Relaxed)); // the HTTP stream was closed
        assert!(b.history().is_empty());
    }

    #[tokio::test]
    async fn history_forget_and_idle() {
        let (_t, b, fake) = orb("or-history").await;
        for a in ["Paris.", "About two million.", "Hi.", "Hello."] {
            fake.reply(answer(&[a], 0.0));
        }
        let roles = |i: usize| -> Vec<String> {
            fake.body(i)["messages"]
                .as_array()
                .unwrap()
                .iter()
                .map(|m| m["role"].as_str().unwrap().to_string())
                .collect()
        };
        ask(&b, "capital of France?").await;
        ask(&b, "how many people live there?").await;
        assert_eq!(roles(1), ["system", "user", "assistant", "user"]);
        assert_eq!(fake.body(1)["messages"][2]["content"], "Paris.");
        b.forget();
        ask(&b, "hi").await;
        assert_eq!(roles(2), ["system", "user"]);
        b.set_idle_since(Duration::from_secs(11 * 60)); // brain_idle_min (10) passed
        ask(&b, "hello").await;
        assert_eq!(roles(3), ["system", "user"]);
    }

    #[tokio::test]
    async fn history_is_trimmed_by_whole_turns() {
        let t = agent("or-trim", false).await;
        let b = t.st.openrouter();
        for i in 0..40 {
            b.remember(vec![
                json!({"role": "user", "content": format!("q{i}")}),
                json!({"role": "assistant", "content": "", "tool_calls": []}),
                json!({"role": "tool", "content": "r"}),
                json!({"role": "assistant", "content": format!("a{i}")}),
            ]);
        }
        let h = b.history();
        assert!(h.len() <= HISTORY_MESSAGES);
        assert_eq!((h[0]["role"].as_str(), h.last().unwrap()["content"].as_str()), (Some("user"), Some("a39")));
    }

    #[tokio::test]
    async fn model_list_cached_and_offline() {
        let (_t, b, fake) = orb("or-models").await;
        let (models, suggested, err) = b.list_models(false).await;
        assert!(err.is_none());
        let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
        assert_eq!(ids, [MODEL, "test/no-tools"]); // the image model is out
        assert!(
            (models[0]["in"].as_f64().unwrap() - 0.1).abs() < 1e-9
                && (models[0]["out"].as_f64().unwrap() - 0.4).abs() < 1e-9
        );
        assert_eq!(models[0]["tools"], true);
        assert_eq!(suggested.as_deref(), Some(MODEL)); // no preferred one: the cheapest with tools
        b.list_models(false).await;
        assert_eq!(fake.model_fetches.load(Ordering::Relaxed), 1);
        b.list_models(true).await;
        assert_eq!(fake.model_fetches.load(Ordering::Relaxed), 2);
        fake.offline.store(true, Ordering::Relaxed);
        let (models, _, err) = b.list_models(true).await;
        assert!(err.is_some() && models.len() == 2);
    }

    // --- the dashboard API ------------------------------------------------------------------------

    async fn call(
        app: &axum::Router,
        method: &str,
        path: &str,
        body: Option<Value>,
        tok: Option<&str>,
    ) -> (u16, String) {
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        let mut b = axum::http::Request::builder().method(method).uri(path).header("host", "localhost:8670");
        if let Some(t) = tok {
            b = b.header("x-stick-token", t);
        }
        let body = body.map(|v| axum::body::Body::from(v.to_string())).unwrap_or_default();
        let resp = app.clone().oneshot(b.body(body).unwrap()).await.unwrap();
        let s = resp.status().as_u16();
        (s, String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap())
    }

    #[tokio::test]
    async fn key_api_is_write_only_and_validates() {
        let (t, _b, _fake) = orb("or-api-key").await;
        let app = crate::api::router(t.st.clone());
        let tok = t.st.token.clone();
        let other = format!("sk-or-v1-{}", "fedcba9876543210".repeat(4));
        let (s, _) = call(&app, "PUT", "/api/brain/openrouter/key", Some(json!({"key": other})), None).await;
        assert_eq!(s, 403); // needs the token
        let (s, body) =
            call(&app, "PUT", "/api/brain/openrouter/key", Some(json!({"key": format!("  {other}\n")})), Some(&tok))
                .await;
        assert_eq!(s, 200);
        assert!(!body.contains(&other));
        let v: Value = serde_json::from_str(&body).unwrap();
        if std::env::var("STICK_AGENT_OPENROUTER_KEY").is_err() {
            assert_eq!(v["key"]["hint"], "…3210");
            assert_eq!(t.st.keys.get("openrouter").as_deref(), Some(other.as_str()));
        }
        for path in ["/api/brain", "/api/config", "/api/status"] {
            let (_, text) = call(&app, "GET", path, None, None).await;
            assert!(!text.contains(&other), "{path}");
        }
        assert!(!std::fs::read_to_string(t.st.paths.config()).unwrap_or_default().contains(&other));
        let (_, body) = call(&app, "DELETE", "/api/brain/openrouter/key", None, Some(&tok)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        if std::env::var("STICK_AGENT_OPENROUTER_KEY").is_err() {
            assert_eq!(v["key"]["saved"], false);
            assert_eq!(t.st.keys.get("openrouter"), None);
        }
        for bad in [
            Value::Null,
            json!(""),
            json!("short"),
            json!("has spaces in it 1234567890"),
            json!(42),
            json!("x".repeat(400)),
        ] {
            let (s, _) = call(&app, "PUT", "/api/brain/openrouter/key", Some(json!({"key": bad})), Some(&tok)).await;
            assert_eq!(s, 400, "{bad}");
        }
    }

    #[tokio::test]
    async fn brain_status_models_and_test_api() {
        let (t, _b, fake) = orb("or-api").await;
        let app = crate::api::router(t.st.clone());
        let tok = t.st.token.clone();
        let (_, body) = call(&app, "GET", "/api/brain", None, None).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((v["brain"].as_str(), v["choices"].clone()), (Some("claude"), json!(["claude", "openrouter"])));
        assert_eq!(
            (v["openrouter"]["key"]["saved"].as_bool(), v["openrouter"]["model"].as_str()),
            (Some(true), Some(MODEL))
        );
        let (s, body) = call(&app, "GET", "/api/brain/openrouter/models", None, None).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!((s, v["suggested"].as_str(), v["models"].as_array().unwrap().len()), (200, Some(MODEL), 2));
        call(&app, "GET", "/api/brain/openrouter/models", None, None).await;
        assert_eq!(fake.model_fetches.load(Ordering::Relaxed), 1);
        call(&app, "GET", "/api/brain/openrouter/models?refresh=1", None, None).await;
        assert_eq!(fake.model_fetches.load(Ordering::Relaxed), 2);

        fake.reply(answer(&["Hello from the test model."], 0.00001));
        let (_, body) = call(&app, "POST", "/api/brain/test", Some(json!({"model": "test/other"})), Some(&tok)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(v["ok"] == true && v["reply"] == "Hello from the test model." && v["model"] == "test/other", "{v}");
        assert!((v["cost"].as_f64().unwrap() - 0.00001).abs() < 1e-12 && v["total_ms"].is_u64());
        let req = fake.body(0);
        assert!(req["model"] == "test/other" && req.get("tools").is_none() && req["max_tokens"] == 60);
        let (s, _) = call(&app, "POST", "/api/brain/test", Some(json!({"model": "bad model id!"})), Some(&tok)).await;
        assert_eq!(s, 400);
        fake.reply(Reply::Status(401, "no".into()));
        let (_, body) = call(&app, "POST", "/api/brain/test", Some(json!({})), Some(&tok)).await;
        let v: Value = serde_json::from_str(&body).unwrap();
        assert!(v["ok"] == false && v["error"].as_str().unwrap().contains("rejected the key"));
        for bad in [
            json!({"brain": "gpt"}),
            json!({"openrouter_model": "no spaces allowed"}),
            json!({"openrouter_max_tokens": 5}),
            json!({"openrouter_model": 3}),
        ] {
            let (s, _) = call(&app, "PUT", "/api/config", Some(bad.clone()), Some(&tok)).await;
            assert_eq!(s, 400, "{bad}");
        }
    }

    #[test]
    fn suggest_prefers_known_fast_models() {
        let models = vec![
            json!({"id": "x/pricey", "in": 5.0, "out": 20.0, "tools": true}),
            json!({"id": "anthropic/claude-haiku-5.5", "in": 0.1, "out": 0.5, "tools": true}),
            json!({"id": "x/cheap:free", "in": 0.0, "out": 0.0, "tools": true}),
        ];
        assert_eq!(suggest(&models).as_deref(), Some("anthropic/claude-haiku-5.5"));
        assert_eq!(suggest(&[models[0].clone(), models[2].clone()]).as_deref(), Some("x/pricey"));
        assert_eq!(suggest(&[]), None);
    }

    #[test]
    fn describe_lines() {
        assert_eq!(describe("open_url", &json!({"url": "https://example.com/docs"})), "Opening example.com/docs");
        assert_eq!(describe("stick_status", &json!({})), "Checking the stick");
        assert_eq!(describe("mystery", &json!({})), "mystery");
        assert_eq!(error_message(r#"{"error": {"code": 401, "message": "bad key"}}"#), "bad key");
    }
}
