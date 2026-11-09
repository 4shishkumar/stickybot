//! StickyBot's dashboard and local API, served by axum, on top of the stick link.
//!
//! It serves the Python agent's own page (`mac-agent/stick_agent/dashboard.html` + `static/`), so
//! the two apps look the same; the API is ported one vertical slice at a time ([`api::PORTED`]).
//! Everything else answers 404, which the page already shows as "not available in this version".

pub mod api;
pub mod approvals;
pub mod brain;
pub mod clips;
pub mod commands;
pub mod config;
pub mod firmware;
pub mod hub;
pub mod python_agent;
pub mod speech;
pub mod talk;

use serde_json::{Map, Value, json};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use stick_link::{Backend, Link, LinkState, Target};

pub use config::DEFAULT_PORT;

/// How to start.
#[derive(Debug, Clone)]
pub struct Options {
    pub port: u16,
    pub home: PathBuf,
    /// a stick in memory instead of Bluetooth (demos, screenshots, UI work while the Python agent
    /// has the real stick)
    pub fake_stick: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, home: config::default_home(), fake_stick: false }
    }
}

impl Options {
    /// From `STICKYBOT_RS_PORT`, `STICKYBOT_RS_HOME`, `STICKYBOT_RS_FAKE_STICK=1`.
    pub fn from_env() -> Self {
        let mut o = Self::default();
        if let Some(p) = std::env::var("STICKYBOT_RS_PORT").ok().and_then(|p| p.parse().ok()) {
            o.port = p;
        }
        o.fake_stick = std::env::var("STICKYBOT_RS_FAKE_STICK").is_ok_and(|v| v == "1");
        o
    }
}

/// Everything the API handlers share.
pub struct AppState {
    pub link: Link<Backend>,
    pub cfg: Mutex<Map<String, Value>>,
    pub paths: config::Paths,
    pub token: String,
    pub hub: hub::Hub,
    pub port: u16,
    pub started: f64,
    pub fake: bool,
    /// speech to text (the model download + its worker thread)
    pub speech: Arc<speech::Speech>,
    pub commands: commands::Store,
    pub apps: commands::Apps,
    /// runs quick commands' `open` / shell (tests swap it)
    runner: Mutex<commands::Runner>,
    brain: Mutex<Arc<dyn talk::Brain>>,
    /// what the agent is doing beyond the link: `(state, detail)` while it listens, transcribes,
    /// thinks or works; `None` = idle
    activity: Mutex<Option<(String, String)>>,
    stats: Mutex<Map<String, Value>>,
    /// the last good brain reply `{"t", "text"}` (the home card, later)
    pub last_reply: Mutex<Option<Value>>,
    /// firmware update jobs (Bluetooth)
    pub jobs: firmware::Jobs,
    /// Claude Code permission prompts waiting on the stick
    pub approvals: approvals::Approvals,
    /// how long a firmware update waits for things (tests shorten it)
    pub ota_timings: Mutex<firmware::UpdateTimings>,
}

impl AppState {
    pub fn new(opts: &Options, backend: Backend) -> Result<Arc<Self>, String> {
        let paths = config::Paths::new(&opts.home)?;
        paths.ensure().map_err(|e| format!("{}: {e}", paths.home.display()))?;
        let token = paths.token_value().map_err(|e| format!("token: {e}"))?;
        let cfg = config::load(&paths);
        let fake = backend.is_fake();
        let commands = commands::Store::new(paths.commands());
        if let Err(e) = commands.ensure() {
            log::warn!("{}: {e}", commands.path.display());
        }
        let started = hub::now();
        let st = Arc::new(Self {
            link: Link::new(backend),
            cfg: Mutex::new(cfg),
            paths,
            token,
            hub: hub::Hub::default(),
            port: opts.port,
            started,
            fake,
            speech: Arc::new(speech::Speech::default()),
            commands,
            apps: commands::Apps::system(),
            runner: Mutex::new(commands::system_runner()),
            brain: Mutex::new(Arc::new(talk::NoBrain)),
            activity: Mutex::new(None),
            stats: Mutex::new(
                json!({"requests": 0, "quick": 0, "claude": 0, "started": started})
                    .as_object()
                    .cloned()
                    .unwrap_or_default(),
            ),
            last_reply: Mutex::new(None),
            jobs: firmware::Jobs::default(),
            approvals: approvals::Approvals::default(),
            ota_timings: Mutex::new(firmware::UpdateTimings::default()),
        });
        let weak = Arc::downgrade(&st);
        st.speech.on_change(move |p| {
            if let Some(st) = weak.upgrade() {
                st.hub.emit("speech", p.to_json());
            }
        });
        Ok(st)
    }

    /// Firmware bundles on this Mac, best first (`~/.stickybot-rs/firmware`, `$STICKYBOT_RS_FIRMWARE`).
    pub fn firmware(&self) -> Vec<Value> {
        let extra = std::env::var_os("STICKYBOT_RS_FIRMWARE").map(std::path::PathBuf::from);
        firmware::bundles(&firmware::firmware_root(&self.paths.home), extra.as_deref())
    }

    pub fn runner(&self) -> commands::Runner {
        self.runner.lock().unwrap().clone()
    }

    pub fn set_runner(&self, r: commands::Runner) {
        *self.runner.lock().unwrap() = r;
    }

    pub fn brain(&self) -> Arc<dyn talk::Brain> {
        self.brain.lock().unwrap().clone()
    }

    pub fn set_brain(&self, b: Arc<dyn talk::Brain>) {
        *self.brain.lock().unwrap() = b;
    }

    /// The agent is busy with a talk / request (core.py `set_state`).
    pub fn set_activity(&self, state: &str, detail: &str) {
        *self.activity.lock().unwrap() = Some((state.to_string(), detail.to_string()));
        self.hub.emit("state", json!({"state": state, "detail": detail}));
    }

    /// Back to idle: the pill follows the link again.
    pub fn set_idle(&self) {
        *self.activity.lock().unwrap() = None;
        self.hub.emit("state", json!({"state": self.agent_state(), "detail": self.detail()}));
    }

    pub fn count(&self, what: &str) {
        let mut s = self.stats.lock().unwrap();
        let n = s.get(what).and_then(Value::as_u64).unwrap_or(0);
        s.insert(what.into(), json!(n + 1));
    }

    /// One line in `history.jsonl` (the Activity page).
    pub fn history(&self, entry: &Value) {
        use std::io::Write;
        let r = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.paths.history())
            .and_then(|mut f| writeln!(f, "{entry}"));
        if let Err(e) = r {
            log::warn!("history: {e}");
        }
    }

    /// The word hints for the speech model (config `stt_hints`).
    pub fn stt_hints(&self) -> Vec<String> {
        let cfg = self.cfg.lock().unwrap();
        match cfg.get("stt_hints").and_then(Value::as_array) {
            Some(a) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
            None => speech::DEFAULT_HINTS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Gets the speech model if needed (progress as `speech` events), then loads it.
    pub fn spawn_speech(self: &Arc<Self>) {
        let st = self.clone();
        tokio::task::spawn_blocking(move || {
            let models = st.paths.models();
            let s2 = st.clone();
            match speech::ensure_model(&models, &speech::ModelSource::default(), &|p| s2.speech.set_phase(p)) {
                Ok(dir) => st.speech.start(speech::parakeet(&dir, &st.stt_hints())),
                Err(e) => {
                    log::error!("speech model: {e:#}");
                    st.speech.set_phase(speech::Phase::Failed(format!("{e:#}")));
                }
            }
        });
    }

    pub fn url(&self) -> String {
        format!("http://localhost:{}/", self.port)
    }

    pub fn stick(&self) -> Option<Target> {
        let cfg = self.cfg.lock().unwrap();
        cfg.get("stick").and_then(|s| serde_json::from_value(s.clone()).ok())
    }

    /// What the agent is doing, for the status pill: `ready` when the stick is connected
    /// (there's no speech model to wait for yet), else the link's state.
    pub fn agent_state(&self) -> String {
        if let Some((s, _)) = self.activity.lock().unwrap().as_ref() {
            return s.clone();
        }
        match self.link.state() {
            LinkState::Connected => "ready".into(),
            s => s.as_str().into(),
        }
    }

    /// The status line under the pill: why the link waits / searches, else what this app is.
    pub fn detail(&self) -> String {
        if let Some((_, d)) = self.activity.lock().unwrap().as_ref() {
            return d.clone();
        }
        match (self.fake, self.link.held_back(), self.link.reason()) {
            (true, _, _) => "Rust preview with a fake stick".to_string(),
            (false, Some(why), _) => format!("Waiting: {why}"),
            (false, None, Some(why)) => format!("Searching: {why}"),
            (false, None, None) => "Rust preview".to_string(),
        }
    }

    /// `GET /api/status`: the Python shape, with what isn't ported left empty (the pages cope).
    pub fn status(&self) -> Value {
        let cfg = self.cfg.lock().unwrap();
        json!({
            "state": self.agent_state(),
            "detail": self.detail(),
            "held_back": self.link.held_back(),
            "link": self.link.state().as_str(),
            "stick": cfg.get("stick").cloned().unwrap_or(Value::Null),
            "stt_ready": self.speech.ready(),
            "speech": self.speech.phase().to_json(),
            "busy": self.brain().busy(),
            "mtu": self.link.mtu(),
            "stats": Value::Object(self.stats.lock().unwrap().clone()),
            "prompts": self.approvals.list(),
            "asks": [],
            "mcp": false,
            "model": cfg.get("model").cloned().unwrap_or(Value::Null),
            "weather": null,
            "github": null,
            "stick_approvals": cfg.get("stick_approvals").cloned().unwrap_or(json!(true)),
            "wireless_mic": null,
            "update": {},
            "brain": self.brain().status(),
            "app": {"name": "StickyBot (Rust)", "window": true, "rust": true, "fake_stick": self.fake},
        })
    }

    /// Settings changed (PUT /api/config): save, tell the pages, follow a new stick.
    pub fn update_config(&self, changes: Map<String, Value>) -> Result<Map<String, Value>, String> {
        let stick_changed;
        let hints_changed;
        let snapshot = {
            let mut cfg = self.cfg.lock().unwrap();
            stick_changed = changes.get("stick").is_some_and(|s| cfg.get("stick") != Some(s));
            hints_changed = changes.get("stt_hints").is_some_and(|s| cfg.get("stt_hints") != Some(s));
            cfg.extend(changes);
            config::save(&self.paths, &cfg).map_err(|e| format!("couldn't save the settings: {e}"))?;
            cfg.clone()
        };
        if stick_changed {
            self.link.set_target(self.stick());
        }
        let dir = self.paths.models().join(speech::MODEL_NAME);
        if hints_changed && self.speech.ready() && speech::model_ready(&dir) {
            log::info!("word hints changed: reloading the speech model");
            self.speech.start(speech::parakeet(&dir, &self.stt_hints()));
        }
        self.hub.emit("config", json!({"config": snapshot}));
        Ok(snapshot)
    }
}

/// A running server.
pub struct Running {
    pub state: Arc<AppState>,
    pub addr: SocketAddr,
}

/// Binds 127.0.0.1:`port`, starts the link (only if a stick was picked in *this* app's settings)
/// and serves the dashboard. Needs a tokio runtime.
pub async fn start(opts: Options) -> Result<Running, String> {
    let backend = if opts.fake_stick { Backend::Fake(stick_link::fake::FakeStick::new()) } else { Backend::system() };
    let state = AppState::new(&opts, backend)?;
    if state.fake && state.stick().is_none() {
        // the fake stick is "picked" so the pages show a connected stick right away (not saved)
        state.cfg.lock().unwrap().insert("stick".into(), json!({"address": "FAKE", "name": "StickyBot (fake)"}));
    }
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", opts.port))
        .await
        .map_err(|e| format!("can't listen on 127.0.0.1:{}: {e}", opts.port))?;
    let addr = listener.local_addr().map_err(|e| e.to_string())?;
    let weak = Arc::downgrade(&state);
    let cfg: brain::Config =
        Arc::new(move || weak.upgrade().map(|s| s.cfg.lock().unwrap().clone()).unwrap_or_else(config::defaults));
    state.set_brain(Arc::new(brain::ClaudeBrain::new(cfg, state.paths.home.join("workspace"))));
    spawn_link_watcher(state.clone());
    talk::spawn(state.clone());
    if std::env::var("STICKYBOT_RS_NO_STT").as_deref() != Ok("1") {
        state.spawn_speech();
    }
    if !state.fake && std::env::var("STICKYBOT_RS_ALLOW_SHARED").as_deref() != Ok("1") {
        let port =
            std::env::var("STICKYBOT_RS_PYTHON_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(python_agent::PORT);
        state.link.set_gate(Arc::new(move || Box::pin(python_agent::check(port))));
    }
    state.link.spawn();
    state.link.set_target(state.stick());
    let app = api::router(state.clone());
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            log::error!("dashboard server stopped: {e}");
        }
    });
    log::info!("dashboard on {}", state.url());
    Ok(Running { state, addr })
}

/// Link changes become `link` + `state` events (the pill, Overview), like core.py `_on_link`.
fn spawn_link_watcher(state: Arc<AppState>) {
    let mut w = state.link.watch_state();
    let mut why = state.link.watch_reason();
    let mut msgs = state.link.messages();
    let st = state.clone();
    tokio::spawn(async move {
        let mut last = None;
        loop {
            let s = *w.borrow_and_update();
            why.borrow_and_update();
            if last != Some(s) {
                st.hub.emit("link", json!({"state": s.as_str()}));
                if s == LinkState::Connected {
                    st.brain().prewarm(); // Claude Code ready before the first request (if picked)
                }
                last = Some(s);
            }
            st.hub.emit("state", json!({"state": st.agent_state(), "detail": st.detail()}));
            tokio::select! {
                r = w.changed() => if r.is_err() { return; },
                r = why.changed() => if r.is_err() { return; },
            }
        }
    });
    tokio::spawn(async move {
        loop {
            match msgs.recv().await {
                Ok(m) => log::debug!("stick: {m}"),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return,
            }
        }
    });
}
