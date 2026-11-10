//! The dashboard in its own window (window.py's job in the Python app).

use std::sync::Arc;
use stickybot_server::AppState;
use tauri::webview::NewWindowResponse;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub const WINDOW: &str = "dashboard";

/// A page of this app's dashboard (stays in the window); anything else opens in the default
/// browser / its app (links to GitHub, OpenRouter's keys page, System Settings).
pub fn is_dashboard(url: &tauri::Url, port: u16) -> bool {
    match url.scheme() {
        "about" | "blob" | "data" => true,
        "http" => matches!(url.host_str(), Some("localhost" | "127.0.0.1")) && url.port() == Some(port),
        _ => false,
    }
}

/// Opens `url` outside the window (window.py does the same for links and `target=_blank`).
fn open_outside(url: &tauri::Url) {
    if matches!(url.scheme(), "http" | "https" | "mailto" | "x-apple.systempreferences") {
        let _ = std::process::Command::new("open").arg(url.as_str()).spawn();
    } else {
        log::warn!("not opening {url}");
    }
}

/// Shows and focuses the dashboard window (made if it was closed), at `#page` if given.
pub fn show_dashboard_at(app: &AppHandle, page: &str) {
    if let Some(w) = app.get_webview_window(WINDOW) {
        if !page.is_empty() {
            let _ = w.eval(format!("location.hash = {page:?}")); // letters, digits and - (checked)
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        return;
    }
    let st = app.state::<Arc<AppState>>();
    let mut url = st.url();
    if !page.is_empty() {
        url = format!("{url}#{page}");
    }
    let url = match url.parse() {
        Ok(u) => u,
        Err(e) => {
            log::error!("bad dashboard url: {e}");
            return;
        }
    };
    let port = st.port;
    match WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(url))
        .title(st.app_name())
        .inner_size(1180.0, 820.0)
        .min_inner_size(420.0, 500.0)
        .on_navigation(move |u| {
            let ours = is_dashboard(u, port);
            if !ours {
                open_outside(u);
            }
            ours
        })
        .on_new_window(|u, _| {
            open_outside(&u); // target=_blank / window.open: the browser, not a second window
            NewWindowResponse::Deny
        })
        .build()
    {
        Ok(w) => {
            let _ = w.set_focus();
        }
        Err(e) => log::error!("can't open the dashboard window: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_dashboard_stays_in_the_window() {
        let u = |s: &str| s.parse::<tauri::Url>().unwrap();
        assert!(is_dashboard(&u("http://localhost:8670/#settings"), 8670));
        assert!(is_dashboard(&u("http://127.0.0.1:8670/static/app.js"), 8670));
        assert!(is_dashboard(&u("about:blank"), 8670));
        assert!(!is_dashboard(&u("http://localhost:8660/"), 8670), "the other app's dashboard");
        assert!(!is_dashboard(&u("https://openrouter.ai/keys"), 8670));
        assert!(!is_dashboard(&u("x-apple.systempreferences:com.apple.LoginItems-Settings.extension"), 8670));
    }
}
