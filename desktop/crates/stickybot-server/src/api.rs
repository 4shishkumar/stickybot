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
use axum::routing::{get, post, put};
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
    "POST /api/firmware/flash",
    "GET /api/firmware/jobs/{id}",
    "POST /api/firmware/jobs/{id}/cancel",
    "POST /api/wireless-mic/show",
    "POST /api/wireless-mic/check",
    "GET /api/import",
    "POST /api/import",
    "GET /api/login-item",
    "PUT /api/login-item",
    "GET /api/switch-over",
    "POST /api/switch-over",
    "GET /static-rs/*",
    "POST /mcp",
    "GET /api/mcp",
    "POST /api/mcp/terminal/{action}",
    "POST /api/mcp/answer",
    "GET /api/voices",
    "POST /api/voice/test",
    "GET /api/brain",
    "PUT /api/brain/openrouter/key",
    "DELETE /api/brain/openrouter/key",
    "GET /api/brain/openrouter/models",
    "POST /api/brain/test",
    "GET /api/pet",
    "POST /api/events/test",
    "GET /api/events/history",
    "GET /api/events/kinds",
    "POST /api/claude-event",
    "GET /api/logs",
    "GET /api/update",
    "POST /api/update/check",
    "POST /api/update/agent",
    "POST /api/update/firmware/{updater}",
    "GET /api/update/jobs/{id}",
    // the IR remote (stick 2.5.0; ir.rs)
    "GET /api/ir",
    "POST /api/ir/send",
    "POST /api/ir/learn",
    "POST /api/ir/learn/cancel",
    "POST /api/ir/rename",
    "POST /api/ir/remove",
    "POST /api/ir/fav",
    "POST /api/ir/brand",
    "GET /api/ir/export",
    "POST /api/ir/import",
    "GET /api/ir/diag",
    // face packages (faces.rs)
    "GET /api/faces/catalog",
    "GET /api/faces/preview/{id}/{side}",
    "POST /api/faces/install",
    "GET /api/faces/jobs/{id}",
    "POST /api/faces/jobs/{id}/cancel",
    "POST /api/faces/remove",
    "POST /api/faces/use",
    // the setup page, components, the stick wizard's progress, restart / window (setup.rs)
    "GET /api/onboarding",
    "POST /api/onboarding",
    "GET /api/install",
    "GET /api/install/{step}",
    "POST /api/install/{step}/skip|unskip|run",
    "POST /api/install/github/repo",
    "GET /api/install/jobs/{job}",
    "POST /api/install/jobs/{job}/cancel",
    "GET /api/components",
    "POST /api/components/{id}/install|uninstall",
    "GET /api/components/jobs/{job}",
    "POST /api/restart",
    "POST /api/window",
    // "Report an issue" (feedback.rs)
    "GET /api/feedback",
    "POST /api/feedback",
    "GET /api/feedback/diagnostics",
    "GET /api/feedback/reports",
    "GET /api/feedback/reports/{id}",
    "POST /api/feedback/reports/{id}/retry",
];

/// The Python agent's endpoints that aren't here yet, by the phase that brings them.
pub const NOT_PORTED: &[(&str, &str)] = &[];

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
        .route("/api/firmware/flash", post(usb_flash))
        .route("/api/firmware/jobs/{id}", get(job_events))
        .route("/api/firmware/jobs/{id}/cancel", post(job_cancel))
        .route("/api/wireless-mic/show", post(wireless_mic_show))
        .route("/api/wireless-mic/check", post(wireless_mic_check))
        .route("/api/import", get(import_info).post(import_run))
        .route("/api/login-item", get(login_item).put(set_login_item))
        .route("/api/switch-over", get(switch_info).post(switch_run))
        .route("/mcp", post(mcp_post).get(mcp_405).delete(mcp_405))
        .route("/api/mcp", get(mcp_status))
        .route("/api/mcp/terminal/{action}", post(mcp_terminal))
        .route("/api/mcp/answer", post(mcp_answer))
        .route("/api/voices", get(voices))
        .route("/api/voice/test", post(voice_test))
        .route("/api/brain", get(brain_status))
        .route("/api/brain/openrouter/key", put(brain_key_put).delete(brain_key_delete))
        .route("/api/brain/openrouter/models", get(brain_models))
        .route("/api/brain/test", post(brain_test))
        .route("/static-rs/{*path}", get(rust_static))
        .route("/api/pet", get(pet_status))
        .route("/api/events/test", post(event_test))
        .route("/api/events/history", get(event_history))
        .route("/api/events/kinds", get(event_kinds))
        .route("/api/claude-event", post(claude_event))
        .route("/api/logs", get(logs_tail))
        .route("/api/update", get(update_info))
        .route("/api/update/check", post(update_check))
        .route("/api/update/agent", post(update_agent))
        .route("/api/update/firmware/{updater}", post(update_firmware))
        .route("/api/update/jobs/{id}", get(job_events))
        .merge(crate::ir::routes())
        .merge(crate::faces::routes())
        .merge(crate::setup::routes())
        .merge(crate::feedback::routes())
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
            .replace("<title>StickyBot</title>", &format!("<title>{}</title>", st.app_name()))
            .replace("</body>", "<script type=\"module\" src=\"/static-rs/rust.js\"></script>\n</body>"),
    )
}

/// This app's own additions to the shared page (`assets/`): e.g. Settings → Import from StickyBot.
const RUST_ASSETS: &[(&str, &str, &str)] = &[
    ("rust.js", "text/javascript", include_str!("../assets/rust.js")),
    ("feedback.js", "text/javascript", include_str!("../assets/feedback.js")),
];

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

// --- Settings → Brain (brain_api.py): the key is write-only, no answer carries it -------------------

fn key_ok(k: &str) -> bool {
    (20..=300).contains(&k.len()) && k.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn model_ok(m: &str) -> bool {
    (1..=200).contains(&m.len()) && m.chars().all(|c| c.is_ascii_alphanumeric() || "._-/:~".contains(c))
}

async fn key_status(st: &Arc<AppState>) -> Value {
    let st = st.clone();
    tokio::task::spawn_blocking(move || st.keys.status("openrouter")).await.unwrap_or(Value::Null)
}

async fn brain_status(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    let status = st.brain().status();
    axum::Json(json!({"brain": crate::brains::selected(&cfg), "choices": crate::brains::NAMES, "status": status,
        "openrouter": {"key": key_status(&st).await,
                       "model": cfg.get("openrouter_model").and_then(Value::as_str).unwrap_or(""),
                       "max_tokens": cfg.get("openrouter_max_tokens").cloned().unwrap_or(Value::Null)}}))
}

async fn brain_key_put(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let key = body_obj(&body).get("key").and_then(Value::as_str).map(|k| k.trim().to_string()).unwrap_or_default();
    if !key_ok(&key) {
        return err(StatusCode::BAD_REQUEST, "That doesn't look like an API key (letters, digits, - and _ only).");
    }
    let st2 = st.clone();
    match tokio::task::spawn_blocking(move || st2.keys.save("openrouter", &key)).await {
        Ok(Ok(())) => axum::Json(json!({"ok": true, "key": key_status(&st).await})).into_response(),
        Ok(Err(e)) => {
            log::warn!("openrouter key: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Couldn't save it in the Keychain: {e}"))
        }
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

async fn brain_key_delete(State(st): State<Arc<AppState>>) -> Response {
    let st2 = st.clone();
    match tokio::task::spawn_blocking(move || st2.keys.remove("openrouter")).await {
        Ok(Ok(had)) => axum::Json(json!({"ok": true, "removed": had, "key": key_status(&st).await})).into_response(),
        Ok(Err(e)) => err(StatusCode::INTERNAL_SERVER_ERROR, &format!("Couldn't remove it from the Keychain: {e}")),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
}

#[derive(serde::Deserialize)]
struct ModelsQuery {
    refresh: Option<String>,
}

async fn brain_models(State(st): State<Arc<AppState>>, Query(q): Query<ModelsQuery>) -> Response {
    let (models, suggested, error) = st.openrouter().list_models(q.refresh.as_deref() == Some("1")).await;
    let status = if !models.is_empty() || error.is_none() { StatusCode::OK } else { StatusCode::BAD_GATEWAY };
    (status, axum::Json(json!({"models": models, "suggested": suggested, "error": error}))).into_response()
}

async fn brain_test(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let model = match body_obj(&body).get("model") {
        None | Some(Value::Null) => None,
        Some(Value::String(m)) if model_ok(m) => Some(m.clone()),
        Some(Value::String(m)) if m.is_empty() => None,
        _ => return err(StatusCode::BAD_REQUEST, "not a model id"),
    };
    axum::Json(st.openrouter().test(model).await).into_response()
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

use crate::python_port;

/// The switch-over (switch.rs): done?, this app's port, the Python app still running?
async fn switch_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let switched = crate::switch::switched(&st.paths.home);
    let python = !switched && crate::switch::listening(python_port()).await;
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from).unwrap_or_default();
    let login_item = if st.fake { None } else { crate::switch::python_login_item(&home, st.import_from.as_deref()) };
    axum::Json(json!({"switched": switched, "fresh": crate::switch::fresh(&st.paths.home), "port": st.port, "python_running": python,
                      "restart_needed": switched != (st.port == crate::switch::PORT),
                      "python_login_item": login_item}))
}

/// `{"confirm": true}`: take the Python app's place from the next start; `{"undo": true}`: back.
async fn switch_run(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    if b.get("undo") == Some(&json!(true)) {
        return match crate::switch::undo(&st.paths) {
            Ok(()) => axum::Json(json!({"ok": true, "restart_needed": true})).into_response(),
            Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e),
        };
    }
    if b.get("confirm") != Some(&json!(true)) {
        return err(StatusCode::BAD_REQUEST, "needs {\"confirm\": true}");
    }
    let Some(from) = st.import_from.clone() else {
        return err(StatusCode::NOT_FOUND, "no StickyBot (Python) folder to take over from");
    };
    match crate::switch::switch_over(&st.paths, &from, python_port()).await {
        Ok(()) => {
            log::info!("switched over: port {} and the Python app's token from the next start", crate::switch::PORT);
            axum::Json(json!({"ok": true, "restart_needed": true})).into_response()
        }
        Err(e) => err(StatusCode::CONFLICT, &e),
    }
}

/// Start at login: `{"available", "state": "on|off|needs_approval|unavailable", "on", "detail"}`.
async fn login_item(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let item = st.login_item.lock().unwrap().clone();
    axum::Json(item.state().to_json())
}

/// `{"on": bool}` registers / unregisters the menu bar app as a login item.
async fn set_login_item(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let Some(on) = body_obj(&body).get("on").and_then(Value::as_bool) else {
        return err(StatusCode::BAD_REQUEST, "needs {\"on\": true | false}");
    };
    let item = st.login_item.lock().unwrap().clone();
    match tokio::task::spawn_blocking(move || item.set(on)).await {
        Ok(Ok(s)) => {
            log::info!("start at login: {:?}", s);
            axum::Json(s.to_json()).into_response()
        }
        Ok(Err(e)) => err(StatusCode::CONFLICT, &e),
        Err(e) => err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    }
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
    let mut bad = crate::config::invalid(&allowed);
    if bad.is_empty() {
        let tunes = crate::events::tune_list(st.sounds().as_ref());
        bad = crate::events::invalid_sounds(&allowed, tunes.as_deref(), &st.cfg.lock().unwrap());
    }
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
    let ble = st.link.connected();
    if ble && let Some(reply) = st.link.request(line).await {
        let ok = reply.get("ok").and_then(Value::as_bool).unwrap_or(true);
        let error = reply.get("error").cloned().unwrap_or(Value::Null);
        return axum::Json(json!({"ok": ok, "via": "ble", "reply": reply, "error": error}));
    }
    // over USB (firmware_api.py `Stick.run`): when Bluetooth isn't there or didn't answer
    let r = st.usb.run(line).await;
    if ble && r["via"].is_null() && !st.usb.is_flashing() {
        return axum::Json(json!({"ok": false, "via": "ble", "reply": null, "error": "the stick didn't answer"}));
    }
    axum::Json(r)
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

async fn get_sounds(State(st): State<Arc<AppState>>) -> Response {
    match st.sound(json!({"get": 1})).await {
        Some(s) => axum::Json(s).into_response(),
        None => err(StatusCode::SERVICE_UNAVAILABLE, "the stick didn't answer (connected?)"),
    }
}

async fn put_sounds(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let payload: Map<String, Value> =
        body_obj(&body).into_iter().filter(|(k, _)| matches!(k.as_str(), "set" | "on" | "volume")).collect();
    match st.sound(Value::Object(payload)).await {
        Some(s) => axum::Json(s).into_response(),
        None => err(StatusCode::SERVICE_UNAVAILABLE, "the stick didn't answer"),
    }
}

async fn play_sound(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let name = body_obj(&body).get("name").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
    let ok = st.sound(json!({"play": name.unwrap_or_else(|| "test".into())})).await.is_some();
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

/// The Events page's ▶: `{"kind", "level"?, "sound"?}` -> the kind's sample on the stick, with the
/// level and sound the page shows (saved or not).
async fn event_test(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let kind = b.get("kind").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()));
    let (level, sound) = (b.get("level").and_then(Value::as_str), b.get("sound").and_then(Value::as_str));
    let ev = crate::events::test(&st, &kind.unwrap_or_else(|| "test".into()), level, sound).await;
    axum::Json(json!({"ok": true, "event": ev}))
}

async fn event_history(State(st): State<Arc<AppState>>, Query(q): Query<HistoryQuery>) -> axum::Json<Value> {
    axum::Json(Value::Array(crate::events::history(&st, q.n.unwrap_or(50))))
}

async fn event_kinds(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    axum::Json(crate::events::kinds_json(&cfg, st.sounds().as_ref()))
}

/// The Claude Code Stop hook (`claude_hooks.py`): `{"elapsed_s", "cwd", "prompt"}`.
async fn claude_event(State(st): State<Arc<AppState>>, body: Bytes) -> axum::Json<Value> {
    let b = body_obj(&body);
    let elapsed = b.get("elapsed_s").and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))).unwrap_or(0);
    let s = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let ev = crate::home::claude_finished(&st, elapsed, &s("cwd"), &s("prompt")).await;
    axum::Json(json!({"ok": true, "shown": ev.is_some()}))
}

/// The update status with the release notes, how this app is installed, the firmware updaters.
async fn update_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    axum::Json(st.updates.full(&cfg))
}

/// Checks now (lists releases, downloads the latest firmware) -> the same.
async fn update_check(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(crate::updates::check_now(&st).await)
}

/// The app updating itself isn't ported (selfupdate.py).
async fn update_agent(State(st): State<Arc<AppState>>) -> Response {
    let cfg = st.cfg.lock().unwrap().clone();
    let install = st.updates.full(&cfg)["install"].clone();
    (StatusCode::CONFLICT, axum::Json(json!({"error": crate::updates::NO_SELF_UPDATE, "install": install})))
        .into_response()
}

/// No extra firmware updaters here (Bluetooth and USB have their own endpoints).
async fn update_firmware() -> Response {
    err(StatusCode::NOT_FOUND, "no such firmware updater")
}

/// The log file's last `n` (300) lines, oldest first (server.py `logs`).
async fn logs_tail(State(st): State<Arc<AppState>>, Query(q): Query<HistoryQuery>) -> axum::Json<Value> {
    let n = q.n.unwrap_or(300).min(5000);
    axum::Json(json!(crate::logs::tail(&st.paths.log(), n)))
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

/// The bundled firmware, sticks on USB (with their versions), the connected stick.
async fn firmware_info(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    let bundled = st.firmware().first().map(crate::firmware::describe).unwrap_or(Value::Null);
    let usb = st.usb.sticks().await;
    axum::Json(json!({"bundled": bundled, "usb": usb, "connected": crate::firmware::connected(&st.link).await}))
}

/// Starts a firmware update over Bluetooth: `{"force"}` -> `{"job"}`.
/// Flashes the stick on USB with esptool: `{"port", "backup"}` -> `{"job"}` (firmware_api.py `flash`).
async fn usb_flash(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let b = body_obj(&body);
    let port = b.get("port").and_then(Value::as_str).unwrap_or("").to_string();
    let backup = b.get("backup").and_then(Value::as_bool).unwrap_or(false);
    let st2 = st.clone();
    let r = tokio::task::spawn_blocking(move || st2.usb.backend().ports())
        .await
        .map(|ports| ports.iter().any(|p| p.port == port));
    if !matches!(r, Ok(true)) {
        return err(StatusCode::BAD_REQUEST, &format!("{port} isn't an Espressif USB device (not a stick) — refusing"));
    }
    match crate::flasher::start(&st, &port, backup) {
        Ok(job) => axum::Json(json!({"job": job})).into_response(),
        Err((code, e)) => err(StatusCode::from_u16(code).unwrap_or(StatusCode::BAD_REQUEST), &e),
    }
}

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
    stream_job(job)
}

/// A job's events as server-sent events (firmware_api.py `stream_job`; face installs use it too).
pub(crate) fn stream_job(job: Arc<crate::firmware::Job>) -> Response {
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
        assert!(html.contains("<title>StickyBot (Rust)</title>")); // side by side (8670): told apart
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
        assert_eq!(st.sounds().unwrap()["volume"], 99); // kept for the menu bar
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
        let (s, v, _) = call(&app, req("GET", "/api/nope", None, None)).await;
        assert_eq!((s, v["missing"].as_bool()), (StatusCode::NOT_FOUND, Some(true)));
        let (s, _, _) = call(&app, req("POST", "/api/onboarding", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST); // ported: the step is missing
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
    async fn update_check_downloads_firmware_for_the_bluetooth_and_usb_buttons() {
        let (app, st, _) = app("update", true).await;
        st.updates.set_sources(Arc::new(|| {
            let mut g = crate::releases::tests::FakeGitHub::default();
            g.add_release("v2.2.0", "9.9.0", false);
            vec![Box::new(g) as Box<dyn crate::releases::Source>]
        }));
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("GET", "/api/update", None, None)).await;
        assert_eq!((v["latest"].clone(), v["firmware_updaters"].clone()), (Value::Null, json!([])));
        let (s, v, _) = call(&app, req("POST", "/api/update/check", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["latest"]["firmware"], "9.9.0");
        // the fake stick runs older firmware
        assert_eq!((v["firmware_update"].as_bool(), v["title"].as_str()), (Some(true), Some("Stick firmware 9.9.0 available")));
        let (_, v, _) = call(&app, req("GET", "/api/firmware", None, None)).await;
        assert_eq!((v["bundled"]["version"].as_str(), v["bundled"]["source"].as_str()), (Some("9.9.0"), Some("release")));
        let (_, v, _) = call(&app, req("GET", "/api/status", None, None)).await;
        assert_eq!(v["update"]["available"], true);
        let (s, v, _) = call(&app, req("POST", "/api/update/agent", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert!(v["error"].as_str().unwrap().contains("doesn't update itself"));
        // one toast on the stick per release
        let ev = crate::updates::notify(&st).await.expect("toasted");
        assert_eq!((ev["title"].as_str(), ev["kind"].as_str()), (Some("Update firmware 9.9.0 ready"), Some("update")));
        assert!(crate::updates::notify(&st).await.is_none());
        let saved = std::fs::read_to_string(st.paths.home.join("updates.json")).unwrap();
        assert!(saved.contains("\"notified\""), "{saved}");
    }

    #[tokio::test]
    async fn proto_falls_back_to_usb_and_usb_sticks_are_listed() {
        let (app, st, fake) = app("usb", true).await;
        let usb = crate::usb::FakeUsb::stick("/dev/cu.usbmodem9", "2.1.0");
        st.usb.set_backend(usb.clone());
        let tok = Some(st.token.as_str());
        // Bluetooth first
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(v["via"], "ble");
        // Bluetooth silent: the stick on USB answers
        fake.set_silent(true);
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!((v["via"].as_str(), v["reply"]["fw"].as_str()), (Some("usb"), Some("2.1.0")));
        let (_, v, _) = call(&app, req("GET", "/api/firmware", None, None)).await;
        assert_eq!(v["usb"], json!([{"port": "/dev/cu.usbmodem9", "serial": "SN1", "version": "2.1.0", "board": "m5stick-s3"}]));
        // no stick on USB either: Bluetooth's answer
        usb.ports.lock().unwrap().clear();
        let (_, v, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!((v["via"].as_str(), v["error"].as_str()), (Some("ble"), Some("the stick didn't answer")));
    }

    #[tokio::test]
    async fn usb_flash_runs_esptool_as_a_job() {
        let (app, st, _) = app("flash", false).await;
        let usb = crate::usb::FakeUsb::stick("/dev/cu.usbmodem3", "2.1.0");
        st.usb.set_backend(usb.clone());
        let tool = Arc::new(crate::flasher::tests::FakeEsptool::default());
        st.set_esptool(tool.clone());
        *st.flash_timings.lock().unwrap() = crate::flasher::FlashTimings {
            settle: Duration::ZERO,
            poll: Duration::from_millis(1),
            bootloader: Duration::from_millis(100),
        };
        let tok = Some(st.token.as_str());
        let (s, v, _) = call(&app, req("POST", "/api/firmware/flash", Some(json!({"port": "/dev/cu.other"})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        let (s, v, _) = call(&app, req("POST", "/api/firmware/flash", Some(json!({"port": "/dev/cu.usbmodem3"})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert!(v["error"].as_str().unwrap().contains("no firmware"));
        // a bundle on this Mac
        let d = st.paths.home.join("firmware/local");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("app.bin"), vec![0xE9u8; 64]).unwrap();
        std::fs::write(d.join("manifest.json"), json!({"version": "2.3.0", "parts": [{"file": "app.bin", "offset": "0x10000", "size": 64}]}).to_string()).unwrap();
        let (s, v, _) = call(&app, req("POST", "/api/firmware/flash", Some(json!({"port": "/dev/cu.usbmodem3"})), tok)).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        let job = st.jobs.get(v["job"].as_str().unwrap()).unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !job.is_done() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        let evs = job.events_from(0);
        assert_eq!(evs.last().unwrap()["ok"], true, "{evs:?}");
        assert_eq!(tool.runs.lock().unwrap().len(), 1); // no backup asked
        assert_eq!(*usb.touches.lock().unwrap(), vec!["/dev/cu.usbmodem3".to_string()]);
        assert!(!st.usb.is_flashing());
    }

    #[tokio::test]
    async fn the_log_page_reads_the_last_lines() {
        let (app, st, _) = app("logs", false).await;
        let (_, v, _) = call(&app, req("GET", "/api/logs", None, None)).await;
        assert_eq!(v, json!([]));
        let text: String = (0..400).map(|i| format!("12:00:00 x: line {i}\n")).collect();
        std::fs::write(st.paths.log(), text).unwrap();
        let (_, v, _) = call(&app, req("GET", "/api/logs?n=2", None, None)).await;
        assert_eq!(v, json!(["12:00:00 x: line 398", "12:00:00 x: line 399"]));
        let (_, v, _) = call(&app, req("GET", "/api/logs", None, None)).await;
        assert_eq!(v.as_array().unwrap().len(), 300);
    }

    #[tokio::test]
    async fn start_at_login() {
        let (app, st, _) = app("login", false).await;
        let tok = Some(st.token.as_str());
        let (_, v, _) = call(&app, req("GET", "/api/login-item", None, None)).await;
        assert_eq!((v["available"].as_bool(), v["state"].as_str()), (Some(false), Some("unavailable")));
        let (s, _, _) = call(&app, req("PUT", "/api/login-item", Some(json!({"on": true})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT); // the headless binary has none
        *st.login_item.lock().unwrap() = Arc::new(crate::login::FakeLoginItem::default());
        let (s, _, _) = call(&app, req("PUT", "/api/login-item", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, v, _) = call(&app, req("PUT", "/api/login-item", Some(json!({"on": true})), tok)).await;
        assert_eq!((s, v["state"].as_str(), v["on"].as_bool()), (StatusCode::OK, Some("on"), Some(true)));
        let (_, v, _) = call(&app, req("GET", "/api/login-item", None, None)).await;
        assert_eq!(v["state"], "on");
        assert_eq!(crate::login::LoginState::from_status(2).to_json()["state"], "needs_approval");
        assert_eq!(crate::login::LoginState::from_status(3).to_json()["state"], "off"); // never registered
        assert_eq!(crate::login::LoginState::from_status(-1).to_json()["state"], "unavailable");
        // the switch-over needs a confirm and a Python folder (taking over is unit-tested in switch.rs)
        let (s, _, _) = call(&app, req("POST", "/api/switch-over", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _, _) = call(&app, req("POST", "/api/switch-over", Some(json!({"confirm": true})), tok)).await;
        assert_eq!(s, StatusCode::NOT_FOUND); // tests have no import_from
        let (s, v, _) = call(&app, req("POST", "/api/switch-over", Some(json!({"undo": true})), tok)).await;
        assert_eq!((s, v["ok"].as_bool()), (StatusCode::OK, Some(true)));
        assert!(!crate::switch::switched(&st.paths.home));
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
        assert_eq!(h["speak"], true); // the stick's reply view may ask us to read aloud (2.6.0)
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
                        "sub": "web - Add stick agent", "icon": "merge", "sound": "applause", "kind": "pr_merged"}})]
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

    // the Events page: ▶ plays what the page shows; a sound per kind, checked against the stick's list
    #[tokio::test]
    async fn event_test_takes_the_pages_level_and_sound_and_event_sounds_are_checked() {
        let (app, st, fake) = app("evsounds", true).await;
        let tok = Some(st.token.as_str());
        let last_event = || stick_lines(&fake).into_iter().rev().find_map(|l| l.get("event").cloned()).unwrap();
        // the stick hasn't told us its sounds yet: the kind's own goes out (2.7.0 falls back by itself)
        let (_, v, _) = call(&app, req("GET", "/api/events/kinds", None, None)).await;
        assert_eq!((v["sounds"]["pr_merged"].as_str(), v["tunes_known"].as_bool()), (Some("applause"), Some(false)));
        let (_, v, _) =
            call(&app, req("POST", "/api/events/test", Some(json!({"kind": "ci_failed", "level": "celebrate", "sound": "cheer"})), tok))
                .await;
        assert_eq!((v["event"]["level"].as_str(), v["event"]["sound"].as_str()), (Some("celebrate"), Some("cheer")));
        assert_eq!(last_event()["sound"], "cheer");
        assert_eq!(st.cfg.lock().unwrap()["event_rules"], json!({})); // ▶ saves nothing
        // the fake stick is older firmware (no celebration clips): a name it lacks goes as "" (its level's)
        let (_, s, _) = call(&app, req("GET", "/api/sounds", None, None)).await;
        assert!(s["tunes"].as_array().is_some_and(|t| !t.contains(&json!("applause"))));
        let (_, v, _) = call(&app, req("POST", "/api/events/test", Some(json!({"kind": "pr_merged"})), tok)).await;
        assert_eq!((v["event"]["level"].as_str(), v["event"]["sound"].as_str()), (Some("celebrate"), Some("")));
        let (_, v, _) = call(&app, req("GET", "/api/events/kinds", None, None)).await;
        assert_eq!(v["tunes_known"], true);
        assert!(v["tunes"].as_array().unwrap().contains(&json!("beep")));
        // junk level / sound: the saved ones
        let (_, v, _) =
            call(&app, req("POST", "/api/events/test", Some(json!({"kind": "ci_failed", "level": "loud", "sound": "../x"})), tok)).await;
        assert_eq!((v["event"]["level"].as_str(), v["event"]["sound"].as_str()), (Some("headsup"), Some("")));
        // saving: only names the stick has
        let (code, v, _) = call(&app, req("PUT", "/api/config", Some(json!({"event_sounds": {"pr_merged": "coins"}})), tok)).await;
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(v["error"].as_str().unwrap().contains("no sound 'coins'"), "{v}");
        let (code, _, _) =
            call(&app, req("PUT", "/api/config", Some(json!({"event_sounds": {"pr_merged": "beep", "ci_failed": "off"}})), tok)).await;
        assert_eq!(code, StatusCode::OK);
        let (_, v, _) = call(&app, req("POST", "/api/events/test", Some(json!({"kind": "pr_merged"})), tok)).await;
        assert_eq!(v["event"]["sound"], "beep");
        let (_, v, _) = call(&app, req("POST", "/api/events/test", Some(json!({"kind": "ci_failed"})), tok)).await;
        assert_eq!(v["event"]["sound"], "off");
        let (_, v, _) = call(&app, req("GET", "/api/events/kinds", None, None)).await;
        assert_eq!((v["sounds"]["pr_merged"].as_str(), v["sounds"]["release"].as_str()), (Some("beep"), Some("party_horn")));
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

    // test_pet.py: the agent takes game events (firmware 2.5.0)
    #[tokio::test]
    async fn game_events_give_xp_and_go_to_the_dashboard_and_the_faces() {
        let (_app, st, _fake) = app("games", true).await;
        let (_, mut rx) = st.hub.subscribe(0);
        let xp0 = st.home.pet.lock().unwrap().s["xp"].as_i64().unwrap();
        let worlds = |st: &AppState| st.home.world_sent.lock().unwrap().clone();
        let w0 = worlds(&st);
        let m = json!({"src": "agent", "evt": "game", "id": "catch", "score": 14, "best": 20, "new_best": true});
        crate::home::on_game(&st, &m).await;
        let ev = loop {
            let e = rx.recv().await.unwrap();
            if e["kind"] == "game" {
                break e;
            }
        };
        assert_eq!((ev["id"].as_str(), ev["score"].as_i64(), ev["best"].as_i64()), (Some("catch"), Some(14), Some(20)));
        assert_eq!((ev["new_best"].as_bool(), ev["xp"].as_i64()), (Some(true), Some(crate::pet::XP_GAME_BEST)));
        assert_eq!(st.home.pet.lock().unwrap().s["xp"].as_i64().unwrap(), xp0 + crate::pet::XP_GAME_BEST);
        assert_ne!(worlds(&st), w0); // the new XP went to the stick's faces
        crate::home::on_game(&st, &json!({"src": "agent", "evt": "game", "id": "nope", "score": 1})).await;
        assert!(rx.try_recv().map(|e| e["kind"] != "game").unwrap_or(true));
        assert_eq!(st.home.pet.lock().unwrap().summary()["games"]["best"]["catch"], 20);
    }

    // --- test_ir.py: the IR remote (stick 2.5.0) against the fake stick's @ir ----------------------

    fn remotes(fake: &FakeStick, rows: &[(&str, &str, &str, bool)]) {
        fake.ir(|ir| {
            ir.devices.clear();
            for (d, b, c, f) in rows {
                ir.add(d, b, c, *f);
            }
            ir.sent.clear();
        });
    }

    fn sent(fake: &FakeStick) -> Vec<(String, String, i64)> {
        fake.ir(|ir| ir.sent.clone())
    }

    fn t3(a: &str, b: &str, n: i64) -> (String, String, i64) {
        (a.into(), b.into(), n)
    }

    #[tokio::test]
    async fn ir_list_and_send() {
        let (_app, st, fake) = app("ir-list", true).await;
        let mut rows = vec![("tv", "power", "nec:4:8", true), ("tv", "volume up", "nec:4:2", false)];
        let names: Vec<String> = (16..31).map(|t| format!("cool {t}")).collect();
        rows.extend(names.iter().map(|n| ("ac", n.as_str(), "raw:38:1,2,3", false)));
        remotes(&fake, &rows);
        let d = crate::ir::devices(&st, true).await.unwrap();
        let tv = d["devices"].as_array().unwrap().iter().find(|x| x["id"] == "tv").unwrap().clone();
        assert_eq!(tv["buttons"][0], json!({"name": "power", "proto": "nec", "fav": true}));
        let ac = d["devices"].as_array().unwrap().iter().find(|x| x["id"] == "ac").unwrap().clone();
        assert_eq!(ac["buttons"].as_array().unwrap().len(), 15); // two pages
        assert_eq!(crate::ir::send(&st, "TV", "Volume Up", 2).await.unwrap(), "tv: volume up sent");
        assert_eq!(sent(&fake), vec![t3("tv", "volume up", 2)]);
        let e = crate::ir::send(&st, "tv", "mute", 0).await.unwrap_err();
        assert!(e.contains("no 'mute' button on the 'tv' remote"), "{e}");
    }

    #[tokio::test]
    async fn ir_brand_fallback() {
        let (_app, st, fake) = app("ir-brand", true).await;
        remotes(&fake, &[]);
        st.update_config(json!({"ir_brands": {"tv": "samsung"}}).as_object().unwrap().clone()).unwrap();
        assert_eq!(crate::ir::send(&st, "tv", "power", 0).await.unwrap(), "tv: power sent");
        assert_eq!(sent(&fake), vec![t3("code", "samsung:7:2", 0)]);
        let listing = crate::ir::listing(&st).await.unwrap();
        let tv = &listing.iter().find(|(d, _)| d == "tv").unwrap().1;
        assert!(tv.contains(&"power".to_string()) && tv.contains(&"mute".to_string()));
        remotes(&fake, &[("tv", "power", "nec:4:8", false)]); // a learned one wins
        crate::ir::send(&st, "tv", "power", 0).await.unwrap();
        assert_eq!(sent(&fake).last().unwrap(), &t3("tv", "power", 0));
    }

    #[tokio::test]
    async fn ir_old_firmware_and_no_stick() {
        let (_app, st, fake) = app("ir-old", true).await;
        fake.ir(|ir| ir.unsupported = true);
        let e = crate::ir::send(&st, "tv", "power", 0).await.unwrap_err();
        assert!(e.contains("needs 2.5.0"), "{e}");
        let state = crate::ir::ask(&st, "@state").await;
        assert_eq!(
            crate::ir::supported(state.as_ref()).1.unwrap(),
            "the stick's firmware (2.3.0) has no IR remote: it needs 2.5.0"
        );
        st.link.pause(true);
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s != LinkState::Connected)).await.unwrap().unwrap();
        let e = crate::ir::devices(&st, false).await.unwrap_err();
        assert!(e.contains("isn't connected"), "{e}");
    }

    #[tokio::test]
    async fn ir_learning_reports_its_steps() {
        let (_app, st, fake) = app("ir-learn", true).await;
        crate::ir::spawn(st.clone());
        let (_, mut rx) = st.hub.subscribe(0);
        let st2 = st.clone();
        let task = tokio::spawn(async move {
            crate::ir::learn_and_wait(&st2, "TV", "Power", false, Duration::from_secs(5)).await
        });
        for _ in 0..100 {
            if fake.ir(|ir| ir.learning.is_some()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(fake.ir(|ir| ir.learning.clone()), Some(("tv".into(), "power".into())));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(fake.ir_press("nec:4:8"));
        let fin = tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap().unwrap();
        assert_eq!((fin["state"].as_str(), fin["btn"].as_str()), (Some("saved"), Some("power")));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut seen = Vec::new();
        while let Ok(e) = rx.try_recv() {
            if e["kind"] == "ir" {
                seen.push((e["state"].as_str().unwrap_or("").to_string(), e["step"].as_i64()));
            }
        }
        assert!(seen.contains(&("learning".into(), Some(1))) && seen.contains(&("learning".into(), Some(2))), "{seen:?}");
        assert_eq!(seen.last().unwrap().0, "saved");
        assert_eq!(st.ir.learn.lock().unwrap().as_ref().unwrap()["state"], "saved");
        assert_eq!(fake.ir(|ir| ir.devices["tv"].clone()), vec![json!({"n": "power", "c": "nec:4:8"})]);
    }

    #[tokio::test]
    async fn ir_export_import_round_trip_and_brands_on_the_stick() {
        let (_app, st, fake) = app("ir-export", true).await;
        remotes(&fake, &[("tv", "power", "nec:4:8", true), ("ac", "cool 24", "raw:38:3400,1700,420:0120", false)]);
        let mut doc = crate::ir::export(&st).await.unwrap();
        assert_eq!(
            doc,
            json!({"stickybot_ir": 1, "devices": {"ac": [{"n": "cool 24", "c": "raw:38:3400,1700,420:0120"}],
                                                  "tv": [{"n": "power", "c": "nec:4:8", "f": 1}]}})
        );
        remotes(&fake, &[]);
        doc["devices"]["tv"].as_array_mut().unwrap().push(json!({"n": "bad", "c": "nec:zz"}));
        let r = crate::ir::import_doc(&st, &doc).await.unwrap();
        assert_eq!((r["added"].as_i64(), r["skipped"].as_array().unwrap().len()), (Some(2), 1));
        assert_eq!(fake.ir(|ir| ir.devices["tv"].clone()), vec![json!({"n": "power", "c": "nec:4:8", "f": 1})]);
        let e = crate::ir::import_doc(&st, &json!({"nope": 1})).await.unwrap_err();
        assert!(e.contains("expected"), "{e}");
        // a brand's buttons on the stick
        let r = crate::ir::install_brand(&st, "Bedroom TV", "lg").await.unwrap();
        assert_eq!(r, json!({"dev": "bedroom-tv", "n": 7}));
        assert!(fake.ir(|ir| ir.devices["bedroom-tv"].contains(&json!({"n": "power", "c": "nec:4:8", "f": 1}))));
    }

    #[tokio::test]
    async fn ir_voice_quick_commands() {
        let (_app, st, fake) = app("ir-voice", true).await;
        st.commands.ensure().unwrap(); // the defaults have the TV phrases
        remotes(&fake, &[("tv", "power", "nec:4:8", false)]);
        crate::talk::handle_text(st.clone(), "Turn off the TV.".into(), "typed").await;
        assert_eq!(sent(&fake), vec![t3("tv", "power", 0)]);
        let screens = fake.screens();
        assert!(screens.iter().any(|(s, _, t)| s == "done" && t == "tv: power sent"), "{screens:?}");
        crate::talk::handle_text(st.clone(), "mute the tv".into(), "typed").await; // no such button, no brand: says so
        let (s, _, t) = fake.screens().last().unwrap().clone();
        assert!(s == "error" && t.contains("mute"), "{s} {t}");
        assert_eq!(st.status()["stats"]["quick"], 2);
        let h = std::fs::read_to_string(st.paths.history()).unwrap();
        assert!(h.lines().last().unwrap().contains("\"via\":\"quick\""));
    }

    #[tokio::test]
    async fn ir_voice_commands_are_added_once() {
        let (_app, st, _fake) = app("ir-once", false).await;
        let doc = json!({"commands": [{"name": "Mine", "phrases": ["hello"], "shell": "echo hi"}]});
        std::fs::write(&st.commands.path, doc.to_string()).unwrap();
        assert!(crate::ir::ensure_voice_commands(&st));
        let cmds = st.commands.load();
        assert_eq!(cmds[0]["name"], "Mine");
        assert!(cmds.iter().any(|c| c.get("ir") == Some(&json!({"dev": "tv", "btn": "power"}))));
        assert_eq!(st.cfg.lock().unwrap()["ir_voice_added"], true);
        assert!(!crate::ir::ensure_voice_commands(&st)); // once
        assert_eq!(st.commands.load().len(), cmds.len());
    }

    #[tokio::test]
    async fn ir_stick_tools() {
        let (_app, st, fake) = app("ir-mcp", true).await;
        remotes(&fake, &[("ac", "cool 24", "raw:38:1,2,3", false), ("ac", "off", "nec:1:2", false)]);
        let (text, err, res) = crate::mcp::call(&st, "ir_list", &json!({}), "test").await;
        assert!(!err && text.contains("ac: cool 24, off"), "{text}");
        assert_eq!(res.unwrap()["devices"]["ac"], json!(["cool 24", "off"]));
        let (text, err, _) = crate::mcp::call(&st, "ir_send", &json!({"device": "ac", "button": "cool 24"}), "test").await;
        assert!(!err && text == "ac: cool 24 sent", "{text}");
        let (text, err, _) = crate::mcp::call(&st, "ir_send", &json!({"device": "ac", "button": "heat 30"}), "test").await;
        assert!(err && text.contains("no 'heat 30' button"), "{text}");
        assert!(crate::mcp::allowed_tool_names().contains(&"mcp__stick__ir_send".to_string()));
        assert!(crate::mcp::openai_tools().iter().any(|t| t["function"]["name"] == "ir_list"));
    }

    #[tokio::test]
    async fn ir_api() {
        let (app, st, fake) = app("ir-api", true).await;
        let tok = Some(st.token.as_str());
        remotes(&fake, &[("tv", "power", "nec:4:8", false)]);
        let (_, r, _) = call(&app, req("GET", "/api/ir", None, None)).await;
        assert!(r["supported"] == true && r["devices"][0]["buttons"][0]["name"] == "power", "{r}");
        assert!(r["brands"].get("samsung").is_some());
        let (_, r, _) = call(&app, req("POST", "/api/ir/send", Some(json!({"dev": "tv", "btn": "power"})), tok)).await;
        assert_eq!(r, json!({"ok": true, "result": "tv: power sent"}));
        let (s, _, _) = call(&app, req("POST", "/api/ir/send", Some(json!({"brand": "sony", "btn": "Mute"})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(sent(&fake).last().unwrap(), &t3("code", "sony12:1:14", 0));
        let (s, r, _) = call(&app, req("POST", "/api/ir/send", Some(json!({"dev": "tv", "btn": "nope"})), tok)).await;
        assert_eq!(s, StatusCode::CONFLICT);
        assert!(r["error"].as_str().unwrap().starts_with("No 'nope'"), "{r}");
        let (_, r, _) = call(&app, req("POST", "/api/ir/brand", Some(json!({"dev": "TV", "brand": "lg"})), tok)).await;
        assert_eq!(r["mapped"], json!({"tv": "lg"}));
        assert_eq!(st.cfg.lock().unwrap()["ir_brands"], json!({"tv": "lg"}));
        let (s, _, _) = call(&app, req("POST", "/api/ir/brand", Some(json!({"dev": "tv", "brand": "acme"})), tok)).await;
        assert_eq!(s, StatusCode::NOT_FOUND);
        let (s, _, _) = call(&app, req("POST", "/api/ir/learn", Some(json!({"dev": "tv", "btn": "input"})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(fake.ir(|ir| ir.learning.clone()), Some(("tv".into(), "input".into())));
        let (s, _, _) = call(&app, req("POST", "/api/ir/learn/cancel", None, tok)).await;
        assert_eq!(s, StatusCode::OK);
        let b = json!({"dev": "tv", "btn": "power", "to": "on off"});
        let (s, _, _) = call(&app, req("POST", "/api/ir/rename", Some(b), tok)).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(fake.ir(|ir| ir.devices["tv"][0]["n"].clone()), "on off");
        let (s, _, _) = call(&app, req("POST", "/api/ir/fav", Some(json!({"dev": "tv", "btn": "on off", "on": true})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        let resp = app.clone().oneshot(req("GET", "/api/ir/export", None, None)).await.unwrap();
        assert!(resp.headers()["content-disposition"].to_str().unwrap().contains("attachment"));
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let doc: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(doc["devices"]["tv"][0], json!({"n": "on off", "c": "nec:4:8", "f": 1}));
        let (s, r, _) = call(&app, req("POST", "/api/ir/import", Some(json!({"doc": doc})), tok)).await;
        assert_eq!((s, r["added"].as_i64()), (StatusCode::OK, Some(1)));
        let (s, _, _) = call(&app, req("POST", "/api/ir/remove", Some(json!({"dev": "tv"})), tok)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(fake.ir(|ir| ir.devices.is_empty()));
        let (s, _, _) = call(&app, req("POST", "/api/ir/remove", Some(json!({})), tok)).await;
        assert_eq!(s, StatusCode::BAD_REQUEST);
        let (s, _, _) = call(&app, req("POST", "/api/ir/send", Some(json!({"dev": "tv", "btn": "power"})), None)).await;
        assert_eq!(s, StatusCode::FORBIDDEN); // no token
        let (_, r, _) = call(&app, req("GET", "/api/ir/diag", None, None)).await;
        assert_eq!(r["pins"], "ok");
        // not connected: says why
        st.link.pause(true);
        let mut w = st.link.watch_state();
        tokio::time::timeout(Duration::from_secs(5), w.wait_for(|s| *s != LinkState::Connected)).await.unwrap().unwrap();
        let (_, r, _) = call(&app, req("GET", "/api/ir", None, None)).await;
        assert_eq!((r["connected"].as_bool(), r["why"].as_str()), (Some(false), Some("the stick isn't connected by Bluetooth")));
    }

    #[tokio::test]
    async fn the_25_settings_go_through_the_proto_endpoint() {
        // the Mouse & presenter card (@air, @mode) and the Games card (@games) use /api/stick/proto
        let (app, st, _fake) = app("air", true).await;
        let tok = Some(st.token.as_str());
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(r["reply"]["air"]["mode"], "home");
        assert!(r["reply"]["ir"].is_object());
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@mode name=presenter"})), tok)).await;
        assert_eq!(r["ok"], true);
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@air sens=7&invx=1&keys=pages"})), tok)).await;
        assert_eq!((r["reply"]["air"]["sens"].as_i64(), r["reply"]["air"]["invX"].as_bool()), (Some(7), Some(true)));
        assert_eq!(r["reply"]["air"]["mode"], "presenter");
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@games"})), tok)).await;
        assert_eq!(r["reply"]["games"].as_array().unwrap().len(), 7);
    }

    #[tokio::test]
    async fn the_launcher_card_goes_through_the_proto_endpoint() {
        // the Stick page's Launcher card (firmware 2.5.1): "launcher" in @state, @launcher tiles=…|reset=1
        let (app, st, _fake) = app("launcher", true).await;
        let tok = Some(st.token.as_str());
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(r["reply"]["launcher"], "grapmfqs");
        // stick.js: encodeURIComponent("faces,games,air")
        let line = "@launcher tiles=faces%2Cgames%2Cair";
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": line})), tok)).await;
        assert_eq!((r["ok"].as_bool(), r["via"].as_str()), (Some(true), Some("ble")));
        assert_eq!(r["reply"]["tiles"], "faces,games,air,quick,settings");
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@state"})), tok)).await;
        assert_eq!(r["reply"]["launcher"], "fgaqs");
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@launcher tiles=tv"})), tok)).await;
        assert_eq!((r["ok"].as_bool(), r["error"].as_str()), (Some(false), Some("unknown tile: tv")));
        let (_, r, _) = call(&app, req("POST", "/api/stick/proto", Some(json!({"line": "@launcher reset=1"})), tok)).await;
        assert_eq!(r["reply"]["launcher"], "grapmfqs");
        // the card itself comes with the Python app's static files
        let (s, body, _) = call(&app, req("GET", "/static/pages/stick.js", None, None)).await;
        assert_eq!(s, StatusCode::OK);
        let body = body.as_str().unwrap();
        assert!(body.contains("function launcherCard()") && body.contains("send(\"launcher\""));
        // ...and so does the page's layout (its sections), which it imports
        let (s, body, _) = call(&app, req("GET", "/static/pages/stick_layout.js", None, None)).await;
        assert_eq!(s, StatusCode::OK);
        assert!(body.as_str().unwrap().contains("export const SECTIONS"));
    }
}
