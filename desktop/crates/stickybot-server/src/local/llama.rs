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
    _tracked: tempdir::Tracked,
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

/// The server's arguments; the key is read from `key_file` (not on the command line, where `ps`
/// would show it to every local user).
pub fn args(model: &Path, port: u16, key_file: &Path) -> Vec<OsString> {
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
    ] {
        a.push(s.into());
    }
    a.push("--api-key-file".into());
    a.push(key_file.into());
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
            // alive isn't enough: a server that stopped answering (502s for every later question)
            // is replaced, and its last log lines say why
            if health(r.port).await {
                return Ok((r.port, r.key.clone()));
            }
            let log = std::fs::read_to_string(r._scratch.path().join("llama.log")).unwrap_or_default();
            let tail: String = log.lines().rev().take(5).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" | ");
            log::warn!("local model stopped answering; restarting it. Last log: {}", crate::talk::cut(&tail, 600));
            let _ = r.child.start_kill();
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
            app_port: None,
        };
        // confine() runs the sandbox self-test first and refuses to start anything if it fails;
        // the app owns the TCP port and relays it to a Unix socket in scratch (sandbox/listener.rs)
        let key_file = scratch.path().join("api-key");
        write_private(&key_file, &key)?;
        let mut cmd = sandbox::confine_async(server, &args(model, port, &key_file), &policy)
            .await
            .map_err(|e| format!("sandbox check failed, not starting the model: {e}"))?;
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(scratch.path().join("llama.log")).map_err(|e| e.to_string())?)
            .kill_on_drop(true);
        let mut child = cmd.spawn().map_err(|e| format!("llama-server: {e}"))?;
        let tracked = tempdir::Tracked::new(child.id(), scratch.path());
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
        *running = Some(Running { child, port, key: key.clone(), _tracked: tracked, _scratch: scratch });
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

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| e.to_string())?;
    f.write_all(text.as_bytes()).map_err(|e| e.to_string())
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

/// Private temporary folders for the sandboxed processes, and their cleanup after a hard quit.
pub mod tempdir {
    use std::path::{Path, PathBuf};

    /// Folder-name prefixes of our scratch folders (in the temp folder).
    pub const PREFIXES: [&str; 2] = ["stickybot-llama-", "stickybot-goose-"];
    const RECORD: &str = "stickybot-local-";

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

    /// A running sandboxed process and its folder, written down outside the sandbox (the process
    /// can't change it) so [`sweep`] can stop it and remove the folder if the app quits hard.
    pub struct Tracked(PathBuf);
    impl Tracked {
        pub fn new(pid: Option<u32>, dir: &Path) -> Self {
            let p = std::env::temp_dir().join(format!("{RECORD}{}.pid", crate::config::new_token()));
            if let Some(pid) = pid {
                // the third line is the app that started it: another StickyBot never stops it
                let _ = super::write_private(&p, &format!("{pid}\n{}\n{}\n", dir.display(), std::process::id()));
            }
            Self(p)
        }
    }
    impl Drop for Tracked {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// A record whose app (its third line) is another process that is still running: a second
    /// StickyBot (the agent test runner, a test build) shares the runtime and keeps its model.
    fn owned_by_another_running_app(record: &str) -> bool {
        let owner = record.lines().nth(2).and_then(|l| l.trim().parse::<i32>().ok()).filter(|p| *p > 1);
        owner.is_some_and(|o| o as u32 != std::process::id() && unsafe { libc::kill(o, 0) } == 0)
    }

    /// The executable a process runs, if it's alive.
    fn command_of(pid: u32) -> Option<PathBuf> {
        let out = std::process::Command::new("/bin/ps").args(["-o", "comm=", "-p", &pid.to_string()]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // compare real paths: /var/… and /private/var/… are the same folder
        (out.status.success() && !s.is_empty()).then(|| {
            let p = PathBuf::from(s);
            p.canonicalize().unwrap_or(p)
        })
    }

    /// Stops every recorded process that runs one of our pinned programs (under `runtime_root`)
    /// and removes its scratch folder. At start-up (after a crash) and when quitting.
    pub fn sweep(runtime_root: &Path) {
        let tmp = std::env::temp_dir();
        let root = runtime_root.canonicalize().unwrap_or_else(|_| runtime_root.to_path_buf());
        let Ok(entries) = std::fs::read_dir(&tmp) else { return };
        let mut stale = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if PREFIXES.iter().any(|p| name.starts_with(p)) {
                stale.push(e.path());
                continue;
            }
            if !(name.starts_with(RECORD) && name.ends_with(".pid")) {
                continue;
            }
            let text = std::fs::read_to_string(e.path()).unwrap_or_default();
            if owned_by_another_running_app(&text) {
                continue;
            }
            let mut lines = text.lines();
            if let Some(pid) = lines.next().and_then(|p| p.parse::<i32>().ok()).filter(|p| *p > 1)
                && command_of(pid as u32).is_some_and(|c| c.starts_with(&root))
            {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
                log::info!("local assistant: stopped a leftover process ({pid})");
            }
            if let Some(dir) = lines.next().map(PathBuf::from) {
                remove_scratch(&dir);
            }
            let _ = std::fs::remove_file(e.path());
        }
        // unrecorded folders (an older version, a crash before the record): only old ones, so a
        // starting process in this run keeps its folder
        let old = std::time::Duration::from_secs(3600);
        for dir in stale {
            let age = std::fs::metadata(&dir).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok());
            if age.is_some_and(|a| a > old) {
                remove_scratch(&dir.canonicalize().unwrap_or(dir));
            }
        }
    }

    /// Removes one of our scratch folders (only those: a direct child of the temp folder with our prefix).
    fn remove_scratch(dir: &Path) {
        let tmp = std::env::temp_dir().canonicalize().unwrap_or_else(|_| std::env::temp_dir());
        let ours = dir.parent() == Some(tmp.as_path())
            && dir.file_name().is_some_and(|n| PREFIXES.iter().any(|p| n.to_string_lossy().starts_with(p)));
        if ours {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_args_bind_loopback_with_a_key_and_no_thinking() {
        let a: Vec<String> =
            args(Path::new("/m.gguf"), 4321, Path::new("/s/key")).iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let at = |k: &str| a.iter().position(|x| x == k).map(|i| a[i + 1].as_str());
        assert_eq!(at("--host"), Some("127.0.0.1"));
        assert_eq!(at("--port"), Some("4321"));
        assert_eq!(at("--api-key-file"), Some("/s/key"));
        assert!(!a.iter().any(|x| x == "--api-key")); // never the key itself
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

    /// A leftover sandboxed process of ours is stopped and its folder removed; others are left alone.
    #[test]
    fn sweep_leaves_another_running_apps_model_alone() {
        let root = std::env::temp_dir().join(format!("stickybot-sweep-owner-{}", crate::config::new_token()));
        std::fs::create_dir_all(&root).unwrap();
        let model = root.join("sleep");
        std::fs::copy("/bin/sleep", &model).unwrap();
        let mut child = std::process::Command::new(&model).arg("30").spawn().unwrap();
        let mut other_app = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let record = std::env::temp_dir().join(format!("stickybot-local-{}.pid", crate::config::new_token()));
        std::fs::write(&record, format!("{}\n/nonexistent\n{}\n", child.id(), other_app.id())).unwrap();
        tempdir::sweep(&root);
        assert!(child.try_wait().unwrap().is_none(), "another app's model keeps running");
        assert!(record.exists());
        // once that app is gone, its leftover is ours to stop
        other_app.kill().unwrap();
        other_app.wait().unwrap();
        tempdir::sweep(&root);
        assert!(child.wait().unwrap().code().is_none());
        assert!(!record.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn sweep_stops_only_our_leftovers() {
        let root = std::env::temp_dir().join(format!("stickybot-sweep-root-{}", crate::config::new_token()));
        std::fs::create_dir_all(&root).unwrap();
        let ours = root.join("sleep");
        std::fs::copy("/bin/sleep", &ours).unwrap();
        let mut a = std::process::Command::new(&ours).arg("30").spawn().unwrap();
        let mut b = std::process::Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let dir_a = tempdir::Dir::new("stickybot-goose").unwrap();
        let kept = dir_a.path().to_path_buf();
        std::mem::forget(dir_a); // as after a hard quit
        let ra = tempdir::Tracked::new(Some(a.id()), &kept);
        let rb = tempdir::Tracked::new(Some(b.id()), Path::new("/tmp"));
        std::mem::forget((ra, rb));
        tempdir::sweep(&root);
        assert!(a.wait().unwrap().code().is_none()); // killed
        assert!(b.try_wait().unwrap().is_none()); // not ours: still running
        assert!(!kept.exists());
        assert!(Path::new("/tmp").exists());
        let _ = b.kill();
        let _ = b.wait();
        std::fs::remove_dir_all(&root).unwrap();
    }
}
