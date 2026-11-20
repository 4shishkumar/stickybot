//! A small Agent Client Protocol (ACP v1) client: JSON-RPC lines over the agent's stdin/stdout.
//!
//! The app is the client and offers the agent nothing: no file system, no terminal. Any request
//! the agent makes is refused, and permission requests are answered "cancelled" (the tool broker
//! asks on the stick for anything that needs approval).

use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

/// Longest line read from the agent (a guard against a runaway process).
const MAX_LINE: usize = 4 << 20;

pub struct Acp {
    stdin: tokio::sync::Mutex<Box<dyn AsyncWrite + Send + Unpin>>,
    pending: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    next: AtomicU64,
    updates: Mutex<Option<mpsc::UnboundedSender<Value>>>,
    alive: AtomicBool,
}

impl Acp {
    /// Talks to an agent over `stdin`/`stdout` (a child process's pipes, or test streams).
    pub fn new<R, W>(stdout: R, stdin: W) -> Arc<Self>
    where
        R: tokio::io::AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        let me = Arc::new(Self {
            stdin: tokio::sync::Mutex::new(Box::new(stdin)),
            pending: Mutex::new(HashMap::new()),
            next: AtomicU64::new(0),
            updates: Mutex::new(None),
            alive: AtomicBool::new(true),
        });
        let weak = Arc::downgrade(&me);
        tokio::spawn(read(BufReader::new(stdout), weak));
        me
    }

    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Where `session/update` notifications go (their params).
    pub fn set_updates(&self, tx: Option<mpsc::UnboundedSender<Value>>) {
        *self.updates.lock().unwrap() = tx;
    }

    async fn write(&self, msg: &Value) -> Result<(), String> {
        let mut line = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
        line.push(b'\n');
        let mut w = self.stdin.lock().await;
        w.write_all(&line).await.map_err(|e| format!("the agent stopped: {e}"))?;
        w.flush().await.map_err(|e| format!("the agent stopped: {e}"))
    }

    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, String> {
        if !self.alive() {
            return Err("the agent stopped".into());
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        if let Err(e) = self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).await {
            self.pending.lock().unwrap().remove(&id);
            return Err(e);
        }
        let reply = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => return Err("the agent stopped".into()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                return Err(format!("the agent didn't answer {method} in time"));
            }
        };
        if let Some(e) = reply.get("error") {
            let msg = e.get("message").and_then(Value::as_str).unwrap_or("error");
            return Err(match e.get("data").and_then(Value::as_str) {
                Some(data) => format!("{method}: {msg} ({})", data.chars().take(200).collect::<String>()),
                None => format!("{method}: {msg}"),
            });
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    pub async fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.write(&json!({"jsonrpc": "2.0", "method": method, "params": params})).await
    }

    async fn handle(&self, msg: Value) {
        let method = msg.get("method").and_then(Value::as_str);
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        match (method, id) {
            (None, Some(id)) => {
                if let Some(tx) = id.as_u64().and_then(|n| self.pending.lock().unwrap().remove(&n)) {
                    let _ = tx.send(msg);
                }
            }
            (Some("session/update"), None) => {
                if let Some(tx) = self.updates.lock().unwrap().as_ref() {
                    let _ = tx.send(msg.get("params").cloned().unwrap_or(Value::Null));
                }
            }
            (Some("session/request_permission"), Some(id)) => {
                log::warn!("local assistant: the agent asked for a permission; refused");
                let _ = self
                    .write(&json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": {"outcome": "cancelled"}}}))
                    .await;
            }
            (Some(m), Some(id)) => {
                log::warn!("local assistant: refused agent request {m}");
                let _ = self
                    .write(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "not supported"}}))
                    .await;
            }
            _ => {}
        }
    }
}

async fn read<R: tokio::io::AsyncBufRead + Unpin>(mut r: R, me: Weak<Acp>) {
    let mut line = Vec::new();
    loop {
        line.clear();
        let n: usize = (&mut r).take(MAX_LINE as u64).read_until(b'\n', &mut line).await.unwrap_or_default();
        if n == 0 || (n >= MAX_LINE && line.last() != Some(&b'\n')) {
            break;
        }
        let Some(me) = me.upgrade() else { return };
        if let Ok(msg) = serde_json::from_slice::<Value>(&line) {
            me.handle(msg).await;
        }
    }
    if let Some(me) = me.upgrade() {
        me.alive.store(false, Ordering::Release);
        me.pending.lock().unwrap().clear(); // waiting requests fail at once
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    /// A scripted agent on the other end of two pipes.
    async fn pair() -> (Arc<Acp>, tokio::io::Lines<BufReader<tokio::io::DuplexStream>>, tokio::io::DuplexStream) {
        let (client_out, agent_in) = tokio::io::duplex(1 << 16);
        let (agent_out, client_in) = tokio::io::duplex(1 << 16);
        (Acp::new(client_in, client_out), BufReader::new(agent_in).lines(), agent_out)
    }

    #[tokio::test]
    async fn requests_updates_and_refusals() {
        let (acp, mut from_client, mut to_client) = pair().await;
        let (tx, mut updates) = mpsc::unbounded_channel();
        acp.set_updates(Some(tx));
        let a2 = acp.clone();
        let call = tokio::spawn(async move { a2.request("session/new", json!({}), Duration::from_secs(5)).await });
        let sent: Value = serde_json::from_str(&from_client.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(sent["method"], "session/new");
        let id = sent["id"].clone();
        // the agent asks for a terminal and a permission before answering
        for m in [
            json!({"jsonrpc":"2.0","id":"t1","method":"terminal/create","params":{"command":"rm"}}),
            json!({"jsonrpc":"2.0","id":7,"method":"session/request_permission","params":{"options":[{"optionId":"a","kind":"allow_once"}]}}),
            json!({"jsonrpc":"2.0","method":"session/update","params":{"update":{"sessionUpdate":"agent_message_chunk"}}}),
            json!({"jsonrpc":"2.0","id":id,"result":{"sessionId":"s1"}}),
        ] {
            to_client.write_all(format!("{m}\n").as_bytes()).await.unwrap();
        }
        assert_eq!(call.await.unwrap().unwrap(), json!({"sessionId":"s1"}));
        let r1: Value = serde_json::from_str(&from_client.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!((r1["id"].as_str(), r1["error"]["code"].as_i64()), (Some("t1"), Some(-32601)));
        let r2: Value = serde_json::from_str(&from_client.next_line().await.unwrap().unwrap()).unwrap();
        assert_eq!(r2["result"]["outcome"]["outcome"], "cancelled"); // never "allow"
        assert_eq!(updates.recv().await.unwrap()["update"]["sessionUpdate"], "agent_message_chunk");
    }

    #[tokio::test]
    async fn a_stopped_agent_fails_waiting_requests() {
        let (acp, _from, to_client) = pair().await;
        let a2 = acp.clone();
        let call = tokio::spawn(async move { a2.request("session/prompt", json!({}), Duration::from_secs(30)).await });
        tokio::time::sleep(Duration::from_millis(50)).await;
        drop(to_client);
        assert!(call.await.unwrap().is_err());
        assert!(!acp.alive());
        assert!(acp.request("x", json!({}), Duration::from_secs(1)).await.is_err());
    }

    #[tokio::test]
    async fn errors_and_timeouts() {
        let (acp, mut from_client, mut to_client) = pair().await;
        let a2 = acp.clone();
        let call = tokio::spawn(async move { a2.request("initialize", json!({}), Duration::from_secs(5)).await });
        let sent: Value = serde_json::from_str(&from_client.next_line().await.unwrap().unwrap()).unwrap();
        let m = json!({"jsonrpc":"2.0","id":sent["id"],"error":{"code":1,"message":"nope"}});
        to_client.write_all(format!("{m}\n").as_bytes()).await.unwrap();
        assert_eq!(call.await.unwrap().unwrap_err(), "initialize: nope");
        assert!(acp.request("slow", json!({}), Duration::from_millis(20)).await.unwrap_err().contains("in time"));
    }
}
