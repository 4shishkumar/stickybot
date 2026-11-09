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
}
