//! StickyBot (Rust): a menu bar app (no Dock icon) that runs the dashboard server and the stick
//! link, and shows the dashboard in its own window.
//!
//! Phase 1 runs next to the Python agent: its own port (8670), its own folder (~/.stickybot-rs),
//! its own bundle id (com.stickybot.rs), and it never connects to a stick until one is picked in
//! *its* dashboard.

use std::sync::Arc;
use stick_link::LinkState;
use stickybot_server::{AppState, Options};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

const WINDOW: &str = "dashboard";

fn label(state: LinkState, stick: Option<&str>, reason: Option<&str>) -> String {
    let name = stick.unwrap_or("stick");
    match state {
        LinkState::Starting => "Starting…".into(),
        LinkState::NoStick => "No stick picked yet".into(),
        LinkState::Searching => match reason {
            Some(why) => format!("{name}: {why}"),
            None => format!("Looking for {name}…"),
        },
        LinkState::Connected => format!("{name}: connected"),
        LinkState::Paused => "Paused".into(),
    }
}

fn show_dashboard(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let st = app.state::<Arc<AppState>>();
    let url = match st.url().parse() {
        Ok(u) => u,
        Err(e) => {
            log::error!("bad dashboard url: {e}");
            return;
        }
    };
    match WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(url))
        .title("StickyBot (Rust)")
        .inner_size(1180.0, 820.0)
        .min_inner_size(420.0, 500.0)
        .build()
    {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::error!("can't open the dashboard window: {e}"),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let opts = Options::from_env();

    let app = tauri::Builder::default()
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // menu bar only, no Dock icon

            let running = tauri::async_runtime::block_on(stickybot_server::start(opts.clone()))?;
            let state = running.state.clone();
            app.manage(state.clone());

            let status = MenuItem::with_id(app, "status", "Starting…", false, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open dashboard", true, None::<&str>)?;
            let browser = MenuItem::with_id(app, "browser", "Open in browser", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit StickyBot (Rust)", true, Some("cmd+q"))?;
            let sep1 = PredefinedMenuItem::separator(app)?;
            let sep2 = PredefinedMenuItem::separator(app)?;
            let menu = Menu::with_items(app, &[&status, &sep1, &open, &browser, &sep2, &quit])?;

            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("StickyBot (Rust)")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "open" => show_dashboard(app),
                    "browser" => {
                        let url = app.state::<Arc<AppState>>().url();
                        let _ = std::process::Command::new("open").arg(url).spawn();
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            let tray = tray.build(app)?;

            // keep the status line and the tooltip in step with the link
            let mut w = state.link.watch_state();
            let mut why = state.link.watch_reason();
            tauri::async_runtime::spawn(async move {
                loop {
                    let s = *w.borrow_and_update();
                    let reason = why.borrow_and_update().clone();
                    let text = label(s, state.stick().as_ref().map(|t| t.name.as_str()), reason.as_deref());
                    let _ = status.set_text(&text);
                    let _ = tray.set_tooltip(Some(format!("StickyBot (Rust) — {text}")));
                    tokio::select! {
                        r = w.changed() => if r.is_err() { break; },
                        r = why.changed() => if r.is_err() { break; },
                    }
                }
            });
            log::info!("StickyBot (Rust) is in the menu bar; dashboard at {}", running.state.url());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("couldn't start the app");

    app.run(|_app, event| {
        // closing the window keeps the menu bar app running; Quit (or cmd+q in the menu) ends it
        if let RunEvent::ExitRequested { api, code: None, .. } = event {
            api.prevent_exit();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_labels() {
        assert_eq!(label(LinkState::Connected, Some("Arbaaz"), None), "Arbaaz: connected");
        assert_eq!(label(LinkState::Searching, None, None), "Looking for stick…");
        assert_eq!(
            label(LinkState::Searching, Some("Arbaaz"), Some(stick_link::NOT_FOUND_REASON)),
            "Arbaaz: stick not found — is it awake?"
        );
        assert_eq!(label(LinkState::NoStick, Some("x"), None), "No stick picked yet");
    }
}
