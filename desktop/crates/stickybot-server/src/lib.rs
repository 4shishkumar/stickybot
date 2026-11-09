//! StickyBot's dashboard and local API, served by axum, on top of the stick link.
//!
//! It serves the Python agent's own page (`mac-agent/stick_agent/dashboard.html` + `static/`), so
//! the two apps look the same; the API is ported one vertical slice at a time ([`api::PORTED`]).
//! Everything else answers 404, which the page already shows as "not available in this version".

pub mod api;
pub mod config;
pub mod hub;

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
}

impl AppState {
    pub fn new(opts: &Options, backend: Backend) -> Result<Arc<Self>, String> {
        let paths = config::Paths::new(&opts.home)?;
        paths.ensure().map_err(|e| format!("{}: {e}", paths.home.display()))?;
        let token = paths.token_value().map_err(|e| format!("token: {e}"))?;
        let cfg = config::load(&paths);
        let fake = backend.is_fake();
        Ok(Arc::new(Self {
            link: Link::new(backend),
            cfg: Mutex::new(cfg),
            paths,
            token,
            hub: hub::Hub::default(),
            port: opts.port,
            started: hub::now(),
            fake,
        }))
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
    pub fn agent_state(&self) -> &'static str {
        match self.link.state() {
            LinkState::Connected => "ready",
            s => s.as_str(),
        }
    }

    /// `GET /api/status`: the Python shape, with what isn't ported left empty (the pages cope).
    pub fn status(&self) -> Value {
        let cfg = self.cfg.lock().unwrap();
        json!({
            "state": self.agent_state(),
            "detail": if self.fake { "Rust preview with a fake stick" } else { "Rust preview: Bluetooth link + Stick page" },
            "link": self.link.state().as_str(),
            "stick": cfg.get("stick").cloned().unwrap_or(Value::Null),
            "stt_ready": false,
            "busy": false,
            "mtu": self.link.mtu(),
            "stats": {"requests": 0, "quick": 0, "claude": 0, "started": self.started},
            "prompts": [],
            "asks": [],
            "mcp": false,
            "model": cfg.get("model").cloned().unwrap_or(Value::Null),
            "weather": null,
            "github": null,
            "stick_approvals": cfg.get("stick_approvals").cloned().unwrap_or(json!(true)),
            "wireless_mic": null,
            "update": {},
            "brain": null,
            "app": {"name": "StickyBot (Rust)", "window": true, "rust": true, "fake_stick": self.fake},
        })
    }

    /// Settings changed (PUT /api/config): save, tell the pages, follow a new stick.
    pub fn update_config(&self, changes: Map<String, Value>) -> Result<Map<String, Value>, String> {
        let stick_changed;
        let snapshot = {
            let mut cfg = self.cfg.lock().unwrap();
            stick_changed = changes.get("stick").is_some_and(|s| cfg.get("stick") != Some(s));
            cfg.extend(changes);
            config::save(&self.paths, &cfg).map_err(|e| format!("couldn't save the settings: {e}"))?;
            cfg.clone()
        };
        if stick_changed {
            self.link.set_target(self.stick());
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
    spawn_link_watcher(state.clone());
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
    let mut msgs = state.link.messages();
    let st = state.clone();
    tokio::spawn(async move {
        loop {
            let s = *w.borrow_and_update();
            st.hub.emit("link", json!({"state": s.as_str()}));
            st.hub.emit("state", json!({"state": st.agent_state(), "detail": ""}));
            if w.changed().await.is_err() {
                return;
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
