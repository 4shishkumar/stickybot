//! The inference proxy between Goose and llama.cpp, on its own loopback port with its own key.
//!
//! Only chat completions (and a fixed model list) pass. Every request's tool list must be our
//! read-only catalog: if Goose ever offers the model a tool of its own (shell, file edits,
//! extensions), the request is refused and the brain stops the session (fail closed).

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use hyper_util::rt::TokioIo;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

const BODY_LIMIT: usize = 4 << 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    pub port: u16,
    pub key: String,
}

pub struct Shared {
    key: String,
    allowed: BTreeSet<String>,
    upstream: Mutex<Option<Upstream>>,
    violation: Mutex<Option<String>>,
    on_use: Box<dyn Fn() + Send + Sync>,
}

pub struct Proxy {
    pub port: u16,
    pub key: String,
    shared: Arc<Shared>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Proxy {
    /// `allowed`: the exact tool names Goose may offer the model.
    pub async fn start(allowed: BTreeSet<String>, on_use: Box<dyn Fn() + Send + Sync>) -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let key = crate::config::new_token();
        let shared = Arc::new(Shared {
            key: key.clone(),
            allowed,
            upstream: Mutex::new(None),
            violation: Mutex::new(None),
            on_use,
        });
        let app = Router::new()
            .route("/v1/chat/completions", post(chat))
            .route("/v1/models", get(models))
            .layer(DefaultBodyLimit::max(BODY_LIMIT))
            .with_state(shared.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Ok(Self { port, key, shared, task })
    }

    pub fn set_upstream(&self, up: Upstream) {
        *self.shared.upstream.lock().unwrap() = Some(up);
    }

    /// The first refused tool list since the last call, if any.
    pub fn take_violation(&self) -> Option<String> {
        self.shared.violation.lock().unwrap().take()
    }
}

/// Every offered tool must be one of `allowed` (no tools at all is fine: e.g. a title request).
pub fn check_tools(req: &Value, allowed: &BTreeSet<String>) -> Result<(), String> {
    let Some(tools) = req.get("tools") else { return Ok(()) };
    if tools.is_null() {
        return Ok(());
    }
    let tools = tools.as_array().ok_or("tools is not a list")?;
    for t in tools {
        let name = t.pointer("/function/name").and_then(Value::as_str).unwrap_or("");
        if t.get("type").and_then(Value::as_str) != Some("function") || !allowed.contains(name) {
            return Err(format!("unexpected tool offered to the model: {}", if name.is_empty() { "?" } else { name }));
        }
    }
    if req.get("functions").is_some() {
        return Err("legacy functions offered to the model".into());
    }
    Ok(())
}

fn authorized(st: &Shared, headers: &HeaderMap) -> bool {
    let bearer = headers.get(header::AUTHORIZATION).map(|v| v.as_bytes()).unwrap_or_default();
    crate::api::same(bearer.strip_prefix(b"Bearer ").unwrap_or_default(), st.key.as_bytes())
}

async fn models(State(st): State<Arc<Shared>>, headers: HeaderMap) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    axum::Json(json!({"object": "list", "data": [{"id": super::llama::ALIAS, "object": "model", "owned_by": "local"}]}))
        .into_response()
}

fn refuse(code: StatusCode, msg: &str) -> Response {
    (code, axum::Json(json!({"error": {"message": msg, "type": "stickybot_policy"}}))).into_response()
}

async fn chat(State(st): State<Arc<Shared>>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&st, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Ok(Value::Object(mut req)) = serde_json::from_slice::<Value>(&body) else {
        return refuse(StatusCode::BAD_REQUEST, "not a JSON object");
    };
    if let Err(e) = check_tools(&Value::Object(req.clone()), &st.allowed) {
        log::warn!("local assistant: {e}");
        st.violation.lock().unwrap().get_or_insert(e.clone());
        return refuse(StatusCode::FORBIDDEN, &e);
    }
    let Some(up) = st.upstream.lock().unwrap().clone() else {
        return refuse(StatusCode::SERVICE_UNAVAILABLE, "the local model isn't running");
    };
    (st.on_use)();
    req.insert("model".into(), json!(super::llama::ALIAS));
    let body = serde_json::to_vec(&req).unwrap_or_default();
    match forward(&up, body).await {
        Ok(r) => r,
        Err(e) => refuse(StatusCode::BAD_GATEWAY, &e),
    }
}

/// POSTs to llama-server and streams its answer back (SSE when the client asked to stream).
async fn forward(up: &Upstream, body: Vec<u8>) -> Result<Response, String> {
    let stream = tokio::net::TcpStream::connect(("127.0.0.1", up.port)).await.map_err(|e| e.to_string())?;
    let (mut send, conn) =
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await.map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    let req = Request::post("/v1/chat/completions")
        .header(header::HOST, format!("127.0.0.1:{}", up.port))
        .header(header::AUTHORIZATION, format!("Bearer {}", up.key))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::CONTENT_LENGTH, body.len())
        .body(http_body_util::Full::new(Bytes::from(body)))
        .map_err(|e| e.to_string())?;
    let resp = send.send_request(req).await.map_err(|e| e.to_string())?;
    let (parts, incoming) = resp.into_parts();
    let mut out = Response::builder().status(parts.status);
    if let Some(ct) = parts.headers.get(header::CONTENT_TYPE) {
        out = out.header(header::CONTENT_TYPE, ct);
    }
    out.body(Body::new(incoming)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed() -> BTreeSet<String> {
        ["stickybot__system_status", "load_skill"].iter().map(|s| s.to_string()).collect()
    }
    fn tool(name: &str) -> Value {
        json!({"type": "function", "function": {"name": name, "parameters": {}}})
    }

    #[test]
    fn only_our_tools_may_be_offered() {
        let a = allowed();
        assert!(check_tools(&json!({"messages": []}), &a).is_ok());
        assert!(check_tools(&json!({"tools": []}), &a).is_ok());
        assert!(check_tools(&json!({"tools": [tool("stickybot__system_status"), tool("load_skill")]}), &a).is_ok());
        for bad in ["developer__shell", "write", "stickybot__quick_command", "", "extensionmanager__enable"] {
            let e = check_tools(&json!({"tools": [tool("stickybot__system_status"), tool(bad)]}), &a).unwrap_err();
            assert!(e.contains("unexpected tool"), "{bad}: {e}");
        }
        assert!(check_tools(&json!({"tools": [{"type": "code_interpreter"}]}), &a).is_err());
        assert!(check_tools(&json!({"tools": "all"}), &a).is_err());
        assert!(check_tools(&json!({"functions": [{"name": "x"}]}), &a).is_err());
    }

    /// A stand-in llama-server: answers with what it was sent.
    async fn echo_upstream() -> Upstream {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let app = Router::new().route(
            "/v1/chat/completions",
            post(|headers: HeaderMap, body: Bytes| async move {
                let auth = headers.get(header::AUTHORIZATION).unwrap().to_str().unwrap().to_string();
                let req: Value = serde_json::from_slice(&body).unwrap();
                axum::Json(json!({"auth": auth, "model": req["model"]}))
            }),
        );
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        Upstream { port, key: "up-key".into() }
    }

    async fn post_json(port: u16, key: &str, body: Value) -> (u16, Value) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let body = body.to_string();
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let req = format!(
            "POST /v1/chat/completions HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {key}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        s.write_all(req.as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        let code = out[9..12].parse().unwrap();
        let json = out.find("\r\n\r\n").map(|i| &out[i + 4..]).unwrap_or("");
        // tolerate chunked framing from the forwarded body
        let start = json.find('{').unwrap_or(0);
        let end = json.rfind('}').map(|i| i + 1).unwrap_or(json.len());
        (code, serde_json::from_str(&json[start..end]).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn forwards_allowed_requests_and_refuses_others() {
        let used = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let u2 = used.clone();
        let p = Proxy::start(
            allowed(),
            Box::new(move || {
                u2.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }),
        )
        .await
        .unwrap();
        let ok = json!({"model": "whatever", "messages": [], "tools": [tool("stickybot__system_status")]});
        // no upstream yet
        assert_eq!(post_json(p.port, &p.key, ok.clone()).await.0, 503);
        p.set_upstream(echo_upstream().await);
        assert_eq!(post_json(p.port, "wrong", ok.clone()).await.0, 401);
        let (code, v) = post_json(p.port, &p.key, ok.clone()).await;
        assert_eq!(code, 200);
        assert_eq!(v["auth"], "Bearer up-key"); // the model's key never leaves the proxy
        assert_eq!(v["model"], "local");
        assert_eq!(used.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert!(p.take_violation().is_none());
        let bad = json!({"messages": [], "tools": [tool("developer__shell")]});
        assert_eq!(post_json(p.port, &p.key, bad).await.0, 403);
        assert!(p.take_violation().unwrap().contains("developer__shell"));
        assert!(p.take_violation().is_none());
    }
}
