//! `POST /api/restart` and `POST /api/window` (server.py `restart` / `window`): what only the menu
//! bar app can do. The server sees this trait; the Tauri app sets the real one
//! (`stickybot-app/src/main.rs`), the headless binary and the tests have none / a fake.

pub trait AppControl: Send + Sync {
    /// Restarts the app soon (after the answer went out). Err: why it can't.
    fn restart(&self) -> Result<(), String>;
    /// Shows and focuses the dashboard window at `#page` ("" = where it is). false: no window here
    /// (the caller opens a browser instead).
    fn show_window(&self, page: &str) -> bool;
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
