//! The HTTP API (server.py + firmware_api.py), first slice: what the Stick page and the page shell
//! need. Same rules as server.py: only `Host: localhost|127.0.0.1:<port>`, and anything that isn't
//! a GET needs the token in `X-Stick-Token`.

use crate::AppState;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Path, Query, Request, State};
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
    "GET /api/history",
    "POST /api/ask",
    "GET /api/commands",
    "PUT /api/commands",
    "POST /api/commands/test",
    "GET /api/speech",
    "POST /api/speech/retry",
    "GET /api/clips",
    "POST /api/clips/play",
    "POST /api/permission",
    "POST /api/permission/decide",
    "POST /api/new-chat",
    "GET /api/firmware",
    "POST /api/firmware/ota",
    "GET /api/firmware/jobs/{id}",
    "POST /api/firmware/jobs/{id}/cancel",
    "POST /api/wireless-mic/show",
    "POST /api/wireless-mic/check",
    "GET /api/import",
    "POST /api/import",
    "GET /static-rs/*",
    "POST /mcp",
    "GET /api/mcp",
    "POST /api/mcp/terminal/{action}",
    "POST /api/mcp/answer",
    "GET /api/voices",
    "POST /api/voice/test",
    "GET /api/pet",
    "POST /api/events/test",
    "GET /api/events/history",
    "GET /api/events/kinds",
    "POST /api/claude-event",
];

/// The Python agent's endpoints that aren't here yet, by the phase that brings them.
pub const NOT_PORTED: &[(&str, &str)] = &[
    ("POST /api/firmware/flash", "firmware over USB (esptool)"),
    ("GET /api/onboarding", "setup wizard progress (the wizard keeps it in the browser meanwhile)"),
    ("POST /api/onboarding", "setup wizard progress"),
    ("GET /api/install", "setup page (#install)"),
    ("/api/install/*", "setup page steps and jobs"),
    ("GET /api/logs", "log viewer (Logs)"),
    ("GET /api/brain", "brains (Claude Code / OpenRouter)"),
    ("/api/brain/*", "brains (OpenRouter key, models, test)"),
    ("GET /api/update", "update check (releases)"),
    ("/api/update/*", "updates (agent / firmware)"),
    ("GET /api/components", "installer components"),
    ("/api/components/*", "installer components"),
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
        .route("/api/history", get(history))
        .route("/api/ask", post(ask))
        .route("/api/commands", get(get_commands).put(put_commands))
        .route("/api/commands/test", post(test_command))
        .route("/api/speech", get(speech_status))
        .route("/api/speech/retry", post(speech_retry))
        .route("/api/new-chat", post(new_chat))
        .route("/api/permission", post(permission))
        .route("/api/permission/decide", post(permission_decide))
        .route("/api/clips", get(clip_list))
        .route("/api/clips/play", post(clip_play))
        .route("/api/firmware", get(firmware_info))
        .route("/api/firmware/ota", post(ota_update))
        .route("/api/firmware/jobs/{id}", get(job_events))
        .route("/api/firmware/jobs/{id}/cancel", post(job_cancel))
        .route("/api/wireless-mic/show", post(wireless_mic_show))
        .route("/api/wireless-mic/check", post(wireless_mic_check))
        .route("/api/import", get(import_info).post(import_run))
        .route("/mcp", post(mcp_post).get(mcp_405).delete(mcp_405))
        .route("/api/mcp", get(mcp_status))
        .route("/api/mcp/terminal/{action}", post(mcp_terminal))
        .route("/api/mcp/answer", post(mcp_answer))
        .route("/api/voices", get(voices))
        .route("/api/voice/test", post(voice_test))
        .route("/static-rs/{*path}", get(rust_static))
        .route("/api/pet", get(pet_status))
        .route("/api/events/test", post(event_test))
        .route("/api/events/history", get(event_history))
        .route("/api/events/kinds", get(event_kinds))
        .route("/api/claude-event", post(claude_event))
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
    let mcp = req.uri().path() == "/mcp"; // every method; MCP clients send the token as a bearer token
    if mcp || (req.method() != Method::GET && req.method() != Method::HEAD) {
        let h = |k: &str| req.headers().get(k).map(|v| v.as_bytes()).unwrap_or(b"");
        let bearer = h("authorization").strip_prefix(b"Bearer ").unwrap_or(b"");
        let ok = same(h("x-stick-token"), st.token.as_bytes()) || (mcp && same(bearer, st.token.as_bytes()));
        if !ok {
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
    Html(
        PAGE.replace("{{TOKEN}}", &st.token)
            .replace("<title>StickyBot</title>", "<title>StickyBot (Rust)</title>")
            .replace("</body>", "<script type=\"module\" src=\"/static-rs/rust.js\"></script>\n</body>"),
    )
}

/// This app's own additions to the shared page (`assets/`): e.g. Settings → Import from StickyBot.
const RUST_ASSETS: &[(&str, &str, &str)] = &[("rust.js", "text/javascript", include_str!("../assets/rust.js"))];

async fn rust_static(Path(path): Path<String>) -> Response {
    match RUST_ASSETS.iter().find(|(p, _, _)| *p == path) {
        Some((_, mime, body)) => ([(header::CONTENT_TYPE, *mime)], *body).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn mcp_post(State(st): State<Arc<AppState>>, headers: axum::http::HeaderMap, body: Bytes) -> Response {
    crate::mcp::post(st, headers, &body).await
}

async fn mcp_405() -> Response {
    (StatusCode::METHOD_NOT_ALLOWED, [(header::ALLOW, "POST")]).into_response()
}

/// The server switch, recent use, and whether terminal Claude Code sessions have the tools.
async fn mcp_status(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    let claude = crate::brain::claude_path(&cfg);
    let run = st.claude_runner.lock().unwrap().clone();
    let (port, token) = (st.port, st.token.clone());
    let terminal =
        tokio::task::spawn_blocking(move || crate::mcp::terminal_status(claude.as_deref(), port, &token, &run))
            .await
            .unwrap_or(Value::Null);
    let tools: Vec<Value> = crate::mcp::tools()
        .iter()
        .map(|t| {
            json!({"name": t["name"], "title": t["title"],
                   "approval": crate::mcp::MAC_TOOLS.contains(&t["name"].as_str().unwrap_or(""))})
        })
        .collect();
    axum::Json(json!({"enabled": cfg.get("mcp_server").and_then(Value::as_bool).unwrap_or(true),
                      "url": crate::mcp::url(st.port), "terminal": terminal,
                      "calls": *st.mcp.calls.lock().unwrap(), "last_call": st.mcp.last_call.lock().unwrap().clone(),
                      "tools": tools}))
}

/// `claude mcp add|remove` for terminal sessions; the page shows the command first, the request
/// must say `{"confirm": true}`.
async fn mcp_terminal(State(st): State<Arc<AppState>>, Path(action): Path<String>, body: Bytes) -> Response {
    if action != "install" && action != "uninstall" {
        return err(StatusCode::NOT_FOUND, "install or uninstall");
    }
    if body_obj(&body).get("confirm") != Some(&json!(true)) {
        return err(StatusCode::BAD_REQUEST, "needs {\"confirm\": true}");
    }
    let claude = crate::brain::claude_path(&st.cfg.lock().unwrap());
    let run = st.claude_runner.lock().unwrap().clone();
    let (port, token) = (st.port, st.token.clone());
    let (ok, msg, ran) =
        tokio::task::spawn_blocking(move || crate::mcp::terminal(&action, claude.as_deref(), port, &token, &run))
            .await
            .unwrap_or((false, "failed".into(), Vec::new()));
    log::info!("mcp for terminal sessions: {msg}");
    let status = if ok { StatusCode::OK } else { StatusCode::INTERNAL_SERVER_ERROR };
    (status, axum::Json(json!({"ok": ok, "message": msg, "ran": ran}))).into_response()
}

/// A question from a Claude session answered on the dashboard: `{"id", "choice": 0 | 1 | null}`.
async fn mcp_answer(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    let id = b.get("id").and_then(Value::as_str).unwrap_or("");
    let decision = match b.get("choice").and_then(Value::as_i64) {
        Some(0) => "once",
        Some(1) => "deny",
        _ => "dismiss",
    };
    let ok = st.approvals.answer(id, decision, "dashboard");
    (if ok { StatusCode::OK } else { StatusCode::NOT_FOUND }, axum::Json(json!({"ok": ok}))).into_response()
}

/// Installed macOS voices `[{name, lang}]` (`say -v ?`).
async fn voices() -> axum::Json<Value> {
    let out = tokio::process::Command::new("say").args(["-v", "?"]).output().await;
    let text = out.map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    axum::Json(Value::Array(crate::speak::parse_voices(&text)))
}

async fn voice_test(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let text = b.get("text").and_then(Value::as_str).filter(|t| !t.is_empty());
    crate::speak::speak(&st, text.unwrap_or("It's 28 degrees in Bangalore with light rain."), false, true);
    axum::Json(json!({"ok": true}))
}

/// Is there a Python app to import from, and the last import.
async fn import_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let from = st.import_from.clone();
    let available = from.as_ref().is_some_and(|f| f.join("config.json").is_file() || f.join("commands.json").is_file());
    let last = std::fs::read_to_string(st.paths.home.join(crate::migrate::MARKER))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok());
    axum::Json(json!({"available": available, "from": from, "last": last}))
}

/// Settings → Import from StickyBot: the Python app's settings, commands, sounds, history (read
/// only there; this app's replaced files are kept as `*.before-import`).
async fn import_run(State(st): State<Arc<AppState>>) -> Response {
    let st2 = st.clone();
    match tokio::task::spawn_blocking(move || st2.import_python(crate::migrate::Mode::Explicit)).await {
        Ok(Ok(r)) => axum::Json(r.to_json()).into_response(),
        Ok(Err(e)) => err(StatusCode::NOT_FOUND, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
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

#[derive(serde::Deserialize)]
struct HistoryQuery {
    n: Option<usize>,
}

/// The faces' pet (level, streak, city…) and the world the stick gets.
async fn pet_status(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let pet = st.home.pet.lock().unwrap().summary();
    axum::Json(json!({"pet": pet, "world": crate::home::world(&st)}))
}

/// The Events page's Test button: `{"kind"}` -> the kind's sample on the stick.
async fn event_test(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let kind = b.get("kind").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
    let ev = crate::events::test(&st, &kind.unwrap_or_else(|| "test".into())).await;
    axum::Json(json!({"ok": true, "event": ev}))
}

async fn event_history(State(st): State<Arc<AppState>>, Query(q): Query<HistoryQuery>) -> axum::Json<Value> {
    axum::Json(Value::Array(crate::events::history(&st, q.n.unwrap_or(50))))
}

async fn event_kinds(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    axum::Json(crate::events::kinds_json(&cfg))
}

/// The Claude Code Stop hook (`claude_hooks.py`): `{"elapsed_s", "cwd", "prompt"}`.
async fn claude_event(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let elapsed = b.get("elapsed_s").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0);
    let s = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let ev = crate::home::claude_finished(&st, elapsed, &s("cwd"), &s("prompt")).await;
    axum::Json(json!({"ok": true, "shown": ev.is_some()}))
}

/// The last `n` (50) requests, newest first (server.py `history`).
async fn history(State(st): State<Arc<AppState>>, Query(q): Query<HistoryQuery>) -> axum::Json<Value> {
    let text = std::fs::read_to_string(st.paths.history()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let n = q.n.unwrap_or(50);
    let out: Vec<Value> =
        lines[lines.len().saturating_sub(n)..].iter().rev().filter_map(|l| serde_json::from_str(l).ok()).collect();
    axum::Json(Value::Array(out))
}

/// A typed request (`{"text"}`), handled like a spoken one.
async fn ask(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let text = body_obj(&body).get("text").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if text.is_empty() {
        return err(StatusCode::BAD_REQUEST, "empty");
    }
    tokio::spawn(crate::talk::handle_text(st.clone(), text, "typed"));
    axum::Json(json!({"ok": true})).into_response()
}

async fn get_commands(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(st.commands.read())
}

async fn put_commands(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    match st.commands.save(&body_obj(&body)) {
        Ok(()) => axum::Json(json!({"ok": true})).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, &e),
    }
}

async fn test_command(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let text = body_obj(&body).get("text").and_then(Value::as_str).unwrap_or("").to_string();
    let hit = st.commands.find(&text).map(|(c, slots)| json!({"name": c.get("name"), "slots": slots}));
    axum::Json(json!({"normalized": crate::commands::normalize(&text), "match": hit.unwrap_or(Value::Null)}))
}

/// The speech model: `{"phase": "downloading", "pct", …}` / `ready` / `failed`, the hints, where.
async fn speech_status(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let mut v = st.speech.phase().to_json();
    if let Value::Object(m) = &mut v {
        m.insert("model".into(), json!(crate::speech::MODEL_NAME));
        m.insert("dir".into(), json!(st.paths.models().join(crate::speech::MODEL_NAME)));
        m.insert("hints".into(), json!(st.stt_hints()));
    }
    axum::Json(v)
}

/// Tries the download / load again after a failure.
async fn speech_retry(State(st): State<Arc<AppState>>) -> Response {
    match st.speech.phase() {
        crate::speech::Phase::Failed(_) | crate::speech::Phase::Idle => {
            st.spawn_speech();
            axum::Json(json!({"ok": true})).into_response()
        }
        _ => err(StatusCode::CONFLICT, "the speech model is already loading or ready"),
    }
}

/// The next request starts a new conversation.
/// The Bluetooth mic device ("<stick>-bt-mic") for a minute, so it can be ranked in Wispr's settings.
/// `{"secs"}` (5-300, 60) -> `{"ok", "error", "secs", "name"}`, 409 when it can't be now.
async fn wireless_mic_show(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let secs = body_obj(&body).get("secs").and_then(Value::as_u64).unwrap_or(crate::wmic::SHOW_S).clamp(5, 300);
    let why = crate::wmic::show_mic(&st, secs).await;
    let name = st.wmic.device().name;
    let code = if why.is_empty() { StatusCode::OK } else { StatusCode::CONFLICT };
    let error = if why.is_empty() { Value::Null } else { json!(why) };
    (code, axum::Json(json!({"ok": why.is_empty(), "error": error, "secs": secs, "name": name}))).into_response()
}

/// Runs the Apple audio tap check now (inaudible; the first one makes macOS ask for the System
/// Audio Recording permission). -> `{"ok", "state", "detail", "wireless_mic"}`.
async fn wireless_mic_check(State(st): State<Arc<AppState>>) -> Response {
    if !crate::wmic::status(&st)["tap"]["supported"].as_bool().unwrap_or(false) {
        let e = json!({"ok": false, "error": "the Apple audio tap needs macOS 14.2 or newer"});
        return (StatusCode::CONFLICT, axum::Json(e)).into_response();
    }
    match crate::wmic::check_tap(&st, true).await {
        None => {
            let e = json!({"ok": false, "error": "not now: the mic is in use (try again in a moment)"});
            (StatusCode::CONFLICT, axum::Json(e)).into_response()
        }
        Some(res) => axum::Json(json!({
            "ok": res.state == Some(macos_audio::carrier::CheckState::Ok),
            "state": res.state, "detail": res.detail, "wireless_mic": crate::wmic::status(&st),
        }))
        .into_response(),
    }
}

async fn new_chat(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    st.brain().forget();
    st.hub.emit("info", json!({"text": "Next request starts a new conversation"}));
    axum::Json(json!({"ok": true}))
}

/// Long-poll from the Claude Code hook: waits for A / B on the stick or the dashboard.
/// `{"tool","hint","from","wait"}` -> `{"decision": "allow"|"deny"|null}`.
async fn permission(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let s = |k: &str, d: &str| b.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let wait = b.get("wait").and_then(Value::as_f64).unwrap_or(30.0).clamp(0.0, 120.0);
    let d = crate::approvals::ask_permission(
        &st,
        &s("tool", "?"),
        &s("hint", ""),
        &s("from", "terminal"),
        Duration::from_secs_f64(wait),
    )
    .await;
    axum::Json(json!({"decision": d}))
}

async fn permission_decide(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let id = b.get("id").and_then(Value::as_str).unwrap_or("");
    let allow = b.get("allow").and_then(Value::as_bool).unwrap_or(false);
    st.approvals.decide(id, if allow { "allow" } else { "deny" }, "dashboard");
    axum::Json(json!({"ok": true}))
}

async fn clip_list(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(Value::Array(crate::clips::library(&crate::clips::sounds_dir(&st.paths.home))))
}

/// Plays a sound-pack clip on the stick's speaker (core.py `play_clip`).
async fn clip_play(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let s =
        |k: &str| b.get(k).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let (pack, name) = (s("pack"), s("name"));
    let Some(f) = crate::clips::find(&crate::clips::sounds_dir(&st.paths.home), &pack, &name) else {
        return axum::Json(json!({"ok": false, "error": "no such sound"}));
    };
    let pcm = match tokio::task::spawn_blocking(move || crate::clips::load(&f, false)).await {
        Ok(Ok(p)) => p,
        Ok(Err(e)) => return axum::Json(json!({"ok": false, "error": e})),
        Err(e) => return axum::Json(json!({"ok": false, "error": e.to_string()})),
    };
    let t = std::time::Instant::now();
    let pkts = crate::clips::packets(&pcm);
    let times = st.link.send_audio_timed(&pkts).await;
    let ok = times.is_some();
    log::info!(
        "clip {pack}/{name}: {:.2}s of audio sent in {:.2}s ({} packets; {})",
        pcm.len() as f64 / crate::clips::RATE as f64,
        t.elapsed().as_secs_f64(),
        pkts.len(),
        times.map(|t| t.to_string()).unwrap_or_else(|| "failed".into())
    );
    axum::Json(json!({"ok": ok, "error": if ok { Value::Null } else { json!("stick not connected") }}))
}

/// The bundled firmware, sticks on USB (not in the Rust app: always empty), the connected stick.
async fn firmware_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let bundled = st.firmware().first().map(crate::firmware::describe).unwrap_or(Value::Null);
    axum::Json(json!({"bundled": bundled, "usb": [], "connected": crate::firmware::connected(&st.link).await}))
}

/// Starts a firmware update over Bluetooth: `{"force"}` -> `{"job"}`.
async fn ota_update(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let force = body_obj(&body).get("force").and_then(Value::as_bool).unwrap_or(false);
    let t = st.ota_timings.lock().unwrap().clone();
    match crate::firmware::start_ota(&st, force, t) {
        Ok(job) => axum::Json(json!({"job": job})).into_response(),
        Err((code, e)) => err(StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST), &e),
    }
}

/// A job's events as server-sent events, until it's done.
async fn job_events(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some(job) = st.jobs.get(&id) else {
        return err(StatusCode::NOT_FOUND, "no such job");
    };
    let s = stream::unfold((job, 0usize, false), |(job, i, finished)| async move {
        if finished {
            return None;
        }
        loop {
            let watch = job.clone();
            let changed = watch.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            let evs = job.events_from(i);
            if !evs.is_empty() {
                let n = i + evs.len();
                let done = job.is_done() && job.events_from(n).is_empty();
                let data: String = evs.iter().map(|e| format!("data: {e}\n\n")).collect();
                return Some((Ok::<_, Infallible>(Bytes::from(data)), (job, n, done)));
            }
            if job.is_done() {
                return None;
            }
            if tokio::time::timeout(Duration::from_secs(15), changed).await.is_err() {
                return Some((Ok(Bytes::from_static(b": keep-alive\n\n")), (job, i, false)));
            }
        }
    });
    (
        [(header::CONTENT_TYPE, "text/event-stream"), (header::CACHE_CONTROL, "no-cache")],
        axum::body::Body::from_stream(s),
    )
        .into_response()
}

async fn job_cancel(State(st): State<Arc<AppState>>, Path(id): Path<String>) -> Response {
    let Some(job) = st.jobs.get(&id) else {
        return err(StatusCode::NOT_FOUND, "no such job");
    };
    if job.cancel() {
        axum::Json(json!({"ok": true})).into_response()
    } else {
        err(StatusCode::CONFLICT, "nothing to cancel")
    }
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
        let opts = Options { port: PORT, home, fake_stick: true, import_from: None };
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
        let (s, v, _) = call(&app, req("GET", "/api/onboarding", None, None)).await;
        assert_eq!((s, v["missing"].as_bool()), (StatusCode::NOT_FOUND, Some(true)));
        let (s, _, _) = call(&app, req("POST", "/api/firmware/flash", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (_, v, _) = call(&app, req("GET", "/api/rust", None, None)).await;
        assert!(v["ported"].as_array().unwrap().iter().any(|e| e == "POST /api/stick/proto"));
    }

    // --- firmware over Bluetooth (test_ota.py's API part) -------------------------------------------

    fn bundle(st: &AppState, version: &str, n: usize) {
        let dir = st.paths.home.join("firmware/local");
        std::fs::create_dir_all(&dir).unwrap();
        let mut app: Vec<u8> = (0..n).map(|i| (i * 13 % 251) as u8).collect();
        app[0] = 0xE9;
        std::fs::write(dir.join("app.bin"), &app).unwrap();
        let m = json!({"version": version, "parts": [{"file": "app.bin", "offset": "0x10000", "size": n}]});
        std::fs::write(dir.join("manifest.json"), m.to_string()).unwrap();
    }

    fn quick_ota(st: &AppState) {
        *st.ota_timings.lock().unwrap() = crate::firmware::UpdateTimings {
            transfer: stick_link::ota::Timings {
                ack_wait: Duration::from_millis(50),
                stall: Duration::from_secs(2),
                reconnect: Duration::from_secs(10),
                reply: Duration::from_millis(500),
                end_reply: Duration::from_millis(500),
                gap_debounce: Duration::from_millis(500),
            },
            reboot: Duration::from_secs(20),
            poll: Duration::from_millis(200),
        };
    }

    /// The job's SSE stream, read to its end (it ends when the job is done).
    async fn job_stream(app: &Router, id: &str) -> Vec<Value> {
        let (_, body, ctype) = call(app, req("GET", &format!("/api/firmware/jobs/{id}"), None, None)).await;
        assert_eq!(ctype, "text/event-stream");
        body.as_str()
            .unwrap()
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    #[tokio::test]
    async fn ota_needs_the_token_the_link_and_a_bundle() {
        let (app, st, _) = app("ota-checks", false).await;
        let tok = Some(st.token.as_str());
        let (s, _, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), None)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, v, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert!(v["error"].as_str().unwrap().contains("Bluetooth"));
        let (app2, st, _) = super::tests::app("ota-nobundle", true).await;
        let (s, v, _) = call(&app2, req("POST", "/api/firmware/ota", Some(json!({})), Some(st.token.as_str()))).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("no firmware"));
        let (_, v, _) = call(&app2, req("GET", "/api/firmware", None, None)).await;
        assert_eq!(v["bundled"], Value::Null);
        assert_eq!(v["connected"], json!({"name": "StickyBot (fake)", "version": "2.3.0", "ota": true}));
        let (s, _, _) = call(&app2, req("GET", "/api/firmware/jobs/nope", None, None)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
    }

    #[tokio::test(start_paused = true)]
    async fn ota_runs_a_job_streams_it_and_waits_for_the_new_version() {
        let (app, st, fake) = app("ota-run", true).await;
        bundle(&st, "2.4.0", 30_000);
        quick_ota(&st);
        let (_, v, _) = call(&app, req("GET", "/api/firmware", None, None)).await;
        assert_eq!((v["bundled"]["version"].as_str(), v["bundled"]["source"].as_str()), (Some("2.4.0"), Some("local")));
        let (s, v, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), Some(st.token.as_str()))).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let events = job_stream(&app, v["job"].as_str().unwrap()).await;
        let last = events.last().unwrap();
        assert_eq!((last["done"].as_bool(), last["ok"].as_bool()), (Some(true), Some(true)), "{events:?}");
        let r = &last["result"];
        assert_eq!(
            (r["from"].as_str(), r["to"].as_str(), r["version"].as_str()),
            (Some("2.3.0"), Some("2.4.0"), Some("2.4.0"))
        );
        assert_eq!(r["confirmed"], true);
        assert!(events.iter().any(|e| e["stage"] == "reboot" && e["progress"] == 100));
        assert!(events.iter().any(|e| e["line"].as_str().is_some_and(|l| l.contains("confirmed"))));
        assert!(fake.ota(|o| o.committed));
        assert_eq!(fake.stick_state()["fw"], "2.4.0");
        // the same version again needs force
        let (s, v, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), Some(st.token.as_str()))).await;
        assert_eq!(s, StatusCode::OK);
        let events = job_stream(&app, v["job"].as_str().unwrap()).await;
        assert!(events.last().unwrap()["error"].as_str().unwrap().contains("force"), "{events:?}");
    }

    #[tokio::test(start_paused = true)]
    async fn ota_reports_a_rollback() {
        let (app, st, fake) = app("ota-rollback", true).await;
        bundle(&st, "2.4.0", 8_000);
        quick_ota(&st);
        fake.ota(|o| o.rollback = true);
        let (_, v, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), Some(st.token.as_str()))).await;
        let events = job_stream(&app, v["job"].as_str().unwrap()).await;
        let last = events.last().unwrap();
        assert_eq!(last["ok"], false);
        assert!(last["error"].as_str().unwrap().contains("rolled back"), "{last}");
    }

    #[tokio::test(start_paused = true)]
    async fn ota_cancel_reaches_the_transfer() {
        let (app, st, fake) = app("ota-cancel", true).await;
        bundle(&st, "2.4.0", 400_000);
        quick_ota(&st);
        fake.ota(|o| o.window = 1024);
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), tok)).await;
        let id = v["job"].as_str().unwrap().to_string();
        let (s, _, _) = call(&app, req("POST", "/api/firmware/ota", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT); // one at a time
        for _ in 0..200 {
            if fake.ota(|o| o.received) > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        let (s, _, _) = call(&app, req("POST", &format!("/api/firmware/jobs/{id}/cancel"), Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        let events = job_stream(&app, &id).await;
        let last = events.last().unwrap();
        assert_eq!((last["ok"].as_bool(), last["error"].as_str()), (Some(false), Some("cancelled")));
        assert!(!fake.ota(|o| o.committed));
        let (s, _, _) = call(&app, req("POST", &format!("/api/firmware/jobs/{id}/cancel"), Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn clips_are_listed_and_played() {
        let (app, st, fake) = app("clips", true).await;
        let pack = crate::clips::sounds_dir(&st.paths.home).join("test-pack");
        std::fs::create_dir_all(&pack).unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(pack.join("beep.wav"), spec).unwrap();
        for i in 0..4000 {
            w.write_sample(((i as f64 / 3.0).sin() * 8000.0) as i16).unwrap();
        }
        w.finalize().unwrap();
        let (_, v, _) = call(&app, req("GET", "/api/clips", None, None)).await;
        assert_eq!(v, json!([{"pack": "test-pack", "name": "beep"}]));
        let tok = Some(st.token.as_str());
        let (_, v, _) =
            call(&app, req("POST", "/api/clips/play", Some(json!({"pack": "test-pack", "name": "beep"})), tok)).await;
        assert_eq!(v, json!({"ok": true, "error": null}));
        let clips = fake.clips();
        assert_eq!(clips.len(), 1);
        assert_eq!(u32::from_le_bytes(clips[0][0][1..5].try_into().unwrap()), 4000);
        assert_eq!(clips[0].len(), 1 + 13 + 1); // start, 4000 / 320 frames, end
        let (_, v, _) = call(&app, req("POST", "/api/clips/play", Some(json!({"pack": "..", "name": "x"})), tok)).await;
        assert_eq!(v["error"], "no such sound");
    }

    #[tokio::test]
    async fn the_hook_long_polls_and_the_dashboard_decides() {
        let (app, st, fake) = app("perm", true).await;
        crate::talk::spawn(st.clone());
        let tok = st.token.clone();
        let a = app.clone();
        let t2 = tok.clone();
        let poll = tokio::spawn(async move {
            call(&a, req("POST", "/api/permission", Some(json!({"tool": "Bash", "hint": "ls", "wait": 5})), Some(&t2)))
                .await
        });
        let mut id = String::new();
        for _ in 0..100 {
            if let Some(i) = st.status()["prompts"][0]["id"].as_str() {
                id = i.to_string();
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(String::from_utf8(fake.writes().concat()).unwrap().contains(&id));
        call(&app, req("POST", "/api/permission/decide", Some(json!({"id": id, "allow": true})), Some(&tok))).await;
        let (_, v, _) = poll.await.unwrap();
        assert_eq!(v, json!({"decision": "allow"}));
    }

    #[tokio::test]
    async fn status_with_the_claude_brain_doesnt_deadlock() {
        let (app, st, _) = app("status-brain", false).await;
        let weak = Arc::downgrade(&st);
        let cfg: crate::brain::Config = Arc::new(move || weak.upgrade().unwrap().cfg.lock().unwrap().clone());
        st.set_brain(Arc::new(crate::brain::ClaudeBrain::new(cfg, st.paths.home.join("workspace"))));
        let r = tokio::time::timeout(Duration::from_secs(5), call(&app, req("GET", "/api/status", None, None))).await;
        let (_, v, _) = r.expect("status answers");
        assert_eq!(v["brain"]["mode"], "session");
        assert_eq!(v["busy"], false);
    }

    #[tokio::test]
    async fn quick_commands_history_and_typed_requests() {
        let (app, st, fake) = app("cmds", true).await;
        crate::talk::spawn(st.clone());
        st.set_runner(Arc::new(|_| String::new()));
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("GET", "/api/commands", None, None)).await;
        assert_eq!(v["commands"][0]["name"], "GitHub PR");
        let (s, _, _) = call(&app, req("PUT", "/api/commands", Some(json!({"nope": 1})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let cmds =
            json!({"commands": [{"name": "Docs", "phrases": ["docs {q}"], "open_url": "https://x.test/?q={q}"}]});
        let (s, _, _) = call(&app, req("PUT", "/api/commands", Some(cmds), tok)).await;
        assert_eq!(s, StatusCode::OK);
        let (_, v, _) =
            call(&app, req("POST", "/api/commands/test", Some(json!({"text": "Docs: the pairing!"})), tok)).await;
        assert_eq!(v["normalized"], "docs pairing");
        assert_eq!(v["match"], json!({"name": "Docs", "slots": {"q": "pairing"}}));
        let (s, _, _) = call(&app, req("POST", "/api/ask", Some(json!({"text": " "})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        call(&app, req("POST", "/api/ask", Some(json!({"text": "docs bluetooth"})), tok)).await;
        for _ in 0..200 {
            if fake.screens().iter().any(|(s, _, _)| s == "done") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(fake.screens(), vec![("done".into(), "docs bluetooth".into(), "Opened Docs".into())]);
        let (_, v, _) = call(&app, req("GET", "/api/history?n=5", None, None)).await;
        assert_eq!(v[0]["via"], "quick");
        assert_eq!(v[0]["source"], "typed");
        let (_, v, _) = call(&app, req("GET", "/api/speech", None, None)).await;
        assert_eq!(v["phase"], "idle");
        assert_eq!(v["hints"], json!(["Tauri", "StickyBot", "Claude"]));
    }

    /// The wireless mic with fake audio (BlackHole present; `tap`: this Mac has process taps).
    fn fake_audio(st: &AppState, tap: bool) -> macos_audio::fake::FakeCoreAudio {
        use macos_audio::fake::{FakeCoreAudio, FakeOutputs};
        let ca = FakeCoreAudio::new();
        let out = FakeOutputs::new(vec![FakeOutputs::speakers(), FakeOutputs::blackhole()]);
        st.wmic.use_audio(macos_audio::Audio::fake(&ca, &out, tap));
        ca
    }

    /// test_stick_mic_device.py: the dashboard API
    #[tokio::test]
    async fn wireless_mic_show_from_the_dashboard() {
        let (app, st, fake) = app("wmshow", true).await;
        let ca = fake_audio(&st, false);
        crate::wmic::spawn(st.clone()); // the stick's messages (start() runs it)
        let tok = Some(st.token.as_str());
        let (s, _, _) = call(&app, req("POST", "/api/wireless-mic/show", Some(json!({})), None)).await;
        assert_eq!(s, StatusCode::FORBIDDEN); // needs the token
        let (s, v, _) = call(&app, req("POST", "/api/wireless-mic/show", Some(json!({"secs": 60})), tok)).await;
        assert_eq!((s, v["error"].as_str()), (StatusCode::CONFLICT, Some("the wireless Wispr mic is off")));
        let (s, _, _) = call(&app, req("PUT", "/api/config", Some(json!({"wireless_mic": true})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        let (s, v, _) = call(&app, req("POST", "/api/wireless-mic/show", Some(json!({"secs": 60})), tok)).await;
        assert_eq!((s, v["ok"].as_bool(), v["name"].as_str()), (StatusCode::OK, Some(true), Some("fake-bt-mic")));
        assert_eq!(ca.ours(macos_audio::naming::RUST_UID).len(), 1);
        let (_, v, _) = call(&app, req("GET", "/api/status", None, None)).await;
        let w = &v["wireless_mic"];
        assert!(w["device_mic"]["present"] == true && w["device_mic"]["showing_s"].as_u64().unwrap() > 50);
        assert_eq!((w["unavailable"].as_str(), w["carrier"].as_str()), (Some(""), Some("blackhole")));
        // the stick on USB: refused, and it goes
        fake.notify(&json!({"src": "agent", "evt": "usb", "mounted": true}));
        for _ in 0..200 {
            if ca.ours(macos_audio::naming::RUST_UID).is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(ca.ours(macos_audio::naming::RUST_UID).is_empty());
        let (s, v, _) = call(&app, req("POST", "/api/wireless-mic/show", Some(json!({})), tok)).await;
        assert!(s == StatusCode::CONFLICT && v["error"].as_str().unwrap().contains("USB"));
        let (_, v, _) = call(&app, req("GET", "/api/status", None, None)).await;
        assert!(v["wireless_mic"]["unavailable"].as_str().unwrap().contains("USB"));
        // timings and a bad carrier
        let (s, _, _) =
            call(&app, req("PUT", "/api/config", Some(json!({"wispr_switch_ms": 400, "wispr_tail_ms": 1200})), tok))
                .await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(crate::wmic::settings(&st), (400, 1200));
        let (s, _, _) =
            call(&app, req("PUT", "/api/config", Some(json!({"wireless_mic_carrier": "cable"})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
    }

    /// test_tap_mic.py: the dashboard API
    #[tokio::test]
    async fn wireless_mic_check_from_the_dashboard() {
        let (app, st, _) = app("wmcheck", true).await;
        let tok = Some(st.token.as_str());
        fake_audio(&st, false); // older macOS
        let (s, v, _) = call(&app, req("POST", "/api/wireless-mic/check", Some(json!({})), tok)).await;
        assert!(s == StatusCode::CONFLICT && v["error"].as_str().unwrap().contains("14.2"));
        let ca = fake_audio(&st, true);
        let (s, _, _) = call(&app, req("POST", "/api/wireless-mic/check", Some(json!({})), None)).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        let (s, v, _) = call(&app, req("POST", "/api/wireless-mic/check", Some(json!({})), tok)).await;
        assert_eq!((s, v["ok"].as_bool(), v["state"].as_str()), (StatusCode::OK, Some(true), Some("ok")));
        assert_eq!(v["wireless_mic"]["tap"]["state"], "ok");
        assert_eq!(ca.measured(), 1);
        ca.set(|s| s.tap_hears = false);
        let (_, v, _) = call(&app, req("POST", "/api/wireless-mic/check", Some(json!({})), tok)).await;
        assert_eq!((v["ok"].as_bool(), v["state"].as_str()), (Some(false), Some("failed")));
        assert!(v["detail"].as_str().unwrap().contains("System Audio Recording"));
    }

    #[tokio::test]
    async fn import_from_the_python_app_applies_the_settings() {
        // a fake home: the Python folder is made up here, never the real ~/.stick-agent
        let root = std::env::temp_dir().join(format!("stickybot-rs-api-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let py = root.join(".stick-agent");
        std::fs::create_dir_all(&py).unwrap();
        let opts =
            Options { port: PORT, home: root.join(".stickybot-rs"), fake_stick: true, import_from: Some(py.clone()) };
        let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap();
        st.link.spawn();
        let app = router(st.clone());
        let tok = Some(st.token.as_str());

        let (_, v, _) = call(&app, req("GET", "/api/import", None, None)).await;
        assert_eq!((v["available"].as_bool(), v["last"].clone()), (Some(false), Value::Null));
        let (s, v, _) = call(&app, req("POST", "/api/import", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        assert!(v["error"].as_str().unwrap().contains("no StickyBot settings"));

        std::fs::write(
            py.join("config.json"),
            json!({"stick": {"address": "S1", "name": "Ash"}, "github": {"on": true, "interval_s": 120,
                   "primary_repo": "acme/w"}, "model": "opus"})
            .to_string(),
        )
        .unwrap();
        let (_, v, _) = call(&app, req("GET", "/api/import", None, None)).await;
        assert_eq!(v["available"], true);
        let (s, v, _) = call(&app, req("POST", "/api/import", Some(json!({})), None)).await;
        assert_eq!((s, v["error"].as_str()), (StatusCode::FORBIDDEN, Some("bad token")));
        let (s, v, _) = call(&app, req("POST", "/api/import", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        assert!(v["imported"][0].as_str().unwrap().starts_with("config: 3 settings"), "{v}");
        // applied to the running app: the link follows the imported stick
        assert_eq!(st.cfg.lock().unwrap()["github"]["primary_repo"], "acme/w");
        assert_eq!(st.stick().unwrap().address, "S1");
        let (_, v, _) = call(&app, req("GET", "/api/config", None, None)).await;
        assert_eq!(v["model"], "opus");
        let (_, v, _) = call(&app, req("GET", "/api/import", None, None)).await;
        assert_eq!(v["last"]["mode"], "Explicit");

        // the page loads the Rust-only script that adds the button
        let (_, v, _) = call(&app, req("GET", "/", None, None)).await;
        assert!(v.as_str().unwrap().contains("/static-rs/rust.js"));
        let (s, v, ctype) = call(&app, req("GET", "/static-rs/rust.js", None, None)).await;
        assert_eq!((s, ctype.as_str()), (StatusCode::OK, "text/javascript"));
        assert!(v.as_str().unwrap().contains("/api/import"));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The JSON lines the fake stick got.
    fn stick_lines(fake: &FakeStick) -> Vec<Value> {
        String::from_utf8(fake.writes().concat()).unwrap().lines().filter_map(|l| serde_json::from_str(l).ok()).collect()
    }

    #[tokio::test]
    async fn the_hello_carries_the_home_card_world_and_weather() {
        let (_app, st, fake) = app("hello-home", true).await;
        // the hello on connect: home + world, wmic / wready, like core.py `_hello_extra`
        let h = fake.last_hello().unwrap();
        assert_eq!(h["home"], json!({"k": "hello", "a": h["home"]["a"], "b": "Hold B to ask me anything."}));
        assert!(h["home"]["a"].as_str().unwrap().starts_with("Good "));
        let w = &h["world"];
        assert_eq!((w["merged"].as_i64(), w["ci"].as_str(), w["asks"].as_i64(), w["reviews"].as_i64()), (Some(0), Some("none"), Some(0), Some(0)));
        assert_eq!(w["claude"], json!([]));
        for k in ["xp", "streak", "acc", "seed", "greens", "towers"] {
            assert!(w["pet"].get(k).is_some(), "{k}");
        }
        assert_eq!(h["wready"], true);
        assert!(h.get("wx").is_none());
        // with weather (stick-safe place name)
        st.home.weather.lock().unwrap().now = Some(json!({"t": 18, "i": "cloudy", "c": "cloudy", "day": 1, "hi": 25, "lo": 11, "at": "Zürich"}));
        let x = crate::home::hello_extra(&st);
        assert_eq!(x["wx"]["at"], "Zurich");
        assert_eq!(st.status()["weather"]["t"], 18);
        // an approval waiting pins the card
        let st2 = st.clone();
        let wait = tokio::spawn(async move { crate::approvals::ask_permission(&st2, "Bash", "ls", "terminal", Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let card = crate::home::home_card(&st);
        assert_eq!((card["k"].as_str(), card["u"].as_bool()), (Some("approval"), Some(true)));
        assert_eq!(card["a"], "Claude wants Bash");
        assert_eq!(crate::home::world(&st)["asks"], 1);
        wait.abort();
    }

    #[tokio::test]
    async fn events_pet_and_the_claude_stop_hook() {
        let (app, st, fake) = app("events", true).await;
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("GET", "/api/events/kinds", None, None)).await;
        assert_eq!(v["kinds"]["pr_merged"], json!({"default": "celebrate", "icon": "merge", "title": "PR #5507 merged!"}));
        assert_eq!(v["levels"], json!(["celebrate", "nice", "headsup", "off"]));
        assert_eq!(v["rules"]["ci_failed"], "headsup");

        let (_, v, _) = call(&app, req("POST", "/api/events/test", Some(json!({"kind": "pr_merged"})), tok)).await;
        let ev = &v["event"];
        assert_eq!((ev["id"].as_str(), ev["level"].as_str(), ev["icon"].as_str()), (Some("e1"), Some("celebrate"), Some("merge")));
        let sent: Vec<Value> = stick_lines(&fake).into_iter().filter(|l| l.get("event").is_some()).collect();
        assert_eq!(
            sent,
            vec![json!({"src": "agent", "event": {"id": "e1", "level": "celebrate", "title": "PR #5507 merged!",
                        "sub": "web - Add stick agent", "icon": "merge", "sound": "", "kind": "pr_merged"}})]
        );
        let (_, h, _) = call(&app, req("GET", "/api/events/history?n=5", None, None)).await;
        assert_eq!((h[0]["kind"].as_str(), h[0]["sent"].as_bool(), h[0]["quiet"].as_bool()), (Some("pr_merged"), Some(true), Some(false)));
        // the stick says it showed it (the loops: offline in fake mode)
        crate::home::spawn(st.clone());
        tokio::time::sleep(Duration::from_millis(20)).await;
        fake.notify(&json!({"src": "agent", "evt": "event_shown", "id": "e1"}));
        for _ in 0..50 {
            if st.home.notifier.shown.lock().unwrap().contains("e1") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(st.home.notifier.shown.lock().unwrap().contains("e1"));
        // a kind turned off: nothing
        st.update_config(json!({"event_rules": {"test": "off"}}).as_object().unwrap().clone()).unwrap();
        let (_, v, _) = call(&app, req("POST", "/api/events/test", Some(json!({"kind": "test"})), tok)).await;
        assert_eq!(v["event"], Value::Null);

        // the Stop hook: short tasks feed the pet only; long ones show "Claude finished"
        let (_, p0, _) = call(&app, req("GET", "/api/pet", None, None)).await;
        assert_eq!(p0["pet"]["level"], 1);
        let (_, v, _) = call(&app, req("POST", "/api/claude-event", Some(json!({"elapsed_s": 30, "cwd": "/x/web"})), None)).await;
        assert_eq!(v["error"], "bad token");
        let (_, v, _) = call(&app, req("POST", "/api/claude-event", Some(json!({"elapsed_s": 30, "cwd": "/x/web"})), tok)).await;
        assert_eq!(v, json!({"ok": true, "shown": false}));
        let (_, v, _) = call(
            &app,
            req("POST", "/api/claude-event", Some(json!({"elapsed_s": 245, "cwd": "/Users/a/code/web/", "prompt": "fix the tests"})), tok),
        )
        .await;
        assert_eq!(v, json!({"ok": true, "shown": true}));
        let last = stick_lines(&fake).into_iter().rev().find(|l| l.get("event").is_some()).unwrap();
        assert_eq!(last["event"]["title"], "Claude finished in web");
        assert_eq!(last["event"]["sub"], "took 4 min - fix the tests");
        assert!(stick_lines(&fake).iter().any(|l| l.get("world").is_some())); // pushed at once
        let (_, p1, _) = call(&app, req("GET", "/api/pet", None, None)).await;
        assert_eq!(p1["pet"]["claude"], p0["pet"]["claude"].as_i64().unwrap() + 2);
        assert_eq!(p1["pet"]["xp"], p0["pet"]["xp"].as_i64().unwrap() + 4);
        assert_eq!(p1["pet"]["streak"], 1);
        assert!(p1["world"]["pet"]["seed"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn the_github_watcher_turns_changes_into_events() {
        let (_app, st, fake) = app("github", true).await;
        let gh = crate::github::FakeGh::default();
        st.home.set_sources(Arc::new(crate::net::FakeHttp::default()), Arc::new(gh.clone()), Arc::new(|| 0));
        // no gh: says so
        crate::home::github_poll(&st).await.unwrap();
        assert!(st.status()["github"]["error"].as_str().unwrap().contains("gh CLI not found"));
        let pr = |merged: bool, ci: &str| {
            json!({"mine": {"nodes": [{"number": 7, "title": "Dark mode", "state": if merged { "MERGED" } else { "OPEN" },
                    "merged": merged, "createdAt": "2020-01-01T00:00:00Z", "repository": {"nameWithOwner": "acme/web"},
                    "reviewDecision": null, "commits": {"nodes": [{"commit": {"statusCheckRollup": {"state": ci}}}]},
                    "reviews": {"nodes": []}}]},
                   "asks": {"nodes": []}, "primary": {"releases": {"nodes": []}}})
        };
        st.update_config(json!({"github": {"on": true, "interval_s": 120, "primary_repo": "acme/web"}}).as_object().unwrap().clone())
            .unwrap();
        gh.set(Ok(pr(false, "PENDING")));
        crate::home::github_poll(&st).await.unwrap();
        assert!(stick_lines(&fake).iter().all(|l| l.get("event").is_none())); // first look: no events
        let status = st.status()["github"].clone();
        assert_eq!((status["tracked"].as_i64(), status["error"].as_str()), (Some(1), Some("")));
        assert!(st.paths.home.join("github_state.json").exists());
        gh.set(Ok(pr(true, "SUCCESS")));
        crate::home::github_poll(&st).await.unwrap();
        let evs: Vec<Value> = stick_lines(&fake).into_iter().filter_map(|l| l.get("event").cloned()).collect();
        assert_eq!(evs.len(), 1);
        assert_eq!((evs[0]["title"].as_str(), evs[0]["sub"].as_str()), (Some("PR #7 merged!"), Some("web #7 - Dark mode")));
        let pet = st.home.pet.lock().unwrap().summary();
        assert_eq!((pet["merged"].as_i64(), pet["greens"].as_i64(), pet["xp"].as_i64()), (Some(1), Some(1), Some(13)));
        // the world went out with the change
        let w = stick_lines(&fake).into_iter().rev().find_map(|l| l.get("world").cloned()).unwrap();
        assert_eq!(w["merged"], 1);
        // an error is kept for the status
        gh.set(Err("HTTP 401".into()));
        assert!(crate::home::github_poll(&st).await.is_err());
    }
}
