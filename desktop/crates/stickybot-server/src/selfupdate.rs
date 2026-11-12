//! The Mac app installing its own update ("Install and restart"), which `updates.rs` only found
//! and `selfupdate.py` did for the Python app. The server decides everything here (what may be
//! installed, when, in which order, what the person is told); what only the menu bar app can do
//! — download the signed update, check its minisign signature, swap the `.app` — sits behind
//! [`Installer`], which `stickybot-app/src/updater.rs` implements with Tauri's updater plugin.
//! The headless binary and the tests have [`NoInstaller`] / [`FakeInstaller`].
//!
//! ```text
//! start ── refuse in words? ──▶ downloading ──▶ (waiting while the stick is busy) ──▶ installing ──▶ restarting
//!               (409)               │  the plugin checks the signature here             notify the stick,    bye,
//!                                   ▼                                                   write `pending`,     AppControl::
//!                                 error ◀─────────────────────────────────────────────  swap the bundle     restart
//! ```
//!
//! The busy gate sits between "verified" and "replace": downloading disturbs nothing, the restart
//! is what would cut a talk, a prompt or a firmware update. The state shows on `/api/update`
//! (`state`, `progress`, `waiting`, `install_error`, `updated`) and as hub `update` events.

use crate::AppState;
use crate::releases::{self, DEFAULT_REPO};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::future::BoxFuture;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Progress events to the dashboard: at most 5 a second (a download calls back per chunk, a few
/// hundred a second), but always the last one.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(200);
/// How often a verified download looks again whether the stick is free: a talk or a face install
/// ends in seconds, so a slower look would only make the person wait.
pub const WAIT_POLL: Duration = Duration::from_secs(5);
/// How long it waits at most: a firmware update over Bluetooth takes minutes; much longer and the
/// app would restart hours later, when nobody expects it. Then it says so and the person asks again.
pub const WAIT_MAX: Duration = Duration::from_secs(30 * 60);
/// `update_auto` installs only when no button was pressed on the stick for this long: someone
/// at the desk is the one who'd be cut off by a restart.
pub const AUTO_IDLE: Duration = Duration::from_secs(10 * 60);
/// A version that was tried (any way) is not tried again by `update_auto` for this long, so a
/// failing install doesn't repeat every minute (the person can still press the button).
pub const AUTO_RETRY: Duration = Duration::from_secs(24 * 3600);
/// "Updated to vX" stays on `/api/update` this long after the restart.
pub const UPDATED_SHOWN: Duration = Duration::from_secs(24 * 3600);
/// A `pending` record older than this is from an install that died long ago, not from the one
/// that just restarted us: forgotten without a word.
pub const PENDING_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// After the restart the "Updated to vX" toast waits this long for the stick to connect.
pub const TOAST_WAIT: Duration = Duration::from_secs(10 * 60);
pub const TOAST_POLL: Duration = Duration::from_secs(2);

pub const ONLY_MENU_BAR_APP: &str = "only the menu bar app updates itself — download the new StickyBot-<version>-macos-arm64.zip from the release (README: Install the Mac app), or build it from the repo";

/// The update file every installable release carries (`tools/release_tool.py latest-json`).
pub const UPDATER_ASSET: &str = "latest.json";

// --- the installer the app provides ------------------------------------------------------------------------

/// `progress(bytes so far, total if the server said)`.
pub type Progress = Arc<dyn Fn(u64, Option<u64>) + Send + Sync>;

/// What only the menu bar app can do for an app update (the Tauri updater plugin).
pub trait Installer: Send + Sync {
    /// Why this app can't replace itself here, in words (`None` = it can): the headless binary,
    /// a translocated or read-only place, not inside a `.app`.
    fn unavailable(&self) -> Option<String>;
    /// Downloads the update at `endpoint` (a `latest.json`) and checks its signature. Nothing on
    /// disk is replaced yet.
    fn download(
        &self,
        endpoint: String,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Staged>, InstallError>>;
}

/// A downloaded, verified update, waiting to be put in place.
pub trait Staged: Send {
    fn version(&self) -> String;
    /// Replaces the app (blocking). The relaunch is `AppControl::restart`.
    fn install(self: Box<Self>) -> Result<(), InstallError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    /// the download isn't signed with the key the app carries
    Signature(String),
    Network(String),
    /// the folder StickyBot sits in can't be written (the detail is the place)
    NotWritable(String),
    NoUpdate(String),
    Other(String),
}

impl InstallError {
    /// The short name the dashboard keys its heading on.
    pub fn kind(&self) -> &'static str {
        match self {
            InstallError::Signature(_) => "signature",
            InstallError::Network(_) => "network",
            InstallError::NotWritable(_) => "not-writable",
            InstallError::NoUpdate(_) => "no-update",
            InstallError::Other(_) => "other",
        }
    }

    /// What the person reads. The download page is the way out for every kind (the dashboard and
    /// the menu show it next to this).
    pub fn words(&self) -> String {
        match self {
            InstallError::Signature(_) => {
                "The download didn't match StickyBot's signing key, so it was not installed.".into()
            }
            InstallError::Network(_) => "Couldn't download the update (no connection?).".into(),
            InstallError::NotWritable(at) => {
                format!("StickyBot can't replace itself at {at} (move it to Applications, or download it by hand).")
            }
            InstallError::NoUpdate(m) if m.is_empty() => "There's no newer StickyBot to install.".into(),
            InstallError::NoUpdate(m) | InstallError::Other(m) => m.clone(),
        }
    }
}

impl std::fmt::Display for InstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallError::Signature(d)
            | InstallError::Network(d)
            | InstallError::NotWritable(d)
            | InstallError::NoUpdate(d)
            | InstallError::Other(d) => write!(f, "{}: {d}", self.kind()),
        }
    }
}

/// The headless binary: it can't replace itself.
pub struct NoInstaller;

impl Installer for NoInstaller {
    fn unavailable(&self) -> Option<String> {
        Some(ONLY_MENU_BAR_APP.into())
    }
    fn download(&self, _endpoint: String, _p: Progress) -> BoxFuture<'static, Result<Box<dyn Staged>, InstallError>> {
        Box::pin(async { Err(InstallError::Other(ONLY_MENU_BAR_APP.into())) })
    }
}

/// For tests: a scripted installer that records what it was asked.
pub struct FakeInstaller {
    pub why: Mutex<Option<String>>,
    /// the version the downloaded update says it is
    pub version: Mutex<String>,
    pub download_fails: Mutex<Option<InstallError>>,
    pub install_fails: Mutex<Option<InstallError>>,
    /// how long the download takes (it reports 3 chunks of 1 kB meanwhile)
    pub delay: Mutex<Duration>,
    pub endpoints: Mutex<Vec<String>>,
    pub installed: Arc<std::sync::atomic::AtomicUsize>,
}

impl FakeInstaller {
    pub fn new(version: &str) -> Arc<Self> {
        Arc::new(Self {
            why: Mutex::new(None),
            version: Mutex::new(version.into()),
            download_fails: Mutex::new(None),
            install_fails: Mutex::new(None),
            delay: Mutex::new(Duration::ZERO),
            endpoints: Mutex::new(Vec::new()),
            installed: Arc::default(),
        })
    }
}

struct FakeStaged {
    version: String,
    fails: Option<InstallError>,
    installed: Arc<std::sync::atomic::AtomicUsize>,
}

impl Staged for FakeStaged {
    fn version(&self) -> String {
        self.version.clone()
    }
    fn install(self: Box<Self>) -> Result<(), InstallError> {
        match self.fails {
            Some(e) => Err(e),
            None => {
                self.installed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(())
            }
        }
    }
}

impl Installer for FakeInstaller {
    fn unavailable(&self) -> Option<String> {
        self.why.lock().unwrap().clone()
    }
    fn download(
        &self,
        endpoint: String,
        progress: Progress,
    ) -> BoxFuture<'static, Result<Box<dyn Staged>, InstallError>> {
        self.endpoints.lock().unwrap().push(endpoint);
        let (delay, fails) = (*self.delay.lock().unwrap(), self.download_fails.lock().unwrap().clone());
        let staged = FakeStaged {
            version: self.version.lock().unwrap().clone(),
            fails: self.install_fails.lock().unwrap().clone(),
            installed: self.installed.clone(),
        };
        Box::pin(async move {
            for n in 1..=3u64 {
                progress(n * 1000, Some(3000));
                tokio::time::sleep(delay / 3).await;
            }
            match fails {
                Some(e) => Err(e),
                None => Ok(Box::new(staged) as Box<dyn Staged>),
            }
        })
    }
}

// --- channel -> endpoint ---------------------------------------------------------------------------------------

/// `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…`: the only plain-http address an
/// update may come from, and only in debug builds and tests (a local fake release server).
pub fn local_url(u: &str) -> bool {
    let Some(rest) = u.strip_prefix("http://") else { return false };
    let host_port = rest.split('/').next().unwrap_or("");
    let Some((host, port)) = host_port.split_once(':') else { return false };
    (host == "127.0.0.1" || host == "localhost") && port.parse::<u16>().is_ok()
}

fn tag_ok(tag: &str) -> bool {
    !tag.is_empty() && tag.len() <= 80 && tag.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn repo_ok(repo: &str) -> bool {
    let mut p = repo.split('/');
    let part = |s: Option<&str>| {
        s.is_some_and(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)))
    };
    part(p.next()) && part(p.next()) && p.next().is_none()
}

/// Where the updater reads `latest.json` from. Stable: GitHub's "latest" (it never is a
/// prerelease). Beta: the release the check picked, by its tag — built here from the compiled-in
/// repo and a validated tag, never copied from what the GitHub API answered. Nothing in the settings
/// or the environment changes it, except in debug builds and tests where `override_url` (from
/// `STICKYBOT_RS_UPDATE_ENDPOINT`) may point at a local server. `Err`: why in-app install is off.
pub fn endpoint(
    repo: &str,
    channel: &str,
    tag: &str,
    override_url: Option<&str>,
    debug_build: bool,
) -> Result<String, String> {
    if debug_build && let Some(o) = override_url.filter(|o| !o.is_empty()) {
        if local_url(o) {
            return Ok(o.to_string());
        }
        log::warn!(
            "ignoring the update endpoint {o}: only http://127.0.0.1:<port>/… or http://localhost:<port>/… is accepted"
        );
    }
    if repo != DEFAULT_REPO || !repo_ok(repo) {
        return Err("this build checks another repo — download the update by hand".into());
    }
    if channel == "beta" {
        if !tag_ok(tag) {
            return Err("the release's tag looks odd — download the update by hand".into());
        }
        return Ok(format!("https://github.com/{repo}/releases/download/{tag}/{UPDATER_ASSET}"));
    }
    Ok(format!("https://github.com/{repo}/releases/latest/download/{UPDATER_ASSET}"))
}

// --- the busy gate -----------------------------------------------------------------------------------------------

/// What is going on that a restart would cut off.
#[derive(Debug, Clone, Default)]
pub struct Busy {
    /// a firmware update over Bluetooth, or a USB flash (the flasher uses the same jobs)
    pub firmware: bool,
    pub usb_flash: bool,
    /// listening, transcribing, thinking or working on a request
    pub voice: bool,
    pub prompt: bool,
    pub asking: bool,
    pub wireless_mic: bool,
    pub face_install: bool,
}

impl Busy {
    pub fn of(st: &AppState) -> Self {
        Self {
            firmware: st.jobs.busy(),
            usb_flash: st.usb.is_flashing(),
            voice: matches!(st.agent_state().as_str(), "listening" | "transcribing" | "thinking" | "working")
                || st.brain().busy(),
            prompt: st.approvals.pending(),
            asking: st.approvals.asking(),
            wireless_mic: st.wmic.streaming(),
            face_install: st.faces.busy() || st.faces.jobs.busy(),
        }
    }
}

/// Why the app must not restart now, in words (`None` = it may). Being paused isn't busy: pausing
/// lets go of the stick, so replacing the app then disturbs nothing.
pub fn busy_reason(b: &Busy) -> Option<&'static str> {
    if b.firmware {
        Some("a firmware update is running")
    } else if b.usb_flash {
        Some("the stick is being flashed over USB")
    } else if b.prompt {
        Some("a permission prompt is waiting on the stick")
    } else if b.asking {
        Some("a question is waiting on the stick")
    } else if b.voice {
        Some("a voice request is in progress")
    } else if b.wireless_mic {
        Some("the wireless mic is in use")
    } else if b.face_install {
        Some("a face is being installed")
    } else {
        None
    }
}

// --- what the person may ask for ---------------------------------------------------------------------------------

/// Why an install isn't started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// one at a time
    Running,
    NothingNew,
    /// the release isn't newer than this app (never a lower or equal version)
    NotNewer {
        release: String,
        current: String,
    },
    /// the release was cut without an update file (older ones, or no signing key)
    NoUpdateFile,
    Unavailable(String),
}

impl Refusal {
    pub fn words(&self) -> String {
        match self {
            Refusal::Running => "An update is already being installed.".into(),
            Refusal::NothingNew => "There's no newer StickyBot to install — check for updates first.".into(),
            Refusal::NotNewer { release, current } => {
                format!("v{release} isn't newer than this StickyBot (v{current}), so it won't be installed.")
            }
            Refusal::NoUpdateFile => {
                "This release can't be installed from inside the app (it has no update file) — download it from the release page.".into()
            }
            Refusal::Unavailable(why) => {
                let mut w: String = why.chars().take(1).flat_map(char::to_uppercase).collect();
                w.extend(why.chars().skip(1));
                if w.ends_with('.') { w } else { format!("{w}.") }
            }
        }
    }
}

/// What an install would do: the release it is for and where its update file is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub version: String,
    pub tag: String,
    pub endpoint: String,
}

/// The release summary (`updates.rs` `fetch`), this app's version and channel -> what to install,
/// or why not. Pure: the installer's word and the endpoint override come in as arguments.
pub fn plan(
    latest: Option<&Value>,
    current: &str,
    repo: &str,
    channel: &str,
    unavailable: Option<String>,
    override_url: Option<&str>,
    debug_build: bool,
) -> Result<Plan, Refusal> {
    let lat = latest.ok_or(Refusal::NothingNew)?;
    let s = |k: &str| lat.get(k).and_then(Value::as_str);
    let (Some(version), Some(tag)) = (s("version"), s("tag")) else { return Err(Refusal::NothingNew) };
    if s("app_asset").is_none() {
        return Err(Refusal::NothingNew); // a release without the Mac app isn't an app update
    }
    if !releases::newer(Some(version), Some(current)) {
        return Err(Refusal::NotNewer { release: version.into(), current: current.into() });
    }
    if s("updater_asset").is_none() {
        return Err(Refusal::NoUpdateFile);
    }
    if let Some(why) = unavailable {
        return Err(Refusal::Unavailable(why));
    }
    let endpoint = endpoint(repo, channel, tag, override_url, debug_build).map_err(Refusal::Unavailable)?;
    Ok(Plan { version: version.into(), tag: tag.into(), endpoint })
}

// --- the state ---------------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Downloading,
    /// downloaded and verified, waiting for the stick to be free (shown as `available` + `waiting`)
    Waiting,
    Installing,
    Restarting,
    Error,
}

impl Phase {
    pub fn running(self) -> bool {
        matches!(self, Phase::Downloading | Phase::Waiting | Phase::Installing | Phase::Restarting)
    }

    /// The word in `state`; `None` for the phases the check state decides (`idle`, `available`).
    pub fn state(self) -> Option<&'static str> {
        match self {
            Phase::Idle | Phase::Waiting => None,
            Phase::Downloading => Some("downloading"),
            Phase::Installing => Some("installing"),
            Phase::Restarting => Some("restarting"),
            Phase::Error => Some("error"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub kind: String,
    pub message: String,
}

/// An install that has started replacing the app: written to `updates.json` first, read after the
/// restart.
#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub from: String,
    pub to: String,
    pub at: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Updated {
    pub from: String,
    pub to: String,
    pub at: f64,
}

/// What start-up finds in `pending`.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Nothing,
    Updated { from: String, to: String },
    DidntTake { to: String, now: String },
}

/// `pending` against the version that runs now. Pure.
pub fn resolve_pending(p: Option<&Pending>, current: &str, now: f64) -> Outcome {
    let Some(p) = p else { return Outcome::Nothing };
    if now - p.at > PENDING_MAX_AGE.as_secs_f64() {
        return Outcome::Nothing;
    }
    if p.to.trim_start_matches('v') == current.trim_start_matches('v') {
        Outcome::Updated { from: p.from.clone(), to: p.to.clone() }
    } else {
        Outcome::DidntTake { to: p.to.clone(), now: current.into() }
    }
}

pub struct Timings {
    pub poll: Duration,
    pub max_wait: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self { poll: WAIT_POLL, max_wait: WAIT_MAX }
    }
}

struct RunState {
    phase: Phase,
    progress: Option<(u64, Option<u64>)>,
    last_progress_emit: Option<Instant>,
    waiting: Option<String>,
    error: Option<Failure>,
    cancelled: bool,
    cancel: Arc<tokio::sync::Notify>,
    pending: Option<Pending>,
    /// version -> when an install of it was last started (`hub::now()`)
    tried: BTreeMap<String, f64>,
    updated: Option<Updated>,
}

/// The install in progress (at most one), what it tried, and what the restart left.
pub struct Run {
    installer: Mutex<Arc<dyn Installer>>,
    s: Mutex<RunState>,
    pub timings: Mutex<Timings>,
}

impl Run {
    /// From `updates.json`'s `pending` and `tried`.
    pub fn load(saved: Option<&Value>) -> Self {
        let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let pending = saved.and_then(|v| v.get("pending")).filter(|p| p.is_object()).map(|p| Pending {
            from: text(p, "from"),
            to: text(p, "to"),
            at: num(p, "at"),
        });
        let tried = saved
            .and_then(|v| v.get("tried"))
            .and_then(Value::as_object)
            .map(|m| m.iter().filter_map(|(k, v)| v.as_f64().map(|t| (k.clone(), t))).collect())
            .unwrap_or_default();
        Self {
            installer: Mutex::new(Arc::new(NoInstaller)),
            s: Mutex::new(RunState {
                phase: Phase::Idle,
                progress: None,
                last_progress_emit: None,
                waiting: None,
                error: None,
                cancelled: false,
                cancel: Arc::new(tokio::sync::Notify::new()),
                pending: pending.filter(|p| !p.to.is_empty()),
                tried,
                updated: None,
            }),
            timings: Mutex::new(Timings::default()),
        }
    }

    /// What `updates.json` keeps of it.
    pub fn save_into(&self, out: &mut Map<String, Value>) {
        let s = self.s.lock().unwrap();
        out.insert(
            "pending".into(),
            match &s.pending {
                Some(p) => json!({"from": p.from, "to": p.to, "at": p.at}),
                None => Value::Null,
            },
        );
        let keep = s.tried.len().saturating_sub(20);
        let tried: Map<String, Value> = s.tried.iter().skip(keep).map(|(k, v)| (k.clone(), json!(v))).collect();
        out.insert("tried".into(), Value::Object(tried));
    }

    pub fn installer(&self) -> Arc<dyn Installer> {
        self.installer.lock().unwrap().clone()
    }

    pub fn set_installer(&self, i: Arc<dyn Installer>) {
        *self.installer.lock().unwrap() = i;
    }

    pub fn phase(&self) -> Phase {
        self.s.lock().unwrap().phase
    }

    fn set_phase(&self, p: Phase) {
        let mut s = self.s.lock().unwrap();
        s.phase = p;
        if p != Phase::Waiting {
            s.waiting = None;
        }
    }

    /// Idle or failed -> downloading; `false` while one runs.
    fn begin(&self) -> Option<Arc<tokio::sync::Notify>> {
        let mut s = self.s.lock().unwrap();
        if s.phase.running() {
            return None;
        }
        s.phase = Phase::Downloading;
        s.progress = None;
        s.last_progress_emit = None;
        s.waiting = None;
        s.error = None;
        s.cancelled = false;
        s.cancel = Arc::new(tokio::sync::Notify::new());
        Some(s.cancel.clone())
    }

    /// Keeps the numbers; `true` when the dashboard should be told (at most [`PROGRESS_EVERY`], and the last).
    fn progress(&self, bytes: u64, total: Option<u64>) -> bool {
        let mut s = self.s.lock().unwrap();
        if s.phase != Phase::Downloading {
            return false;
        }
        s.progress = Some((bytes, total));
        let last = total.is_some_and(|t| bytes >= t);
        let due = s.last_progress_emit.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY);
        if due || last {
            s.last_progress_emit = Some(Instant::now());
        }
        due || last
    }

    fn set_waiting(&self, reason: &str) -> bool {
        let mut s = self.s.lock().unwrap();
        let text = format!("busy — will install when idle: {reason}");
        let changed = s.waiting.as_deref() != Some(&text) || s.phase != Phase::Waiting;
        s.phase = Phase::Waiting;
        s.waiting = Some(text);
        changed
    }

    fn fail(&self, kind: &str, message: String) {
        let mut s = self.s.lock().unwrap();
        s.phase = Phase::Error;
        s.waiting = None;
        s.error = Some(Failure { kind: kind.into(), message });
    }

    fn cancelled(&self) -> bool {
        self.s.lock().unwrap().cancelled
    }

    /// The person gave up on a download or a wait (`true`), or dismissed an error (`true`).
    pub fn cancel(&self) -> bool {
        let mut s = self.s.lock().unwrap();
        match s.phase {
            Phase::Downloading | Phase::Waiting => {
                s.cancelled = true;
                s.cancel.notify_one();
                true
            }
            Phase::Error => {
                s.phase = Phase::Idle;
                s.error = None;
                true
            }
            _ => false,
        }
    }

    fn set_pending(&self, p: Option<Pending>) {
        self.s.lock().unwrap().pending = p;
    }

    pub fn tried_at(&self, version: &str) -> Option<f64> {
        self.s.lock().unwrap().tried.get(version).copied()
    }

    fn mark_tried(&self, version: &str) {
        self.s.lock().unwrap().tried.insert(version.into(), crate::hub::now());
    }

    /// `pending` against the running version, once: it's gone afterwards.
    pub fn take_pending(&self, current: &str) -> Outcome {
        let mut s = self.s.lock().unwrap();
        let o = resolve_pending(s.pending.as_ref(), current, crate::hub::now());
        s.pending = None;
        o
    }

    /// `progress`, `waiting`, `install_error` and `updated` for the status.
    pub fn fields(&self, download: &str) -> Map<String, Value> {
        let s = self.s.lock().unwrap();
        let mut m = Map::new();
        m.insert(
            "progress".into(),
            match s.progress {
                Some((bytes, total)) if s.phase == Phase::Downloading => json!({"bytes": bytes, "total": total}),
                _ => Value::Null,
            },
        );
        m.insert("waiting".into(), s.waiting.as_ref().map_or(Value::Null, |w| json!(w)));
        m.insert(
            "install_error".into(),
            match (&s.error, s.phase) {
                (Some(e), Phase::Error) => json!({"kind": e.kind, "message": e.message, "download": download}),
                _ => Value::Null,
            },
        );
        let fresh = |u: &&Updated| crate::hub::now() - u.at < UPDATED_SHOWN.as_secs_f64();
        m.insert(
            "updated".into(),
            match s.updated.as_ref().filter(fresh) {
                Some(u) => json!({"from": u.from, "to": u.to, "at": u.at}),
                None => Value::Null,
            },
        );
        m
    }
}

// --- the install ----------------------------------------------------------------------------------------------------

fn debug_build() -> bool {
    cfg!(debug_assertions)
}

impl crate::updates::Updates {
    /// What an install would do now: the release the last check found, this app, the channel, the
    /// installer's word.
    pub fn plan(&self, cfg: &Map<String, Value>) -> Result<Plan, Refusal> {
        self.plan_with(self.latest().as_ref(), cfg)
    }

    /// [`Self::plan`] for a release the caller already holds (the status does, under its lock).
    pub fn plan_with(&self, latest: Option<&Value>, cfg: &Map<String, Value>) -> Result<Plan, Refusal> {
        let over = std::env::var("STICKYBOT_RS_UPDATE_ENDPOINT").ok();
        plan(
            latest,
            self.current(),
            self.repo(),
            &crate::updates::channel(cfg),
            self.run.installer().unavailable(),
            over.as_deref(),
            debug_build(),
        )
    }
}

/// Starts installing the update the last check found (`by`: who asked, for the log). The work runs
/// in the background; the progress is on `/api/update` and in the hub's `update` events. Needs a
/// tokio runtime.
pub fn start(st: &Arc<AppState>, by: &str) -> Result<(), Refusal> {
    if st.updates.run.phase().running() {
        return Err(Refusal::Running);
    }
    let cfg = st.cfg.lock().unwrap().clone();
    let plan = st.updates.plan(&cfg)?;
    let Some(cancel) = st.updates.run.begin() else { return Err(Refusal::Running) };
    st.updates.run.mark_tried(&plan.version);
    st.updates.save();
    log::info!("installing StickyBot {} ({by}) from {}", plan.version, plan.endpoint);
    crate::updates::emit(st);
    tokio::spawn(run(st.clone(), plan, cancel));
    Ok(())
}

/// Gives up on the download or the wait; `false`: nothing to cancel (or too late: replacing).
pub fn cancel(st: &AppState) -> bool {
    let ok = st.updates.run.cancel();
    if ok {
        log::info!("update install cancelled");
        // the task ends and says so; an error dismissed has nobody to tell the pages
        if st.updates.run.phase() == Phase::Idle {
            crate::updates::emit(st);
        }
    }
    ok
}

fn failed(st: &AppState, e: &InstallError) {
    log::warn!("update install failed: {e}");
    st.updates.run.fail(e.kind(), e.words());
    crate::updates::emit(st);
}

fn dropped(st: &AppState) {
    log::info!("update install cancelled");
    st.updates.run.set_phase(Phase::Idle);
    crate::updates::emit(st);
}

async fn run(st: Arc<AppState>, plan: Plan, cancel: Arc<tokio::sync::Notify>) {
    let installer = st.updates.run.installer();
    let seen = st.clone();
    let progress: Progress = Arc::new(move |bytes, total| {
        if seen.updates.run.progress(bytes, total) {
            crate::updates::emit(&seen);
        }
    });
    // 1. download + signature check (the plugin does both before it answers)
    let staged = tokio::select! {
        r = installer.download(plan.endpoint.clone(), progress) => r,
        () = cancel.notified() => return dropped(&st),
    };
    let staged = match staged {
        Ok(s) => s,
        Err(e) => return failed(&st, &e),
    };
    let got = staged.version();
    if got.trim_start_matches('v') != plan.version || !releases::newer(Some(&got), Some(st.updates.current())) {
        let m = format!(
            "The downloaded update is v{}, not the v{} that was announced — it was not installed. Check for updates and try again.",
            got.trim_start_matches('v'),
            plan.version
        );
        return failed(&st, &InstallError::Other(m));
    }
    // 2. wait until nothing would be cut off by the restart
    let (poll, max_wait) = {
        let t = st.updates.run.timings.lock().unwrap();
        (t.poll, t.max_wait)
    };
    let waited = Instant::now();
    while let Some(why) = busy_reason(&Busy::of(&st)) {
        if st.updates.run.cancelled() {
            return dropped(&st);
        }
        if waited.elapsed() >= max_wait {
            return failed(
                &st,
                &InstallError::Other("StickyBot is still busy — try the update again when the stick is idle.".into()),
            );
        }
        if st.updates.run.set_waiting(why) {
            crate::updates::emit(&st);
        }
        tokio::select! {
            () = tokio::time::sleep(poll) => {}
            () = cancel.notified() => return dropped(&st),
        }
    }
    if st.updates.run.cancelled() {
        return dropped(&st);
    }
    // 3. replace: tell the stick first, and write down what we're about to do
    st.updates.run.set_phase(Phase::Installing);
    crate::updates::emit(&st);
    crate::events::notify(&st, "update", "Updating StickyBot…", "back in a moment", None, None, None).await;
    let from = st.updates.current().to_string();
    st.updates.run.set_pending(Some(Pending { from, to: plan.version.clone(), at: crate::hub::now() }));
    st.updates.save();
    let r = tokio::task::spawn_blocking(move || staged.install())
        .await
        .unwrap_or_else(|e| Err(InstallError::Other(format!("the installer stopped unexpectedly: {e}"))));
    if let Err(e) = r {
        st.updates.run.set_pending(None); // nothing was replaced
        st.updates.save();
        return failed(&st, &e);
    }
    // 4. relaunch (the pages poll until the app answers again)
    st.updates.run.set_phase(Phase::Restarting);
    crate::updates::emit(&st);
    st.link.bye().await;
    let app = st.setup.app.lock().unwrap().clone();
    if let Err(why) = app.restart() {
        let m = format!("StickyBot v{} is installed — quit and open it again to use it ({why}).", plan.version);
        failed(&st, &InstallError::Other(m));
    }
}

// --- after the restart --------------------------------------------------------------------------------------------

/// Start-up: if an install was under way and this app is the version it was for, say "Updated to
/// vX" once (the pages' notice now, the stick's toast when it connects); if it isn't, the update
/// didn't take: say that once. Needs a tokio runtime.
pub fn after_restart(st: &Arc<AppState>) {
    let outcome = st.updates.run.take_pending(st.updates.current());
    if outcome == Outcome::Nothing {
        return;
    }
    st.updates.save(); // `pending` is gone: once
    match outcome {
        Outcome::Updated { from, to } => {
            log::info!("updated from v{from} to v{to}");
            st.updates.run.s.lock().unwrap().updated =
                Some(Updated { from: from.clone(), to: to.clone(), at: crate::hub::now() });
            st.hub.emit("updated", json!({"from": from, "to": to}));
            crate::updates::emit(st);
            let weak = Arc::downgrade(st);
            tokio::spawn(async move { toast_when_connected(weak, to, TOAST_POLL, TOAST_WAIT).await });
        }
        Outcome::DidntTake { to, now } => {
            let m = format!("The update to v{to} didn't take — still on v{now}.");
            log::warn!("{m}");
            st.updates.run.fail("other", m);
            crate::updates::emit(st);
        }
        Outcome::Nothing => {}
    }
}

/// One "Updated to vX" toast on the stick, once it's connected (within `max`).
pub async fn toast_when_connected(st: Weak<AppState>, to: String, poll: Duration, max: Duration) -> Option<Value> {
    let until = Instant::now() + max;
    loop {
        let st = st.upgrade()?;
        if st.link.connected() {
            return crate::events::notify(
                &st,
                "update",
                &format!("Updated to v{to}"),
                "StickyBot is up to date",
                None,
                None,
                None,
            )
            .await;
        }
        drop(st);
        if Instant::now() >= until {
            return None;
        }
        tokio::time::sleep(poll).await;
    }
}

// --- update_auto ------------------------------------------------------------------------------------------------------

/// What the automatic install looks at.
#[derive(Debug, Clone, Default)]
pub struct AutoFacts {
    pub on: bool,
    /// the update check is on (no check, no update)
    pub check_on: bool,
    /// an installable update is there
    pub can_install: bool,
    pub running: bool,
    pub busy: bool,
    /// seconds since a button was pressed on the stick (`None`: not since start-up)
    pub idle_s: Option<f64>,
    /// seconds since an install of this version was last started
    pub tried_s: Option<f64>,
}

/// Quiet hours don't matter here: they mean "don't disturb me", and that is when nobody is mid-request.
pub fn auto_due(f: &AutoFacts) -> bool {
    f.on && f.check_on
        && f.can_install
        && !f.running
        && !f.busy
        && f.idle_s.is_none_or(|s| s >= AUTO_IDLE.as_secs_f64())
        && f.tried_s.is_none_or(|s| s >= AUTO_RETRY.as_secs_f64())
}

/// The check loop's tick: installs by itself when `update_auto` says so. `true` = started.
pub fn auto_tick(st: &Arc<AppState>) -> bool {
    let cfg = st.cfg.lock().unwrap().clone();
    if cfg.get("update_auto").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let plan = st.updates.plan(&cfg).ok();
    let now = crate::hub::now();
    let facts = AutoFacts {
        on: true,
        check_on: cfg.get("update_check").and_then(Value::as_bool) != Some(false),
        can_install: plan.is_some(),
        running: st.updates.run.phase().running(),
        busy: busy_reason(&Busy::of(st)).is_some(),
        idle_s: st.last_input().map(|t| now - t),
        tried_s: plan.as_ref().and_then(|p| st.updates.run.tried_at(&p.version)).map(|t| now - t),
    };
    auto_due(&facts) && start(st, "automatic").is_ok()
}

// --- the API ------------------------------------------------------------------------------------------------------------

/// A refusal's answer: 409, the words, and the state (the page keeps its card; `install.download`
/// is the way out).
fn conflict(st: &AppState, words: String) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let full = st.updates.full(&cfg);
    let body = json!({"error": words, "install": full["install"], "state": full["state"],
                      "progress": full["progress"], "waiting": full["waiting"]});
    (StatusCode::CONFLICT, axum::Json(body)).into_response()
}

/// `POST /api/update/install` (and `/api/update/agent`, which the shared page's old path calls):
/// 202 + the status once it started, 409 + words when it can't.
pub async fn install_route(State(st): State<Arc<AppState>>) -> Response {
    match start(&st, "dashboard") {
        Ok(()) => {
            let cfg = st.cfg.lock().unwrap().clone();
            (StatusCode::ACCEPTED, axum::Json(st.updates.full(&cfg))).into_response()
        }
        Err(r) => conflict(&st, r.words()),
    }
}

/// `POST /api/update/cancel`: drops a download or a wait (or dismisses an error).
pub async fn cancel_route(State(st): State<Arc<AppState>>) -> Response {
    if cancel(&st) {
        let cfg = st.cfg.lock().unwrap().clone();
        axum::Json(st.updates.full(&cfg)).into_response()
    } else {
        conflict(&st, "There's no download to cancel.".into())
    }
}

#[cfg(test)]
#[path = "selfupdate_tests.rs"]
mod tests;
