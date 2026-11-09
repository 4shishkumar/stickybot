//! The HTTP API (server.py + firmware_api.py), first slice: what the Stick page and the page shell
//! need. Same rules as server.py: only `Host: localhost|127.0.0.1:<port>`, and anything that isn't
//! a GET needs the token in `X-Stick-Token`.

use crate::AppState;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use futures_util::{Stream, StreamExt, stream};
use rust_embed::RustEmbed;
use serde_json::{Map, Value, json};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use stick_link::proto;

/// The Python agent's page and assets, reused as they are (in debug builds rust-embed reads them
/// from disk, so edits show on reload; release builds carry them inside the binary).
#[derive(RustEmbed)]
#[folder = "../../../mac-agent/stick_agent/static/"]
struct Static;

const PAGE: &str = include_str!("../../../../mac-agent/stick_agent/dashboard.html");

/// Endpoints this app answers (the rest are 404, which the page shows as "not available yet").
pub const PORTED: &[&str] = &[
    "GET /",
    "GET /static/*",
    "GET /api/status",
    "GET /api/events",
    "GET /api/config",
    "PUT /api/config",
    "POST /api/stick/proto",
    "GET /api/scan",
    "POST /api/pause",
    "GET /api/sounds",
    "PUT /api/sounds",
    "POST /api/sounds/play",
    "GET /api/rust",
];

/// The Python agent's endpoints that aren't here yet, by the phase that brings them.
pub const NOT_PORTED: &[(&str, &str)] = &[
    ("GET /api/pet", "faces / pet (the Stick page's Face card hides the pet line)"),
    ("GET /api/firmware", "firmware: bundled build, USB sticks (the Stick page says updates aren't available)"),
    ("POST /api/firmware/flash", "firmware over USB (esptool)"),
    ("POST /api/firmware/ota", "firmware over Bluetooth (needs canSendWriteWithoutResponse pacing)"),
    ("GET /api/firmware/jobs/{id}", "firmware job progress (SSE)"),
    ("POST /api/firmware/jobs/{id}/cancel", "firmware job cancel"),
    ("GET /api/onboarding", "setup wizard progress (the wizard keeps it in the browser meanwhile)"),
    ("POST /api/onboarding", "setup wizard progress"),
    ("GET /api/install", "setup page (#install)"),
    ("/api/install/*", "setup page steps and jobs"),
    ("GET /api/history", "voice requests (Activity)"),
    ("GET /api/logs", "log viewer (Logs)"),
    ("POST /api/ask", "typed request to the brain"),
    ("GET /api/commands", "quick commands"),
    ("PUT /api/commands", "quick commands"),
    ("POST /api/commands/test", "quick commands"),
    ("POST /api/permission", "Claude Code approvals (hook long-poll)"),
    ("POST /api/permission/decide", "Claude Code approvals"),
    ("GET /api/clips", "sound clips library"),
    ("POST /api/clips/play", "sound clips (AUDIO_IN streaming)"),
    ("GET /api/voices", "macOS voices (say)"),
    ("POST /api/voice/test", "speech"),
    ("POST /api/events/test", "stick events"),
    ("GET /api/events/history", "stick events"),
    ("GET /api/events/kinds", "stick events"),
    ("POST /api/claude-event", "Claude Code Stop hook"),
    ("POST /api/wireless-mic/show", "wireless Wispr mic (Core Audio tap)"),
    ("POST /api/wireless-mic/check", "wireless Wispr mic (Core Audio tap)"),
    ("GET /api/mcp", "stick tools (MCP)"),
    ("POST /mcp", "stick tools (MCP server)"),
    ("POST /api/mcp/terminal/{action}", "stick tools (MCP)"),
    ("POST /api/mcp/answer", "stick tools (MCP)"),
    ("GET /api/brain", "brains (Claude Code / OpenRouter)"),
    ("/api/brain/*", "brains (OpenRouter key, models, test)"),
    ("GET /api/update", "update check (releases)"),
    ("/api/update/*", "updates (agent / firmware)"),
    ("GET /api/components", "installer components"),
    ("/api/components/*", "installer components"),
    ("POST /api/new-chat", "brain conversation"),
    ("POST /api/restart", "restart the agent"),
    ("POST /api/window", "show the window at a page"),
];

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/static/{*path}", get(static_file))
        .route("/api/status", get(status))
        .route("/api/events", get(events))
        .route("/api/config", get(get_config).put(put_config))
        .route("/api/stick/proto", post(stick_proto))
        .route("/api/scan", get(scan))
        .route("/api/pause", post(pause))
        .route("/api/sounds", get(get_sounds).put(put_sounds))
        .route("/api/sounds/play", post(play_sound))
        .route("/api/rust", get(rust_info))
        .fallback(not_ported)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .with_state(state)
}

fn err(status: StatusCode, msg: &str) -> Response {
    (status, axum::Json(json!({"error": msg}))).into_response()
}

fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn guard(State(st): State<Arc<AppState>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if host != format!("localhost:{}", st.port) && host != format!("127.0.0.1:{}", st.port) {
        return err(StatusCode::FORBIDDEN, "bad host");
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        let tok = req.headers().get("x-stick-token").map(|v| v.as_bytes()).unwrap_or(b"");
        if !same(tok, st.token.as_bytes()) {
            return err(StatusCode::FORBIDDEN, "bad token");
        }
    }
    let revalidate = req.uri().path() == "/" || req.uri().path().starts_with("/static/");
    let mut resp = next.run(req).await;
    if revalidate {
        resp.headers_mut().entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-cache"));
    }
    resp
}

async fn not_ported(req: Request) -> Response {
    let what = format!("{} {}", req.method(), req.uri().path());
    (StatusCode::NOT_FOUND, axum::Json(json!({"error": "Not in the Rust app yet.", "missing": true, "endpoint": what})))
        .into_response()
}

async fn page(State(st): State<Arc<AppState>>) -> Html<String> {
    Html(PAGE.replace("{{TOKEN}}", &st.token).replace("<title>StickyBot</title>", "<title>StickyBot (Rust)</title>"))
}

async fn static_file(Path(path): Path<String>) -> Response {
    if path.split('/').any(|p| p == ".." || p.is_empty()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    match Static::get(&path) {
        Some(f) => {
            let mime = f.metadata.mimetype().to_string();
            ([(header::CONTENT_TYPE, mime)], f.data.into_owned()).into_response()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn status(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(st.status())
}

async fn events(State(st): State<Arc<AppState>>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let (old, rx) = st.hub.subscribe(150);
    let catch_up = stream::iter(old.into_iter().map(|e| Ok(Event::default().data(e.to_string()))));
    let live = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(e) => return Some((Ok(Event::default().data(e.to_string())), rx)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return None,
            }
        }
    });
    Sse::new(catch_up.chain(live)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)).text("keep-alive"))
}

/// A JSON object body; anything else counts as `{}` (like the Python handlers' `.get()`s).
fn body_obj(b: &Bytes) -> Map<String, Value> {
    match serde_json::from_slice::<Value>(b) {
        Ok(Value::Object(m)) => m,
        _ => Map::new(),
    }
}

async fn get_config(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(Value::Object(st.cfg.lock().unwrap().clone()))
}

async fn put_config(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let known = crate::config::defaults();
    let allowed: Map<String, Value> = body_obj(&body).into_iter().filter(|(k, _)| known.contains_key(k)).collect();
    let bad = crate::config::invalid(&allowed);
    if !bad.is_empty() {
        return err(StatusCode::BAD_REQUEST, &bad);
    }
    match st.update_config(allowed) {
        Ok(cfg) => axum::Json(Value::Object(cfg)).into_response(),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// `{"line": "@state"}` -> `{"ok", "via", "reply", "error"}` (firmware_api.py `Stick.run`; Bluetooth
/// only for now, USB serial comes later).
async fn stick_proto(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let line = b.get("line").and_then(Value::as_str).unwrap_or("");
    let line = match proto::check_proto_line(line) {
        Ok(l) => l,
        Err(e) => return axum::Json(json!({"ok": false, "via": null, "reply": null, "error": e})),
    };
    if st.link.connected() {
        if let Some(reply) = st.link.request(line).await {
            let ok = reply.get("ok").and_then(Value::as_bool).unwrap_or(true);
            let error = reply.get("error").cloned().unwrap_or(Value::Null);
            return axum::Json(json!({"ok": ok, "via": "ble", "reply": reply, "error": error}));
        }
        return axum::Json(json!({"ok": false, "via": "ble", "reply": null, "error": "the stick didn't answer"}));
    }
    axum::Json(json!({"ok": false, "via": null, "reply": null,
                      "error": "no stick connected by Bluetooth (USB isn't in the Rust app yet)"}))
}

async fn scan(State(st): State<Arc<AppState>>) -> Response {
    match st.link.scan(Duration::from_secs(5)).await {
        Ok(found) => axum::Json(json!(found)).into_response(),
        Err(e) => err(StatusCode::SERVICE_UNAVAILABLE, &e.to_string()),
    }
}

async fn pause(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let paused = body_obj(&body).get("paused").and_then(Value::as_bool).unwrap_or(false);
    st.link.pause(paused);
    axum::Json(json!({"ok": true}))
}

/// Sends a sound message and waits up to 3 s for the stick's report (core.py `sound`).
async fn sound(st: &AppState, payload: Value) -> Option<Value> {
    let mut rx = st.link.messages();
    if !st.link.send(&proto::sound(payload)).await {
        return None;
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            match rx.recv().await {
                Ok(m) => {
                    if let Some(s) = proto::sounds_report(&m) {
                        return Some(s.clone());
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => return None,
            }
        }
    })
    .await
    .ok()
    .flatten()
}

async fn get_sounds(State(st): State<Arc<AppState>>) -> Response {
    match sound(&st, json!({"get": 1})).await {
        Some(s) => {
            st.hub.emit("sounds", json!({"sounds": s}));
            axum::Json(s).into_response()
        }
        None => err(StatusCode::SERVICE_UNAVAILABLE, "the stick didn't answer (connected?)"),
    }
}

async fn put_sounds(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let payload: Map<String, Value> =
        body_obj(&body).into_iter().filter(|(k, _)| matches!(k.as_str(), "set" | "on" | "volume")).collect();
    match sound(&st, Value::Object(payload)).await {
        Some(s) => {
            st.hub.emit("sounds", json!({"sounds": s}));
            axum::Json(s).into_response()
        }
        None => err(StatusCode::SERVICE_UNAVAILABLE, "the stick didn't answer"),
    }
}

async fn play_sound(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let name = body_obj(&body).get("name").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
    let ok = sound(&st, json!({"play": name.unwrap_or_else(|| "test".into())})).await.is_some();
    axum::Json(json!({"ok": ok}))
}

/// What this app is and what it can do (for the README and for checking a build).
async fn rust_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(json!({
        "app": "stickybot-rs", "version": env!("CARGO_PKG_VERSION"), "port": st.port, "fake_stick": st.fake,
        "home": st.paths.home, "ported": PORTED,
        "not_ported": NOT_PORTED.iter().map(|(e, why)| json!({"endpoint": e, "what": why})).collect::<Vec<_>>(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Options;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use stick_link::{Backend, LinkState, fake::FakeStick};
    use tower::ServiceExt;

    const PORT: u16 = 8670;

    async fn app(name: &str, connect: bool) -> (Router, Arc<AppState>, FakeStick) {
        let home = std::env::temp_dir().join(format!("stickybot-rs-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let opts = Options { port: PORT, home, fake_stick: true };
        let fake = FakeStick::new();
        let st = AppState::new(&opts, Backend::Fake(fake.clone())).unwrap();
        st.link.spawn();
        if connect {
            st.update_config(json!({"stick": {"address": "FAKE", "name": "fake"}}).as_object().unwrap().clone())
                .unwrap();
            let mut w = st.link.watch_state();
            tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected))
                .await
                .unwrap()
                .unwrap();
        }
        (router(st.clone()), st, fake)
    }

    fn req(method: &str, path: &str, body: Option<Value>, token: Option<&str>) -> Request {
        let mut b = Request::builder().method(method).uri(path).header("host", format!("localhost:{PORT}"));
        if let Some(t) = token {
            b = b.header("x-stick-token", t);
        }
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_default()).unwrap()
    }

    async fn call(app: &Router, r: Request) -> (StatusCode, Value, String) {
        let resp = app.clone().oneshot(r).await.unwrap();
        let status = resp.status();
        let ctype = resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let v =
            serde_json::from_slice(&bytes).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into()));
        (status, v, ctype)
    }

    #[tokio::test]
    async fn other_hosts_and_missing_tokens_are_refused() {
        let (app, st, _) = app("guard", false).await;
        let mut r = req("GET", "/api/status", None, None);
        r.headers_mut().insert("host", HeaderValue::from_static("evil.example:8670"));
        assert_eq!(call(&app, r).await.0, StatusCode::FORBIDDEN);
        let (s, v, _) = call(&app, req("PUT", "/api/config", Some(json!({"model": "opus"})), None)).await;
        assert_eq!((s, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("bad token")));
        let (s, _, _) = call(&app, req("PUT", "/api/config", Some(json!({"model": "opus"})), Some("nope"))).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, _, _) = call(&app, req("PUT", "/api/config", Some(json!({"model": "opus"})), Some(&st.token))).await;
        assert_eq!(s, StatusCode::OK);
        let mut r = req("GET", "/api/status", None, None);
        r.headers_mut().insert("host", HeaderValue::from_static("127.0.0.1:8670"));
        assert_eq!(call(&app, r).await.0, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_page_has_the_token_and_the_static_files_are_served() {
        let (app, st, _) = app("page", false).await;
        let (s, v, ctype) = call(&app, req("GET", "/", None, None)).await;
        let html = v.as_str().unwrap();
        assert_eq!(s, StatusCode::OK);
        assert!(ctype.starts_with("text/html"));
        assert!(html.contains(&format!("window.STICK_TOKEN = \"{}\"", st.token)) && !html.contains("{{TOKEN}}"));
        let resp = app.clone().oneshot(req("GET", "/static/app.js", None, None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers()["content-type"].to_str().unwrap().contains("javascript"));
        assert_eq!(resp.headers()["cache-control"], "no-cache");
        let (s, _, ctype) = call(&app, req("GET", "/static/faces/classic.png", None, None)).await;
        assert_eq!((s, ctype.as_str()), (StatusCode::OK, "image/png"));
        assert_eq!(call(&app, req("GET", "/static/../dashboard.html", None, None)).await.0, StatusCode::NOT_FOUND);
        assert_eq!(call(&app, req("GET", "/static/nope.js", None, None)).await.0, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stick_settings_go_over_the_link() {
        let (app, st, fake) = app("proto", true).await;
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["via"], "ble");
        assert_eq!(v["reply"]["board"], "m5stick-s3");
        let (_, v, _) =
            call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@brightness pct=70"})), tok)).await;
        assert_eq!(v["ok"], true);
        assert_eq!(fake.stick_state()["brightness"], 70);
        let (_, v, _) =
            call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@brightness pct=1"})), tok)).await;
        assert_eq!((v["ok"].as_bool(), v["error"].as_str()), (Some(false), Some("pct must be 10-100")));
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "state"})), tok)).await;
        assert_eq!(v["error"], "expected a line like @state");
        fake.set_silent(true);
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(v["error"], "the stick didn't answer");
    }

    #[tokio::test]
    async fn without_a_stick_proto_says_so() {
        let (app, st, _) = app("nostick", false).await;
        let (_, v, _) =
            call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), Some(&st.token))).await;
        assert_eq!(v["ok"], false);
        assert!(v["error"].as_str().unwrap().contains("no stick connected"));
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::NoStick)).await.unwrap().unwrap();
        let (_, v, _) = call(&app, req("GET", "/api/status", None, None)).await;
        assert_eq!(v["link"], "no-stick");
        assert_eq!(v["stick"], Value::Null);
    }

    #[tokio::test]
    async fn settings_are_saved_like_the_python_api() {
        let (app, st, _) = app("config", false).await;
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("PUT", "/api/config", Some(json!({"model": "opus", "nonsense": 1})), tok)).await;
        assert_eq!(v["model"], "opus");
        assert!(v.get("nonsense").is_none());
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(st.paths.config()).unwrap()).unwrap();
        assert_eq!(saved["model"], "opus");
        let (s, v, _) = call(&app, req("PUT", "/api/config", Some(json!({"brain": "gpt"})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("brain"));
        let (_, v, _) = call(&app, req("GET", "/api/config", None, None)).await;
        assert_eq!(v["model"], "opus");
        assert_eq!(v["update_channel"], "stable");
        // picking a stick starts the link
        call(&app, req("PUT", "/api/config", Some(json!({"stick": {"address": "FAKE", "name": "f"}})), tok)).await;
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Connected))
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn sounds_round_trip() {
        let (app, st, _) = app("sounds", true).await;
        let tok = Some(st.token.as_str());
        let (s, v, _) = call(&app, req("GET", "/api/sounds", None, None)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["volume"], 160);
        let (_, v, _) = call(&app, req("PUT", "/api/sounds", Some(json!({"volume": 99, "bogus": 1})), tok)).await;
        assert_eq!(v["volume"], 99);
        let (_, v, _) = call(&app, req("POST", "/api/sounds/play", Some(json!({"name": "test"})), tok)).await;
        assert_eq!(v["ok"], true);
    }

    #[tokio::test]
    async fn pause_scan_status_and_unported() {
        let (app, st, _) = app("misc", true).await;
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("GET", "/api/status", None, None)).await;
        assert_eq!((v["state"].as_str(), v["link"].as_str()), (Some("ready"), Some("connected")));
        assert_eq!(v["mtu"], 247);
        let (_, v, _) = call(&app, req("GET", "/api/scan", None, None)).await;
        assert_eq!(v[0]["rssi"], -42);
        call(&app, req("POST", "/api/pause", Some(json!({"paused": true})), tok)).await;
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s == LinkState::Paused)).await.unwrap().unwrap();
        let (s, v, _) = call(&app, req("GET", "/api/pet", None, None)).await;
        assert_eq!((s, v["missing"].as_bool()), (StatusCode::NOT_FOUND, Some(true)));
        let (s, _, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v, _) = call(&app, req("GET", "/api/rust", None, None)).await;
        assert!(v["ported"].as_array().unwrap().iter().any(|e| e == "POST /api/stick/proto"));
    }
}
