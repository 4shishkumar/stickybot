//! Connections: places StickyBot reads from to know about your day. Today there is one, the Mac's
//! Calendar (`calendar.rs`); GitHub and Claude Code are meant to become connections too, so they
//! share one small shape and one list for the dashboard's Connections page.
//!
//! ```json
//! {"id": "calendar", "title": "Calendar", "status": "off | needs_permission | connected | error",
//!  "detail": "3 calendars · next at 10:30", "settings": { … }}
//! ```
//!
//! `GET /api/connections` lists them; each connection has its own routes under
//! `/api/connections/<id>/…`. Settings live in the config under `connections.<id>`.
//!
//! The choosing is pure code in `agenda.rs` (with tests); the OS is behind `macos-calendar`'s trait.

pub mod agenda;
pub mod calendar;

use crate::AppState;
use axum::Router;
use serde_json::{Map, Value, json};
use std::sync::Arc;

/// Where a connection stands: what the pill on the page says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Off,
    /// switched on, but macOS has not allowed it
    NeedsPermission,
    Connected,
    /// the last read failed; the detail says why
    Error,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Off => "off",
            Status::NeedsPermission => "needs_permission",
            Status::Connected => "connected",
            Status::Error => "error",
        }
    }
}

/// The common shape; `extra` fields (a connection's own, like Calendar's `permission`) go beside it.
pub fn shape(id: &str, title: &str, status: Status, detail: &str, settings: Value, extra: Value) -> Value {
    let mut v = json!({"id": id, "title": title, "status": status.as_str(), "detail": detail, "settings": settings});
    if let (Some(m), Value::Object(x)) = (v.as_object_mut(), extra) {
        m.extend(x);
    }
    v
}

/// `GET /api/connections`
pub fn list(st: &AppState) -> Value {
    json!({"connections": [calendar::connection(st)]})
}

/// What is wrong with the `connections` part of a `PUT /api/config` body, in words, or `""`.
pub fn invalid_config(body: &Map<String, Value>) -> String {
    let Some(c) = body.get("connections") else { return String::new() };
    let Some(c) = c.as_object() else { return "connections must map a connection's name to its settings".into() };
    if let Some(k) = c.keys().find(|k| k.as_str() != "calendar") {
        return format!("connections: no connection '{k}'");
    }
    match c.get("calendar") {
        None => String::new(),
        Some(Value::Object(m)) => agenda::invalid_settings(m),
        Some(_) => "connections.calendar must be an object of settings".into(),
    }
}

/// `changes` with its `connections` merged into the saved ones, so a body that names one setting of one
/// connection does not switch the others back to their defaults (the config is merged one level deep).
pub fn merge_changes(cfg: &Map<String, Value>, mut changes: Map<String, Value>) -> Map<String, Value> {
    if let Some(Value::Object(new)) = changes.get("connections") {
        let mut merged = cfg.get("connections").and_then(Value::as_object).cloned().unwrap_or_default();
        for (id, settings) in new {
            match (merged.get_mut(id), settings) {
                (Some(Value::Object(old)), Value::Object(new)) => {
                    old.extend(new.iter().map(|(k, v)| (k.clone(), v.clone())))
                }
                _ => {
                    merged.insert(id.clone(), settings.clone());
                }
            }
        }
        changes.insert("connections".into(), Value::Object(merged));
    }
    changes
}

/// Settings changed: the connections read again (and a Calendar that was just switched off is cleared).
pub fn config_changed(st: &Arc<AppState>, changes: &Map<String, Value>) {
    if changes.contains_key("connections") {
        calendar::settings_changed(st);
    }
}

/// The loops that keep the connections fresh.
pub fn spawn(st: Arc<AppState>) {
    calendar::spawn(st);
}

pub fn routes() -> Router<Arc<AppState>> {
    calendar::routes()
}
