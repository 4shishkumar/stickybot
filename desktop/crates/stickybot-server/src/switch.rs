//! The switch-over from the Python app: this app takes its place on port 8660 with its token, so
//! everything that talks to "StickyBot" keeps working unchanged — the Claude Code hooks
//! (`hook.py` approvals, `claude_hooks.py` "Claude finished" / sessions), terminal sessions' stick
//! tools (`claude mcp add … http://127.0.0.1:8660/mcp` with that token), bookmarks of the dashboard.
//!
//! `POST /api/switch-over {"confirm": true}` (only while nothing answers on 8660, i.e. the Python
//! app is quit): copies the Python app's token (read-only there) into this app's `token` (this
//! app's own is kept as `token.before-switch`) and writes the marker `switched-over`. From the next
//! start on: port 8660 (unless `$STICKYBOT_RS_PORT`), and no "the Python agent has the stick" check
//! (it would ask itself). `{"undo": true}` removes the marker (back to 8670 and the check; the
//! token stays). Import the settings first (`migrate.rs`).

use crate::config::Paths;
use std::path::Path;
use std::time::Duration;

pub const MARKER: &str = "switched-over";
/// The Python app's port, which this app takes over.
pub const PORT: u16 = crate::python_agent::PORT;

pub fn switched(home: &Path) -> bool {
    home.join(MARKER).exists()
}

/// Something answers on 127.0.0.1:`port` (the Python app, still running).
pub async fn listening(port: u16) -> bool {
    matches!(
        tokio::time::timeout(Duration::from_millis(500), tokio::net::TcpStream::connect(("127.0.0.1", port))).await,
        Ok(Ok(_))
    )
}

/// Takes the Python app's token and writes the marker. `python_port` must be free.
pub async fn switch_over(paths: &Paths, from: &Path, python_port: u16) -> Result<(), String> {
    if listening(python_port).await {
        return Err(format!(
            "something still answers on port {python_port}: quit the StickyBot (Python) app first (menu bar → Quit)"
        ));
    }
    let token = std::fs::read_to_string(from.join("token")).map_err(|e| format!("the Python app's token: {e}"))?;
    let token = token.trim();
    if token.is_empty() {
        return Err("the Python app's token is empty".into());
    }
    let mine = paths.token();
    if let Ok(old) = std::fs::read_to_string(&mine)
        && old.trim() != token
    {
        let _ = std::fs::write(paths.home.join("token.before-switch"), old);
    }
    write_private(&mine, token).map_err(|e| format!("{}: {e}", mine.display()))?;
    std::fs::write(paths.home.join(MARKER), format!("{}\n", crate::hub::now())).map_err(|e| e.to_string())?;
    Ok(())
}

/// The Python app's LaunchAgent (`stickybot start` / its installer): `RunAtLoad` + `KeepAlive`, so
/// it starts at every login and takes port 8660 back unless it's disabled (SWITCHOVER.md step 3).
pub const PYTHON_LABEL: &str = "com.stickkey.agent";

/// Where the Python app's LaunchAgent plist is, if it's installed (`~/Library/LaunchAgents`, or the
/// Python app's folder when that wasn't writable). Only looked at, never changed.
pub fn python_login_item(user_home: &Path, python_home: Option<&Path>) -> Option<std::path::PathBuf> {
    let name = format!("{PYTHON_LABEL}.plist");
    std::iter::once(user_home.join("Library/LaunchAgents").join(&name))
        .chain(python_home.map(|p| p.join(&name)))
        .find(|p| p.exists())
}

/// The marker's note for a Mac that never had the Python app.
pub const FRESH_NOTE: &str = "fresh install: no StickyBot (Python) app on this Mac";

/// A Mac without the Python app: on this app's **first start** (no `config.json` and no marker yet
/// in `home`), when there's no Python app folder (`python_home`, `~/.stick-agent`) and nothing
/// answers on `python_port`, this app is *the* StickyBot right away: it writes the marker (with
/// [`FRESH_NOTE`]; its own token stays), so it serves port 8660 as "StickyBot" without the "the
/// Python agent has the stick" check. With the Python app there (its folder, or something on
/// 8660) nothing changes: side by side on 8670 until the switch-over (SWITCHOVER.md). Never
/// again after the first start, so Settings → Switch over → Undo sticks. -> it adopted.
pub fn adopt_if_fresh(home: &Path, python_home: Option<&Path>, python_port: u16, listening: impl Fn(u16) -> bool) -> bool {
    if switched(home) || home.join("config.json").exists() {
        return false;
    }
    if python_home.is_some_and(Path::exists) || listening(python_port) {
        return false;
    }
    let ok = std::fs::create_dir_all(home)
        .and_then(|_| std::fs::write(home.join(MARKER), format!("{}\n{FRESH_NOTE}\n", crate::hub::now())));
    match ok {
        Ok(()) => {
            log::info!("no StickyBot (Python) app on this Mac: this is StickyBot (port {PORT})");
            true
        }
        Err(e) => {
            log::warn!("{}: {e}", home.join(MARKER).display());
            false
        }
    }
}

/// Something answers on 127.0.0.1:`port` (blocking, 500 ms; for start-up, before the runtime).
pub fn listening_now(port: u16) -> bool {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)).is_ok()
}

/// The marker says this app never had a Python app to take over from.
pub fn fresh(home: &Path) -> bool {
    std::fs::read_to_string(home.join(MARKER)).is_ok_and(|t| t.contains(FRESH_NOTE))
}

pub fn undo(paths: &Paths) -> Result<(), String> {
    match std::fs::remove_file(paths.home.join(MARKER)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

fn write_private(p: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(p)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_home(name: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("stickybot-fresh-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_mac_without_the_python_app_is_stickybot_from_the_first_start() {
        let root = fake_home("none");
        let (home, py) = (root.join(".stickybot-rs"), root.join(".stick-agent"));
        assert!(adopt_if_fresh(&home, Some(&py), 8660, |_| false));
        assert!(switched(&home) && fresh(&home));
        // the app then starts as StickyBot on 8660 (lib.rs: Options / app_name), no Python gate
        assert_eq!(crate::app_name(PORT), "StickyBot");
        // once only: an undo (marker gone, config saved) stays undone
        let paths = Paths::new(home.clone()).unwrap();
        paths.ensure().unwrap();
        crate::config::save(&paths, &crate::config::defaults()).unwrap();
        undo(&paths).unwrap();
        assert!(!adopt_if_fresh(&home, Some(&py), 8660, |_| false));
        assert!(!switched(&home));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn with_the_python_app_nothing_changes() {
        // its folder is there (not running)
        let root = fake_home("py");
        let (home, py) = (root.join(".stickybot-rs"), root.join(".stick-agent"));
        std::fs::create_dir_all(&py).unwrap();
        assert!(!adopt_if_fresh(&home, Some(&py), 8660, |_| false));
        assert!(!switched(&home) && !home.join(MARKER).exists());
        // no folder, but something answers on 8660 (a Python app with another home)
        std::fs::remove_dir_all(&py).unwrap();
        let asked = std::sync::Mutex::new(Vec::new());
        assert!(!adopt_if_fresh(&home, Some(&py), 8660, |p| {
            asked.lock().unwrap().push(p);
            true
        }));
        assert_eq!(*asked.lock().unwrap(), [8660]);
        assert!(!switched(&home));
        // not a first start (this app ran before, side by side): never on its own
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.json"), "{}").unwrap();
        assert!(!adopt_if_fresh(&home, Some(&py), 8660, |_| false));
        // a real switch-over's marker isn't a fresh one
        std::fs::write(home.join(MARKER), "1700000000\n").unwrap();
        assert!(switched(&home) && !fresh(&home));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn listening_now_sees_a_listener() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(listening_now(port));
        drop(l);
        assert!(!listening_now(port));
    }

    #[tokio::test]
    async fn takes_the_token_only_once_the_python_app_is_gone() {
        let root = std::env::temp_dir().join(format!("stickybot-switch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let py = root.join(".stick-agent");
        std::fs::create_dir_all(&py).unwrap();
        std::fs::write(py.join("token"), "python-token\n").unwrap();
        let paths = Paths::new(root.join(".stickybot-rs")).unwrap();
        paths.ensure().unwrap();
        let mine = paths.token_value().unwrap();
        // a fake "Python app" still listening
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        let e = switch_over(&paths, &py, port).await.unwrap_err();
        assert!(e.contains("quit the StickyBot (Python) app first"), "{e}");
        assert!(!switched(&paths.home));
        drop(l);
        switch_over(&paths, &py, port).await.unwrap();
        assert!(switched(&paths.home));
        assert_eq!(paths.token_value().unwrap(), "python-token");
        assert_eq!(std::fs::read_to_string(paths.home.join("token.before-switch")).unwrap(), mine);
        assert_eq!(std::fs::read_to_string(py.join("token")).unwrap(), "python-token\n"); // untouched
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(paths.token()).unwrap().permissions().mode() & 0o777, 0o600);
        }
        undo(&paths).unwrap();
        assert!(!switched(&paths.home));
        undo(&paths).unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn finds_the_python_launch_agent() {
        let root = std::env::temp_dir().join(format!("stickybot-switch-la-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let py = root.join(".stick-agent");
        std::fs::create_dir_all(&py).unwrap();
        assert_eq!(python_login_item(&root, Some(&py)), None);
        std::fs::write(py.join("com.stickkey.agent.plist"), "").unwrap();
        assert_eq!(python_login_item(&root, Some(&py)), Some(py.join("com.stickkey.agent.plist")));
        let la = root.join("Library/LaunchAgents");
        std::fs::create_dir_all(&la).unwrap();
        std::fs::write(la.join("com.stickkey.agent.plist"), "").unwrap();
        assert_eq!(python_login_item(&root, Some(&py)), Some(la.join("com.stickkey.agent.plist")));
        let _ = std::fs::remove_dir_all(&root);
    }
}
