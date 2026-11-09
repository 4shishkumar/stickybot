//! `POST /api/restart` and `POST /api/window` for the menu bar app (the server's `AppControl`).

use stickybot_server::appctl::AppControl;
use tauri::AppHandle;

pub struct TauriApp(pub AppHandle);

impl AppControl for TauriApp {
    /// Restarts after the answer went out; the `Exit` event still runs (the wireless mic's device
    /// is removed first).
    fn restart(&self) -> Result<(), String> {
        let app = self.0.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            app.request_restart();
        });
        Ok(())
    }

    fn show_window(&self, page: &str) -> bool {
        let (app, page) = (self.0.clone(), page.to_string());
        let h = app.clone();
        app.run_on_main_thread(move || crate::show_dashboard_at(&h, &page)).is_ok()
    }
}
