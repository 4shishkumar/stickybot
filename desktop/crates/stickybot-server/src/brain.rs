//! The Claude brain (a port of `brain.py`, session mode): one long-lived
//! `claude -p --input-format stream-json --output-format stream-json` process; each request is a
//! user message on its stdin, so there's no start-up per request. The user's own Claude Code
//! login; safe tools (config `safe_tools`) run without asking, anything else goes to Claude
//! Code's permission prompt (the hook -> the stick, approvals.rs).
//!
//! The stream-json protocol as brain.py documents it (Claude Code 2.1.x):
//! - in: `{"type":"user","message":{"role":"user","content":…},"parent_tool_use_id":null,"session_id":…}`
//! - cancel: `{"type":"control_request","request_id":"req_<n>_<hex>","request":{"subtype":"interrupt"}}`
//!   (the turn then ends with a `result`, subtype `error_during_execution`)
//! - out: `system` events, `assistant` messages (tool_use blocks -> progress lines), with
//!   `--include-partial-messages` `stream_event`s (message_start, content_block_start text|tool_use,
//!   content_block_delta text_delta), and one `result` per turn.
//!
//! `brain_mode: "oneshot"`: `claude -p "<text>"` per request (the original way), `--resume` within
//! `followup_minutes`; cancel kills it. Both modes get the stick tools (`--mcp-config`, see
//! `mcp.rs`) when `mcp_server` is on.

use crate::talk::{Brain, BrainEvent, plain};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;

pub const TIMEOUT: Duration = Duration::from_secs(240);
/// After an interrupt the turn should end at once; else the process is killed.
pub const INTERRUPT_WAIT: Duration = Duration::from_secs(3);
pub const CLAUDE_MD: &str = include_str!("../../../../mac-agent/stick_agent/defaults/CLAUDE.md");

/// A tool call as a line for the stick (brain.py `describe_tool`).
pub fn describe_tool(name: &str, input: &Value) -> String {
    let s = |k: &str, n: usize| input.get(k).and_then(Value::as_str).unwrap_or("").chars().take(n).collect::<String>();
    match name {
        "Bash" => format!("Running: {}", s("command", 120)),
        "WebSearch" => format!("Searching: {}", s("query", 100)),
        "WebFetch" => {
            let url = input.get("url").and_then(Value::as_str).unwrap_or("");
            let url = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
            format!("Reading {}", url.chars().take(80).collect::<String>())
        }
        _ => {
            for k in ["file_path", "path", "pattern"] {
                if let Some(v) = input.get(k) {
                    let v = v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string());
                    return format!("{name}: {}", v.chars().take(90).collect::<String>());
                }
            }
            name.to_string()
        }
    }
}

/// One request's stream-json events -> progress lines, the answer as it's written, the result
/// (brain.py `Turn`). Text that turns out to be a preamble before a tool call is withdrawn with "".
struct Turn {
    out: mpsc::UnboundedSender<BrainEvent>,
    started: Instant,
    first_token: Option<Duration>,
    events: usize,
    text: String,
    result: Option<String>,
    ok: bool,
    session: Option<String>,
}

impl Turn {
    fn new(out: mpsc::UnboundedSender<BrainEvent>) -> Self {
        Self {
            out,
            started: Instant::now(),
            first_token: None,
            events: 0,
            text: String::new(),
            result: None,
            ok: false,
            session: None,
        }
    }

    fn set_text(&mut self, t: String) {
        if t != self.text {
            self.text = t;
            let _ = self.out.send(BrainEvent::Text(self.text.clone()));
        }
    }

    /// Returns true once the turn is over.
    fn feed(&mut self, ev: &Value) -> bool {
        self.events += 1;
        match ev.get("type").and_then(Value::as_str) {
            Some("stream_event") if ev.get("parent_tool_use_id").is_none_or(Value::is_null) => {
                let e = &ev["event"];
                match e["type"].as_str() {
                    Some("message_start") => self.set_text(String::new()),
                    Some("content_block_start") if e["content_block"]["type"] == "tool_use" => {
                        self.set_text(String::new()) // what it said so far was a preamble
                    }
                    Some("content_block_delta") if e["delta"]["type"] == "text_delta" => {
                        if self.first_token.is_none() {
                            self.first_token = Some(self.started.elapsed());
                        }
                        let t = format!("{}{}", self.text, e["delta"]["text"].as_str().unwrap_or(""));
                        self.set_text(t);
                    }
                    _ => {}
                }
            }
            Some("assistant") => {
                for part in ev["message"]["content"].as_array().into_iter().flatten() {
                    if part["type"] == "tool_use" {
                        let line = describe_tool(part["name"].as_str().unwrap_or(""), &part["input"]);
                        let _ = self.out.send(BrainEvent::Progress(line));
                    }
                }
            }
            Some("result") => {
                self.session = ev.get("session_id").and_then(Value::as_str).map(str::to_string);
                self.ok = !ev.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                let mut result = ev.get("result").and_then(Value::as_str).unwrap_or("").to_string();
                if result.is_empty()
                    && let Some(d) = ev.get("permission_denials").and_then(Value::as_array).and_then(|d| d.first())
                {
                    result = format!(
                        "Not allowed: {}",
                        describe_tool(d["tool_name"].as_str().unwrap_or("?"), &d["tool_input"])
                    );
                }
                if result.is_empty() && ev.get("subtype").and_then(Value::as_str) == Some("error_during_execution") {
                    result = "Stopped.".into();
                }
                self.result = Some(result);
                return true;
            }
            _ => {}
        }
        false
    }
}

/// Where Claude Code is: config `claude_path`, else the usual places (config.py `claude_path`).
pub fn claude_path(cfg: &Map<String, Value>) -> Option<PathBuf> {
    if let Some(p) = cfg.get("claude_path").and_then(Value::as_str).filter(|p| !p.is_empty()) {
        return Some(PathBuf::from(p));
    }
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    let on_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join("claude")).collect::<Vec<_>>())
        .unwrap_or_default();
    on_path
        .into_iter()
        .chain([home.join(".local/bin/claude"), "/opt/homebrew/bin/claude".into(), "/usr/local/bin/claude".into()])
        .find(|p| p.is_file())
}

pub type Config = Arc<dyn Fn() -> Map<String, Value> + Send + Sync>;

struct Proc {
    child: Child,
    sig: String,
    stderr: Arc<Mutex<VecDeque<String>>>,
    stderr_done: Arc<AtomicBool>,
}

struct Inner {
    cfg: Config,
    workspace: PathBuf,
    timeout: Mutex<Duration>,
    interrupt_wait: Mutex<Duration>,
    proc: Mutex<Option<Proc>>,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    /// stdout events of the current process; `None` = it ended
    events: tokio::sync::Mutex<Option<mpsc::UnboundedReceiver<Option<Value>>>>,
    busy: AtomicBool,
    cancelled: AtomicBool,
    fresh: AtomicBool,
    session: Mutex<Option<String>>,
    last_at: Mutex<Option<Instant>>,
    last: Mutex<Value>,
    recent: Mutex<VecDeque<Value>>,
    turn_no: AtomicU64,
    req: AtomicU64,
    idle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// writes the stick tools' `--mcp-config` file (set by the server)
    mcp: Mutex<Option<McpConfig>>,
    /// one-shot mode: stops this request's process (cancel), and its pid
    oneshot_kill: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    oneshot_pid: Mutex<Option<u32>>,
}

type McpConfig = Box<dyn Fn() -> Option<PathBuf> + Send + Sync>;

/// The last `n` bytes of `s` (on a character boundary).
fn tail(s: &str, n: usize) -> &str {
    let mut i = s.len().saturating_sub(n);
    while !s.is_char_boundary(i) {
        i += 1;
    }
    &s[i..]
}

fn mode(cfg: &Map<String, Value>) -> &'static str {
    if cfg.get("brain_mode").and_then(Value::as_str) == Some("oneshot") { "oneshot" } else { "session" }
}

/// Claude Code as the brain. Cheap to clone.
#[derive(Clone)]
pub struct ClaudeBrain {
    inner: Arc<Inner>,
}

impl ClaudeBrain {
    pub fn new(cfg: Config, workspace: PathBuf) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                workspace,
                timeout: Mutex::new(TIMEOUT),
                interrupt_wait: Mutex::new(INTERRUPT_WAIT),
                proc: Mutex::new(None),
                stdin: tokio::sync::Mutex::new(None),
                events: tokio::sync::Mutex::new(None),
                busy: AtomicBool::new(false),
                cancelled: AtomicBool::new(false),
                fresh: AtomicBool::new(false),
                session: Mutex::new(None),
                last_at: Mutex::new(None),
                last: Mutex::new(Value::Null),
                recent: Mutex::new(VecDeque::new()),
                turn_no: AtomicU64::new(0),
                req: AtomicU64::new(0),
                idle: Mutex::new(None),
                mcp: Mutex::new(None),
                oneshot_kill: Mutex::new(None),
                oneshot_pid: Mutex::new(None),
            }),
        }
    }

    /// Shorter waits for tests.
    pub fn set_timings(&self, timeout: Duration, interrupt_wait: Duration) {
        *self.inner.timeout.lock().unwrap() = timeout;
        *self.inner.interrupt_wait.lock().unwrap() = interrupt_wait;
    }

    /// A Claude Code process is running (the session's, or a one-shot request's).
    pub fn alive(&self) -> bool {
        self.session_alive() || self.inner.oneshot_pid.lock().unwrap().is_some()
    }

    fn session_alive(&self) -> bool {
        self.inner.proc.lock().unwrap().as_mut().is_some_and(|p| matches!(p.child.try_wait(), Ok(None)))
    }

    pub fn pid(&self) -> Option<u32> {
        if let Some(p) = *self.inner.oneshot_pid.lock().unwrap() {
            return Some(p);
        }
        if !self.session_alive() {
            return None;
        }
        self.inner.proc.lock().unwrap().as_ref().and_then(|p| p.child.id())
    }

    pub fn session(&self) -> Option<String> {
        self.inner.session.lock().unwrap().clone()
    }

    fn idle_after(cfg: &Map<String, Value>) -> Duration {
        Duration::from_secs_f64(cfg.get("brain_idle_min").and_then(Value::as_f64).unwrap_or(10.0).max(0.0) * 60.0)
    }

    fn signature(&self, cfg: &Map<String, Value>, claude: &Path) -> String {
        json!([claude, cfg.get("model"), cfg.get("safe_tools"), self.inner.workspace, self.mcp(cfg).is_some()])
            .to_string()
    }

    /// The stick tools' `--mcp-config` file, when they're on (config `mcp_server`) and served.
    fn mcp(&self, cfg: &Map<String, Value>) -> Option<PathBuf> {
        if !cfg.get("mcp_server").and_then(Value::as_bool).unwrap_or(true) {
            return None;
        }
        self.inner.mcp.lock().unwrap().as_ref().and_then(|f| f())
    }

    /// The stick tools (`--mcp-config`): `f` writes the config file and returns its path.
    pub fn set_mcp(&self, f: impl Fn() -> Option<PathBuf> + Send + Sync + 'static) {
        *self.inner.mcp.lock().unwrap() = Some(Box::new(f));
    }

    /// brain.py `_common_args`: stream-json out, the model, the stick tools, the safe tools.
    fn common_args(&self, cfg: &Map<String, Value>) -> Vec<String> {
        let mut tools: Vec<String> = cfg
            .get("safe_tools")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let model = cfg.get("model").and_then(Value::as_str).filter(|m| !m.is_empty()).unwrap_or("sonnet");
        let mut a: Vec<String> =
            ["--output-format", "stream-json", "--verbose", "--include-partial-messages", "--model", model]
                .iter()
                .map(|s| s.to_string())
                .collect();
        if let Some(mcp) = self.mcp(cfg) {
            // the feedback tools run without asking; open_url / open_app / quick_command change the
            // Mac, so Claude Code asks (our hook -> A / B on the stick). --mcp-config takes several
            // values, so an option must follow it.
            a.extend(["--mcp-config".to_string(), mcp.display().to_string()]);
            for t in crate::mcp::allowed_tool_names() {
                if !tools.contains(&t) {
                    tools.push(t);
                }
            }
        }
        a.extend(["--allowedTools".to_string(), tools.join(",")]);
        a
    }

    fn args(&self, cfg: &Map<String, Value>, session: Option<&str>) -> Vec<String> {
        let mut a: Vec<String> = ["-p", "--input-format", "stream-json"].iter().map(|s| s.to_string()).collect();
        a.extend(self.common_args(cfg));
        if let Some(s) = session {
            a.extend(["--resume".to_string(), s.to_string()]); // same conversation after a restart
        }
        a
    }

    fn oneshot_args(&self, cfg: &Map<String, Value>, text: &str) -> Vec<String> {
        let mut a = vec!["-p".to_string(), text.to_string()];
        a.extend(self.common_args(cfg));
        let follow = Duration::from_secs_f64(
            cfg.get("followup_minutes").and_then(Value::as_f64).unwrap_or(10.0).max(0.0) * 60.0,
        );
        let recent = self.inner.last_at.lock().unwrap().is_some_and(|t| t.elapsed() < follow);
        if let Some(s) = self.session().filter(|_| recent) {
            a.extend(["--resume".to_string(), s]);
        }
        a
    }

    /// brain.py `_ask_oneshot`: `claude -p "<text>"` for this request only; `--resume` within the
    /// follow-up time. Cancel kills it.
    async fn ask_oneshot(
        &self,
        text: &str,
        cfg: &Map<String, Value>,
        claude: &Path,
        turn: &mut Turn,
        info: &mut Value,
    ) -> (bool, String) {
        let _ = std::fs::create_dir_all(&self.inner.workspace);
        let md = self.inner.workspace.join("CLAUDE.md");
        if !md.exists() {
            let _ = std::fs::write(&md, CLAUDE_MD);
        }
        let args = self.oneshot_args(cfg, text);
        log::info!(
            "claude {}",
            args.iter()
                .map(|a| if a.contains(' ') { format!("{a:?}") } else { a.clone() })
                .collect::<Vec<_>>()
                .join(" ")
        );
        let t = Instant::now();
        let mut child = match Command::new(claude)
            .args(&args)
            .current_dir(&self.inner.workspace)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return (false, format!("Couldn't start Claude Code: {e}")),
        };
        info["spawn_ms"] = json!(t.elapsed().as_millis() as u64);
        let (kill_tx, mut kill_rx) = tokio::sync::oneshot::channel::<()>();
        *self.inner.oneshot_kill.lock().unwrap() = Some(kill_tx);
        *self.inner.oneshot_pid.lock().unwrap() = child.id();
        let mut err = child.stderr.take().expect("piped");
        let stderr = tokio::spawn(async move {
            let mut s = String::new();
            let _ = tokio::io::AsyncReadExt::read_to_string(&mut err, &mut s).await;
            s
        });
        let mut lines = BufReader::new(child.stdout.take().expect("piped")).lines();
        let timeout = *self.inner.timeout.lock().unwrap();
        let read = async {
            loop {
                tokio::select! {
                    line = lines.next_line() => match line {
                        Ok(Some(line)) => {
                            if let Ok(ev) = serde_json::from_str::<Value>(&line)
                                && turn.feed(&ev)
                                && let Some(s) = turn.session.clone()
                            {
                                *self.inner.session.lock().unwrap() = Some(s);
                            }
                        }
                        _ => return true,
                    },
                    _ = &mut kill_rx => return false,
                }
            }
        };
        let finished = tokio::time::timeout(timeout, read).await;
        self.inner.oneshot_kill.lock().unwrap().take();
        self.inner.oneshot_pid.lock().unwrap().take();
        match finished {
            Err(_) => {
                let _ = child.kill().await;
                return (false, "Took too long, stopped.".into());
            }
            Ok(false) => {
                let _ = child.kill().await;
                return (false, "Cancelled.".into());
            }
            Ok(true) => {
                let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
            }
        }
        match turn.result.clone() {
            Some(r) => (turn.ok, plain(&r)),
            None => {
                let err = stderr.await.unwrap_or_default();
                let err = err.trim();
                log::error!("claude failed: {}", tail(err, 500));
                (false, plain(err.lines().last().unwrap_or("Claude Code failed.")))
            }
        }
    }

    /// After the idle time a request starts a new conversation.
    fn expire(&self, cfg: &Map<String, Value>) {
        let stale = self.inner.last_at.lock().unwrap().is_some_and(|t| t.elapsed() >= Self::idle_after(cfg));
        if stale {
            *self.inner.session.lock().unwrap() = None;
        }
    }

    async fn start(&self, cfg: &Map<String, Value>, claude: &Path) -> std::io::Result<u64> {
        let _ = std::fs::create_dir_all(&self.inner.workspace);
        let md = self.inner.workspace.join("CLAUDE.md");
        if !md.exists() {
            let _ = std::fs::write(&md, CLAUDE_MD);
        }
        let session = self.session();
        let args = self.args(cfg, session.as_deref());
        log::info!("claude session: {}", args.join(" "));
        let t = Instant::now();
        let mut child = Command::new(claude)
            .args(&args)
            .current_dir(&self.inner.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let (tx, rx) = mpsc::unbounded_channel();
        let stdout = child.stdout.take().expect("piped");
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && tx.send(Some(v)).is_err()
                {
                    return;
                }
            }
            let _ = tx.send(None);
        });
        let stderr = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_done = Arc::new(AtomicBool::new(false));
        let (buf, done) = (stderr.clone(), stderr_done.clone());
        let err = child.stderr.take().expect("piped");
        tokio::spawn(async move {
            // read it all, or a chatty process would block on a full pipe
            let mut lines = BufReader::new(err).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut b = buf.lock().unwrap();
                if b.len() == 40 {
                    b.pop_front();
                }
                b.push_back(line);
            }
            done.store(true, Ordering::Relaxed);
        });
        *self.inner.stdin.lock().await = child.stdin.take();
        *self.inner.events.lock().await = Some(rx);
        let sig = self.signature(cfg, claude);
        *self.inner.proc.lock().unwrap() = Some(Proc { child, sig, stderr, stderr_done });
        Ok(t.elapsed().as_millis() as u64)
    }

    /// Kills the process first: a turn waiting on its events (holding them) then sees the end.
    async fn kill(&self) {
        let p = self.inner.proc.lock().unwrap().take();
        if let Some(mut p) = p {
            let _ = p.child.kill().await;
        }
        *self.inner.stdin.lock().await = None;
        *self.inner.events.lock().await = None;
    }

    /// End of input: Claude Code exits on its own (killed after 3 s).
    async fn stop(&self) {
        if let Some(h) = self.inner.idle.lock().unwrap().take() {
            h.abort();
        }
        *self.inner.stdin.lock().await = None;
        *self.inner.events.lock().await = None;
        let p = self.inner.proc.lock().unwrap().take();
        if let Some(mut p) = p
            && tokio::time::timeout(Duration::from_secs(3), p.child.wait()).await.is_err()
        {
            let _ = p.child.kill().await;
        }
    }

    async fn send(&self, obj: &Value) -> bool {
        let mut g = self.inner.stdin.lock().await;
        let Some(stdin) = g.as_mut() else { return false };
        let line = format!("{obj}\n");
        stdin.write_all(line.as_bytes()).await.is_ok() && stdin.flush().await.is_ok()
    }

    /// Drops events left over between requests (hook events, a late result). True if the process
    /// has ended meanwhile.
    async fn drain(&self) -> bool {
        let mut g = self.inner.events.lock().await;
        let Some(rx) = g.as_mut() else { return true };
        while let Ok(ev) = rx.try_recv() {
            if ev.is_none() {
                return true;
            }
        }
        false
    }

    fn arm_idle(&self, cfg: &Map<String, Value>) {
        let me = self.clone();
        let after = Self::idle_after(cfg);
        let h = tokio::spawn(async move {
            tokio::time::sleep(after).await;
            if me.inner.busy.load(Ordering::Relaxed) {
                return;
            }
            log::info!("claude session idle: stopping it (next request: a new conversation)");
            *me.inner.session.lock().unwrap() = None;
            me.inner.idle.lock().unwrap().take();
            me.stop().await;
        });
        if let Some(old) = self.inner.idle.lock().unwrap().replace(h) {
            old.abort();
        }
    }

    /// Starts the session process ahead of the first request (when the stick connects).
    pub async fn prewarm_now(&self) {
        let cfg = (self.inner.cfg)();
        if cfg.get("brain").and_then(Value::as_str).is_some_and(|b| b != "claude")
            || !cfg.get("brain_prewarm").and_then(Value::as_bool).unwrap_or(true)
            || mode(&cfg) == "oneshot"
            || self.inner.busy.load(Ordering::Relaxed)
            || self.session_alive()
        {
            return;
        }
        let Some(claude) = claude_path(&cfg) else { return };
        self.expire(&cfg);
        if let Err(e) = self.start(&cfg, &claude).await {
            log::warn!("pre-warm failed: {e}");
            return;
        }
        self.arm_idle(&cfg);
    }

    async fn ask_inner(&self, text: String, out: mpsc::UnboundedSender<BrainEvent>) -> (bool, String) {
        let cfg = (self.inner.cfg)();
        let Some(claude) = claude_path(&cfg) else {
            return (false, "Claude Code not found. Install it, or set its path in the dashboard.".into());
        };
        let mut turn = Turn::new(out);
        let mode = mode(&cfg);
        let mut info = json!({"mode": mode, "warm": false, "spawn_ms": null});
        let r = if mode == "oneshot" {
            if self.session_alive() {
                self.stop().await; // switched from session mode
            }
            self.ask_oneshot(&text, &cfg, &claude, &mut turn, &mut info).await
        } else {
            self.ask_session(&text, &cfg, &claude, &mut turn, &mut info).await
        };
        *self.inner.last_at.lock().unwrap() = Some(Instant::now());
        let cancelled = self.inner.cancelled.load(Ordering::Relaxed);
        let mut last = info;
        last["at"] = json!(crate::hub::now());
        last["ok"] = json!(r.0);
        last["cancelled"] = json!(cancelled);
        last["first_token_ms"] = json!(turn.first_token.map(|d| d.as_millis() as u64));
        last["total_ms"] = json!(turn.started.elapsed().as_millis() as u64);
        if !cancelled && r.0 {
            let mut rec = self.inner.recent.lock().unwrap();
            if rec.len() == 20 {
                rec.pop_front();
            }
            rec.push_back(last.clone());
        }
        log::info!(
            "{mode}{}: first token {}, total {:.1} s{}",
            if last["warm"] == true { " (warm)" } else { "" },
            turn.first_token.map(|d| format!("{:.2} s", d.as_secs_f64())).unwrap_or_else(|| "-".into()),
            turn.started.elapsed().as_secs_f64(),
            if cancelled { " (cancelled)" } else { "" }
        );
        *self.inner.last.lock().unwrap() = last;
        if mode == "session" {
            self.arm_idle(&cfg);
        }
        r
    }

    async fn ask_session(
        &self,
        text: &str,
        cfg: &Map<String, Value>,
        claude: &Path,
        turn: &mut Turn,
        info: &mut Value,
    ) -> (bool, String) {
        self.expire(cfg);
        let sig = self.signature(cfg, claude);
        let changed = self.inner.proc.lock().unwrap().as_ref().is_some_and(|p| p.sig != sig);
        let fresh = self.inner.fresh.swap(false, Ordering::Relaxed);
        if self.session_alive() && (fresh || changed) {
            log::info!("claude session: {}, restarting it", if fresh { "new chat" } else { "settings changed" });
            self.stop().await;
        }
        let timeout = *self.inner.timeout.lock().unwrap();
        for attempt in 1..=2 {
            let mut warm = self.session_alive();
            if warm && self.drain().await {
                // it died while idle
                self.kill().await;
                warm = false;
            }
            if !warm {
                match self.start(cfg, claude).await {
                    Ok(ms) => info["spawn_ms"] = json!(ms),
                    Err(e) => return (false, format!("Couldn't start Claude Code: {e}")),
                }
            }
            info["warm"] = json!(warm);
            let msg = json!({"type": "user", "message": {"role": "user", "content": text},
                             "parent_tool_use_id": null, "session_id": self.session().unwrap_or_default()});
            let run = async {
                if !self.send(&msg).await {
                    return;
                }
                let mut g = self.inner.events.lock().await;
                let Some(rx) = g.as_mut() else { return };
                while let Some(Some(ev)) = rx.recv().await {
                    if turn.feed(&ev) {
                        break;
                    }
                }
            };
            if tokio::time::timeout(timeout, run).await.is_err() {
                log::warn!("claude session: no result in {} s, stopping it", timeout.as_secs_f64());
                self.kill().await;
                return (false, "Took too long, stopped.".into());
            }
            if let Some(result) = turn.result.clone() {
                if let Some(s) = turn.session.clone() {
                    *self.inner.session.lock().unwrap() = Some(s);
                }
                return (turn.ok, plain(&result));
            }
            // the process ended without a result: killed (cancel), or it crashed
            let stderr = self.inner.proc.lock().unwrap().as_ref().map(|p| (p.stderr.clone(), p.stderr_done.clone()));
            self.kill().await;
            if self.inner.cancelled.load(Ordering::Relaxed) {
                return (false, "Cancelled.".into());
            }
            let mut err = String::new();
            if let Some((buf, done)) = stderr {
                for _ in 0..50 {
                    if done.load(Ordering::Relaxed) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                err = buf.lock().unwrap().iter().cloned().collect::<Vec<_>>().join("\n").trim().to_string();
            }
            log::error!("claude session ended mid-request: {}", tail(&err, 500));
            if attempt == 1 && warm && turn.events == 0 {
                log::info!("claude session: retrying in a new process (--resume)");
                continue; // it had died while idle: start again (same conversation) and resend
            }
            let last = err.lines().last().unwrap_or("Claude Code stopped unexpectedly.");
            return (false, plain(last));
        }
        (false, "Claude Code stopped unexpectedly.".into())
    }
}

impl Brain for ClaudeBrain {
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
            me.inner.turn_no.fetch_add(1, Ordering::Relaxed);
            let r = me.ask_inner(text, events).await;
            me.inner.busy.store(false, Ordering::Relaxed);
            r
        })
    }

    fn cancel(&self) {
        if !self.busy() {
            return;
        }
        self.inner.cancelled.store(true, Ordering::Relaxed);
        if let Some(kill) = self.inner.oneshot_kill.lock().unwrap().take() {
            let _ = kill.send(()); // one-shot: the process is the request
            return;
        }
        let me = self.clone();
        let turn = self.inner.turn_no.load(Ordering::Relaxed);
        let n = self.inner.req.fetch_add(1, Ordering::Relaxed) + 1;
        let mut b = [0u8; 4];
        let _ = getrandom::fill(&mut b);
        let id = format!("req_{n}_{}", b.iter().map(|x| format!("{x:02x}")).collect::<String>());
        let wait = *self.inner.interrupt_wait.lock().unwrap();
        tokio::spawn(async move {
            let msg = json!({"type": "control_request", "request_id": id, "request": {"subtype": "interrupt"}});
            if !me.send(&msg).await {
                me.kill().await;
                return;
            }
            tokio::time::sleep(wait).await;
            if me.busy() && me.inner.turn_no.load(Ordering::Relaxed) == turn && me.session_alive() {
                log::warn!("interrupt not answered in {} s: stopping Claude Code", wait.as_secs_f64());
                me.kill().await; // the next request starts it again with --resume
            }
        });
    }

    fn cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Relaxed)
    }

    fn forget(&self) {
        *self.inner.session.lock().unwrap() = None;
        self.inner.fresh.store(true, Ordering::Relaxed);
        if !self.busy() && self.session_alive() {
            let me = self.clone();
            tokio::spawn(async move { me.stop().await });
        }
    }

    fn prewarm(&self) {
        let me = self.clone();
        tokio::spawn(async move { me.prewarm_now().await });
    }

    fn status(&self) -> Value {
        let cfg = (self.inner.cfg)();
        let rec = self.inner.recent.lock().unwrap();
        let mut a = Map::new();
        for m in ["session", "oneshot"] {
            let rows: Vec<&Value> = rec.iter().filter(|r| r["mode"] == m).collect();
            if rows.is_empty() {
                continue;
            }
            let avg = |k: &str| {
                let v: Vec<u64> = rows.iter().filter_map(|r| r[k].as_u64()).collect();
                if v.is_empty() { Value::Null } else { json!(v.iter().sum::<u64>() / v.len() as u64) }
            };
            a.insert(
                m.into(),
                json!({"n": rows.len(), "first_token_ms": avg("first_token_ms"), "total_ms": avg("total_ms")}),
            );
        }
        json!({"brain": cfg.get("brain").cloned().unwrap_or(json!("claude")), "mode": mode(&cfg),
               "alive": self.alive(), "session": self.session().is_some(),
               "last": self.inner.last.lock().unwrap().clone(), "avg": a})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../mac-agent/tests/fake_claude.py");

    struct Fake {
        brain: ClaudeBrain,
        dir: PathBuf,
        cfg: Arc<Mutex<Map<String, Value>>>,
    }

    impl Fake {
        fn calls(&self) -> Vec<Vec<String>> {
            std::fs::read_to_string(self.dir.join("calls.jsonl"))
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
        fn messages(&self) -> Vec<Value> {
            std::fs::read_to_string(self.dir.join("calls.jsonl.msgs"))
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
        fn set(&self, k: &str, v: Value) {
            self.cfg.lock().unwrap().insert(k.into(), v);
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            if let Some(mut p) = self.brain.inner.proc.lock().unwrap().take() {
                let _ = p.child.start_kill();
            }
        }
    }

    /// The brain pointed at fake_claude.py (through a wrapper that sets its environment).
    fn fake(name: &str, env: &str) -> Fake {
        let dir = std::env::temp_dir().join(format!("stickybot-brain-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let wrapper = dir.join("claude");
        std::fs::write(
            &wrapper,
            format!(
                "#!/bin/sh\nFAKE_CLAUDE_LOG={} {env} exec /usr/bin/python3 -I {FAKE} \"$@\"\n",
                dir.join("calls.jsonl").display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let mut c = crate::config::defaults();
        c.insert("claude_path".into(), json!(wrapper));
        let cfg = Arc::new(Mutex::new(c));
        let c2 = cfg.clone();
        let brain = ClaudeBrain::new(Arc::new(move || c2.lock().unwrap().clone()), dir.join("workspace"));
        Fake { brain, dir, cfg }
    }

    struct Answer {
        ok: bool,
        reply: String,
        progress: Vec<String>,
        texts: Vec<String>,
    }

    async fn ask(b: &ClaudeBrain, text: &str) -> Answer {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (ok, reply) = b.ask(text.into(), tx).await;
        let (mut progress, mut texts) = (Vec::new(), Vec::new());
        while let Ok(ev) = rx.try_recv() {
            match ev {
                BrainEvent::Progress(p) => progress.push(p),
                BrainEvent::Text(t) => texts.push(t),
            }
        }
        Answer { ok, reply, progress, texts }
    }

    async fn until(cond: impl Fn() -> bool) {
        for _ in 0..250 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("timed out");
    }

    fn arg<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
        args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).map(String::as_str)
    }

    // --- test_brain.py (session mode) ----------------------------------------------------------------

    #[test]
    fn describe_and_plain() {
        assert_eq!(describe_tool("WebFetch", &json!({"url": "https://example.com/a"})), "Reading example.com/a");
        assert_eq!(describe_tool("Read", &json!({"file_path": "/x/y"})), "Read: /x/y");
        assert_eq!(describe_tool("Bash", &json!({"command": "df -h /"})), "Running: df -h /");
        assert_eq!(describe_tool("Mystery", &json!({})), "Mystery");
    }

    #[tokio::test]
    async fn claude_that_isnt_there() {
        let f = fake("missing", "");
        f.set("claude_path", json!("/nonexistent/claude"));
        let a = ask(&f.brain, "hi").await;
        assert!(!a.ok && a.reply.contains("Couldn't start Claude Code"), "{}", a.reply);
        assert!(!f.brain.busy());
    }

    #[tokio::test]
    async fn session_starts_once_and_is_reused() {
        let f = fake("reuse", "");
        let a = ask(&f.brain, "hello").await;
        assert!(a.ok, "{}", a.reply);
        assert_eq!(a.reply, "You said: hello.");
        assert_eq!(a.texts.last().unwrap(), "You said: hello."); // streamed as it was written
        assert!(a.texts.len() > 2);
        let a = ask(&f.brain, "again").await;
        assert_eq!((a.ok, a.reply.as_str()), (true, "You said: again."));
        let starts = f.calls();
        assert_eq!(starts.len(), 1); // one process for both requests
        assert_eq!(starts[0][..3], ["-p", "--input-format", "stream-json"]);
        for flag in ["--output-format", "--verbose", "--include-partial-messages", "--model", "--allowedTools"] {
            assert!(starts[0].iter().any(|a| a == flag), "{flag}");
        }
        assert!(!starts[0].iter().any(|a| a == "--resume"));
        let msgs = f.messages();
        assert_eq!(msgs[0]["message"], json!({"role": "user", "content": "hello"}));
        assert_eq!(msgs[1]["message"], json!({"role": "user", "content": "again"}));
        assert_eq!(msgs[0]["parent_tool_use_id"], Value::Null);
        let st = f.brain.status();
        assert_eq!(st["last"]["warm"], true);
        assert!(st["last"]["first_token_ms"].is_u64());
        assert_eq!(st["avg"]["session"]["n"], 2);
        assert!(f.brain.session().unwrap().starts_with("sess-") && f.brain.pid().is_some());
        assert!(f.dir.join("workspace/CLAUDE.md").exists());
    }

    #[tokio::test]
    async fn a_preamble_before_a_tool_is_withdrawn() {
        let f = fake("chatter", "");
        let a = ask(&f.brain, "chatter").await;
        assert_eq!((a.ok, a.reply.as_str()), (true, "It's 28°C in Bengaluru. Sunny all day."));
        assert_eq!(a.progress, vec!["Searching: weather bengaluru"]);
        let i = a.texts.iter().position(|t| t == "Let me check the weather.").unwrap();
        assert_eq!(a.texts[i + 1], ""); // withdrawn when the tool call started
        assert_eq!(a.texts.last().unwrap(), &a.reply);
    }

    #[tokio::test]
    async fn busy_rejects_a_second_request_and_cancel_keeps_the_process() {
        let f = fake("cancel", "");
        let b = f.brain.clone();
        let first = tokio::spawn(async move { ask(&b, "slow").await });
        until(|| f.brain.busy() && f.messages().len() == 1).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let a = ask(&f.brain, "another").await;
        assert!(!a.ok && a.reply.contains("Still working"));
        let pid = f.brain.pid();
        let t = Instant::now();
        f.brain.cancel();
        let a = tokio::time::timeout(Duration::from_secs(5), first).await.unwrap().unwrap();
        assert!(!a.ok && f.brain.cancelled() && !f.brain.busy());
        assert!(t.elapsed() < Duration::from_millis(1500)); // the interrupt ended the turn
        assert_eq!(f.brain.pid(), pid); // same process, still warm
        let a = ask(&f.brain, "next").await;
        assert_eq!((a.ok, a.reply.as_str()), (true, "You said: next."));
        assert_eq!(f.calls().len(), 1);
    }

    #[tokio::test]
    async fn cancel_kills_when_the_interrupt_is_ignored() {
        let f = fake("ignore", "FAKE_IGNORE_INTERRUPT=1");
        f.brain.set_timings(TIMEOUT, Duration::from_millis(300));
        ask(&f.brain, "hi").await;
        let sid = f.brain.session().unwrap();
        let b = f.brain.clone();
        let task = tokio::spawn(async move { ask(&b, "slow").await });
        until(|| f.messages().len() == 2).await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        f.brain.cancel();
        let a = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(!a.ok && f.brain.cancelled() && !f.brain.alive());
        assert!(ask(&f.brain, "after").await.ok); // a new process, same conversation
        assert_eq!(arg(&f.calls()[1], "--resume"), Some(sid.as_str()));
    }

    #[tokio::test]
    async fn crash_mid_request_then_resume() {
        let f = fake("crash", "");
        ask(&f.brain, "hi").await;
        let sid = f.brain.session().unwrap();
        let a = ask(&f.brain, "die").await;
        assert!(!a.ok && a.reply.contains("something broke"), "{}", a.reply);
        assert!(!f.brain.alive());
        assert!(ask(&f.brain, "still there?").await.ok);
        assert_eq!(arg(&f.calls()[1], "--resume"), Some(sid.as_str()));
        assert_eq!(f.calls().len(), 2); // no retry of a request that crashed it
    }

    #[tokio::test]
    async fn died_while_idle_is_restarted_with_resume() {
        let f = fake("idle-death", "");
        ask(&f.brain, "hi").await;
        let sid = f.brain.session().unwrap();
        f.brain.inner.proc.lock().unwrap().as_mut().unwrap().child.start_kill().unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let a = ask(&f.brain, "you ok?").await;
        assert_eq!((a.ok, a.reply.as_str()), (true, "You said: you ok?."));
        let starts = f.calls();
        assert_eq!(starts.len(), 2);
        assert_eq!(arg(&starts[1], "--resume"), Some(sid.as_str()));
    }

    #[tokio::test]
    async fn crash_at_start_is_reported() {
        let f = fake("crash-start", "FAKE_CLAUDE_MODE=crash");
        let a = ask(&f.brain, "hi").await;
        assert!(!a.ok && a.reply.contains("not logged in"), "{}", a.reply);
        assert_eq!(f.calls().len(), 1);
    }

    #[tokio::test]
    async fn idle_restart_starts_a_new_conversation() {
        let f = fake("idle", "");
        f.set("brain_idle_min", json!(0.005)); // 0.3 s
        ask(&f.brain, "hi").await;
        assert!(f.brain.alive() && f.brain.session().is_some());
        until(|| !f.brain.alive()).await;
        assert_eq!(f.brain.session(), None);
        ask(&f.brain, "hi again").await;
        assert_eq!(f.calls().len(), 2);
        assert!(!f.calls()[1].iter().any(|a| a == "--resume"));
    }

    #[tokio::test]
    async fn settings_change_restarts_with_resume() {
        let f = fake("settings", "");
        ask(&f.brain, "hi").await;
        let sid = f.brain.session().unwrap();
        f.set("model", json!("opus"));
        assert!(ask(&f.brain, "hi").await.ok);
        let second = &f.calls()[1];
        assert_eq!(arg(second, "--model"), Some("opus"));
        assert_eq!(arg(second, "--resume"), Some(sid.as_str()));
    }

    #[tokio::test]
    async fn forget_starts_a_new_process() {
        let f = fake("forget", "");
        ask(&f.brain, "hi").await;
        f.brain.forget();
        until(|| !f.brain.alive()).await;
        ask(&f.brain, "hi").await;
        assert_eq!(f.calls().len(), 2);
        assert!(!f.calls()[1].iter().any(|a| a == "--resume"));
    }

    #[tokio::test]
    async fn prewarm() {
        let f = fake("prewarm", "");
        f.brain.prewarm_now().await;
        assert!(f.brain.alive());
        until(|| f.calls().len() == 1).await;
        f.brain.prewarm_now().await; // once
        let a = ask(&f.brain, "hi").await;
        assert!(a.ok);
        assert_eq!(f.brain.status()["last"]["warm"], true);
        assert_eq!(f.calls().len(), 1);
    }

    #[tokio::test]
    async fn prewarm_off() {
        let f = fake("prewarm-off", "");
        f.set("brain_prewarm", json!(false));
        f.brain.prewarm_now().await;
        f.set("brain_prewarm", json!(true));
        f.set("brain", json!("openrouter"));
        f.brain.prewarm_now().await;
        assert!(!f.brain.alive());
        assert!(f.calls().is_empty());
    }

    #[tokio::test]
    async fn session_timeout() {
        let f = fake("timeout", "FAKE_IGNORE_INTERRUPT=1");
        f.brain.set_timings(Duration::from_millis(500), INTERRUPT_WAIT);
        let a = ask(&f.brain, "slow").await;
        assert_eq!((a.ok, a.reply.as_str()), (false, "Took too long, stopped."));
        assert!(!f.brain.alive());
    }

    // --- test_brain.py (one-shot mode) ----------------------------------------------------------------

    fn oneshot(name: &str, mode: &str) -> Fake {
        let f = fake(name, &format!("FAKE_CLAUDE_MODE={mode}"));
        f.set("brain_mode", json!("oneshot"));
        f
    }

    #[tokio::test]
    async fn oneshot_answer_progress_and_plain_text() {
        let f = oneshot("os-answer", "answer");
        let a = ask(&f.brain, "weather?").await;
        assert!(a.ok);
        assert_eq!(a.reply, "It's 28°C in Bengaluru."); // markdown and code blocks stripped
        assert_eq!(a.progress, ["Running: df -h /", "Searching: weather bengaluru"]);
        assert_eq!(f.brain.session().as_deref(), Some("sess-1"));
        let args = &f.calls()[0];
        assert_eq!(args[..2], ["-p", "weather?"]);
        assert!(!args.contains(&"--resume".to_string()) && !args.contains(&"--input-format".to_string()));
        assert_eq!(arg(args, "--model"), Some("sonnet"));
        assert!(arg(args, "--allowedTools").unwrap().contains("WebSearch"));
        assert!(args.contains(&"--include-partial-messages".to_string()));
        let st = f.brain.status();
        assert_eq!((st["mode"].as_str(), st["last"]["mode"].as_str()), (Some("oneshot"), Some("oneshot")));
        assert!(st["last"]["first_token_ms"].is_u64());
        assert_eq!(st["avg"]["oneshot"]["n"], 1);
        assert!(!f.brain.alive() && !f.brain.busy());
    }

    #[tokio::test]
    async fn oneshot_follow_up_resumes_the_session() {
        let f = oneshot("os-follow", "answer");
        ask(&f.brain, "weather?").await;
        ask(&f.brain, "and tomorrow?").await;
        assert_eq!(arg(&f.calls()[1], "--resume"), Some("sess-1"));
        f.brain.forget();
        ask(&f.brain, "weather?").await;
        assert!(!f.calls()[2].contains(&"--resume".to_string()));
        // after the follow-up time: a new conversation
        f.set("followup_minutes", json!(0));
        ask(&f.brain, "again?").await;
        assert!(!f.calls()[3].contains(&"--resume".to_string()));
    }

    #[tokio::test]
    async fn oneshot_denied_error_and_crash() {
        let f = oneshot("os-denied", "denied");
        assert_eq!(ask(&f.brain, "x").await.reply, "Not allowed: Running: rm -rf build/");
        let f = oneshot("os-error", "error");
        let a = ask(&f.brain, "x").await;
        assert_eq!((a.ok, a.reply.as_str()), (false, "Rate limited"));
        let f = oneshot("os-crash", "crash");
        let a = ask(&f.brain, "x").await;
        assert!(!a.ok && a.reply.contains("not logged in"), "{}", a.reply);
    }

    #[tokio::test]
    async fn oneshot_timeout_and_cancel() {
        let f = oneshot("os-slow", "slow");
        f.brain.set_timings(Duration::from_millis(500), INTERRUPT_WAIT);
        let a = ask(&f.brain, "x").await;
        assert_eq!((a.ok, a.reply.as_str()), (false, "Took too long, stopped."));
        f.brain.set_timings(TIMEOUT, INTERRUPT_WAIT);
        let b = f.brain.clone();
        let task = tokio::spawn(async move { ask(&b, "long job").await });
        until(|| f.brain.alive()).await;
        assert!(f.brain.busy() && f.brain.pid().is_some());
        f.brain.cancel();
        let a = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(!a.ok && f.brain.cancelled() && !f.brain.busy() && !f.brain.alive());
    }

    #[tokio::test]
    async fn switching_to_oneshot_stops_the_session_and_skips_prewarm() {
        let f = fake("os-switch", "");
        assert!(ask(&f.brain, "hi").await.ok);
        assert!(f.brain.alive());
        f.set("brain_mode", json!("oneshot"));
        let a = ask(&f.brain, "hi").await;
        assert!(a.ok, "{}", a.reply);
        assert!(!f.brain.alive());
        assert_eq!(f.calls()[1][..2], ["-p", "hi"]);
        f.brain.prewarm_now().await;
        assert!(!f.brain.alive() && f.calls().len() == 2);
    }

    #[tokio::test]
    async fn the_stick_tools_are_given_to_both_modes() {
        let f = fake("os-mcp", "FAKE_CLAUDE_MODE=answer");
        let file = f.dir.join("mcp-voice.json");
        let f2 = file.clone();
        f.brain.set_mcp(move || Some(f2.clone()));
        ask(&f.brain, "hi").await;
        let args = &f.calls()[0];
        assert_eq!(arg(args, "--mcp-config"), Some(file.to_str().unwrap()));
        let allowed = arg(args, "--allowedTools").unwrap();
        assert!(allowed.contains("mcp__stick__notify") && allowed.contains("WebSearch"));
        assert!(!allowed.contains("mcp__stick__open_url"));
        // off in Settings: neither the config nor the tools; the session restarts for it
        f.set("mcp_server", json!(false));
        f.set("brain_mode", json!("oneshot"));
        ask(&f.brain, "hi").await;
        let args = &f.calls()[1];
        assert!(arg(args, "--mcp-config").is_none());
        assert!(!arg(args, "--allowedTools").unwrap().contains("mcp__"));
    }

    #[test]
    fn tail_keeps_whole_characters() {
        assert_eq!(tail("héllo", 4), "llo");
        assert_eq!(tail("ab", 10), "ab");
    }

    #[tokio::test]
    async fn a_long_reply_streams_to_the_stick() {
        // the whole agent: talk.rs streams it as "working" updates, then "done"
        let t = crate::talk::tests::agent("brain-stream", false).await;
        let f = fake("stream", "");
        t.st.set_brain(Arc::new(f.brain.clone()));
        t.st.cfg.lock().unwrap().insert("stream_interval_ms".into(), json!(100));
        crate::talk::handle_text(t.st.clone(), "long".into(), "typed").await;
        let screens = t.fake.screens();
        let working = screens.iter().filter(|(s, _, _)| s == "working").count();
        assert_eq!(screens[0].0, "thinking");
        assert!(working >= 2, "{screens:?}");
        let (state, _, text) = screens.last().unwrap();
        assert_eq!(state, "done");
        assert!(text.starts_with("Word0. Word1.") && text.ends_with('…'), "{text}");
    }
}
