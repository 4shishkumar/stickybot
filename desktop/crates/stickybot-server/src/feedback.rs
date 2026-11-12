//! "Report an issue" (the Feedback page): the page is ours; the team's Google Sheet is the
//! database, behind a Google Apps Script web app bound to it (docs/feedback/apps-script.gs). The
//! app is public, so nothing secret is in it: the script's `/exec` address takes anonymous JSON
//! posts. The server posts (not the page: no CORS), with a timeout, at most [`PER_HOUR`] reports an
//! hour, and keeps a redacted copy of each report in `<home>/feedback/` ("My reports"; unsent ones
//! can be retried).
//!
//! Diagnostics are opt-in and **redacted** before the page shows them (the page shows exactly what
//! is sent; the user can edit or untick it): see [`redact`]. Setting it up: docs/FEEDBACK.md.

use crate::AppState;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use regex::{Captures, Regex};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// The diagnostics' limit (the script checks it too; a Sheet cell holds 50 000 characters).
pub const MAX_DIAGNOSTICS: usize = 30_000;
/// each written answer, in characters (the script checks it too)
pub const MAX_TEXT: usize = 4_000;
pub const MAX_CONTACT: usize = 200;
/// lines of app.log in the diagnostics
pub const LOG_LINES: usize = 200;
/// reports an hour from this Mac (a sliding hour; the script allows 10 per install)
pub const PER_HOUR: usize = 5;
pub const HOUR: Duration = Duration::from_secs(3600);
pub const TIMEOUT: Duration = Duration::from_secs(20);
pub const TYPES: [&str; 3] = ["Bug", "Idea", "Question"];
/// the payload's version (the script's `v`)
pub const PAYLOAD_VERSION: u32 = 1;
/// reports "My reports" lists
pub const LIST_MAX: usize = 50;

/// The team's Apps Script web app (docs/FEEDBACK.md): it only appends rows to the feedback Sheet,
/// so the address isn't a secret. Config `feedback_url` overrides it.
#[cfg(not(test))]
pub const DEFAULT_URL: &str =
    "https://script.google.com/macros/s/AKfycbz1YO2s19ZgnXvwhYelhTxYu60bSQStxgwNX3G0rAMTcfB4bhLh9f0_uCdDD5brwysr/exec";
/// Tests never reach the real one: unset means "not set up" there.
#[cfg(test)]
pub const DEFAULT_URL: &str = "";

/// A feedback address we post to: an Apps Script web app over https, or this Mac (tests, a
/// stand-in while developing).
pub fn url_ok(s: &str) -> bool {
    url::Url::parse(s).is_ok_and(|u| match u.scheme() {
        "https" => {
            u.host_str() == Some("script.google.com") && u.path().starts_with("/macros/") && u.path().ends_with("/exec")
        }
        "http" => matches!(u.host_str(), Some("localhost" | "127.0.0.1")),
        _ => false,
    })
}

/// Where reports go: config `feedback_url`, else [`DEFAULT_URL`]; "" = not set up.
pub fn feedback_url(cfg: &Map<String, Value>) -> String {
    url_from(cfg, DEFAULT_URL)
}

fn url_from(cfg: &Map<String, Value>, default: &str) -> String {
    let u = cfg.get("feedback_url").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).unwrap_or(default);
    if url_ok(u) { u.to_string() } else { String::new() }
}

/// What's wrong with a `PUT /api/config` body's `feedback_url` / `dock_always`, or "".
pub fn valid_config(body: &Map<String, Value>) -> String {
    if let Some(b) = body.get("dock_always")
        && !b.is_boolean()
    {
        return "dock_always must be true or false".into();
    }
    if let Some(u) = body.get("feedback_url")
        && !u.as_str().is_some_and(|u| u.is_empty() || url_ok(u))
    {
        return "feedback_url must be the feedback web app's https://script.google.com/macros/s/…/exec address".into();
    }
    String::new()
}

// --- the report ----------------------------------------------------------------------------------

/// A report as sent and as kept in `<home>/feedback/<id>.json`.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Report {
    /// `SB-XXXXXX`, made here (the Sheet's ID column)
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub what: String,
    pub expected: String,
    pub steps: String,
    pub contact: String,
    /// "" = not attached
    pub diagnostics: String,
    /// `StickyBot (Rust) 2.4.0`
    pub app: String,
    pub fw: String,
    /// `15.1 aarch64`
    pub macos: String,
    /// `m5stick-s3 · link connected`
    pub stick: String,
    /// RFC 3339
    pub created: String,
    #[serde(default)]
    pub sent: bool,
    #[serde(default)]
    pub sent_at: String,
    /// why the last try failed
    #[serde(default)]
    pub error: String,
}

fn text_field(b: &Map<String, Value>, k: &str, max: usize) -> Result<String, String> {
    let s = match b.get(k) {
        None | Some(Value::Null) => "",
        Some(Value::String(s)) => s.trim(),
        Some(_) => return Err(format!("{k} must be text")),
    };
    if s.chars().count() > max {
        return Err(format!("{k} is too long (at most {max} characters)"));
    }
    Ok(s.to_string())
}

impl Report {
    /// The page's `POST /api/feedback` body -> a report (the diagnostics are cut to size, not refused).
    pub fn from_body(b: &Map<String, Value>) -> Result<Self, String> {
        let kind = b.get("type").and_then(Value::as_str).unwrap_or("");
        if !TYPES.contains(&kind) {
            return Err(format!("type must be one of: {}", TYPES.join(", ")));
        }
        let what = text_field(b, "what", MAX_TEXT)?;
        if what.is_empty() {
            return Err("Say what happened (or what you'd like)".into());
        }
        let diagnostics = match b.get("diagnostics") {
            Some(Value::String(s)) => s.trim().to_string(),
            None | Some(Value::Null) => String::new(),
            Some(_) => return Err("diagnostics must be text".into()),
        };
        Ok(Self {
            kind: kind.into(),
            what,
            expected: text_field(b, "expected", MAX_TEXT)?,
            steps: text_field(b, "steps", MAX_TEXT)?,
            contact: text_field(b, "contact", MAX_CONTACT)?,
            diagnostics: cut_front(&diagnostics, MAX_DIAGNOSTICS),
            ..Default::default()
        })
    }

    /// The facts the Sheet has columns for.
    pub fn stamp(&mut self, f: &Facts) {
        self.app = format!("{} {}", f.app, f.version);
        self.fw = f.firmware.clone().unwrap_or_default();
        self.macos = format!("{} {}", f.macos, f.arch);
        self.stick = format!("{} · link {}", f.model.as_deref().unwrap_or("unknown model"), f.link);
    }

    /// The JSON the script takes (`doPost`): no local bookkeeping (sent, error).
    pub fn payload(&self, install: &str) -> Value {
        json!({
            "v": PAYLOAD_VERSION, "id": self.id, "install": install, "type": self.kind, "what": self.what,
            "expected": self.expected, "steps": self.steps, "contact": self.contact, "app": self.app,
            "fw": self.fw, "macos": self.macos, "stick": self.stick, "diagnostics": self.diagnostics,
            "created": self.created, "hp": "",
        })
    }

    /// The report as plain text ("Copy report").
    pub fn text(&self) -> String {
        let mut out = format!(
            "StickyBot {} report{}\n",
            self.kind.to_lowercase(),
            if self.id.is_empty() { String::new() } else { format!(" {}", self.id) }
        );
        let version = [self.app.as_str(), &self.fw, &self.macos]
            .iter()
            .zip(["", "firmware ", "macOS "])
            .filter(|(v, _)| !v.is_empty())
            .map(|(v, p)| format!("{p}{v}"))
            .collect::<Vec<_>>()
            .join(" · ");
        for (title, v) in [
            ("What happened", self.what.as_str()),
            ("What you expected", &self.expected),
            ("Steps", &self.steps),
            ("Contact", &self.contact),
            ("Version", &version),
            ("Stick", &self.stick),
            ("Diagnostics", &self.diagnostics),
        ] {
            if !v.is_empty() {
                out.push_str(&format!("\n## {title}\n{v}\n"));
            }
        }
        out
    }

    /// For "My reports": no diagnostics, the text cut.
    pub fn summary(&self) -> Value {
        json!({"id": self.id, "type": self.kind, "what": self.what.chars().take(140).collect::<String>(),
               "created": self.created, "sent": self.sent, "sent_at": self.sent_at, "error": self.error,
               "diagnostics": !self.diagnostics.is_empty()})
    }
}

/// Keeps the last `max` bytes (whole lines) of `s`, with a note that the start was cut.
pub fn cut_front(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    const NOTE: &str = "[… the start was cut to fit]\n";
    let mut from = s.len() - (max - NOTE.len());
    while !s.is_char_boundary(from) {
        from += 1;
    }
    let rest = &s[from..];
    let rest = rest.find('\n').map_or(rest, |i| &rest[i + 1..]);
    format!("{NOTE}{rest}")
}

const ID_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0/O, 1/I

/// `SB-` and 6 letters / digits that can't be misread.
pub fn new_id() -> String {
    let mut b = [0u8; 6];
    getrandom::fill(&mut b).expect("system randomness");
    format!("SB-{}", b.iter().map(|x| ID_CHARS[*x as usize % ID_CHARS.len()] as char).collect::<String>())
}

pub fn id_ok(id: &str) -> bool {
    id.strip_prefix("SB-").is_some_and(|r| r.len() == 6 && r.bytes().all(|c| ID_CHARS.contains(&c)))
}

// --- the local copies ----------------------------------------------------------------------------

/// `<home>/feedback/`: one `<id>.json` per report, and `install-id` (a random id the script
/// rate-limits by; nothing about the Mac).
pub struct Store {
    pub dir: PathBuf,
}

impl Store {
    pub fn new(home: &std::path::Path) -> Self {
        Self { dir: home.join("feedback") }
    }

    pub fn install_id(&self) -> String {
        let p = self.dir.join("install-id");
        if let Ok(s) = std::fs::read_to_string(&p)
            && s.trim().len() == 16
        {
            return s.trim().to_string();
        }
        let mut b = [0u8; 8];
        getrandom::fill(&mut b).expect("system randomness");
        let id: String = b.iter().map(|x| format!("{x:02x}")).collect();
        let _ = std::fs::create_dir_all(&self.dir);
        let _ = std::fs::write(&p, &id);
        id
    }

    pub fn save(&self, r: &Report) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let tmp = self.dir.join(format!("{}.json.tmp", r.id));
        std::fs::write(&tmp, serde_json::to_string_pretty(r).unwrap_or_default()).map_err(|e| e.to_string())?;
        std::fs::rename(tmp, self.dir.join(format!("{}.json", r.id))).map_err(|e| e.to_string())
    }

    pub fn load(&self, id: &str) -> Option<Report> {
        if !id_ok(id) {
            return None;
        }
        serde_json::from_str(&std::fs::read_to_string(self.dir.join(format!("{id}.json"))).ok()?).ok()
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Report> {
        let mut out: Vec<Report> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                self.load(name.strip_suffix(".json")?)
            })
            .collect();
        out.sort_by(|a, b| b.created.cmp(&a.created));
        out.truncate(LIST_MAX);
        out
    }
}

// --- sending -------------------------------------------------------------------------------------

/// At most `max` in any `window` (a sliding window).
pub struct RateLimit {
    max: usize,
    window: Duration,
    sent: Mutex<VecDeque<Instant>>,
}

impl RateLimit {
    pub fn new(max: usize, window: Duration) -> Self {
        Self { max, window, sent: Mutex::new(VecDeque::new()) }
    }

    fn forget_old(&self, now: Instant, sent: &mut VecDeque<Instant>) {
        while sent.front().is_some_and(|t| now.saturating_duration_since(*t) >= self.window) {
            sent.pop_front();
        }
    }

    /// Takes one -> Ok, or Err(how long until the next one is allowed).
    pub fn take(&self, now: Instant) -> Result<(), Duration> {
        let mut sent = self.sent.lock().unwrap();
        self.forget_old(now, &mut sent);
        if sent.len() >= self.max {
            let oldest = *sent.front().expect("full");
            return Err(self.window.saturating_sub(now.saturating_duration_since(oldest)));
        }
        sent.push_back(now);
        Ok(())
    }

    pub fn left(&self, now: Instant) -> usize {
        let mut sent = self.sent.lock().unwrap();
        self.forget_old(now, &mut sent);
        self.max.saturating_sub(sent.len())
    }
}

/// POSTs JSON and reads the JSON answer. Blocking: call it from `spawn_blocking`.
pub trait Poster: Send + Sync {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, String>;
}

/// The real one (ureq, [`TIMEOUT`]). Apps Script answers a POST with a 302 to
/// script.googleusercontent.com, where the answer is fetched with a GET: ureq does exactly that
/// for a 301 / 302 / 303 (the body isn't sent again).
pub struct UreqPoster;

impl Poster for UreqPoster {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, String> {
        let agent = ureq::AgentBuilder::new().timeout(TIMEOUT).redirects(5).user_agent(crate::net::USER_AGENT).build();
        let resp = match agent.post(url).set("Content-Type", "application/json").send_string(&body.to_string()) {
            Ok(r) => r,
            Err(ureq::Error::Status(code, _)) => return Err(format!("the feedback service answered {code}")),
            Err(ureq::Error::Transport(t)) => return Err(t.to_string()),
        };
        let text = resp.into_string().map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|_| {
            // a sign-in page: the web app isn't open to "Anyone"
            "the feedback service didn't answer with JSON (is the web app deployed for \"Anyone\"?)".to_string()
        })
    }
}

/// For tests (and the fake stick): records the posts, never touches the network.
#[derive(Default)]
pub struct FakePoster {
    pub sent: Mutex<Vec<(String, Value)>>,
    pub fail: Mutex<Option<String>>,
}

impl Poster for FakePoster {
    fn post_json(&self, url: &str, body: &Value) -> Result<Value, String> {
        if let Some(e) = self.fail.lock().unwrap().clone() {
            return Err(e);
        }
        self.sent.lock().unwrap().push((url.to_string(), body.clone()));
        Ok(json!({"ok": true, "id": body["id"]}))
    }
}

/// The script's answer -> Ok, or why not.
pub fn accepted(answer: &Value, id: &str) -> Result<(), String> {
    if answer["ok"] != true {
        return Err(answer["error"]
            .as_str()
            .unwrap_or("the feedback service refused the report")
            .chars()
            .take(200)
            .collect());
    }
    match answer["id"].as_str() {
        Some(got) if got != id => Err(format!("the feedback service stored it as {got}, not {id}")),
        _ => Ok(()),
    }
}

pub struct Feedback {
    pub limit: RateLimit,
    poster: Mutex<Arc<dyn Poster>>,
}

impl Feedback {
    /// `fake`: the fake stick's app (tests, demos) never posts anywhere.
    pub fn new(fake: bool) -> Self {
        let poster: Arc<dyn Poster> = if fake { Arc::new(FakePoster::default()) } else { Arc::new(UreqPoster) };
        Self { limit: RateLimit::new(PER_HOUR, HOUR), poster: Mutex::new(poster) }
    }

    pub fn poster(&self) -> Arc<dyn Poster> {
        self.poster.lock().unwrap().clone()
    }

    pub fn set_poster(&self, p: Arc<dyn Poster>) {
        *self.poster.lock().unwrap() = p;
    }
}

// --- redaction -----------------------------------------------------------------------------------

/// What this Mac knows that must not leave it, besides what the patterns catch: the app's token,
/// the user and host names, the stick's names and addresses, the GitHub watcher's repos and PR
/// titles, Wi-Fi names the stick reported.
#[derive(Debug, Clone, Default)]
pub struct Known {
    pub token: String,
    pub user: String,
    pub host: String,
    pub sticks: Vec<String>,
    pub addresses: Vec<String>,
    pub repos: Vec<String>,
    pub titles: Vec<String>,
    pub wifi: Vec<String>,
    /// titles, attendees, locations and links of the Calendar connection's cached events
    pub meetings: Vec<String>,
}

impl Known {
    pub fn of(st: &AppState, stick_state: Option<&Value>) -> Self {
        let cfg = st.cfg.lock().unwrap().clone();
        let mut k = Known {
            token: st.token.clone(),
            user: std::env::var("USER").unwrap_or_default(),
            host: hostname(),
            ..Default::default()
        };
        if let Some(s) = cfg.get("stick") {
            push_str(&mut k.sticks, &s["name"]);
            push_str(&mut k.addresses, &s["address"]);
        }
        if let Some(s) = stick_state {
            push_str(&mut k.sticks, &s["name"]);
            push_str(&mut k.wifi, &s["web"]["ssid"]);
            push_str(&mut k.wifi, &s["web"]["homeSsid"]);
        }
        push_str(&mut k.repos, &cfg.get("github").cloned().unwrap_or_default()["primary_repo"]);
        if let Some(g) = st.home.gh_state.lock().unwrap().as_ref() {
            for group in ["prs", "asks"] {
                for pr in g[group].as_object().into_iter().flat_map(Map::values) {
                    push_str(&mut k.repos, &pr["repo"]);
                    push_str(&mut k.titles, &pr["title"]);
                }
            }
            push_str(&mut k.repos, &g["primary"]);
        }
        k.meetings = st.calendar.private_strings();
        k
    }
}

fn push_str(v: &mut Vec<String>, s: &Value) {
    if let Some(s) = s.as_str().map(str::trim).filter(|s| !s.is_empty())
        && !v.iter().any(|x| x == s)
    {
        v.push(s.to_string());
    }
}

fn hostname() -> String {
    std::process::Command::new("/bin/hostname")
        .arg("-s")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// A short, stable stand-in for an identifier (the same stick gives the same hash in two reports).
pub fn short_hash(s: &str) -> String {
    let d = Sha256::digest(s.to_lowercase().as_bytes());
    d.iter().take(3).map(|b| format!("{b:02x}")).collect()
}

fn re(s: &str) -> Regex {
    Regex::new(s).expect("a valid pattern")
}

/// Whole lines (or their ends) that carry what someone said or wrote.
static LINE_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // talk.rs: what the stick heard
        (re(r"(?m)(heard \([^)\n]*\)): .*$"), "$1: [voice transcript]"),
        // events.rs: stick events carry PR titles, Claude's notes, …
        (re(r"(?m)(event [\w-]+ (?:-> [\w-]+|\(off\))): .*$"), "$1: [event text]"),
        // brain.rs: the claude command line carries the request
        (re(r"(?m)(brain: claude) -.*$"), "$1 [arguments]"),
        // mcp.rs: a tool call's arguments and result (notify texts, questions, …)
        (re(r"(?m)(mcp: [^:\n]+: [\w.-]+)\(.*$"), "$1(…)"),
        (re(r"(?m)(the stick is now called ).*$"), "$1[stick name]"),
        (re(r"(?m)(connected to ).+?( \(MTU)"), "$1[stick name]$2"),
        // anything labelled as said / asked / answered
        (
            re(
                r"(?im)\b(heard|transcript|transcribed|reply|replied|answer|said|question|prompt|asked|text|title)(\s*[:=]\s*).+$",
            ),
            "$1$2[text]",
        ),
    ]
});

static KEYS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        (re(r"\bsk-[A-Za-z0-9_-]{10,}"), "[key]"),
        (re(r"\bgithub_pat_[A-Za-z0-9_]{20,}"), "[github token]"),
        (re(r"\bgh[pousr]_[A-Za-z0-9]{20,}"), "[github token]"),
        (
            re(
                r#"(?i)\b(x-stick-token|token|api[_-]?key|secret|password|passwd|authorization)(["']?\s*[:=]\s*["']?)(?:bearer\s+)?[^\s"',;}\[\]][^\s"',;}\]]*"#,
            ),
            "$1$2[redacted]",
        ),
        (re(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=-]+"), "Bearer [redacted]"),
    ]
});

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"));
static HOME_PATH: LazyLock<Regex> = LazyLock::new(|| re(r"/Users/[^/\s:'\x22]+"));
static GITHUB_URL: LazyLock<Regex> = LazyLock::new(|| re(r"(github\.com[/:])[\w.-]+/[\w.-]+"));
static MAC: LazyLock<Regex> = LazyLock::new(|| re(r"\b(?:[0-9A-Fa-f]{2}[:-]){5}[0-9A-Fa-f]{2}\b"));
static UUID: LazyLock<Regex> =
    LazyLock::new(|| re(r"\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b"));
static SERIAL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)\b(serial(?:[ _-]?(?:number|no))?|sn)(\s*[:=#]\s*["']?)([A-Za-z0-9_-]{4,})"#));
static WIFI: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r#"(?i)\b(ssid|home_?ssid|wi-?fi(?: network| name)?|network name)(["']?\s*[:=]\s*)("[^"\n]*"|'[^'\n]*'|[^\s,;}]+)"#,
    )
});

/// Replaces `lit` (case-insensitive) with `with`; as a whole word when it starts and ends with one.
fn replace_literal(text: &str, lit: &str, with: &str) -> String {
    if lit.is_empty() {
        return text.to_string();
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
    let mut pat = format!("(?i){}", regex::escape(lit));
    if word(lit.chars().next()) {
        pat = format!(r"(?i)\b{}", regex::escape(lit));
    }
    if word(lit.chars().last()) {
        pat.push_str(r"\b");
    }
    match Regex::new(&pat) {
        Ok(r) => r.replace_all(text, regex::NoExpand(with)).into_owned(),
        Err(_) => text.replace(lit, with),
    }
}

/// Only secrets: keys, tokens, the app's token (for what the user wrote themselves).
pub fn redact_secrets(text: &str, token: &str) -> String {
    let mut s = if token.len() >= 8 { text.replace(token, "[token]") } else { text.to_string() };
    for (r, with) in KEYS.iter() {
        s = r.replace_all(&s, *with).into_owned();
    }
    s
}

/// Everything personal or secret out of `text` (the diagnostics):
/// - keys and tokens: the app's token, `sk-…` (OpenRouter, Anthropic), `ghp_` / `github_pat_` …,
///   `Bearer …`, `token=` / `api_key:` / `password=` …
/// - email addresses
/// - `/Users/<name>` paths -> `~`; the user name and the Mac's name
/// - GitHub repos (`owner/repo`, the owner, github.com links) and the watcher's PR titles
/// - the Calendar connection's meeting titles, attendees, locations and links -> `[meeting]`
/// - what was said or written: voice transcripts, event texts, the claude command line, tool-call
///   arguments, anything labelled reply / answer / question / text
/// - Wi-Fi names; the stick's name; Bluetooth addresses, device UUIDs and serial numbers -> a short hash
pub fn redact(text: &str, k: &Known) -> String {
    let mut s = text.to_string();
    // the known literals first, longest first (a PR title may hold a repo name)
    let mut lits: Vec<(&str, String)> = Vec::new();
    for t in &k.titles {
        if t.chars().count() >= 4 {
            lits.push((t, "[PR title]".into()));
        }
    }
    for m in &k.meetings {
        // short titles ("1:1", "Gym") too: `replace_literal` matches a literal that starts and ends with a
        // letter or digit as a whole word only, so they do not blank parts of other words
        if m.chars().count() >= 3 {
            lits.push((m, "[meeting]".into()));
        }
    }
    for r in &k.repos {
        lits.push((r, "[repo]".into()));
        if let Some((owner, name)) = r.split_once('/') {
            if owner.len() >= 3 {
                lits.push((owner, "[org]".into()));
            }
            if name.len() >= 4 {
                lits.push((name, "[repo]".into()));
            }
        }
    }
    for w in &k.wifi {
        lits.push((w, "[wifi]".into()));
    }
    for n in &k.sticks {
        if n.chars().count() >= 2 {
            lits.push((n, "[stick name]".into()));
        }
    }
    for a in &k.addresses {
        lits.push((a, format!("[bt:{}]", short_hash(a))));
    }
    lits.sort_by_key(|(l, _)| std::cmp::Reverse(l.len()));
    if k.token.len() >= 8 {
        s = s.replace(&k.token, "[token]");
    }
    for (lit, with) in &lits {
        s = replace_literal(&s, lit, with);
    }
    for (r, with) in LINE_RULES.iter() {
        s = r.replace_all(&s, *with).into_owned();
    }
    s = redact_secrets(&s, "");
    s = EMAIL.replace_all(&s, "[email]").into_owned();
    s = HOME_PATH.replace_all(&s, "~").into_owned();
    s = GITHUB_URL.replace_all(&s, "${1}[repo]").into_owned();
    s = MAC.replace_all(&s, |c: &Captures| format!("[bt:{}]", short_hash(&c[0]))).into_owned();
    s = UUID.replace_all(&s, |c: &Captures| format!("[id:{}]", short_hash(&c[0]))).into_owned();
    s = SERIAL.replace_all(&s, |c: &Captures| format!("{}{}[serial:{}]", &c[1], &c[2], short_hash(&c[3]))).into_owned();
    s = WIFI.replace_all(&s, "$1$2[wifi]").into_owned();
    if k.user.len() >= 3 {
        s = replace_literal(&s, &k.user, "[user]");
    }
    if k.host.len() >= 3 {
        s = replace_literal(&s, &k.host, "[mac name]");
    }
    s
}

// --- diagnostics ---------------------------------------------------------------------------------

/// What the diagnostics say about this Mac and the app.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    pub app: String,
    pub version: String,
    pub firmware: Option<String>,
    pub macos: String,
    pub arch: String,
    pub model: Option<String>,
    pub link: String,
    pub features: Vec<String>,
}

impl Facts {
    /// The form's "version" answer: `StickyBot (Rust) 2.4.0 · firmware 2.5.0 · macOS 15.1 arm64`.
    pub fn version_line(&self) -> String {
        let mut v = format!("{} {}", self.app, self.version);
        if let Some(fw) = &self.firmware {
            v.push_str(&format!(" · firmware {fw}"));
        }
        v.push_str(&format!(" · macOS {} {}", self.macos, self.arch));
        v
    }
}

/// The settings worth knowing about, as `name` / `name=value` (no free text: no repo, no paths).
pub fn features(cfg: &Map<String, Value>, speech_ready: bool) -> Vec<String> {
    let on = |k: &str, d: bool| cfg.get(k).and_then(Value::as_bool).unwrap_or(d);
    let word = |k: &str| {
        cfg.get(k)
            .and_then(Value::as_str)
            .filter(|v| v.len() <= 30 && v.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)))
    };
    let mut out = Vec::new();
    for k in ["brain", "brain_mode", "model", "speak_output", "wireless_mic_carrier", "update_channel"] {
        if let Some(v) = word(k) {
            out.push(format!("{k}={v}"));
        }
    }
    for (k, d) in [
        ("mcp_server", true),
        ("stick_approvals", true),
        ("speak_replies", false),
        ("stream_replies", true),
        ("wireless_mic", false),
        ("weather", true),
        ("ticker", true),
        ("update_check", true),
        ("dock_always", false),
    ] {
        out.push(format!("{k}={}", if on(k, d) { "on" } else { "off" }));
    }
    let gh = cfg.get("github").and_then(|g| g.get("on")).and_then(Value::as_bool).unwrap_or(true);
    out.push(format!("github_watcher={}", if gh { "on" } else { "off" }));
    let calendar =
        cfg.get("connections").and_then(|c| c.get("calendar")).and_then(|c| c.get("on")).and_then(Value::as_bool);
    out.push(format!("calendar={}", if calendar.unwrap_or(false) { "on" } else { "off" }));
    out.push(format!("speech_model={}", if speech_ready { "ready" } else { "not ready" }));
    out
}

/// The diagnostics text (redacted, at most [`MAX_DIAGNOSTICS`] bytes: the oldest log lines go first).
pub fn diagnostics(f: &Facts, log: &[String], k: &Known) -> String {
    let head = format!(
        "App: {} {}\nFirmware: {}\nmacOS: {} ({})\nStick: {} · link {}\nFeatures: {}\n\n--- last {} lines of app.log ---\n",
        f.app,
        f.version,
        f.firmware.as_deref().unwrap_or("unknown (stick not connected)"),
        f.macos,
        f.arch,
        f.model.as_deref().unwrap_or("unknown model"),
        f.link,
        f.features.join(", "),
        log.len(),
    );
    let head = redact(&head, k);
    let mut lines: VecDeque<String> = log.iter().map(|l| redact(l, k)).collect();
    let mut size = head.len() + lines.iter().map(|l| l.len() + 1).sum::<usize>();
    while size > MAX_DIAGNOSTICS {
        match lines.pop_front() {
            Some(l) => size -= l.len() + 1,
            None => break,
        }
    }
    let mut out = head;
    for l in lines {
        out.push_str(&l);
        out.push('\n');
    }
    cut_front(out.trim_end(), MAX_DIAGNOSTICS)
}

fn macos_version() -> String {
    std::process::Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// Reads the facts off the running app (asks a connected stick for its `@state`) -> facts, known
/// secrets.
pub async fn gather(st: &Arc<AppState>) -> (Facts, Known) {
    let stick = if st.link.connected() { st.link.request("@state").await } else { None };
    let cfg = st.cfg.lock().unwrap().clone();
    let macos = tokio::task::spawn_blocking(macos_version).await.unwrap_or_else(|_| "unknown".into());
    let s = |k: &str| stick.as_ref().and_then(|s| s.get(k)).and_then(Value::as_str).map(str::to_string);
    let facts = Facts {
        app: st.app_name().into(),
        version: env!("CARGO_PKG_VERSION").into(),
        firmware: s("fw"),
        macos,
        arch: std::env::consts::ARCH.into(),
        model: s("board").or_else(|| s("model")),
        link: st.link.state().as_str().into(),
        features: features(&cfg, st.speech.ready()),
    };
    let st2 = st.clone();
    let known = tokio::task::spawn_blocking(move || Known::of(&st2, stick.as_ref())).await.unwrap_or_default();
    (facts, known)
}

// --- the API -------------------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/feedback", get(info).post(send))
        .route("/api/feedback/diagnostics", get(diagnostics_api))
        .route("/api/feedback/reports", get(reports))
        .route("/api/feedback/reports/{id}", get(report_one))
        .route("/api/feedback/reports/{id}/retry", post(retry))
}

fn answer(status: StatusCode, v: Value) -> Response {
    (status, axum::Json(v)).into_response()
}

fn store(st: &AppState) -> Store {
    Store::new(&st.paths.home)
}

/// `{configured, left, per_hour, max_diagnostics, max_text, types}`
async fn info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let configured = !feedback_url(&st.cfg.lock().unwrap()).is_empty();
    axum::Json(json!({
        "configured": configured, "types": TYPES, "left": st.feedback.limit.left(Instant::now()),
        "per_hour": PER_HOUR, "max_diagnostics": MAX_DIAGNOSTICS, "max_text": MAX_TEXT,
    }))
}

/// `{text, bytes, version}`: the redacted diagnostics, exactly as they would be sent.
async fn diagnostics_api(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let (facts, known) = gather(&st).await;
    let log = crate::logs::tail(&st.paths.log(), LOG_LINES);
    let text = tokio::task::spawn_blocking({
        let f = facts.clone();
        move || diagnostics(&f, &log, &known)
    })
    .await
    .unwrap_or_default();
    axum::Json(json!({"bytes": text.len(), "text": text, "version": facts.version_line()}))
}

/// "My reports": the local copies, newest first (no diagnostics).
async fn reports(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let st2 = st.clone();
    let list = tokio::task::spawn_blocking(move || store(&st2).list()).await.unwrap_or_default();
    axum::Json(Value::Array(list.iter().map(Report::summary).collect()))
}

/// One report as text (to copy an unsent one).
async fn report_one(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match store(&st).load(&id) {
        Some(r) => axum::Json(json!({"summary": r.summary(), "report": r.text()})).into_response(),
        None => answer(StatusCode::NOT_FOUND, json!({"error": "no such report"})),
    }
}

/// Rate limit, post, note the result in the local copy -> the page's answer.
async fn deliver(st: &Arc<AppState>, mut r: Report) -> Response {
    let url = feedback_url(&st.cfg.lock().unwrap());
    let report = r.text();
    if url.is_empty() {
        return answer(
            StatusCode::CONFLICT,
            json!({"error": "Feedback isn't set up yet in this version — copy the report and send it another way.",
                   "not_configured": true, "id": r.id, "report": report}),
        );
    }
    let store = store(st);
    if let Err(wait) = st.feedback.limit.take(Instant::now()) {
        let min = wait.as_secs().div_ceil(60).max(1);
        r.error = "not sent: too many reports this hour".into();
        let _ = store.save(&r);
        return answer(
            StatusCode::TOO_MANY_REQUESTS,
            json!({"error": format!("That's {PER_HOUR} reports this hour — it's kept under My reports: retry in {min} min, or copy it."),
                   "retry_after": wait.as_secs(), "id": r.id, "report": report}),
        );
    }
    let poster = st.feedback.poster();
    let install = store.install_id();
    let body = r.payload(&install);
    let id = r.id.clone();
    let res = tokio::task::spawn_blocking(move || poster.post_json(&url, &body).and_then(|a| accepted(&a, &id)))
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
    match res {
        Ok(()) => {
            r.sent = true;
            r.sent_at = chrono::Utc::now().to_rfc3339();
            r.error.clear();
            let _ = store.save(&r);
            log::info!("feedback: {} sent", r.id);
            axum::Json(json!({"ok": true, "id": r.id, "sent": true})).into_response()
        }
        Err(e) => {
            log::warn!("feedback: couldn't send {} ({e})", r.id);
            r.error = e.clone();
            let _ = store.save(&r);
            answer(
                StatusCode::BAD_GATEWAY,
                json!({"error": "Couldn't reach the feedback service (are you online?). It's kept under My reports: retry later, or copy it.",
                       "detail": e, "id": r.id, "report": report}),
            )
        }
    }
}

/// `{type, what, expected, steps, contact, diagnostics}` -> `{ok, id, sent}`, or `{error, id,
/// report}` (the report as text, for "Copy report").
async fn send(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = match serde_json::from_slice::<Value>(&body) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let mut r = match Report::from_body(&b) {
        Ok(r) => r,
        Err(e) => return answer(StatusCode::BAD_REQUEST, json!({"error": e})),
    };
    let (facts, known) = gather(&st).await;
    r.stamp(&facts);
    r.id = new_id();
    r.created = chrono::Utc::now().to_rfc3339();
    // what the user wrote keeps their words, minus any key pasted by mistake; the diagnostics
    // (maybe edited) are redacted again
    for t in [&mut r.what, &mut r.expected, &mut r.steps] {
        *t = redact_secrets(t, &known.token);
    }
    r.diagnostics = cut_front(&redact(&r.diagnostics, &known), MAX_DIAGNOSTICS);
    if feedback_url(&st.cfg.lock().unwrap()).is_empty() {
        r.id.clear(); // nothing kept: there's nowhere to send it later
    }
    deliver(&st, r).await
}

/// An unsent report from "My reports", again.
async fn retry(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match store(&st).load(&id) {
        Some(r) if r.sent => answer(StatusCode::CONFLICT, json!({"error": format!("{id} was already sent"), "id": id})),
        Some(r) => deliver(&st, r).await,
        None => answer(StatusCode::NOT_FOUND, json!({"error": "no such report"})),
    }
}

#[cfg(test)]
#[path = "feedback_tests.rs"]
mod tests;
