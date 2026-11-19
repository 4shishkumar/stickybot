//! llama.cpp's server for the local model: started on demand under the sandbox, on a loopback
//! port with its own key (only the inference proxy knows it), stopped after a while unused.

use crate::sandbox;
use std::ffi::OsString;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::process::Child;

pub const IDLE_STOP: Duration = Duration::from_secs(10 * 60);
const START_WAIT: Duration = Duration::from_secs(90);
pub const CONTEXT: u32 = 16384;
/// The name the proxy asks for, whatever the client sends.
pub const ALIAS: &str = "local";

pub struct Running {
    child: Child,
    pub port: u16,
    pub key: String,
    _scratch: tempdir::Dir,
}

#[derive(Default)]
pub struct Llama {
    running: tokio::sync::Mutex<Option<Running>>,
    last_used: Mutex<Option<Instant>>,
}

pub fn free_port() -> std::io::Result<u16> {
    Ok(std::net::TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

pub fn args(model: &Path, port: u16, key: &str) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec!["-m".into(), model.into()];
    for s in [
        "--alias",
        ALIAS,
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "-c",
        &CONTEXT.to_string(),
        "-ngl",
        "99",
        "--jinja",
        "-np",
        "1",
        "--no-webui",
        "--chat-template-kwargs",
        r#"{"enable_thinking":false}"#,
        "--api-key",
        key,
    ] {
        a.push(s.into());
    }
    a
}

impl Llama {
    pub fn touch(&self) {
        *self.last_used.lock().unwrap() = Some(Instant::now());
    }

    pub async fn loaded(&self) -> bool {
        self.running.lock().await.as_mut().is_some_and(|r| matches!(r.child.try_wait(), Ok(None)))
    }

    /// (port, key) of a running server, starting it if needed.
    pub async fn ensure(&self, server: &Path, model: &Path) -> Result<(u16, String), String> {
        self.touch();
        let mut running = self.running.lock().await;
        if let Some(r) = running.as_mut()
            && matches!(r.child.try_wait(), Ok(None))
        {
            return Ok((r.port, r.key.clone()));
        }
        *running = None;
        let port = free_port().map_err(|e| e.to_string())?;
        let key = crate::config::new_token();
        let scratch = tempdir::Dir::new("stickybot-llama")?;
        let mut read_only = vec![model.to_path_buf()];
        if let Some(dir) = server.parent() {
            read_only.push(dir.to_path_buf());
        }
        let policy = sandbox::Policy {
            scratch: scratch.path().to_path_buf(),
            read_only,
            loopback_ports: vec![],
            listen_ports: vec![port],
        };
        sandbox::self_test(&policy).map_err(|e| format!("sandbox check failed, not starting the model: {e}"))?;
        let mut cmd = sandbox::confine(server, &args(model, port, &key), &policy)?;
        cmd.env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", scratch.path())
            .env("TMPDIR", scratch.path())
            .current_dir(scratch.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(scratch.path().join("llama.log")).map_err(|e| e.to_string())?)
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("llama-server: {e}"))?;
        let started = Instant::now();
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                let log = std::fs::read_to_string(scratch.path().join("llama.log")).unwrap_or_default();
                let tail: String = log.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
                return Err(format!("the local model stopped ({status}): {tail}"));
            }
            if health(port).await {
                break;
            }
            if started.elapsed() > START_WAIT {
                return Err("the local model took too long to load".into());
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        log::info!("local model up on 127.0.0.1:{port} in {:?}", started.elapsed());
        *running = Some(Running { child, port, key: key.clone(), _scratch: scratch });
        Ok((port, key))
    }

    /// Stops the server if it's been unused for `idle` (frees ~6 GB).
    pub async fn stop_if_idle(&self, idle: Duration) {
        let unused = self.last_used.lock().unwrap().is_none_or(|t| t.elapsed() >= idle);
        if unused && let Some(mut r) = self.running.lock().await.take() {
            let _ = r.child.kill().await;
            log::info!("local model stopped (idle)");
        }
    }

    pub async fn stop(&self) {
        if let Some(mut r) = self.running.lock().await.take() {
            let _ = r.child.kill().await;
        }
    }
}

async fn health(port: u16) -> bool {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let Ok(Ok(mut s)) =
        tokio::time::timeout(Duration::from_secs(1), tokio::net::TcpStream::connect(("127.0.0.1", port))).await
    else {
        return false;
    };
    if s.write_all(b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").await.is_err() {
        return false;
    }
    let mut buf = [0u8; 32];
    let n = tokio::time::timeout(Duration::from_secs(2), s.read(&mut buf)).await.ok().and_then(Result::ok).unwrap_or(0);
    buf[..n].starts_with(b"HTTP/1.1 200")
}

/// A private temporary folder, removed when dropped.
pub mod tempdir {
    use std::path::{Path, PathBuf};

    pub struct Dir(PathBuf);
    impl Dir {
        pub fn new(prefix: &str) -> Result<Self, String> {
            use std::os::unix::fs::DirBuilderExt;
            let p = std::env::temp_dir().join(format!("{prefix}-{}", crate::config::new_token()));
            std::fs::DirBuilder::new().mode(0o700).create(&p).map_err(|e| e.to_string())?;
            // the sandbox profile matches real paths (/private/var/…, not /var/…)
            Ok(Self(p.canonicalize().map_err(|e| e.to_string())?))
        }
        pub fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_args_bind_loopback_with_a_key_and_no_thinking() {
        let a: Vec<String> =
            args(Path::new("/m.gguf"), 4321, "k3y").iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let at = |k: &str| a.iter().position(|x| x == k).map(|i| a[i + 1].as_str());
        assert_eq!(at("--host"), Some("127.0.0.1"));
        assert_eq!(at("--port"), Some("4321"));
        assert_eq!(at("--api-key"), Some("k3y"));
        assert_eq!(at("-m"), Some("/m.gguf"));
        assert!(a.iter().any(|x| x == "--no-webui"));
        assert!(at("--chat-template-kwargs").unwrap().contains("\"enable_thinking\":false"));
    }

    #[tokio::test]
    async fn refuses_to_start_unsandboxed() {
        let l = Llama::default();
        let e = l.ensure(Path::new("/bin/sleep"), Path::new("/dev/null")).await;
        // not a model server: either the sandbox check or the start fails, and nothing runs
        assert!(e.is_err());
        assert!(!l.loaded().await);
    }
}
