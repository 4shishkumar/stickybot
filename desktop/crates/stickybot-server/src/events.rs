//! Events for the stick: celebrations, nice-to-know toasts and heads-ups (a port of `events.py`).
//!
//! Something happens (a PR merged, Claude Code finished a long task…) -> a *kind*, whose level is
//! set in the dashboard (Events): `celebrate` (confetti + fanfare), `nice` (toast + chime),
//! `headsup` (orange, stays until a button), `off`. During quiet hours events still show, silently.
//!
//! Each kind can have its own sound too (settings `event_sounds` {kind: name | "off"}): a clip or
//! tune the stick has (`applause`), `off`, or `""` for its level's default (picked on the stick:
//! celebrate `jingle_win`, nice `confirmation_002`, headsup `question_001`). PR merged, release and
//! CI fixed have their own by default ([`DEFAULT_SOUNDS`], firmware 2.7.0's celebration clips). A
//! name the stick doesn't list goes out as `""` (2.7.0 falls back by itself; older firmware would
//! play nothing).
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
    ("meeting_soon", "headsup", "bell", "Standup in 5 min", "10:30 · Google Meet"),
    ("test", "nice", "bell", "Test event", "from the dashboard"),
];

/// Kinds whose text is about the person's own day (a meeting's title): the log keeps the kind and
/// the level only. The stick, the Events page and `events.jsonl` still get the text.
const PRIVATE_KINDS: &[&str] = &["meeting_soon"];

/// kind -> its own sound (the rest: `""` = the level's default, chosen on the stick).
pub const DEFAULT_SOUNDS: &[(&str, &str)] = &[("pr_merged", "applause"), ("release", "party_horn"), ("ci_fixed", "cheer")];
/// The stick's sound per level (events.cpp `defaultSound`).
pub const LEVEL_SOUNDS: &[(&str, &str)] =
    &[("celebrate", "jingle_win"), ("nice", "confirmation_002"), ("headsup", "question_001")];
/// Firmware 2.7.0's celebration clips: offered while the stick hasn't told us its list.
pub const CELEBRATION_SOUNDS: &[&str] = &["applause", "clap", "cheer", "party_horn", "tada", "drumroll", "level_up", "coins"];

fn sound_ok(v: &str) -> bool {
    v.is_empty()
        || v == "off"
        || (v.len() <= 32 && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

/// kind -> its sound: a name, `"off"`, or `""` (the level's default).
pub fn sounds(cfg: &Map<String, Value>) -> Map<String, Value> {
    let mut s: Map<String, Value> = KINDS
        .iter()
        .map(|k| (k.0.to_string(), json!(DEFAULT_SOUNDS.iter().find(|d| d.0 == k.0).map(|d| d.1).unwrap_or(""))))
        .collect();
    if let Some(Value::Object(user)) = cfg.get("event_sounds") {
        for (k, v) in user {
            if kind(k).is_some() && v.as_str().is_some_and(sound_ok) {
                s.insert(k.clone(), v.clone());
            }
        }
    }
    s
}

/// The stick's tunes from its last sound report, `None` if it hasn't sent one.
pub fn tune_list(stick_sounds: Option<&Value>) -> Option<Vec<String>> {
    let t = stick_sounds?.get("tunes")?.as_array()?;
    Some(t.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
}

/// The sound to send: a name the stick doesn't have becomes `""` (its level's default).
pub fn for_stick(name: &str, tunes: Option<&[String]>) -> String {
    match tunes {
        Some(t) if !name.is_empty() && name != "off" && !t.iter().any(|x| x == name) => String::new(),
        _ => name.to_string(),
    }
}

/// What's wrong with `event_sounds` in a `PUT /api/config` body, or "". New names must be ones the
/// stick has (when it has told us); a kind's current one (saved or default) may stay, as the page
/// sends them all (another stick, older firmware: they go out as `""` then).
pub fn invalid_sounds(body: &Map<String, Value>, tunes: Option<&[String]>, current: &Map<String, Value>) -> String {
    let Some(es) = body.get("event_sounds") else { return String::new() };
    let Some(es) = es.as_object() else { return "event_sounds must map event kinds to sound names".into() };
    let now = sounds(current);
    for (k, v) in es {
        if kind(k).is_none() {
            return format!("event_sounds: no event kind '{k}'");
        }
        let Some(v) = v.as_str().filter(|v| sound_ok(v)) else { return format!("event_sounds.{k}: not a sound name") };
        let unknown = tunes.is_some_and(|t| !v.is_empty() && v != "off" && !t.iter().any(|x| x == v));
        if unknown && now.get(k).and_then(Value::as_str) != Some(v) {
            return format!("event_sounds.{k}: the stick has no sound '{v}'");
        }
    }
    String::new()
}

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

/// Fires an event of `kind` (its level and sound from the settings unless given; sound `""` = the
/// level's default, `"off"` = silent). `None` if this kind is off.
pub async fn notify(
    st: &AppState,
    kind_: &str,
    title: &str,
    sub: &str,
    icon: Option<&str>,
    level: Option<&str>,
    sound: Option<&str>,
) -> Option<Value> {
    let cfg = st.cfg.lock().unwrap().clone();
    let rule = rules(&cfg).get(kind_).and_then(Value::as_str).unwrap_or("nice").to_string();
    let level = level.map(str::to_string).unwrap_or(rule);
    let private = PRIVATE_KINDS.contains(&kind_);
    if level == "off" {
        if private {
            log::info!("event {kind_} (off)");
            log::debug!("event {kind_} (off): {title}");
        } else {
            log::info!("event {kind_} (off): {title}");
        }
        return None;
    }
    let id = format!("e{}", st.home.notifier.n.fetch_add(1, Ordering::Relaxed) + 1);
    let quiet = quiet_now(&cfg);
    let sound = match sound {
        Some(s) => s.to_string(),
        None => sounds(&cfg).get(kind_).and_then(Value::as_str).unwrap_or("").to_string(),
    };
    let sound = for_stick(&sound, tune_list(st.sounds().as_ref()).as_deref());
    let icon = icon.map(str::to_string).unwrap_or_else(|| kind(kind_).map(|k| k.2).unwrap_or("bell").to_string());
    let ev = json!({"id": id, "level": level, "title": cut(title, 40), "sub": cut(sub, 90), "icon": icon,
                    "sound": if quiet { "off" } else { &sound }, "kind": kind_});
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
    let (shown_title, shown_sub) = (ev["title"].as_str().unwrap_or(""), ev["sub"].as_str().unwrap_or(""));
    let not_connected = if sent { "" } else { " (stick not connected)" };
    if private {
        log::info!("event {kind_} -> {level}{not_connected}");
        log::debug!("event {kind_} -> {level}: {shown_title} / {shown_sub}");
    } else {
        log::info!("event {kind_} -> {level}: {shown_title} / {shown_sub}{not_connected}");
    }
    Some(ev)
}

/// The dashboard's ▶: the kind's sample, with the level / sound picked on the page if given (saved
/// or not; ones that aren't valid are ignored).
pub async fn test(st: &AppState, k: &str, level: Option<&str>, sound: Option<&str>) -> Option<Value> {
    let (name, _, icon, title, sub) = *kind(k).unwrap_or_else(|| kind("test").expect("test kind"));
    let name = if kind(k).is_some() { name } else { k };
    let level = level.filter(|l| LEVELS.contains(l));
    let sound = sound.filter(|s| sound_ok(s));
    notify(st, name, title, sub, Some(icon), level, sound).await
}

/// The last `n` events, newest first.
pub fn history(st: &AppState, n: usize) -> Vec<Value> {
    let text = std::fs::read_to_string(st.paths.home.join("events.jsonl")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].iter().rev().filter_map(|l| serde_json::from_str(l).ok()).collect()
}

/// `GET /api/events/kinds`; `stick_sounds` is the stick's last sound report.
pub fn kinds_json(cfg: &Map<String, Value>, stick_sounds: Option<&Value>) -> Value {
    let kinds: Map<String, Value> =
        KINDS.iter().map(|k| (k.0.to_string(), json!({"default": k.1, "icon": k.2, "title": k.3}))).collect();
    let level_sounds: Map<String, Value> = LEVEL_SOUNDS.iter().map(|(l, s)| (l.to_string(), json!(s))).collect();
    let known = tune_list(stick_sounds);
    // until the stick has told us its list: the level defaults + the celebrations
    let tunes = known.clone().unwrap_or_else(|| {
        LEVEL_SOUNDS.iter().map(|x| x.1).chain(CELEBRATION_SOUNDS.iter().copied()).map(str::to_string).collect()
    });
    json!({"kinds": kinds, "rules": rules(cfg), "levels": LEVELS, "sounds": sounds(cfg), "level_sounds": level_sounds,
           "tunes": tunes, "tunes_known": known.is_some()})
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

    #[test]
    fn sounds_have_defaults_and_take_valid_user_ones() {
        let s = sounds(&cfg(json!({})));
        assert_eq!((&s["pr_merged"], &s["release"], &s["ci_fixed"]), (&json!("applause"), &json!("party_horn"), &json!("cheer")));
        assert_eq!((&s["ci_failed"], &s["claude_done"]), (&json!(""), &json!("")));
        assert_eq!(s.len(), KINDS.len());
        let s = sounds(&cfg(json!({"event_sounds": {"pr_merged": "off", "ci_failed": "tada", "release": "",
                                                     "nope": "tada", "claude_done": "bad name!", "pr_created": 3}})));
        assert_eq!((&s["pr_merged"], &s["ci_failed"], &s["release"]), (&json!("off"), &json!("tada"), &json!("")));
        assert_eq!((&s["claude_done"], &s["pr_created"]), (&json!(""), &json!("")));
        assert!(!s.contains_key("nope"));
    }

    #[test]
    fn a_sound_the_stick_lacks_goes_out_as_the_level_default() {
        let old: Vec<String> = ["jingle_win", "confirmation_002", "question_001"].map(String::from).to_vec();
        assert_eq!(for_stick("applause", Some(&old)), "");
        assert_eq!(for_stick("jingle_win", Some(&old)), "jingle_win");
        assert_eq!((for_stick("off", Some(&old)), for_stick("", Some(&old))), ("off".into(), "".into()));
        assert_eq!(for_stick("applause", None), "applause"); // not known yet: the stick falls back itself
        assert_eq!(tune_list(Some(&json!({"tunes": ["a", 1, "b"]}))), Some(vec!["a".to_string(), "b".to_string()]));
        assert_eq!(tune_list(Some(&json!({"on": true}))), None);
        assert_eq!(tune_list(None), None);
    }

    #[test]
    fn event_sounds_are_checked_against_the_sticks_list() {
        let tunes: Vec<String> = ["jingle_win", "tada"].map(String::from).to_vec();
        let none = Map::new();
        let bad = |b: Value, t: Option<&[String]>, cur: &Map<String, Value>| invalid_sounds(&cfg(b), t, cur);
        assert_eq!(bad(json!({}), Some(&tunes), &none), "");
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "tada", "ci_failed": "off", "release": ""}}), Some(&tunes), &none), "");
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "anything_newer"}}), None, &none), "");
        assert!(bad(json!({"event_sounds": {"pr_merged": "coins"}}), Some(&tunes), &none).contains("no sound 'coins'"));
        let saved = cfg(json!({"event_sounds": {"pr_merged": "coins"}}));
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "coins"}}), Some(&tunes), &saved), ""); // kept as it was
        // the page sends every kind: a default the (older) stick lacks may stay too
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "applause", "ci_failed": "tada"}}), Some(&tunes), &none), "");
        assert!(bad(json!({"event_sounds": {"ci_failed": "applause"}}), Some(&tunes), &none).contains("no sound"));
        assert!(bad(json!({"event_sounds": ["tada"]}), None, &none).contains("must map"));
        assert!(bad(json!({"event_sounds": {"bogus": "tada"}}), None, &none).contains("no event kind"));
        assert!(bad(json!({"event_sounds": {"pr_merged": "../x"}}), None, &none).contains("not a sound name"));
        assert!(bad(json!({"event_sounds": {"pr_merged": 5}}), None, &none).contains("not a sound name"));
    }

    #[test]
    fn kinds_json_lists_sounds_and_tunes() {
        let v = kinds_json(&cfg(json!({})), None);
        assert_eq!(v["sounds"]["pr_merged"], "applause");
        assert_eq!(v["level_sounds"]["headsup"], "question_001");
        assert_eq!(v["tunes_known"], false);
        assert!(v["tunes"].as_array().unwrap().contains(&json!("coins")));
        let v = kinds_json(&cfg(json!({})), Some(&json!({"tunes": ["jingle_win", "beep-1"]})));
        assert_eq!((v["tunes"].clone(), v["tunes_known"].clone()), (json!(["jingle_win", "beep-1"]), json!(true)));
    }
}
