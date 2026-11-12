//! The menu bar icon and its menu, like the Python app's (`menubar.py`):
//!
//! ```text
//! <status line>                      the agent's state ("Ready — hold B on the stick", "Waiting for approval (1)"…)
//! Stick firmware X available — …     only while there is one → Settings
//! Finish setup…                      only while a recommended setup step is open → the setup page
//! ───
//! Open StickyBot                     its own window
//! Open in browser
//! Check for Updates…                 its title shows the result for a few seconds
//! Report an issue…                   the dashboard's Feedback page
//! Stick volume ▸ Mute / 25% / 50% / 75% / 100%   (a check on the current one)
//! Sounds on the stick                (checked while on)
//! ───
//! Pause / Resume
//! Restart
//! ───
//! Quit StickyBot
//! ```
//!
//! The icon is the brand mark (`icon.rs`) and shows the state (eyes, listening, thinking, paused,
//! no stick, a prompt waiting). Refreshed every 0.5 s and the setup check every 30 s, as in Python;
//! everything goes through the server's own functions (no HTTP to itself).

use crate::icon;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stick_link::LinkState;
use stickybot_server::AppState;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};

pub const CHECK: &str = "Check for Updates…";
pub const SETUP: &str = "Finish setup…";
pub const REFRESH_EVERY: Duration = Duration::from_millis(500);
pub const SETUP_EVERY: Duration = Duration::from_secs(30);
/// how long "Check for Updates…" shows the result
pub const CHECK_SAID_FOR: Duration = Duration::from_secs(8);
pub const VOLUMES: [u8; 5] = [0, 25, 50, 75, 100];

/// What the menu shows, read off the running app.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// the agent's state (`AppState::agent_state`)
    pub state: String,
    pub detail: String,
    pub link: Option<LinkState>,
    pub stick: Option<String>,
    pub reason: Option<String>,
    /// permission prompts waiting on the stick
    pub prompts: usize,
    pub paused: bool,
    /// the stick's sound settings, as it last reported them
    pub sounds: Option<Value>,
    /// "Stick firmware X available", or empty
    pub update_title: String,
}

impl Snapshot {
    pub fn of(st: &AppState) -> Self {
        let cfg = st.cfg.lock().unwrap().clone();
        Self {
            state: st.agent_state(),
            detail: st.detail(),
            link: Some(st.link.state()),
            stick: st.stick().map(|t| t.name),
            reason: st.link.reason(),
            prompts: st.prompts_waiting(),
            paused: st.link.paused(),
            sounds: st.sounds(),
            update_title: st.updates.status(&cfg, false)["title"].as_str().unwrap_or("").to_string(),
        }
    }

    /// The icon's state: `approval` while a prompt waits, else the agent's.
    pub fn icon_key(&self) -> &str {
        if self.prompts > 0 { "approval" } else { &self.state }
    }
}

/// The link's own line (the stick's name, why it searches).
pub fn link_label(state: LinkState, stick: Option<&str>, reason: Option<&str>) -> String {
    let name = stick.unwrap_or("stick");
    match state {
        LinkState::Starting => "Starting…".into(),
        LinkState::NoStick => "No stick chosen — open the dashboard".into(),
        LinkState::Searching => match reason {
            Some(why) => format!("{name}: {why}"),
            None => format!("Looking for {name}…"),
        },
        LinkState::Connected => format!("{name}: connected"),
        LinkState::Paused => "Paused".into(),
    }
}

/// The status line (menubar.py `LABELS`, the prompt count, the detail while working / on error).
pub fn status_text(s: &Snapshot) -> String {
    if s.prompts > 0 {
        return format!("Waiting for approval ({})", s.prompts);
    }
    let mut text = match s.state.as_str() {
        "ready" => "Ready — hold B on the stick".to_string(),
        "listening" => "Listening…".into(),
        "transcribing" => "Transcribing…".into(),
        "thinking" => "Thinking…".into(),
        "working" => "Working…".into(),
        "error" => "Error".into(),
        "paused" => "Paused".into(),
        "no-stick" => "No stick chosen — open the dashboard".into(),
        "searching" | "starting" => match s.link {
            Some(l) => link_label(l, s.stick.as_deref(), s.reason.as_deref()),
            None => "Looking for the stick…".into(),
        },
        other => other.to_string(),
    };
    if matches!(s.state.as_str(), "working" | "error") && !s.detail.is_empty() {
        text.push_str(" — ");
        text.extend(s.detail.chars().take(50));
    }
    text
}

/// Which "Stick volume" item has the check (menubar.py: within 12 % of it; Mute only at 0).
pub fn volume_checks(sounds: Option<&Value>) -> [bool; 5] {
    let Some(v) = sounds.and_then(|s| s.get("volume")).and_then(Value::as_f64) else {
        return [false; 5];
    };
    let pct = (v / 2.55).round() as i64;
    VOLUMES.map(|p| (pct - p as i64).abs() < 13 && (p > 0 || pct == 0))
}

pub fn sounds_on(sounds: Option<&Value>) -> bool {
    sounds.and_then(|s| s.get("on")).and_then(Value::as_bool).unwrap_or(false)
}

/// The volume message for `pct` % (the stick's 0-255).
pub fn volume_payload(pct: u8) -> Value {
    json!({"volume": (f64::from(pct) * 2.55).round() as u64})
}

/// "Sounds on the stick" clicked: the opposite of what the stick said (on if it never said).
pub fn sounds_toggle_payload(sounds: Option<&Value>) -> Value {
    let on = sounds.and_then(|s| s.get("on")).and_then(Value::as_bool).unwrap_or(true);
    json!({"on": !on})
}

/// An update check's status -> what "Check for Updates…" says (menubar.py `check_result`).
pub fn check_result(s: &Value) -> String {
    if s["available"] == true {
        return format!("{} — see the item above", s["title"].as_str().unwrap_or("Update available"));
    }
    if let Some(e) = s["error"].as_str().filter(|e| !e.is_empty()) {
        return format!("Couldn't check for updates: {}", e.chars().take(60).collect::<String>());
    }
    format!("Up to date (v{})", s["current"]["agent"].as_str().unwrap_or("?"))
}

/// The "Update available" item's title, `None` = hidden.
pub fn update_text(title: &str) -> Option<String> {
    (!title.is_empty()).then(|| {
        if title.starts_with("Update available") {
            format!("{title} — download…")
        } else {
            format!("{title} — open the dashboard")
        }
    })
}

/// Where the "Update available" item goes: the release page for a new app, `None` = the dashboard.
pub fn update_link(status: &Value, page: &str) -> Option<String> {
    (status["agent_update"] == true).then(|| page.to_string())
}

/// What's on screen now (only changes are sent to the menu).
#[derive(Default, Clone, PartialEq)]
struct Shown {
    icon: Option<icon::Style>,
    status: String,
    pause: String,
    volume: Option<[bool; 5]>,
    sounds: Option<bool>,
    update: Option<Option<String>>,
    setup: Option<bool>,
}

pub struct Tray {
    app: AppHandle,
    st: Arc<AppState>,
    tray: TrayIcon,
    menu: Menu<Wry>,
    status: MenuItem<Wry>,
    update: MenuItem<Wry>,
    setup: MenuItem<Wry>,
    check: MenuItem<Wry>,
    volume: Vec<CheckMenuItem<Wry>>,
    sounds: CheckMenuItem<Wry>,
    pause: MenuItem<Wry>,
    name: &'static str,
    shown: Mutex<Shown>,
    setup_pending: Mutex<bool>,
    /// when "Check for Updates…" last showed a result
    check_said: Mutex<Option<Instant>>,
    /// a click: refresh now
    nudge: tokio::sync::Notify,
}

fn template(style: icon::Style) -> Image<'static> {
    Image::new_owned(icon::rgba(style), icon::SIZE as u32, icon::SIZE as u32)
}

impl Tray {
    /// Makes the icon and its menu and starts refreshing them.
    pub fn start(app: &AppHandle, st: Arc<AppState>) -> tauri::Result<Arc<Self>> {
        let name = st.app_name();
        let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
        let update = MenuItem::with_id(app, "update", "", true, None::<&str>)?;
        let setup = MenuItem::with_id(app, "setup", SETUP, true, None::<&str>)?;
        let open = MenuItem::with_id(app, "open", format!("Open {name}"), true, None::<&str>)?;
        let browser = MenuItem::with_id(app, "browser", "Open in browser", true, None::<&str>)?;
        let check = MenuItem::with_id(app, "check", CHECK, true, None::<&str>)?;
        let feedback = MenuItem::with_id(app, "feedback", "Report an issue…", true, None::<&str>)?;
        let volume = VOLUMES
            .iter()
            .map(|&p| {
                let label = if p == 0 { "Mute".to_string() } else { format!("{p}%") };
                CheckMenuItem::with_id(app, format!("volume-{p}"), label, true, false, None::<&str>)
            })
            .collect::<tauri::Result<Vec<_>>>()?;
        let vol_refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> =
            volume.iter().map(|i| i as &dyn tauri::menu::IsMenuItem<Wry>).collect();
        let volume_menu = Submenu::with_id_and_items(app, "volume", "Stick volume", true, &vol_refs)?;
        let sounds = CheckMenuItem::with_id(app, "sounds", "Sounds on the stick", true, false, None::<&str>)?;
        let pause = MenuItem::with_id(app, "pause", "Pause", true, None::<&str>)?;
        let restart = MenuItem::with_id(app, "restart", "Restart", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", format!("Quit {name}"), true, Some("cmd+q"))?;
        let sep = || PredefinedMenuItem::separator(app);
        // the update and setup items go in at 1 / 2 when there's something to say (`apply`)
        let menu = Menu::with_items(
            app,
            &[
                &status,
                &sep()?,
                &open,
                &browser,
                &check,
                &feedback,
                &volume_menu,
                &sounds,
                &sep()?,
                &pause,
                &restart,
                &sep()?,
                &quit,
            ],
        )?;
        let tray = TrayIconBuilder::with_id("main")
            .tooltip(name)
            .icon(template(icon::Style::Dashed))
            .icon_as_template(true)
            .menu(&menu)
            .show_menu_on_left_click(true)
            .on_menu_event(|app, ev| {
                if let Some(t) = app.try_state::<Arc<Tray>>() {
                    t.clicked(ev.id().as_ref());
                }
            })
            .build(app)?;
        let me = Arc::new(Self {
            app: app.clone(),
            st,
            tray,
            menu,
            status,
            update,
            setup,
            check,
            volume,
            sounds,
            pause,
            name,
            shown: Mutex::new(Shown::default()),
            setup_pending: Mutex::new(false),
            check_said: Mutex::new(None),
            nudge: tokio::sync::Notify::new(),
        });
        app.manage(me.clone());
        let t = me.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                t.refresh();
                tokio::select! {
                    _ = tokio::time::sleep(REFRESH_EVERY) => {}
                    _ = t.nudge.notified() => {}
                }
            }
        });
        let t = me.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let st = t.st.clone();
                let pending = tauri::async_runtime::spawn_blocking(move || stickybot_server::setup::quick_pending(&st))
                    .await
                    .unwrap_or(false);
                *t.setup_pending.lock().unwrap() = pending;
                tokio::time::sleep(SETUP_EVERY).await;
            }
        });
        Ok(me)
    }

    /// Brings the menu and the icon up to date (menubar.py `refresh`). Only the refresh task calls
    /// it, off the main thread; no lock is held while the menu is changed (that waits for the main
    /// thread, where the clicks run).
    fn refresh(&self) {
        if let Some(d) = crate::dock::of(&self.app) {
            d.sync(&self.app); // "Always show in the Dock" changed in Settings
        }
        let snap = Snapshot::of(&self.st);
        let status = status_text(&snap);
        let next = Shown {
            icon: Some(icon::style(snap.icon_key())),
            pause: if snap.paused { "Resume" } else { "Pause" }.into(),
            volume: Some(volume_checks(snap.sounds.as_ref())),
            sounds: Some(sounds_on(snap.sounds.as_ref())),
            update: Some(update_text(&snap.update_title)),
            setup: Some(*self.setup_pending.lock().unwrap()),
            status,
        };
        let said_long_enough = {
            let mut said = self.check_said.lock().unwrap();
            let done = said.is_some_and(|t| t.elapsed() >= CHECK_SAID_FOR);
            if done {
                *said = None;
            }
            done
        };
        if said_long_enough {
            let _ = self.check.set_text(CHECK);
        }
        let shown = self.shown.lock().unwrap().clone();
        // a click on a check item toggles its check on its own: put the stick's word back each time
        if let Some(v) = next.volume {
            for (item, on) in self.volume.iter().zip(v) {
                if item.is_checked().ok() != Some(on) {
                    let _ = item.set_checked(on);
                }
            }
        }
        if let Some(on) = next.sounds
            && self.sounds.is_checked().ok() != Some(on)
        {
            let _ = self.sounds.set_checked(on);
        }
        if shown == next {
            return;
        }
        if shown.icon != next.icon
            && let Some(style) = next.icon
        {
            let _ = self.tray.set_icon_with_as_template(Some(template(style)), true);
        }
        if shown.status != next.status {
            let _ = self.status.set_text(&next.status);
            let _ = self.tray.set_tooltip(Some(format!("{} — {}", self.name, next.status)));
        }
        if shown.pause != next.pause {
            let _ = self.pause.set_text(&next.pause);
        }
        // the optional items: removed and put back at their places (a menu item can't be hidden)
        let (had_update, has_update) =
            (shown.update.clone().flatten().is_some(), next.update.clone().flatten().is_some());
        let (had_setup, has_setup) = (shown.setup == Some(true), next.setup == Some(true));
        if had_update != has_update || had_setup != has_setup {
            if had_update {
                let _ = self.menu.remove(&self.update);
            }
            if had_setup {
                let _ = self.menu.remove(&self.setup);
            }
            let mut at = 1;
            if has_update {
                let _ = self.menu.insert(&self.update, at);
                at += 1;
            }
            if has_setup {
                let _ = self.menu.insert(&self.setup, at);
            }
        }
        if let Some(Some(t)) = &next.update
            && shown.update.as_ref() != next.update.as_ref()
        {
            let _ = self.update.set_text(t);
        }
        *self.shown.lock().unwrap() = next;
    }

    fn clicked(self: &Arc<Self>, id: &str) {
        let st = self.st.clone();
        match id {
            "open" => crate::show_dashboard_at(&self.app, ""),
            "update" => {
                // a new StickyBot.app: its release page (the app can't install itself); firmware:
                // the dashboard's update card
                let cfg = st.cfg.lock().unwrap().clone();
                let s = st.updates.status(&cfg, false);
                match update_link(&s, &st.updates.download_page(&s)) {
                    Some(url) => {
                        let _ = std::process::Command::new("open").arg(url).spawn();
                    }
                    None => crate::show_dashboard_at(&self.app, "settings"),
                }
            }
            "setup" => crate::show_dashboard_at(&self.app, "install"),
            "feedback" => crate::show_dashboard_at(&self.app, "feedback"),
            "browser" => {
                let _ = std::process::Command::new("open").arg(st.url()).spawn();
            }
            "check" => {
                let _ = self.check.set_text("Checking for updates…");
                *self.check_said.lock().unwrap() = None;
                let me = self.clone();
                tauri::async_runtime::spawn(async move {
                    let r = stickybot_server::updates::check_now(&me.st).await;
                    let _ = me.check.set_text(check_result(&r));
                    *me.check_said.lock().unwrap() = Some(Instant::now()); // back to "Check for Updates…" soon
                });
            }
            "sounds" => {
                let payload = sounds_toggle_payload(st.sounds().as_ref());
                tauri::async_runtime::spawn(async move { st.sound(payload).await });
            }
            "pause" => st.link.pause(!st.link.paused()),
            "restart" => self.app.request_restart(), // the Exit event still runs (the wireless mic's device goes)
            "quit" => self.app.exit(0),
            v if v.starts_with("volume-") => {
                let Ok(pct) = v["volume-".len()..].parse::<u8>() else { return };
                tauri::async_runtime::spawn(async move {
                    st.sound(volume_payload(pct)).await;
                    if pct > 0 {
                        st.sound(json!({"play": "answer"})).await; // a sample at the new level
                    }
                });
            }
            _ => {}
        }
        self.nudge.notify_one(); // the checks and titles at once, not half a second later
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(state: &str) -> Snapshot {
        Snapshot { state: state.into(), ..Default::default() }
    }

    #[test]
    fn link_labels() {
        assert_eq!(link_label(LinkState::Connected, Some("Arbaaz"), None), "Arbaaz: connected");
        assert_eq!(link_label(LinkState::Searching, None, None), "Looking for stick…");
        assert_eq!(
            link_label(LinkState::Searching, Some("Arbaaz"), Some(stick_link::NOT_FOUND_REASON)),
            "Arbaaz: stick not found — is it awake?"
        );
        assert_eq!(link_label(LinkState::NoStick, Some("x"), None), "No stick chosen — open the dashboard");
    }

    #[test]
    fn the_status_line_follows_the_python_menu() {
        assert_eq!(status_text(&snap("ready")), "Ready — hold B on the stick");
        assert_eq!(status_text(&snap("listening")), "Listening…");
        assert_eq!(status_text(&snap("paused")), "Paused");
        assert_eq!(status_text(&snap("no-stick")), "No stick chosen — open the dashboard");
        let mut s = snap("searching");
        s.link = Some(LinkState::Searching);
        s.stick = Some("Arbaaz".into());
        assert_eq!(status_text(&s), "Looking for Arbaaz…");
        let mut s = snap("working");
        s.detail = "x".repeat(80);
        assert_eq!(status_text(&s), format!("Working… — {}", "x".repeat(50)));
        s.detail.clear();
        assert_eq!(status_text(&s), "Working…");
        let mut s = snap("ready");
        s.prompts = 2;
        assert_eq!(status_text(&s), "Waiting for approval (2)");
        assert_eq!(s.icon_key(), "approval");
        assert_eq!(snap("thinking").icon_key(), "thinking");
    }

    #[test]
    fn the_volume_check_sits_on_the_nearest_level() {
        assert_eq!(volume_checks(None), [false; 5]);
        assert_eq!(volume_checks(Some(&json!({"volume": 0}))), [true, false, false, false, false]);
        assert_eq!(volume_checks(Some(&json!({"volume": 160}))), [false, false, false, true, false]); // 63 % -> 75
        assert_eq!(volume_checks(Some(&json!({"volume": 255}))), [false, false, false, false, true]);
        assert_eq!(volume_checks(Some(&json!({"volume": 5}))), [false; 5]); // 2 % is neither mute nor 25 %
        assert_eq!(volume_payload(50), json!({"volume": 127})); // as Python rounds 127.4999…
        assert_eq!(volume_payload(100), json!({"volume": 255}));
    }

    #[test]
    fn sounds_on_and_its_toggle() {
        assert!(!sounds_on(None));
        assert!(sounds_on(Some(&json!({"on": true}))));
        assert_eq!(sounds_toggle_payload(Some(&json!({"on": true}))), json!({"on": false}));
        assert_eq!(sounds_toggle_payload(Some(&json!({"on": false}))), json!({"on": true}));
        assert_eq!(sounds_toggle_payload(None), json!({"on": false}));
    }

    #[test]
    fn check_results_and_the_update_item() {
        let up = json!({"available": false, "error": "", "current": {"agent": "0.1.0"}});
        assert_eq!(check_result(&up), "Up to date (v0.1.0)");
        let err = json!({"available": false, "error": "offline ".repeat(20), "current": {}});
        assert_eq!(check_result(&err), format!("Couldn't check for updates: {}", &"offline ".repeat(20)[..60]));
        let fw = json!({"available": true, "title": "Stick firmware 2.6.0 available"});
        assert_eq!(check_result(&fw), "Stick firmware 2.6.0 available — see the item above");
        assert_eq!(update_text(""), None);
        assert_eq!(
            update_text("Stick firmware 2.6.0 available").as_deref(),
            Some("Stick firmware 2.6.0 available — open the dashboard")
        );
        // a new app: the item says so and goes to the release page
        assert_eq!(update_text("Update available: v2.5.0").as_deref(), Some("Update available: v2.5.0 — download…"));
        let page = "https://github.com/acme/stickybot/releases/tag/v2.5.0";
        assert_eq!(update_link(&json!({"agent_update": true}), page).as_deref(), Some(page));
        assert_eq!(update_link(&json!({"agent_update": false, "firmware_update": true}), page), None);
    }
}
