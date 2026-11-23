//! `POST /api/restart` and `POST /api/window` (server.py `restart` / `window`): what only the menu
//! bar app can do. The server sees this trait; the Tauri app sets the real one
//! (`stickybot-app/src/main.rs`), the headless binary and the tests have none / a fake.

pub trait AppControl: Send + Sync {
    /// Restarts the app soon (after the answer went out). Err: why it can't.
    fn restart(&self) -> Result<(), String>;
    /// Shows and focuses the dashboard window at `#page` ("" = where it is). false: no window here
    /// (the caller opens a browser instead).
    fn show_window(&self, page: &str) -> bool;

    /// On failure the currently registered shortcut stays active. Called on a blocking worker.
    fn configure_voice(&self, _enabled: bool, _shortcut: &str) -> Result<(), String> {
        Err("The keyboard shortcut requires the StickyBot Mac app".into())
    }

    /// The microphone (a Core Audio device UID, "" = the system default) and the start/stop sounds.
    fn voice_options(&self, _input: &str, _sounds: bool) -> Result<(), String> {
        Err("The keyboard shortcut requires the StickyBot Mac app".into())
    }

    /// Starts (or stops) a few seconds of listening so Settings can show the microphone's level.
    fn voice_test(&self, _on: bool) -> Result<(), String> {
        Err("The microphone test requires the StickyBot Mac app".into())
    }

    /// Reports microphone permission, input name, and any native error. Null means unsupported.
    fn voice_status(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

pub const DEFAULT_VOICE_SHORTCUT: &str = "Control+Shift+Space";

/// Canonical modifier order, then the physical DOM key code (layout-independent).
pub fn valid_voice_shortcut(shortcut: &str) -> bool {
    let Some((mods, key)) = shortcut.rsplit_once('+') else { return false };
    let valid_key = key == "Space"
        || key.strip_prefix("Key").is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_uppercase())
        || key.strip_prefix("Digit").is_some_and(|s| s.len() == 1 && s.as_bytes()[0].is_ascii_digit());
    let mut order = ["Control", "Option", "Shift", "Command"].into_iter();
    valid_key && mods != "Shift" && mods.split('+').all(|m| order.any(|allowed| m == allowed))
}

/// The headless binary: no window, and it can't restart itself.
pub struct NoApp;

impl AppControl for NoApp {
    fn restart(&self) -> Result<(), String> {
        Err("only the menu bar app restarts itself — stop stickybot-headless (Ctrl-C) and start it again".into())
    }
    fn show_window(&self, _page: &str) -> bool {
        false
    }
}

/// For tests: records the calls.
#[derive(Default)]
pub struct FakeApp {
    pub restarts: std::sync::Mutex<usize>,
    pub windows: std::sync::Mutex<Vec<String>>,
}

impl AppControl for FakeApp {
    fn restart(&self) -> Result<(), String> {
        *self.restarts.lock().unwrap() += 1;
        Ok(())
    }
    fn show_window(&self, page: &str) -> bool {
        self.windows.lock().unwrap().push(page.to_string());
        true
    }
}

/// A page name the window may be sent to (server.py: letters, digits and `-`).
pub fn valid_page(page: &str) -> bool {
    page.len() <= 40 && page.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}
