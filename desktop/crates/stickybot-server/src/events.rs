//! Events for the stick: celebrations, nice-to-know toasts and heads-ups (a port of `events.py`).
//!
//! Something happens (a PR merged, Claude Code finished a long task…) -> a *kind*, whose level is
//! set in the dashboard (Events): `celebrate` (confetti + fanfare), `nice` (toast + chime),
//! `headsup` (orange, stays until a button), `off`. During quiet hours events still show, silently.
//!
//! To the stick: `{"src":"agent","event":{"id","level","title","sub","icon","sound","kind"}}`; it
//! answers `{"src":"agent","evt":"event_shown","id"}`. Logged to `<home>/events.jsonl`.

use crate::AppState;
use crate::talk::stick_text;
use chrono::Timelike;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

pub const LEVELS: &[&str] = &["celebrate", "nice", "headsup", "off"];

/// kind -> (default level, icon, sample title, sample sub); the samples are for the Test buttons.
pub const KINDS: &[(&str, &str, &str, &str, &str)] = &[
    ("pr_merged", "celebrate", "merge", "PR #5507 merged!", "web - Add stick agent"),
    ("release", "celebrate", "star", "Release v2.4.0", "web"),
    ("ci_fixed", "celebrate", "check", "CI is green again", "web #5507"),
    ("pr_created", "nice", "pr", "PR #5508 opened", "web - Fix weather chip"),
    ("pr_approved", "nice", "check", "PR #5507 approved", "web - by a teammate"),
    ("claude_done", "nice", "claude", "Claude finished in web", "took 4 min"),
    ("devin_review", "nice", "review", "Devin reviewed #5507", "web - 3 comments"),
    ("review_requested", "headsup", "review", "Review requested", "web #5510 - Update deps"),
    ("changes_requested", "headsup", "x", "Changes requested", "web #5507"),
    ("ci_failed", "headsup", "x", "CI failed", "web #5507 - build"),
    ("claude_tool", "nice", "claude", "Tests passed", "from web"),
    ("update", "nice", "star", "Update v1.8.0 ready", "see the dashboard"),
    ("test", "nice", "bell", "Test event", "from the dashboard"),
];

pub fn kind(k: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str, &'static str)> {
    KINDS.iter().find(|x| x.0 == k)
}

/// The levels per kind: the defaults with the settings' `event_rules` on top.
pub fn rules(cfg: &Map<String, Value>) -> Map<String, Value> {
    let mut r: Map<String, Value> = KINDS.iter().map(|k| (k.0.to_string(), json!(k.1))).collect();
    if let Some(Value::Object(user)) = cfg.get("event_rules") {
        for (k, v) in user {
            if v.as_str().is_some_and(|v| LEVELS.contains(&v)) {
                r.insert(k.clone(), v.clone());
            }
        }
    }
    r
}

fn minutes(hhmm: &str) -> Option<u32> {
    let (h, m) = hhmm.split_once(':').unwrap_or((hhmm, "0"));
    Some(h.trim().parse::<u32>().ok()? * 60 + if m.is_empty() { 0 } else { m.trim().parse::<u32>().ok()? })
}

/// Quiet hours at `now` (minutes since midnight)? May wrap midnight.
pub fn quiet_at(cfg: &Map<String, Value>, now: u32) -> bool {
    let q = cfg.get("quiet_hours").cloned().unwrap_or(Value::Null);
    if !q.get("on").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    let s = |k: &str, d: &str| q.get(k).and_then(Value::as_str).unwrap_or(d).to_string();
    let (Some(start), Some(end)) = (minutes(&s("from", "19:00")), minutes(&s("to", "09:00"))) else { return false };
    if start <= end { start <= now && now < end } else { now >= start || now < end }
}

pub fn quiet_now(cfg: &Map<String, Value>) -> bool {
    let t = chrono::Local::now();
    quiet_at(cfg, t.hour() * 60 + t.minute())
}

#[derive(Default)]
pub struct Notifier {
    n: AtomicU64,
    /// ids the stick acknowledged
    pub shown: Mutex<HashSet<String>>,
}

impl Notifier {
    pub fn acked(&self, id: &str) {
        self.shown.lock().unwrap().insert(id.to_string());
    }
}

fn cut(s: &str, n: usize) -> String {
    stick_text(s).chars().take(n).collect()
}

/// Fires an event of `kind` (its level from the rules unless given). `None` if this kind is off.
pub async fn notify(
    st: &AppState,
    kind_: &str,
    title: &str,
    sub: &str,
    icon: Option<&str>,
    level: Option<&str>,
    sound: &str,
) -> Option<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    let rule = rules(&cfg).get(kind_).and_then(Value::as_str).unwrap_or("nice").to_string();
    let level = level.map(str::to_string).unwrap_or(rule);
    if level == "off" {
        log::info!("event {kind_} (off): {title}");
        return None;
    }
    let id = format!("e{}", st.home.notifier.n.fetch_add(1, Ordering::Relaxed) + 1);
    let quiet = quiet_now(&cfg);
    let icon = icon.map(str::to_string).unwrap_or_else(|| kind(kind_).map(|k| k.2).unwrap_or("bell").to_string());
    let ev = json!({"id": id, "level": level, "title": cut(title, 40), "sub": cut(sub, 90), "icon": icon,
                    "sound": if quiet { "off" } else { sound }, "kind": kind_});
    let sent = st.link.send(&json!({"src": "agent", "event": ev})).await;
    let mut entry = Map::new();
    entry.insert("t".into(), json!(crate::hub::now()));
    entry.insert("kind".into(), json!(kind_));
    for (k, v) in ev.as_object().expect("object") {
        entry.insert(k.clone(), v.clone());
    }
    entry.insert("sent".into(), json!(sent));
    entry.insert("quiet".into(), json!(quiet));
    let line = Value::Object(entry.clone()).to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(st.paths.home.join("events.jsonl")) {
        let _ = writeln!(f, "{line}");
    }
    // the page's feed: `ev_kind` for the event's kind (`kind` is the feed's own)
    let mut feed: Map<String, Value> = entry.into_iter().filter(|(k, _)| k != "t" && k != "kind").collect();
    feed.insert("ev_kind".into(), json!(kind_));
    st.hub.emit("event", Value::Object(feed));
    log::info!(
        "event {kind_} -> {level}: {} / {}{}",
        ev["title"].as_str().unwrap_or(""),
        ev["sub"].as_str().unwrap_or(""),
        if sent { "" } else { " (stick not connected)" }
    );
    Some(ev)
}

/// The dashboard's Test button: the kind's sample.
pub async fn test(st: &AppState, k: &str) -> Option<Value> {
    let (name, _, icon, title, sub) = *kind(k).unwrap_or_else(|| kind("test").expect("test kind"));
    let name = if kind(k).is_some() { name } else { k };
    notify(st, name, title, sub, Some(icon), None, "").await
}

/// The last `n` events, newest first.
pub fn history(st: &AppState, n: usize) -> Vec<Value> {
    let text = std::fs::read_to_string(st.paths.home.join("events.jsonl")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().rev().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// `GET /api/events/kinds`.
pub fn kinds_json(cfg: &Map<String, Value>) -> Value {
    let kinds: Map<String, Value> =
        KINDS.iter().map(|k| (k.0.to_string(), json!({"default": k.1, "icon": k.2, "title": k.3}))).collect();
    json!({"kinds": kinds, "rules": rules(cfg), "levels": LEVELS})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn rules_take_valid_user_levels() {
        let r = rules(&cfg(json!({"event_rules": {"pr_merged": "off", "ci_failed": "loud", "custom": "nice"}})));
        assert_eq!(r["pr_merged"], "off");
        assert_eq!(r["ci_failed"], "headsup");
        assert_eq!(r["custom"], "nice");
        assert_eq!(r["release"], "celebrate");
    }

    #[test]
    fn quiet_hours_wrap_midnight() {
        let on = cfg(json!({"quiet_hours": {"on": true, "from": "22:00", "to": "07:30"}}));
        assert!(quiet_at(&on, 23 * 60));
        assert!(quiet_at(&on, 7 * 60 + 29));
        assert!(!quiet_at(&on, 7 * 60 + 30));
        assert!(!quiet_at(&on, 12 * 60));
        let day = cfg(json!({"quiet_hours": {"on": true, "from": "12:00", "to": "13:00"}}));
        assert!(quiet_at(&day, 12 * 60 + 5) && !quiet_at(&day, 13 * 60));
        assert!(!quiet_at(&cfg(json!({"quiet_hours": {"on": false}})), 23 * 60));
        assert!(!quiet_at(&cfg(json!({"quiet_hours": {"on": true, "from": "x"}})), 23 * 60));
    }
}
