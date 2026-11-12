//! The Dock icon, Cmd+Tab and the app menu (window.py did the same for the Python app): a menu
//! bar app (`Accessory`: no Dock icon, not in Cmd+Tab) while the dashboard window is closed, a
//! normal one (`Regular`: Dock icon, Cmd+Tab, the app menu with Quit / Copy / Paste) while it's
//! open. Settings → "Always show in the Dock" (config `dock_always`) keeps it `Regular`.
//!
//! Clicking the Dock icon (`RunEvent::Reopen`) or switching to the app with Cmd+Tab while no
//! window is up shows the dashboard. Closing the window never quits the app.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stickybot_server::AppState;
use tauri::menu::{AboutMetadata, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// Dock icon, Cmd+Tab, the app menu
    Regular,
    /// the menu bar icon only
    Accessory,
}

/// The policy for: is the dashboard window open, and is "Always show in the Dock" on.
pub fn policy(window_open: bool, dock_always: bool) -> Policy {
    if window_open || dock_always { Policy::Regular } else { Policy::Accessory }
}

/// Activations this soon after start are the launch itself (Finder, the login item), not the user
/// switching to the app: no window for those.
pub const LAUNCH_GRACE: Duration = Duration::from_secs(3);

/// The app became active (Cmd+Tab, a click on the Dock icon): show the window? Only while it has a
/// Dock icon (`Regular`), after the launch, and when no window is up (closed, or minimized).
pub fn show_on_activate(policy: Policy, since_start: Duration, window_up: bool) -> bool {
    policy == Policy::Regular && since_start >= LAUNCH_GRACE && !window_up
}

/// `config["dock_always"]`
pub fn dock_always(cfg: &serde_json::Map<String, serde_json::Value>) -> bool {
    cfg.get("dock_always").and_then(serde_json::Value::as_bool).unwrap_or(false)
}

pub struct Dock {
    window_open: AtomicBool,
    /// what was last set (None: nothing yet)
    applied: Mutex<Option<Policy>>,
    started: Instant,
}

impl Default for Dock {
    fn default() -> Self {
        Self { window_open: AtomicBool::new(false), applied: Mutex::new(None), started: Instant::now() }
    }
}

impl Dock {
    pub fn current(&self, app: &AppHandle) -> Policy {
        let always = app.try_state::<Arc<AppState>>().is_some_and(|st| dock_always(&st.cfg.lock().unwrap()));
        policy(self.window_open.load(Ordering::SeqCst), always)
    }

    /// Sets the policy if it changed (the menu bar's refresh calls this every half second, so a
    /// changed setting shows at once).
    pub fn sync(&self, app: &AppHandle) {
        let want = self.current(app);
        let mut applied = self.applied.lock().unwrap();
        if *applied == Some(want) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let p = match want {
                Policy::Regular => tauri::ActivationPolicy::Regular,
                Policy::Accessory => tauri::ActivationPolicy::Accessory,
            };
            if let Err(e) = app.set_activation_policy(p) {
                log::warn!("dock: {e}");
                return;
            }
        }
        log::info!("dock: {}", if want == Policy::Regular { "shown" } else { "hidden (menu bar only)" });
        *applied = Some(want);
    }

    /// The window is about to be shown: Regular first (then focusing it brings the app menu up).
    pub fn window_opened(&self, app: &AppHandle) {
        self.window_open.store(true, Ordering::SeqCst);
        self.sync(app);
    }

    pub fn window_closed(&self, app: &AppHandle) {
        self.window_open.store(false, Ordering::SeqCst);
        self.sync(app);
    }

    /// NSApplicationDidBecomeActive: Cmd+Tab to the app with no window up shows it.
    pub fn activated(&self, app: &AppHandle) {
        let up = app.get_webview_window(crate::window::WINDOW).is_some_and(|w| !w.is_minimized().unwrap_or(false));
        if show_on_activate(self.current(app), self.started.elapsed(), up) {
            crate::show_dashboard_at(app, "");
        }
    }
}

pub fn of(app: &AppHandle) -> Option<Arc<Dock>> {
    app.try_state::<Arc<Dock>>().map(|d| d.inner().clone())
}

pub const MENU_QUIT: &str = "app-quit";
pub const MENU_SETTINGS: &str = "app-settings";
pub const MENU_FEEDBACK: &str = "app-feedback";
pub const MENU_RELOAD: &str = "app-reload";

/// The app menu while it's `Regular` (window.py `_main_menu`): StickyBot, Edit (Copy / Paste in
/// the page's fields), View (Reload), Window, Help.
pub fn app_menu(app: &AppHandle, name: &str) -> tauri::Result<Menu<Wry>> {
    let sep = || PredefinedMenuItem::separator(app);
    let about = AboutMetadata { name: Some(name.to_string()), ..Default::default() };
    let app_sub = Submenu::with_items(
        app,
        name,
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&format!("About {name}")), Some(about))?,
            &sep()?,
            &MenuItem::with_id(app, MENU_SETTINGS, "Settings…", true, Some("cmd+,"))?,
            &sep()?,
            &PredefinedMenuItem::hide(app, Some(&format!("Hide {name}")))?,
            &PredefinedMenuItem::hide_others(app, None)?,
            &PredefinedMenuItem::show_all(app, None)?,
            &sep()?,
            &MenuItem::with_id(app, MENU_QUIT, format!("Quit {name}"), true, Some("cmd+q"))?,
        ],
    )?;
    let edit = Submenu::with_items(
        app,
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(app, None)?,
            &PredefinedMenuItem::redo(app, None)?,
            &sep()?,
            &PredefinedMenuItem::cut(app, None)?,
            &PredefinedMenuItem::copy(app, None)?,
            &PredefinedMenuItem::paste(app, None)?,
            &PredefinedMenuItem::select_all(app, None)?,
        ],
    )?;
    let view = Submenu::with_items(
        app,
        "View",
        true,
        &[&MenuItem::with_id(app, MENU_RELOAD, "Reload", true, Some("cmd+r"))?],
    )?;
    let window = Submenu::with_items(
        app,
        "Window",
        true,
        &[
            &PredefinedMenuItem::minimize(app, None)?,
            &PredefinedMenuItem::maximize(app, Some("Zoom"))?,
            &sep()?,
            &PredefinedMenuItem::close_window(app, None)?,
        ],
    )?;
    let help = Submenu::with_items(
        app,
        "Help",
        true,
        &[&MenuItem::with_id(app, MENU_FEEDBACK, "Report an Issue…", true, None::<&str>)?],
    )?;
    Menu::with_items(app, &[&app_sub, &edit, &view, &window, &help])
}

/// A click in the app menu (the tray's own items have other ids).
pub fn menu_clicked(app: &AppHandle, id: &str) {
    match id {
        MENU_QUIT => app.exit(0),
        MENU_SETTINGS => crate::show_dashboard_at(app, "settings"),
        MENU_FEEDBACK => crate::show_dashboard_at(app, "feedback"),
        MENU_RELOAD => {
            if let Some(w) = app.get_webview_window(crate::window::WINDOW) {
                let _ = w.eval("location.reload()");
            }
        }
        _ => {}
    }
}

/// Calls `on_active` on the main thread each time the app becomes active
/// (NSApplicationDidBecomeActiveNotification).
#[cfg(target_os = "macos")]
pub fn on_activate(on_active: impl Fn() + 'static) {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2_foundation::NSString;
    use std::ptr::{self, NonNull};

    let Some(cls) = AnyClass::get(c"NSNotificationCenter") else { return };
    let center: Option<Retained<AnyObject>> = unsafe { msg_send![cls, defaultCenter] };
    let Some(center) = center else { return };
    let name = NSString::from_str("NSApplicationDidBecomeActiveNotification");
    let block = RcBlock::new(move |_note: NonNull<AnyObject>| on_active());
    let observer: Option<Retained<AnyObject>> = unsafe {
        msg_send![&*center, addObserverForName: &*name, object: ptr::null::<AnyObject>(),
                  queue: ptr::null::<AnyObject>(), usingBlock: &*block]
    };
    std::mem::forget(observer); // for the app's lifetime
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_dock_icon_while_the_window_is_open_or_when_asked() {
        assert_eq!(policy(false, false), Policy::Accessory, "menu bar only");
        assert_eq!(policy(true, false), Policy::Regular, "the window is open");
        assert_eq!(policy(false, true), Policy::Regular, "Always show in the Dock");
        assert_eq!(policy(true, true), Policy::Regular);
    }

    #[test]
    fn switching_to_the_app_shows_the_window() {
        let later = LAUNCH_GRACE + Duration::from_secs(1);
        assert!(show_on_activate(Policy::Regular, later, false), "Cmd+Tab with the window closed");
        assert!(!show_on_activate(Policy::Regular, later, true), "it's already up");
        assert!(!show_on_activate(Policy::Accessory, later, false), "a menu bar app isn't in Cmd+Tab");
        assert!(!show_on_activate(Policy::Regular, Duration::from_secs(1), false), "the launch itself");
    }

    #[test]
    fn the_setting() {
        assert!(!dock_always(&serde_json::Map::new()));
        assert!(dock_always(json!({"dock_always": true}).as_object().unwrap()));
        assert!(!dock_always(json!({"dock_always": "yes"}).as_object().unwrap()));
    }
}
