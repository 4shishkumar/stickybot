//! StickyBot: a menu bar app that runs the dashboard server and the stick link, and shows the
//! dashboard in its own window (a Dock icon and Cmd+Tab only while that's open, or always if
//! Settings says so: dock.rs).
//!
//! Phase 1 runs next to the Python agent: its own port (8670), its own folder (~/.stickybot-rs),
//! its own bundle id (com.stickybot.rs), and it never connects to a stick until one is picked in
//! *its* dashboard.

use std::sync::Arc;
use stickybot_server::{AppState, Options};
use tauri::{Manager, RunEvent};

mod appctl;
mod dock;
mod icon;
mod login;
mod tray;
mod window;

pub(crate) use window::show_dashboard_at;

fn main() {
    // Claude Code's hooks run this program with `--hook permission|prompt|stop` (the server's
    // hooks.rs): answer and exit before anything else starts
    if std::env::args().nth(1).as_deref() == Some("--hook") {
        let kind = std::env::args().nth(2).unwrap_or_default();
        std::process::exit(stickybot_server::hooks::main(&kind));
    }
    let opts = Options::from_env();
    stickybot_server::logs::init(&opts.home.join("app.log")); // stderr + app.log + the Logs page

    let app = tauri::Builder::default()
        // closing the dashboard window: back to the menu bar only (unless "Always show in the Dock")
        .on_window_event(|w, ev| {
            if w.label() == window::WINDOW
                && matches!(ev, tauri::WindowEvent::Destroyed)
                && let Some(d) = dock::of(w.app_handle())
            {
                d.window_closed(w.app_handle());
            }
        })
        .on_menu_event(|app, ev| dock::menu_clicked(app, ev.id().as_ref()))
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory); // menu bar only until dock.rs says

            let running = tauri::async_runtime::block_on(stickybot_server::start(opts.clone()))?;
            let state = running.state.clone();
            *state.login_item.lock().unwrap() = Arc::new(login::AppLoginItem);
            *state.setup.app.lock().unwrap() = Arc::new(appctl::TauriApp(app.handle().clone()));
            app.manage(state.clone());

            let dock = Arc::new(dock::Dock::default());
            app.manage(dock.clone());
            app.set_menu(dock::app_menu(app.handle(), state.app_name())?)?;
            dock.sync(app.handle()); // "Always show in the Dock": Regular from the start
            #[cfg(target_os = "macos")]
            {
                let h = app.handle().clone();
                dock::on_activate(move || dock.activated(&h)); // Cmd+Tab with no window up
            }

            tray::Tray::start(app.handle(), state.clone())?;
            log::info!("{} is in the menu bar; dashboard at {}", state.app_name(), running.state.url());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("couldn't start the app");

    app.run(|app, event| match event {
        // closing the window keeps the menu bar app running; Quit (or cmd+q in the menu) ends it
        RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
        // the app opened again (Finder, Spotlight, `open -a`, a click on its Dock icon): the
        // dashboard, like the Python app
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show_dashboard_at(app, ""),
        // the wireless mic's device and tap outlive the process: remove them on the way out
        RunEvent::Exit => {
            if let Some(st) = app.try_state::<Arc<AppState>>() {
                st.shutdown();
            }
        }
        _ => {}
    });
}
