//! The setup page (`#install`, install_flow.py), the installer's components (components_api.py +
//! installer.py's components) and the stick wizard's progress (firmware_api.py's `/api/onboarding`),
//! for this app.
//!
//! Adapted to the Rust app: the speech model is the sherpa-onnx download it already has
//! (`speech.rs`); Claude Code's hooks run this program (`hooks.rs`); there is no Python venv, no
//! `stickybot` command or LaunchAgent (the login item is Settings → *Start at login*), and BlackHole
//! isn't downloaded or installed from here (the Apple audio tap needs no driver; for BlackHole the
//! page says to run `brew install --cask blackhole-2ch`). Everything that runs a program goes through
//! [`System`] (`sysrun.rs`): tests and the fake stick never run `gh`, `claude` or `curl`.
//!
//!   GET  /api/install                      {"steps": [...], "next", "pending", "skipped"}
//!   GET  /api/install/{step}               one step
//!   POST /api/install/{step}/skip|unskip   -> the listing (config `install_skipped`)
//!   POST /api/install/{step}/run           -> {"job"}  (model, claude, hooks, mcp, github, wispr)
//!   POST /api/install/github/repo          {"repo", "alerts"} -> {"ok", "github"}
//!   GET  /api/install/jobs/{job}           SSE
//!   POST /api/install/jobs/{job}/cancel
//!   GET  /api/components                   [{id, title, description, status, detail, needs, ...}]
//!   POST /api/components/{id}/install|uninstall -> {"job"}
//!   GET  /api/components/jobs/{job}        SSE
//!   GET  /api/onboarding, POST {"step", "done"} | {"step": "reset"}
//!   POST /api/restart, POST /api/window {"page"}

use crate::AppState;
use crate::firmware::Job;
use crate::sysrun::System;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

pub const CLAUDE_INSTALL: &str = "curl -fsSL https://claude.ai/install.sh | bash"; // Anthropic's installer
pub const BREW_INSTALL: &str =
    "/bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"";
pub const BLACKHOLE_CASK: &str = "brew install --cask blackhole-2ch";
pub const BLACKHOLE_PIN: &str = "0.7.1";
pub const GH_DEVICE_URL: &str = "https://github.com/login/device";
const MIN_FREE_BYTES: u64 = 1_000_000_000;
const MIN_MACOS: (u64, u64) = (13, 0);
const MODEL_SIZE: &str = "490 MB";
pub const TAP_NOTE: &str = "macOS asks once to allow \"System Audio Recording\" for StickyBot (Rust): it taps only \
                            its own sound (the stick's audio it plays), muted, never other apps.";

struct StepDef {
    id: &'static str,
    title: &'static str,
    kind: &'static str,
    why: &'static str,
}

const STEPS: &[StepDef] = &[
    StepDef {
        id: "welcome",
        title: "Welcome",
        kind: "required",
        why: "A quick check that this Mac can run StickyBot.",
    },
    StepDef {
        id: "model",
        title: "Speech model",
        kind: "recommended",
        why: "Turns what you say into text, on this Mac (nothing is sent anywhere). Needed for hold B.",
    },
    StepDef {
        id: "claude",
        title: "Claude Code",
        kind: "optional",
        why: "Answers your voice requests and does the work. Uses your own Claude login.",
    },
    StepDef {
        id: "hooks",
        title: "Approvals on the stick",
        kind: "optional",
        why: "Approve Claude Code's requests with A / B on the stick, and get a \"Claude finished\" alert.",
    },
    StepDef {
        id: "mcp",
        title: "Stick tools for Claude Code",
        kind: "optional",
        why: "Any Claude Code session can notify you, ask A/B questions and show progress on the stick.",
    },
    StepDef {
        id: "github",
        title: "GitHub alerts",
        kind: "optional",
        why: "Celebrate merged PRs and get a heads-up on review requests and failed CI.",
    },
    StepDef {
        id: "wispr",
        title: "Wireless Wispr mic",
        kind: "optional",
        why: "Dictate with Wispr Flow through the stick's mic without a cable.",
    },
    StepDef {
        id: "stick",
        title: "Your stick",
        kind: "recommended",
        why: "Pair the stick, pick it, name it, update its firmware and try it.",
    },
    StepDef { id: "done", title: "Done", kind: "required", why: "What's set up, and where to change it later." },
];

fn step_def(id: &str) -> Option<&'static StepDef> {
    STEPS.iter().find(|s| s.id == id)
}

fn needs_of(id: &str) -> Value {
    match id {
        "model" => json!({"download": MODEL_SIZE}),
        "claude" => json!({"terminal": "log in once in Terminal"}),
        "github" => json!({"browser": "log in to GitHub"}),
        "wispr" => json!({"terminal": BLACKHOLE_CASK}),
        _ => json!({}),
    }
}

fn skippable(id: &str) -> bool {
    step_def(id).is_some() && id != "welcome" && id != "done"
}

const RUNNABLE: &[&str] = &["model", "claude", "hooks", "mcp", "github", "wispr"];

/// A job and its cancel flag.
type JobEntry = (Arc<Job>, Arc<AtomicBool>);
/// A step's work on a blocking thread.
type StepFn = Box<dyn FnOnce(&AppState, &dyn System, &Arc<Job>, &AtomicBool) -> Result<(), String> + Send>;

/// The jobs and the gh login in progress, the app's window / restart.
pub struct Setup {
    sys: Mutex<Arc<dyn System>>,
    jobs: Mutex<HashMap<String, JobEntry>>,
    /// step (or `component:<id>`) -> its running job
    running: Mutex<HashMap<String, String>>,
    /// `{"code", "url", "job"}` while a gh login waits for the browser
    gh_login: Mutex<Option<Value>>,
    pub app: Mutex<Arc<dyn crate::appctl::AppControl>>,
}

impl Setup {
    pub fn new(sys: Arc<dyn System>) -> Self {
        Self {
            sys: Mutex::new(sys),
            jobs: Mutex::default(),
            running: Mutex::default(),
            gh_login: Mutex::default(),
            app: Mutex::new(Arc::new(crate::appctl::NoApp)),
        }
    }

    pub fn sys(&self) -> Arc<dyn System> {
        self.sys.lock().unwrap().clone()
    }

    pub fn set_sys(&self, s: Arc<dyn System>) {
        *self.sys.lock().unwrap() = s;
    }

    fn job(&self, id: &str) -> Option<JobEntry> {
        self.jobs.lock().unwrap().get(id).cloned()
    }

    fn current(&self, key: &str) -> Option<String> {
        let id = self.running.lock().unwrap().get(key).cloned()?;
        self.job(&id).filter(|(j, _)| !j.is_done()).map(|_| id)
    }

    fn start(&self, key: &str) -> JobEntry {
        let job = Job::new();
        let cancel = Arc::new(AtomicBool::new(false));
        self.jobs.lock().unwrap().insert(job.id.clone(), (job.clone(), cancel.clone()));
        self.running.lock().unwrap().insert(key.to_string(), job.id.clone());
        (job, cancel)
    }

    fn components_busy(&self) -> bool {
        let running: Vec<String> = self
            .running
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| k.starts_with("component:"))
            .map(|(_, v)| v.clone())
            .collect();
        running.iter().any(|id| self.job(id).is_some_and(|(j, _)| !j.is_done()))
    }
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, axum::Json(json!({"error": msg}))).into_response()
}

fn body_obj(b: &Bytes) -> Map<String, Value> {
    match serde_json::from_slice::<Value>(b) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

fn cfg(st: &AppState) -> Map<String, Value> {
    st.cfg.lock().unwrap().clone()
}

fn skipped(cfg: &Map<String, Value>) -> Vec<String> {
    cfg.get("install_skipped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|s| skippable(s))
        .map(str::to_string)
        .collect()
}

fn argv(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

// --- small helpers shared with the components ----------------------------------------------------

static REPO_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"^[A-Za-z0-9][A-Za-z0-9-]{0,38}/[A-Za-z0-9._-]{1,100}$").unwrap());
static GH_CODE_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"one-time code:\s*([A-Z0-9]{4}-[A-Z0-9]{4})").unwrap());
static GH_URL_RE: LazyLock<regex::Regex> =
    LazyLock::new(|| regex::Regex::new(r"(https://github\.com/login/device)\b").unwrap());
static ANSI_RE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap());

/// owner/name as GitHub allows it (and nothing a URL or a shell could trip over).
pub fn valid_repo(repo: &str) -> bool {
    REPO_RE.is_match(repo) && !matches!(repo.split('/').nth(1), Some("." | ".."))
}

/// gh's one-time code and the page to enter it on, from its output so far.
pub fn parse_device_code(text: &str) -> Option<(String, String)> {
    let text = ANSI_RE.replace_all(text, "");
    let code = GH_CODE_RE.captures(&text)?.get(1)?.as_str().to_string();
    let url = GH_URL_RE.captures(&text).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or(GH_DEVICE_URL);
    Some((code, url.to_string()))
}

/// `gh auth login` in its browser (one-time code) flow, asking nothing.
pub fn gh_login_args(gh: &str) -> Vec<String> {
    argv(&[gh, "auth", "login", "--hostname", "github.com", "--git-protocol", "https", "--web"])
}

/// The main repo: GitHub alerts watch it first and the "PR 1234" / Devin review quick commands
/// open its pull requests. `alerts`: Some turns the watcher on / off, None keeps it.
pub fn set_primary_repo(st: &AppState, repo: &str, alerts: Option<bool>) -> Result<Value, String> {
    if !repo.is_empty() {
        let path = st.paths.commands();
        if let Some(mut doc) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
            && let Some(cmds) = doc.get_mut("commands").and_then(Value::as_array_mut)
        {
            static GH: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"(github\.com/)[^/]+/[^/]+(/pull/)").unwrap());
            static DEVIN: LazyLock<regex::Regex> =
                LazyLock::new(|| regex::Regex::new(r"(app\.devin\.ai/review/)[^/]+/[^/]+(/pull/)").unwrap());
            let mut changed = false;
            for c in cmds.iter_mut() {
                let Some(url) = c.get("open_url").and_then(Value::as_str).map(str::to_string) else { continue };
                let new = GH.replace_all(&url, format!("${{1}}{repo}${{2}}").as_str()).into_owned();
                let new = DEVIN.replace_all(&new, format!("${{1}}{repo}${{2}}").as_str()).into_owned();
                if new != url {
                    c["open_url"] = json!(new);
                    changed = true;
                }
            }
            if changed {
                std::fs::write(&path, serde_json::to_string_pretty(&doc).unwrap_or_default())
                    .map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    let mut g = cfg(st).get("github").and_then(Value::as_object).cloned().unwrap_or_default();
    g.insert("primary_repo".into(), json!(repo));
    let on = alerts.unwrap_or_else(|| g.get("on") != Some(&json!(false)));
    g.insert("on".into(), json!(on));
    if !g.contains_key("interval_s") {
        g.insert("interval_s".into(), json!(120));
    }
    let g = Value::Object(g);
    let mut ch = Map::new();
    ch.insert("github".into(), g.clone());
    st.update_config(ch)?;
    Ok(g)
}

fn set(st: &AppState, changes: Value) -> Result<(), String> {
    st.update_config(changes.as_object().cloned().unwrap_or_default()).map(|_| ())
}

fn gh_path(sys: &dyn System) -> Option<String> {
    sys.which("gh").map(|p| p.to_string_lossy().to_string())
}

fn gh_logged_in(sys: &dyn System, gh: &str) -> bool {
    sys.run(&argv(&[gh, "auth", "status"]), Duration::from_secs(15)).0 == Some(0)
}

fn claude_logged_in(sys: &dyn System, claude: &str) -> bool {
    let (code, out) = sys.run(&argv(&[claude, "auth", "status"]), Duration::from_secs(15));
    if code != Some(0) {
        return false;
    }
    match serde_json::from_str::<Value>(&out) {
        Ok(v) => v.get("loggedIn").and_then(Value::as_bool).unwrap_or(false),
        Err(_) => out.to_lowercase().contains("logged in"),
    }
}

fn model_dir(st: &AppState) -> std::path::PathBuf {
    st.paths.models().join(crate::speech::MODEL_NAME)
}

fn version_pair(v: &str) -> (u64, u64) {
    let mut it = v.split('.').map(|p| p.parse::<u64>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// The hooks may replace the Python app's only after the switch-over (they still reach it before).
fn replace_python(st: &AppState) -> bool {
    crate::switch::switched(&st.paths.home)
}

// --- the checks (blocking: run them off the runtime) -----------------------------------------------

type Check = (String, String, Value);

fn c(state: &str, detail: impl Into<String>, data: Value) -> Check {
    (state.to_string(), detail.into(), data)
}

fn check_welcome(st: &AppState, sys: &dyn System) -> Check {
    let (mach, mac, free) = sys.machine();
    let arm = mach == "arm64";
    let mac_ok = !mac.is_empty() && version_pair(&mac) >= MIN_MACOS;
    let need_model = !crate::speech::model_ready(&model_dir(st));
    let mut checks = vec![
        json!({"label": "Apple Silicon", "level": if arm { "ok" } else { "err" },
               "detail": if arm { "this app is built for it".to_string() } else { format!("this Mac is {}; Apple Silicon is needed", if mach.is_empty() { "?" } else { &mach }) }}),
        json!({"label": "macOS", "level": if mac_ok { "ok" } else { "err" },
               "detail": format!("macOS {}{}", if mac.is_empty() { "?" } else { &mac }, if mac_ok { "" } else { " — 13 or newer is needed" })}),
    ];
    if let Some(free) = free {
        let low = need_model && free < MIN_FREE_BYTES;
        let note = if low {
            format!(" — the speech model needs {MODEL_SIZE}; free up some space first")
        } else {
            String::new()
        };
        checks.push(json!({"label": "Free disk space", "level": if low { "warn" } else { "ok" },
                           "detail": format!("{} free{note}", fmt_bytes(free))}));
    }
    let bad = checks.iter().any(|c| c["level"] == "err");
    c(
        if bad { "unavailable" } else { "done" },
        if bad { "this Mac can't run it" } else { "this Mac is ready" },
        json!({"checks": checks}),
    )
}

pub fn fmt_bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.0} MB", n / 1e6)
    } else {
        format!("{:.0} kB", n / 1e3)
    }
}

fn check_model(st: &AppState) -> Check {
    use crate::speech::Phase;
    let ready = st.speech.ready();
    let phase = st.speech.phase();
    let bytes = match phase {
        Phase::Downloading { done, .. } => done,
        _ => 0,
    };
    let busy = matches!(phase, Phase::Downloading { .. } | Phase::Checking | Phase::Unpacking);
    let data = json!({"repo": crate::speech::MODEL_NAME, "bytes": bytes, "stt_ready": ready,
                      "agent_downloading": !ready && busy, "phase": phase.to_json()});
    if crate::speech::model_ready(&model_dir(st)) {
        return c("done", if ready { "downloaded" } else { "downloaded — loading it…" }, data);
    }
    c("todo", format!("not downloaded yet ({MODEL_SIZE}, once)"), data)
}

fn check_claude(st: &AppState, sys: &dyn System) -> Check {
    let Some(claude) = sys.claude(&cfg(st)) else {
        return c(
            "todo",
            "not installed",
            json!({"installed": false, "version": null, "logged_in": false,
                                                  "install_cmd": CLAUDE_INSTALL, "path": null}),
        );
    };
    let cl = claude.to_string_lossy().to_string();
    let (code, out) = sys.run(&argv(&[&cl, "--version"]), Duration::from_secs(15));
    let version = (code == Some(0)).then(|| out.trim().to_string()).filter(|v| !v.is_empty());
    let logged_in = claude_logged_in(sys, &cl);
    let data = json!({"installed": true, "version": version, "logged_in": logged_in,
                      "install_cmd": CLAUDE_INSTALL, "path": cl});
    if !logged_in {
        return c("todo", format!("installed ({}), not logged in", version.as_deref().unwrap_or(&cl)), data);
    }
    c("done", format!("{} · logged in", version.as_deref().unwrap_or("installed")), data)
}

pub fn hooks_preview(exe: &std::path::Path) -> Value {
    let ours = crate::hooks::our_hooks(exe);
    let hooks: Map<String, Value> = ours.into_iter().map(|(k, v)| (k, json!([{"hooks": [v]}]))).collect();
    json!({"hooks": hooks})
}

fn check_hooks(st: &AppState, sys: &dyn System) -> Check {
    let path = sys.claude_settings();
    let exe = sys.exe();
    let mut data = json!({"path": path, "exists": path.exists(), "preview": hooks_preview(&exe),
                          "claude_installed": sys.claude(&cfg(st)).is_some()});
    let settings = match crate::hooks::read_settings(&path) {
        Ok(s) => s,
        Err(_) => {
            return c(
                "unavailable",
                format!("{} isn't valid JSON — fix it first (nothing will be changed)", path.display()),
                data,
            );
        }
    };
    let (current, present, python) = crate::hooks::state(&settings, &exe);
    data["current"] = json!(current);
    data["present"] = json!(present);
    data["python"] = json!(python);
    if current == 3 {
        return c("done", format!("all 3 hooks are in {}", path.display()), data);
    }
    if python == 3 && !replace_python(st) {
        return c(
            "done",
            "StickyBot (Python)'s 3 hooks are in place: they reach this app once it has switched over",
            data,
        );
    }
    let detail = if present > 0 { format!("{current} of 3 hooks in place") } else { "not added".to_string() };
    c("todo", detail, data)
}

fn mcp_status(st: &AppState, sys: &dyn System) -> Option<Value> {
    let claude = sys.claude(&cfg(st))?;
    let run = st.claude_runner.lock().unwrap().clone();
    Some(crate::mcp::terminal_status(Some(&claude), st.port, &st.token, &run))
}

fn check_mcp(st: &AppState, sys: &dyn System) -> Check {
    let Some(s) = mcp_status(st, sys) else {
        return c("unavailable", "needs Claude Code (the step before)", json!({}));
    };
    let detail = s["detail"].as_str().unwrap_or("").to_string();
    let conflict = detail.contains("different");
    let data = json!({"command": s["command"], "conflict": conflict});
    if s["installed"] == true {
        return c("done", detail, data);
    }
    c(if conflict { "unavailable" } else { "todo" }, detail, data)
}

fn check_github(st: &AppState, sys: &dyn System, login: Value) -> Check {
    let g = cfg(st).get("github").cloned().unwrap_or(json!({}));
    let repo = g.get("primary_repo").and_then(Value::as_str).unwrap_or("").to_string();
    let on = g.get("on") != Some(&json!(false));
    let mut data = json!({"gh": false, "brew": sys.which("brew").is_some(), "brew_cmd": "brew install gh",
                          "brew_install": BREW_INSTALL, "logged_in": false, "repo": repo, "on": on, "login": login});
    let Some(gh) = gh_path(sys) else {
        return c("todo", "the GitHub CLI (gh) isn't installed", data);
    };
    data["gh"] = json!(true);
    let logged_in = gh_logged_in(sys, &gh);
    data["logged_in"] = json!(logged_in);
    if !logged_in {
        return c("todo", "gh is installed, not logged in", data);
    }
    if !(on && valid_repo(&repo)) {
        return c("todo", "logged in — pick your main repo", data);
    }
    c("done", format!("watching {repo}"), data)
}

fn check_wispr(st: &AppState, sys: &dyn System) -> Check {
    let cfgv = cfg(st);
    let ws = crate::wmic::status(st);
    let on = cfgv.get("wireless_mic").and_then(Value::as_bool).unwrap_or(false);
    let pref = cfgv.get("wireless_mic_carrier").and_then(Value::as_str).unwrap_or("auto").to_string();
    let supported = ws["tap"]["supported"].as_bool().unwrap_or(false);
    let tap_mode = supported && pref != "blackhole";
    let bh = sys.blackhole();
    let tap = json!({"state": ws["tap"]["state"], "detail": ws["tap"]["detail"], "carrier": ws["carrier"],
                     "hint": ws["tap"]["hint"]});
    let mut data = json!({"blackhole": bh, "version": null, "pinned": BLACKHOLE_PIN, "on": on,
                          "brew": sys.which("brew").is_some(), "brew_cmd": BLACKHOLE_CASK,
                          "mic": crate::wmic::mic_name(st), "tap_supported": supported, "pref": pref,
                          "tap_mode": tap_mode, "tap": tap.clone(), "tap_note": TAP_NOTE});
    if tap_mode {
        data["needs"] = json!({"permission": "System Audio Recording (macOS asks once)"});
        if !on {
            return c("todo", "no driver needed on this Mac — turn it on", data);
        }
        let state = tap["state"].as_str();
        match tap["carrier"].as_str() {
            Some("tap") => return c("done", "on · carried by the Apple audio tap", data),
            _ if state.is_none() || state == Some("checking") => {
                return c("todo", "on · checking the Apple audio tap…", data);
            }
            Some("blackhole") => {
                return c("done", "on · carried by BlackHole 2ch (the Apple audio tap isn't allowed)", data);
            }
            _ => {}
        }
        if state == Some("failed") {
            return c("todo", "on · allow System Audio Recording for StickyBot (Rust)", data);
        }
        let detail = tap["detail"].as_str().unwrap_or("");
        return c(
            "todo",
            format!("on · the Apple audio tap doesn't work here ({detail}) — use BlackHole instead"),
            data,
        );
    }
    match (bh, on) {
        (true, true) => c("done", "on · BlackHole 2ch found", data),
        (true, false) => c("todo", "BlackHole 2ch found · the wireless mic is off", data),
        _ => c("todo", format!("needs BlackHole 2ch (a free audio driver): {BLACKHOLE_CASK}"), data),
    }
}

// --- the stick wizard's progress (firmware_api.py `onboarding_state`) --------------------------------

pub const ONBOARDING_STEPS: [&str; 6] = ["firmware", "paired", "picked", "named", "repo", "tested"];
const DEFAULT_NAMES: [&str; 2] = ["StickyBot", "stick-key"];

pub async fn onboarding_state(st: &Arc<AppState>) -> Value {
    let cfgv = cfg(st);
    let marks = cfgv.get("onboarding").and_then(Value::as_object).cloned().unwrap_or_default();
    let fw = st.firmware().first().and_then(|f| f["version"].as_str().map(str::to_string));
    let conn = crate::firmware::connected(&st.link).await;
    let linked = cfgv.get("stick").is_some_and(|s| !s.is_null()) && st.link.connected();
    let fw_ok = match fw {
        Some(fw) => {
            let mut versions: Vec<String> = conn["version"].as_str().map(str::to_string).into_iter().collect();
            versions.extend(st.usb.sticks().await.iter().filter_map(|u| u["version"].as_str().map(str::to_string)));
            let want = stick_link::ota::version_tuple(&fw);
            versions.iter().any(|v| stick_link::ota::version_tuple(v) >= want)
        }
        None => true,
    };
    let commands = std::fs::read_to_string(st.paths.commands()).unwrap_or_default();
    let history = std::fs::read_to_string(st.paths.history()).unwrap_or_default();
    let named = conn["name"].as_str().is_some_and(|n| !n.is_empty() && !DEFAULT_NAMES.contains(&n));
    let auto: HashMap<&str, bool> = HashMap::from([
        ("firmware", fw_ok),
        ("paired", linked),
        ("picked", linked),
        ("named", named),
        ("repo", !commands.is_empty() && !commands.contains("<org>") && !commands.contains("<repo>")),
        ("tested", history.contains("\"source\":\"voice\"") || history.contains("\"source\": \"voice\"")),
    ]);
    let mut steps = Map::new();
    for k in ONBOARDING_STEPS {
        let v = match marks.get(k) {
            Some(m) => m.as_bool().unwrap_or(false),
            None => auto[k],
        };
        steps.insert(k.into(), json!(v));
    }
    let done = steps.values().all(|v| v == &json!(true));
    json!({"steps": steps, "done": done})
}

async fn check_stick(st: &Arc<AppState>) -> Check {
    let ob = onboarding_state(st).await;
    let steps = ob["steps"].clone();
    let stick = cfg(st).get("stick").cloned().unwrap_or(Value::Null);
    let data = json!({"steps": steps, "stick": stick, "connected": st.link.connected()});
    if steps["picked"] == true && steps["firmware"] != false {
        let name = stick["name"].as_str().unwrap_or("your stick");
        return c("done", format!("using {name}"), data);
    }
    c("todo", if stick.is_null() { "pick your stick" } else { "update its firmware" }, data)
}

// --- the listing ----------------------------------------------------------------------------------------

async fn one(st: &Arc<AppState>, sid: &'static str) -> Value {
    let def = step_def(sid).expect("a known step");
    let sys = st.setup.sys();
    let (mut state, detail, mut data) = match sid {
        "stick" => check_stick(st).await,
        "done" => c("todo", "", json!({})),
        _ => {
            let st2 = st.clone();
            let login = st.setup.gh_login.lock().unwrap().clone().unwrap_or(Value::Null);
            tokio::task::spawn_blocking(move || match sid {
                "welcome" => check_welcome(&st2, sys.as_ref()),
                "model" => check_model(&st2),
                "claude" => check_claude(&st2, sys.as_ref()),
                "hooks" => check_hooks(&st2, sys.as_ref()),
                "mcp" => check_mcp(&st2, sys.as_ref()),
                "github" => check_github(&st2, sys.as_ref(), login),
                _ => check_wispr(&st2, sys.as_ref()),
            })
            .await
            .unwrap_or_else(|e| c("unavailable", e.to_string(), json!({})))
        }
    };
    if state == "todo" && skipped(&cfg(st)).iter().any(|s| s == sid) {
        state = "skipped".into();
    }
    let needs = data.as_object_mut().and_then(|d| d.remove("needs")).unwrap_or_else(|| needs_of(sid));
    json!({"id": sid, "title": def.title, "kind": def.kind, "why": def.why, "needs": needs,
           "skippable": skippable(sid), "runnable": RUNNABLE.contains(&sid), "state": state, "detail": detail,
           "data": data, "job": st.setup.current(sid)})
}

pub async fn listing(st: &Arc<AppState>) -> Value {
    let mut steps: Vec<Value> = futures_util::future::join_all(STEPS.iter().map(|s| one(st, s.id))).await;
    let rest_todo = |steps: &[Value]| steps.iter().filter(|s| s["id"] != "done").any(|s| s["state"] == "todo");
    let all_done = !rest_todo(&steps);
    let summary: Map<String, Value> = steps
        .iter()
        .filter(|s| s["id"] != "done")
        .map(|s| (s["id"].as_str().unwrap_or("").to_string(), s["state"].clone()))
        .collect();
    let next = steps
        .iter()
        .find(|s| s["id"] != "done" && s["state"] == "todo")
        .and_then(|s| s["id"].as_str())
        .unwrap_or("done")
        .to_string();
    let pending = steps
        .iter()
        .filter(|s| s["id"] != "done")
        .any(|s| s["state"] == "todo" && matches!(s["kind"].as_str(), Some("required" | "recommended")));
    if let Some(done) = steps.last_mut() {
        done["state"] = json!(if all_done { "done" } else { "todo" });
        done["data"] = json!({"summary": summary});
    }
    json!({"steps": steps, "next": next, "pending": pending, "skipped": skipped(&cfg(st))})
}

/// For the menu bar: is a required / recommended step still to do (and not skipped)? Cheap.
pub fn quick_pending(st: &AppState) -> bool {
    let c = cfg(st);
    let skip = skipped(&c);
    if !skip.iter().any(|s| s == "stick") && c.get("stick").is_none_or(Value::is_null) {
        return true;
    }
    !skip.iter().any(|s| s == "model") && !crate::speech::model_ready(&model_dir(st))
}

// --- running steps --------------------------------------------------------------------------------------

/// Runs `argv`, its output as `{"line"}` events; `on_output(text so far)` may answer on stdin and
/// rewrite what the lines show (`redact`). -> the exit code.
#[allow(clippy::too_many_arguments)]
fn stream_lines(
    sys: &dyn System,
    argv: &[String],
    env: &[(String, String)],
    timeout: Duration,
    cancel: &AtomicBool,
    job: &Job,
    on_output: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    redact: &dyn Fn(&str) -> String,
) -> Option<i32> {
    let mut text = String::new();
    let mut pending = String::new();
    let code = sys.stream(argv, env, timeout, cancel, &mut |s| {
        text.push_str(s);
        pending.push_str(s);
        let input = on_output(&text);
        let norm = pending.replace('\r', "\n");
        let mut parts: Vec<&str> = norm.split('\n').collect();
        let rest = parts.pop().unwrap_or("").to_string();
        for line in parts {
            if !line.trim().is_empty() {
                job.emit(json!({"line": redact(line.trim_end())}));
            }
        }
        pending = rest;
        input
    });
    if !pending.trim().is_empty() {
        job.emit(json!({"line": redact(pending.trim_end())}));
    }
    code
}

fn say(job: &Job, line: impl Into<String>) {
    job.emit(json!({"line": line.into()}));
}

/// The speech model: starts the download (if it isn't already going) and follows it.
async fn run_model(st: &Arc<AppState>, job: &Arc<Job>, cancel: &AtomicBool) -> Result<(), String> {
    use crate::speech::Phase;
    if st.speech.ready() {
        say(job, "the speech model is already downloaded and loaded");
        return Ok(());
    }
    match st.speech.phase() {
        Phase::Idle | Phase::Failed(_) => {
            say(
                job,
                format!("downloading {} ({MODEL_SIZE}) from sherpa-onnx's GitHub release", crate::speech::MODEL_NAME),
            );
            st.setup.sys().start_speech(st);
        }
        _ => say(job, "already downloading in the background — following it"),
    }
    let mut last = String::new();
    loop {
        let phase = st.speech.phase();
        let name = phase.to_json()["phase"].as_str().unwrap_or("").to_string();
        match &phase {
            Phase::Downloading { done, total } => {
                let pct = if *total > 0 { (done * 100 / total).min(100) } else { 0 };
                job.emit(json!({"progress": pct, "stage": "download", "done": done, "total": total}));
            }
            Phase::Ready { load_s, .. } => {
                job.emit(json!({"progress": 100, "stage": "download"}));
                say(
                    job,
                    format!("the speech model is ready (loaded in {load_s:.1} s) — hold B on the stick and speak"),
                );
                return Ok(());
            }
            Phase::Failed(e) => return Err(format!("the speech model didn't download: {e}")),
            _ => {}
        }
        if name != last && !matches!(phase, Phase::Downloading { .. }) {
            say(
                job,
                match name.as_str() {
                    "checking" => "checking the download",
                    "unpacking" => "unpacking it",
                    "loading" => "loading it",
                    _ => "waiting",
                },
            );
        }
        last = name;
        if cancel.load(Ordering::SeqCst) {
            return Err("stopped following it (the download goes on in the background)".into());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

fn run_claude_install(st: &AppState, sys: &dyn System, job: &Job, cancel: &AtomicBool) -> Result<(), String> {
    if sys.claude(&cfg(st)).is_some() {
        say(job, "Claude Code is already installed");
        return Ok(());
    }
    say(job, format!("running Anthropic's installer: {CLAUDE_INSTALL}"));
    let code = stream_lines(
        sys,
        &argv(&["/bin/sh", "-c", CLAUDE_INSTALL]),
        &[],
        Duration::from_secs(900),
        cancel,
        job,
        &mut |_| None,
        &|l| l.to_string(),
    );
    if code != Some(0) {
        return Err(format!(
            "the installer stopped (exit {}) — see the log above",
            code.map(|c| c.to_string()).unwrap_or("?".into())
        ));
    }
    if sys.claude(&cfg(st)).is_none() {
        return Err("the installer finished but `claude` isn't in ~/.local/bin".into());
    }
    say(job, "Claude Code installed. Next: log in once in Terminal (run `claude`).");
    Ok(())
}

fn install_hooks(st: &AppState, sys: &dyn System, job: &Job) -> Result<(), String> {
    if let Some(claude) = sys.claude(&cfg(st)) {
        let cl = claude.to_string_lossy().to_string();
        let (_, v) = sys.run(&argv(&[&cl, "--version"]), Duration::from_secs(15));
        say(job, format!("Claude Code {}", if v.trim().is_empty() { "found" } else { v.trim() }));
        if !claude_logged_in(sys, &cl) {
            say(job, "not logged in — run `claude` once and log in (your own account; no API key needed)");
        }
    } else {
        say(job, "Claude Code isn't installed yet — the hooks are added now and work once it is");
    }
    let path = sys.claude_settings();
    let settings = crate::hooks::read_settings(&path)?;
    let replace = replace_python(st);
    let (_, _, python) = crate::hooks::state(&settings, &sys.exe());
    if python > 0 && !replace {
        return Err("StickyBot (Python)'s hooks are in place and still in use: they reach this app after the \
                    switch-over (SWITCHOVER.md). Nothing changed."
            .into());
    }
    let (new, changed) = crate::hooks::merge(&settings, &sys.exe(), replace);
    if !changed {
        say(job, "hooks already in place");
        return Ok(());
    }
    let backup = crate::hooks::write_settings(&path, &new).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(b) = backup {
        say(job, format!("backup: {}", b.display()));
    }
    if python > 0 {
        say(job, "replaced StickyBot (Python)'s hooks with this app's");
    }
    say(job, format!("hooks added to {}", path.display()));
    Ok(())
}

fn uninstall_hooks(sys: &dyn System, job: &Job) -> Result<(), String> {
    let path = sys.claude_settings();
    if !path.exists() {
        say(job, "no Claude Code settings file");
        return Ok(());
    }
    let (new, changed) = crate::hooks::remove(&crate::hooks::read_settings(&path)?);
    if !changed {
        say(job, "our hooks weren't there");
        return Ok(());
    }
    if let Some(b) = crate::hooks::write_settings(&path, &new).map_err(|e| e.to_string())? {
        say(job, format!("backup: {}", b.display()));
    }
    say(job, format!("removed our hooks from {} (others kept)", path.display()));
    Ok(())
}

fn mcp_action(st: &AppState, sys: &dyn System, action: &str, job: &Job) -> Result<(), String> {
    let claude = sys.claude(&cfg(st)).ok_or("Claude Code isn't installed (the step before)")?;
    let run = st.claude_runner.lock().unwrap().clone();
    let (ok, msg, ran) = crate::mcp::terminal(action, Some(&claude), st.port, &st.token, &run);
    for r in ran {
        say(job, format!("ran: claude {r}"));
    }
    if ok {
        say(job, msg);
        Ok(())
    } else {
        Err(msg)
    }
}

fn run_gh_login(st: &AppState, sys: &dyn System, job: &Arc<Job>, cancel: &AtomicBool) -> Result<(), String> {
    let gh = gh_path(sys).ok_or("install the GitHub CLI first: brew install gh")?;
    if gh_logged_in(sys, &gh) {
        say(job, "already logged in to GitHub");
        return Ok(());
    }
    let shown: Mutex<Option<String>> = Mutex::new(None);
    let mut entered = false;
    let env: Vec<(String, String)> = [
        ("GH_PROMPT_DISABLED", "1"),
        ("NO_COLOR", "1"),
        ("GH_NO_UPDATE_NOTIFIER", "1"),
        ("GH_BROWSER", "/usr/bin/true"), // the page shows the code first, then opens the page itself
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    let code = stream_lines(
        sys,
        &gh_login_args(&gh),
        &env,
        Duration::from_secs(900),
        cancel,
        job,
        &mut |text| {
            let mut input = None;
            if text.contains("Press Enter") && !entered {
                entered = true; // a terminal-style gh: don't block on it
                input = Some(b"\n".to_vec());
            }
            if let Some((code, url)) = parse_device_code(text) {
                let mut s = shown.lock().unwrap();
                if s.as_deref() != Some(code.as_str()) {
                    *s = Some(code.clone());
                    *st.setup.gh_login.lock().unwrap() = Some(json!({"code": code, "url": url, "job": job.id}));
                    job.emit(json!({"code": code, "url": url}));
                }
            }
            input
        },
        // the one-time code is shown big on the page; it isn't repeated in the log lines
        &|line| match shown.lock().unwrap().as_deref() {
            Some(code) if line.contains(code) => line.replace(code, "••••-••••"),
            _ => line.to_string(),
        },
    );
    *st.setup.gh_login.lock().unwrap() = None;
    if code != Some(0) || !gh_logged_in(sys, &gh) {
        return Err("GitHub login didn't finish — try again, or run `gh auth login` in Terminal".into());
    }
    say(job, "logged in to GitHub ✓");
    Ok(())
}

/// `carrier`: "tap" (on; the tap is checked after), "blackhole" (BlackHole must be installed),
/// None: what this Mac does best.
async fn run_wispr(st: &Arc<AppState>, job: &Arc<Job>, carrier: Option<&str>) -> Result<(), String> {
    let sys = st.setup.sys();
    let supported = crate::wmic::status(st)["tap"]["supported"].as_bool().unwrap_or(false);
    let pref = cfg(st).get("wireless_mic_carrier").and_then(Value::as_str).unwrap_or("auto").to_string();
    let use_tap = match carrier {
        Some("tap") => true,
        Some(_) => false,
        None => supported && pref != "blackhole",
    };
    if !use_tap {
        if !sys.blackhole() {
            return Err(format!(
                "BlackHole 2ch isn't installed. Install it in Terminal: {BLACKHOLE_CASK} (macOS asks for your \
                 password), then press the button again{}",
                if supported { " — or use the Apple audio tap (no driver)" } else { "" }
            ));
        }
        say(job, "BlackHole 2ch found");
        let pref = if supported { "blackhole" } else { "auto" };
        set(st, json!({"wireless_mic": true, "wireless_mic_carrier": pref}))?;
        say(job, "the wireless mic is on, carried by BlackHole 2ch");
        return Ok(());
    }
    if !supported {
        return Err("the Apple audio tap needs macOS 14.2 or newer — use BlackHole 2ch".into());
    }
    let new_pref = if pref == "blackhole" { "auto" } else { pref.as_str() };
    set(st, json!({"wireless_mic": true, "wireless_mic_carrier": new_pref}))?;
    say(job, "the wireless mic is on");
    say(job, "checking the Apple audio tap (inaudible) — if macOS asks about System Audio Recording, press Allow");
    match crate::wmic::check_tap(st, true).await {
        Some(res) if res.state == Some(macos_audio::carrier::CheckState::Ok) => {
            say(job, "the Apple audio tap works ✓");
            Ok(())
        }
        Some(res) => {
            Err(if res.detail.is_empty() { "the Apple audio tap isn't allowed yet".into() } else { res.detail })
        }
        None => Err("not now: the mic is in use (try again in a moment)".into()),
    }
}

async fn run_step(st: Arc<AppState>, sid: String, job: Arc<Job>, cancel: Arc<AtomicBool>, carrier: Option<String>) {
    let blocking = |f: StepFn| {
        let (st, job, cancel) = (st.clone(), job.clone(), cancel.clone());
        async move {
            let sys = st.setup.sys();
            tokio::task::spawn_blocking(move || f(&st, sys.as_ref(), &job, &cancel))
                .await
                .unwrap_or_else(|e| Err(e.to_string()))
        }
    };
    let r = match sid.as_str() {
        "model" => run_model(&st, &job, &cancel).await,
        "claude" => blocking(Box::new(|st, sys, job, cancel| run_claude_install(st, sys, job, cancel))).await,
        "hooks" => blocking(Box::new(|st, sys, job, _| install_hooks(st, sys, job))).await,
        "mcp" => blocking(Box::new(|st, sys, job, _| mcp_action(st, sys, "install", job))).await,
        "github" => blocking(Box::new(|st, sys, job, cancel| run_gh_login(st, sys, job, cancel))).await,
        "wispr" => run_wispr(&st, &job, carrier.as_deref()).await,
        _ => Err("nothing to run".into()),
    };
    log::info!("setup step {sid}: {}", r.as_ref().err().map(String::as_str).unwrap_or("ok"));
    let (ok, error) = match r {
        Ok(()) => (true, Value::Null),
        Err(e) => (false, json!(e)),
    };
    job.finish(json!({"done": true, "ok": ok, "error": error}));
}

// --- the components (installer.py COMPONENTS for this app) -----------------------------------------

struct Comp {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    default: bool,
    from_dashboard: bool,
    removable: bool,
}

const COMPONENTS: &[Comp] = &[
    Comp {
        id: "core",
        title: "StickyBot (Rust) app",
        description: "this app: the menu bar app, the dashboard and its login item (Settings → Start at login)",
        default: true,
        from_dashboard: false,
        removable: false,
    },
    Comp {
        id: "model",
        title: "Speech model",
        description: "speech to text on this Mac (sherpa-onnx Parakeet TDT 0.6b v3)",
        default: true,
        from_dashboard: true,
        removable: true,
    },
    Comp {
        id: "claude",
        title: "Claude Code integration",
        description: "approvals on the stick and \"Claude finished\" alerts (adds 3 hooks to Claude Code's settings)",
        default: true,
        from_dashboard: true,
        removable: true,
    },
    Comp {
        id: "github",
        title: "GitHub alerts",
        description: "celebrate merged PRs, heads-up on review requests / failed CI (uses your `gh` login)",
        default: false,
        from_dashboard: true,
        removable: true,
    },
    Comp {
        id: "firmware",
        title: "Flash a stick",
        description: "the stick's firmware (Stick → Firmware: over USB or Bluetooth)",
        default: false,
        from_dashboard: false,
        removable: false,
    },
    Comp {
        id: "wireless_mic",
        title: "Wireless Wispr mic",
        description: "dictate with Wispr Flow through the stick's mic over Bluetooth (the Apple audio tap, or BlackHole 2ch)",
        default: false,
        from_dashboard: true,
        removable: true,
    },
    Comp {
        id: "mcp",
        title: "Stick tools for Claude Code",
        description: "terminal Claude Code sessions can notify you, ask A/B questions and show progress on the stick",
        default: false,
        from_dashboard: true,
        removable: true,
    },
];

fn comp_status(st: &AppState, sys: &dyn System, id: &str) -> (String, String, Value) {
    let s = |a: &str, b: String| (a.to_string(), b, json!({}));
    match id {
        "core" => {
            let login = st.login_item.lock().unwrap().clone().state();
            let li = match login {
                crate::login::LoginState::On => "login item ✓",
                crate::login::LoginState::NeedsApproval => "login item needs approval",
                crate::login::LoginState::Off => "login item off",
                crate::login::LoginState::Unavailable => "no login item (not the bundled app)",
            };
            let state = if login == crate::login::LoginState::On { "installed" } else { "partial" };
            s(state, format!("dashboard on :{}, {li}", st.port))
        }
        "model" => {
            if crate::speech::model_ready(&model_dir(st)) {
                s(
                    "installed",
                    format!(
                        "{} · {}",
                        crate::speech::MODEL_NAME,
                        if st.speech.ready() { "loaded" } else { "not loaded yet" }
                    ),
                )
            } else {
                s("missing", format!("not downloaded ({MODEL_SIZE})"))
            }
        }
        "claude" => {
            let path = sys.claude_settings();
            let Ok(settings) = crate::hooks::read_settings(&path) else {
                return s("partial", format!("{} isn't valid JSON", path.display()));
            };
            let (current, present, python) = crate::hooks::state(&settings, &sys.exe());
            let mut hooks = format!("hooks {current}/3");
            if present > current {
                hooks += " (some point to another build)";
            }
            if python > 0 {
                hooks += &format!(" · StickyBot (Python)'s: {python}/3");
            }
            let found = if sys.claude(&cfg(st)).is_some() { "Claude Code found" } else { "Claude Code not installed" };
            let state = if current == 3 || (python == 3 && !replace_python(st)) {
                "installed"
            } else if present > 0 {
                "partial"
            } else {
                "missing"
            };
            s(state, format!("{found} · {hooks}"))
        }
        "github" => {
            let Some(gh) = gh_path(sys) else { return s("missing", "the GitHub CLI (gh) isn't installed".into()) };
            if !gh_logged_in(sys, &gh) {
                return s("partial", "gh found, not logged in".into());
            }
            let g = cfg(st).get("github").cloned().unwrap_or(json!({}));
            if g.get("on") == Some(&json!(false)) {
                return s("missing", "off".into());
            }
            let repo = g.get("primary_repo").and_then(Value::as_str).filter(|r| !r.is_empty()).unwrap_or("your PRs");
            s("installed", format!("watching {repo}"))
        }
        "firmware" => match st.firmware().first() {
            Some(fw) => {
                s("installed", format!("firmware {} ready (Stick → Firmware)", fw["version"].as_str().unwrap_or("?")))
            }
            None => s("missing", "no firmware downloaded yet (Updates checks for it)".into()),
        },
        "wireless_mic" => {
            let on = cfg(st).get("wireless_mic").and_then(Value::as_bool).unwrap_or(false);
            let ws = crate::wmic::status(st);
            let carrier = ws["carrier_title"].as_str().map(str::to_string);
            match (on, carrier) {
                (true, Some(c)) => s("installed", format!("on · carried by {c}")),
                (true, None) => s(
                    "partial",
                    format!("on · {}", ws["unavailable"].as_str().unwrap_or("nothing carries the audio yet")),
                ),
                (false, _) => s("missing", "off".into()),
            }
        }
        "mcp" => match mcp_status(st, sys) {
            None => s("missing", "needs Claude Code".into()),
            Some(m) => s(
                if m["installed"] == true { "installed" } else { "missing" },
                m["detail"].as_str().unwrap_or("").to_string(),
            ),
        },
        _ => s("n/a", String::new()),
    }
}

fn comp_needs(st: &AppState, id: &str) -> Value {
    match id {
        "model" => json!({"download": MODEL_SIZE}),
        "github" => json!({"terminal": "log in on the setup page or with `gh auth login`"}),
        "wireless_mic" => {
            if crate::wmic::status(st)["tap"]["supported"] == true {
                json!({"permission": "System Audio Recording (macOS asks once)"})
            } else {
                json!({"terminal": BLACKHOLE_CASK})
            }
        }
        "firmware" => json!({"usb": true}),
        _ => json!({}),
    }
}

pub fn components(st: &AppState) -> Value {
    let sys = st.setup.sys();
    Value::Array(
        COMPONENTS
            .iter()
            .map(|c| {
                let (status, detail, _) = comp_status(st, sys.as_ref(), c.id);
                json!({"id": c.id, "title": c.title, "description": c.description, "default": c.default,
                       "needs": comp_needs(st, c.id), "from_dashboard": c.from_dashboard, "removable": c.removable,
                       "status": status, "detail": detail})
            })
            .collect(),
    )
}

async fn run_component(st: Arc<AppState>, id: &'static str, install: bool, job: Arc<Job>, cancel: Arc<AtomicBool>) {
    let r: Result<(), String> = match (id, install) {
        ("model", true) => run_model(&st, &job, &cancel).await,
        ("model", false) => {
            let dir = model_dir(&st);
            if !dir.exists() {
                say(&job, "the speech model wasn't downloaded");
                Ok(())
            } else {
                match std::fs::remove_dir_all(&dir) {
                    Ok(()) => {
                        say(&job, format!("removed {}", dir.display()));
                        say(&job, "a loaded model stays until the app restarts; hold B needs it downloaded again");
                        Ok(())
                    }
                    Err(e) => Err(format!("{}: {e}", dir.display())),
                }
            }
        }
        ("wireless_mic", true) => run_wispr(&st, &job, None).await,
        ("wireless_mic", false) => {
            set(&st, json!({"wireless_mic": false})).map(|_| say(&job, "the wireless mic is off"))
        }
        _ => {
            let (st2, job2) = (st.clone(), job.clone());
            tokio::task::spawn_blocking(move || {
                let sys = st2.setup.sys();
                let sys = sys.as_ref();
                match (id, install) {
                    ("claude", true) => install_hooks(&st2, sys, &job2),
                    ("claude", false) => uninstall_hooks(sys, &job2),
                    ("mcp", true) => mcp_action(&st2, sys, "install", &job2),
                    ("mcp", false) => mcp_action(&st2, sys, "uninstall", &job2),
                    ("github", true) => {
                        let gh = gh_path(sys)
                            .ok_or("install the GitHub CLI first: brew install gh (or https://cli.github.com)")?;
                        if !gh_logged_in(sys, &gh) {
                            return Err("not logged in to GitHub — log in on the setup page (Settings → Components → \
                                        Open the setup page) or run `gh auth login`, then try again"
                                .into());
                        }
                        let repo = cfg(&st2)
                            .get("github")
                            .and_then(|g| g.get("primary_repo"))
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        set_primary_repo(&st2, &repo, Some(true))?;
                        say(
                            &job2,
                            format!(
                                "GitHub alerts on{}",
                                if repo.is_empty() { String::new() } else { format!(" — main repo {repo}") }
                            ),
                        );
                        Ok(())
                    }
                    ("github", false) => {
                        let mut g = cfg(&st2).get("github").and_then(Value::as_object).cloned().unwrap_or_default();
                        g.insert("on".into(), json!(false));
                        set(&st2, json!({"github": g}))?;
                        say(&job2, "GitHub alerts off");
                        Ok(())
                    }
                    _ => Err("not from here".into()),
                }
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()))
        }
    };
    log::info!(
        "component {id} {}: {}",
        if install { "install" } else { "uninstall" },
        r.as_ref().err().map(String::as_str).unwrap_or("ok")
    );
    let (ok, error) = match r {
        Ok(()) => (true, Value::Null),
        Err(e) => (false, json!(e)),
    };
    job.finish(json!({"done": true, "ok": ok, "error": error}));
}

// --- routes -----------------------------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/install", get(get_listing))
        .route("/api/install/{step}", get(get_step))
        .route("/api/install/{step}/{action}", post(post_step))
        .route("/api/install/jobs/{job}", get(job_events))
        .route("/api/install/jobs/{job}/cancel", post(job_cancel))
        .route("/api/components", get(get_components))
        .route("/api/components/{id}/{action}", post(post_component))
        .route("/api/components/jobs/{job}", get(job_events))
        .route("/api/onboarding", get(get_onboarding).post(post_onboarding))
        .route("/api/restart", post(restart))
        .route("/api/window", post(window))
}

async fn get_listing(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(listing(&st).await)
}

async fn get_step(State(st): State<Arc<AppState>>, Path(step): Path<String>) -> Response {
    let Some(def) = step_def(&step) else { return err(StatusCode::NOT_FOUND, "no such step") };
    if def.id == "done" {
        let l = listing(&st).await;
        return axum::Json(l["steps"].as_array().and_then(|s| s.last()).cloned().unwrap_or(Value::Null))
            .into_response();
    }
    axum::Json(one(&st, def.id).await).into_response()
}

async fn post_step(
    State(st): State<Arc<AppState>>,
    Path((step, action)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    if step == "github" && action == "repo" {
        return set_repo(st, body).await;
    }
    let Some(def) = step_def(&step) else { return err(StatusCode::NOT_FOUND, "no such step") };
    match action.as_str() {
        "skip" | "unskip" => {
            if !skippable(def.id) {
                return err(StatusCode::BAD_REQUEST, &format!("{} can't be skipped", def.title));
            }
            let cur = skipped(&cfg(&st));
            let new: Vec<String> = if action == "skip" {
                if cur.iter().any(|s| s == def.id) {
                    cur
                } else {
                    cur.into_iter().chain([def.id.to_string()]).collect()
                }
            } else {
                cur.into_iter().filter(|s| s != def.id).collect()
            };
            if let Err(e) = set(&st, json!({"install_skipped": new})) {
                return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
            }
            axum::Json(listing(&st).await).into_response()
        }
        "run" => {
            if !RUNNABLE.contains(&def.id) {
                return err(StatusCode::BAD_REQUEST, &format!("{} has nothing to run", def.title));
            }
            if let Some(j) = st.setup.current(def.id) {
                return (StatusCode::CONFLICT, axum::Json(json!({"error": "already running", "job": j})))
                    .into_response();
            }
            let carrier = match body_obj(&body).get("carrier") {
                None | Some(Value::Null) => None,
                Some(Value::String(c)) if c == "tap" || c == "blackhole" => Some(c.clone()),
                _ => return err(StatusCode::BAD_REQUEST, "carrier must be tap or blackhole"),
            };
            let (job, cancel) = st.setup.start(def.id);
            let id = job.id.clone();
            tokio::spawn(run_step(st.clone(), def.id.to_string(), job, cancel, carrier));
            axum::Json(json!({"job": id})).into_response()
        }
        _ => err(StatusCode::NOT_FOUND, "no such action"),
    }
}

async fn set_repo(st: Arc<AppState>, body: Bytes) -> Response {
    let b = body_obj(&body);
    let repo = b.get("repo").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if !valid_repo(&repo) {
        return err(StatusCode::BAD_REQUEST, "use the form owner/name, e.g. acme/web");
    }
    let alerts = match b.get("alerts") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(a)) => Some(*a),
        _ => return err(StatusCode::BAD_REQUEST, "alerts must be true, false or null"),
    };
    let st2 = st.clone();
    match tokio::task::spawn_blocking(move || set_primary_repo(&st2, &repo, alerts)).await {
        Ok(Ok(g)) => axum::Json(json!({"ok": true, "github": g})).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn job_events(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    match st.setup.job(&id) {
        Some((job, _)) => crate::api::job_stream(job),
        None => err(StatusCode::NOT_FOUND, "no such job"),
    }
}

async fn job_cancel(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some((job, cancel)) = st.setup.job(&id) else { return err(StatusCode::NOT_FOUND, "no such job") };
    if job.is_done() {
        return err(StatusCode::CONFLICT, "it already finished");
    }
    cancel.store(true, Ordering::SeqCst);
    axum::Json(json!({"ok": true})).into_response()
}

async fn get_components(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let st2 = st.clone();
    axum::Json(tokio::task::spawn_blocking(move || components(&st2)).await.unwrap_or(json!([])))
}

async fn post_component(State(st): State<Arc<AppState>>, Path((id, action)): Path<(String, String)>) -> Response {
    if action != "install" && action != "uninstall" {
        return err(StatusCode::NOT_FOUND, "install or uninstall");
    }
    let Some(comp) = COMPONENTS.iter().find(|c| c.id == id) else {
        return err(StatusCode::NOT_FOUND, &format!("no component '{id}'"));
    };
    if !comp.from_dashboard {
        return err(StatusCode::BAD_REQUEST, &format!("{} can't be changed from the dashboard", comp.title));
    }
    if action == "uninstall" && !comp.removable {
        return err(StatusCode::BAD_REQUEST, &format!("{} can't be removed", comp.title));
    }
    if st.setup.components_busy() {
        return err(StatusCode::CONFLICT, "another component job is running");
    }
    let (job, cancel) = st.setup.start(&format!("component:{}", comp.id));
    let jid = job.id.clone();
    tokio::spawn(run_component(st.clone(), comp.id, action == "install", job, cancel));
    axum::Json(json!({"job": jid})).into_response()
}

async fn get_onboarding(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(onboarding_state(&st).await)
}

async fn post_onboarding(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    let step = b.get("step").and_then(Value::as_str).unwrap_or("");
    let mut marks = cfg(&st).get("onboarding").and_then(Value::as_object).cloned().unwrap_or_default();
    if step == "reset" {
        marks.clear();
    } else if ONBOARDING_STEPS.contains(&step) {
        let done = match b.get("done") {
            Some(Value::Bool(d)) => *d,
            Some(Value::Null) | None => false,
            Some(v) => v.as_i64().is_some_and(|n| n != 0) || v.as_str().is_some_and(|s| !s.is_empty()),
        };
        marks.insert(step.into(), json!(done));
    } else {
        return err(StatusCode::BAD_REQUEST, &format!("step must be one of {} or reset", ONBOARDING_STEPS.join(", ")));
    }
    if let Err(e) = set(&st, json!({"onboarding": marks})) {
        return err(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    axum::Json(onboarding_state(&st).await).into_response()
}

async fn restart(State(st): State<Arc<AppState>>) -> Response {
    log::info!("restart requested from the dashboard");
    let app = st.setup.app.lock().unwrap().clone();
    match app.restart() {
        Ok(()) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => (StatusCode::CONFLICT, axum::Json(json!({"ok": false, "error": e}))).into_response(),
    }
}

async fn window(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let page =
        b.get("page").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let page = if page == "null" { String::new() } else { page };
    if !crate::appctl::valid_page(&page) {
        return axum::Json(json!({"ok": false}));
    }
    let app = st.setup.app.lock().unwrap().clone();
    let ok = tokio::task::spawn_blocking(move || app.show_window(&page)).await.unwrap_or(false);
    axum::Json(json!({"ok": ok}))
}

#[cfg(test)]
#[path = "setup_tests.rs"]
mod tests;
