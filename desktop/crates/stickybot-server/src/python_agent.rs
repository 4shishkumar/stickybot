//! The stick takes one connection, and the Python agent normally has it. Before every connect the
//! Rust link asks the Python agent's own API (`GET http://127.0.0.1:8660/api/status`, no files, no
//! token needed) whether its link is up; if it is, the Rust app waits. Pause the Python agent
//! (its dashboard → Overview → Pause) and the Rust app connects within a few seconds.
//! `STICKYBOT_RS_ALLOW_SHARED=1` turns this off.

use serde_json::Value;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub const PORT: u16 = 8660;

/// From a `/api/status` body: why not to connect, or `None` (paused, no stick, not running).
pub fn holds_stick(status_body: &str, port: u16) -> Option<String> {
    let v: Value = serde_json::from_str(status_body).ok()?;
    let link = v.get("link")?.as_str()?;
    matches!(link, "connected" | "searching" | "starting").then(|| {
        format!(
            "the Python agent (port {port}) has the stick (its link: {link}); pause it in its dashboard to use this app"
        )
    })
}

/// Asks the running Python agent (1 s at most; nothing listening = go ahead).
pub async fn check(port: u16) -> Option<String> {
    let ask = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.ok()?;
        let req = format!("GET /api/status HTTP/1.0\r\nHost: localhost:{port}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await.ok()?;
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await.ok()?;
        let text = String::from_utf8_lossy(&buf);
        let body = text.split_once("\r\n\r\n")?.1.to_string();
        Some(body)
    };
    let body = tokio::time::timeout(Duration::from_secs(1), ask).await.ok().flatten()?;
    holds_stick(&body, port)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_connected_or_searching_agent_holds_the_stick() {
        assert!(holds_stick(r#"{"link":"connected","state":"ready"}"#, 8660).unwrap().contains("pause it"));
        assert!(holds_stick(r#"{"link":"searching"}"#, 8660).is_some());
        assert_eq!(holds_stick(r#"{"link":"paused"}"#, 8660), None);
        assert_eq!(holds_stick(r#"{"link":"no-stick"}"#, 8660), None);
        assert_eq!(holds_stick("not json", 8660), None);
    }

    #[tokio::test]
    async fn nobody_listening_means_go_ahead() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert_eq!(check(port).await, None);
    }

    #[tokio::test]
    async fn it_reads_a_live_status() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut b = [0u8; 512];
            let _ = s.read(&mut b).await;
            let body = r#"{"link":"connected"}"#;
            let resp = format!(
                "HTTP/1.0 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            s.write_all(resp.as_bytes()).await.unwrap();
        });
        assert!(check(port).await.is_some());
    }
}
