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
use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use stick_link::{Backend, Link};

tokio::task_local! { static DEVICE_TARGETED: bool; }

/// Explicit API/tool targets take precedence over background notification delivery policy.
pub async fn for_device<F: std::future::Future>(future: F) -> F::Output {
    DEVICE_TARGETED.scope(true, future).await
}
pub fn device_targeted() -> bool {
    DEVICE_TARGETED.try_with(|targeted| *targeted).unwrap_or(false)
}

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
pub const DEFAULT_SOUNDS: &[(&str, &str)] =
    &[("pr_merged", "applause"), ("release", "party_horn"), ("ci_fixed", "cheer")];
/// The stick's sound per level (events.cpp `defaultSound`).
pub const LEVEL_SOUNDS: &[(&str, &str)] =
    &[("celebrate", "jingle_win"), ("nice", "confirmation_002"), ("headsup", "question_001")];
/// Firmware 2.7.0's celebration clips: offered while the stick hasn't told us its list.
pub const CELEBRATION_SOUNDS: &[&str] =
    &["applause", "clap", "cheer", "party_horn", "tada", "drumroll", "level_up", "coins"];

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
    /// ids the stick acknowledged
    pub shown: Mutex<HashSet<String>>,
}

impl Notifier {
    pub fn acked(&self, id: &str) {
        self.shown.lock().unwrap().insert(id.to_string());
    }
}

/// Stick-ready text: no control characters (a NUL blanks a field on the stick, others draw as boxes).
fn cut(s: &str, n: usize) -> String {
    stick_text(s).chars().map(|c| if c.is_control() { ' ' } else { c }).take(n).collect()
}

const DELIVERY_TTL: Duration = Duration::from_secs(300);
const DELIVERY_LIMIT: usize = 16;

struct Delivery {
    event: Value,
    sent: Instant,
    generation: u64,
    opened: bool,
    read: bool,
}

/// Each device owns its delivery window. An event is recorded before its BLE write (a stick can
/// answer before the write returns) and forgotten if the write fails; the lock is never held
/// across the write, so a long send doesn't stall the stick's other messages.
#[derive(Default)]
pub(crate) struct Deliveries(VecDeque<Delivery>);

impl Deliveries {
    fn record(&mut self, event: &Value, generation: u64, now: Instant) {
        self.0.retain(|d| d.generation == generation && now.duration_since(d.sent) < DELIVERY_TTL);
        if self.0.len() == DELIVERY_LIMIT {
            self.0.pop_front();
        }
        self.0.push_back(Delivery { event: event.clone(), sent: now, generation, opened: false, read: false });
    }

    fn forget(&mut self, id: &Value) {
        self.0.retain(|d| &d.event["id"] != id);
    }

    fn take(&mut self, msg: &Value, generation: u64, now: Instant) -> Option<DetailAction> {
        let fields = msg.as_object()?;
        let action = msg["action"].as_str()?;
        if fields
            .keys()
            .any(|k| !matches!(k.as_str(), "src" | "evt" | "id" | "action") && !(action == "open" && k == "url"))
        {
            return None;
        }
        let id = msg["id"].as_str()?;
        let d =
            self.0.iter_mut().rev().find(|d| {
                d.event["id"] == id && d.generation == generation && now.duration_since(d.sent) < DELIVERY_TTL
            })?;
        match action {
            "open" if !d.opened => {
                let url = d.event["url"].as_str()?;
                if msg["url"].as_str()? != url || !valid_event_url(url) {
                    return None;
                }
                d.opened = true;
                Some(DetailAction::Open(url.to_owned()))
            }
            "read" if !d.read => {
                let text = ["title", "actor", "sub", "body"]
                    .iter()
                    .filter_map(|key| d.event[*key].as_str().filter(|s| !s.is_empty()))
                    .collect::<Vec<_>>()
                    .join(". ");
                if text.is_empty() {
                    return None;
                }
                d.read = true;
                Some(DetailAction::Read(text))
            }
            _ => None,
        }
    }
}

enum DetailAction {
    Open(String),
    Read(String),
}

fn valid_event_url(url: &str) -> bool {
    let authority = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|s| s.split(['/', '?', '#']).next());
    authority.is_some_and(|s| !s.is_empty() && !s.contains('@'))
        && url.chars().count() <= 200
        && !url.contains(|c: char| c == '\\' || c.is_whitespace() || c.is_control())
        && crate::tool_policy::normalized_url(url).is_ok()
}

fn current(link: &Link<Backend>, generation: u64) -> bool {
    !link.paused()
        && link.connected_peer().is_some_and(|(live, addr)| {
            live == generation && link.target().is_some_and(|target| target.address.eq_ignore_ascii_case(&addr))
        })
}

/// Handle a direct physical gesture on a delivered event. Stick-supplied text is never spoken.
pub(crate) async fn detail_action(st: &Arc<AppState>, generation: u64, msg: &Value) {
    let action = {
        let mut delivered = st.home.deliveries.lock().await;
        if !current(&st.link, generation) {
            return;
        }
        delivered.take(msg, generation, Instant::now())
    };
    match action {
        Some(DetailAction::Open(url)) => {
            let st = st.clone();
            tokio::task::spawn_blocking(move || {
                if current(&st.link, generation) {
                    st.runner()(&crate::commands::Action::Open(vec![url]));
                }
            });
        }
        Some(DetailAction::Read(text)) if current(&st.link, generation) => {
            crate::speak::read_event(st, &text, generation);
        }
        _ => {}
    }
}

struct Destination {
    id: String,
    link: Link<Backend>,
    deliveries: Arc<tokio::sync::Mutex<Deliveries>>,
}

impl From<&AppState> for Destination {
    fn from(st: &AppState) -> Self {
        Self { id: st.device_id.clone(), link: st.link.clone(), deliveries: st.home.deliveries.clone() }
    }
}

impl Destination {
    async fn deliver(&self, event: &Value) -> bool {
        let generation = self.link.bound_generation();
        {
            let mut deliveries = self.deliveries.lock().await;
            if !current(&self.link, generation) {
                return false;
            }
            deliveries.record(event, generation, Instant::now());
        }
        let msg = json!({"src":"agent","event":event});
        if !self.link.with_generation(generation, self.link.send(&msg)).await {
            self.deliveries.lock().await.forget(&event["id"]);
            return false;
        }
        true
    }
}

/// Resolve delivery policy in one place for both availability checks and sends.
fn destinations(st: &AppState, cfg: &Map<String, Value>) -> Vec<Destination> {
    if device_targeted() {
        return vec![Destination::from(st)];
    }
    if let Some(ids) = cfg.get("notification_devices").and_then(Value::as_array) {
        let mut seen = HashSet::new();
        return ids
            .iter()
            .filter_map(|id| st.devices.resolve(id.as_str()?))
            .filter(|device| seen.insert(device.device_id.clone()))
            .map(|device| Destination::from(device.as_ref()))
            .collect();
    }
    if st.device_id == "primary"
        && let Some(device) = st.devices.default_session()
    {
        return vec![Destination::from(device.as_ref())];
    }
    vec![Destination::from(st)]
}

/// A scheduled reminder is consumed only after a successful send or intentional suppression.
/// Offline destinations leave it pending without filling event history on every tick. A scoped
/// Calendar refresh still follows background notification policy, not the API request's target.
pub async fn notify_reminder(st: &AppState, kind: &str, title: &str, sub: &str) -> bool {
    DEVICE_TARGETED
        .scope(false, async {
            let cfg = st.cfg.lock().unwrap().clone();
            if cfg.get("notification_devices").and_then(Value::as_array).is_some_and(Vec::is_empty) {
                return true;
            }
            if !destinations(st, &cfg).iter().any(|d| d.link.connected()) {
                return false;
            }
            notify_with_delivery(st, kind, title, sub, None, None, None, None).await.is_none_or(|(_, sent)| sent)
        })
        .await
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
    notify_with_delivery(st, kind_, title, sub, icon, level, sound, None).await.map(|(event, _)| event)
}

/// [`notify`] with the event's details for the stick's detail page: `actor` (who), `url` (opened on
/// the Mac from the stick), `body` (a few lines of what happened). Each is optional; long ones are cut.
pub async fn notify_with(st: &AppState, kind_: &str, title: &str, sub: &str, extra: &Value) -> Option<Value> {
    notify_with_delivery(st, kind_, title, sub, None, None, None, Some(extra)).await.map(|(event, _)| event)
}

#[allow(clippy::too_many_arguments)]
async fn notify_with_delivery(
    st: &AppState,
    kind_: &str,
    title: &str,
    sub: &str,
    icon: Option<&str>,
    level: Option<&str>,
    sound: Option<&str>,
    extra: Option<&Value>,
) -> Option<(Value, bool)> {
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
    // A restarted Mac cannot reuse an ID that a paired stick might still be displaying.
    let id = format!("e{}", &crate::config::new_token()[..16]);
    let quiet = quiet_now(&cfg);
    let sound = match sound {
        Some(s) => s.to_string(),
        None => sounds(&cfg).get(kind_).and_then(Value::as_str).unwrap_or("").to_string(),
    };
    let sound = for_stick(&sound, tune_list(st.sounds().as_ref()).as_deref());
    let icon = icon.map(str::to_string).unwrap_or_else(|| kind(kind_).map(|k| k.2).unwrap_or("bell").to_string());
    let mut ev = json!({"id": id, "level": level, "title": cut(title, 40), "sub": cut(sub, 90), "icon": icon,
                    "sound": if quiet { "off" } else { &sound }, "kind": kind_});
    for (key, max) in [("actor", 24), ("body", 300)] {
        if let Some(v) = extra.and_then(|e| e[key].as_str()).filter(|s| !s.is_empty()) {
            ev[key] = json!(cut(v, max));
        }
    }
    // URLs are exact: sanitizing or truncating a link can change its destination.
    if let Some(url) = extra.and_then(|e| e["url"].as_str()).filter(|s| valid_event_url(s)) {
        ev["url"] = json!(url);
    }
    let mut sent = false;
    let mut destination_ids = Vec::new();
    for destination in destinations(st, &cfg) {
        sent |= destination.deliver(&ev).await;
        destination_ids.push(destination.id);
    }
    let mut entry = Map::new();
    entry.insert("device_id".into(), json!(st.device_id));
    entry.insert("device_ids".into(), json!(destination_ids));
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
    Some((ev, sent))
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
    text.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| {
            row["device_id"].as_str().unwrap_or("primary") == st.device_id
                || row["device_ids"].as_array().is_some_and(|ids| ids.iter().any(|id| id == &st.device_id))
        })
        .take(n)
        .collect()
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
    #[test]
    fn stick_text_has_no_control_characters_and_failed_sends_are_forgotten() {
        assert_eq!(cut("Fix\u{0}the\ttest\nnow", 40), "Fix the test now");
        let mut d = Deliveries::default();
        let now = std::time::Instant::now();
        let ev = json!({"id": "e1", "title": "t", "url": "https://github.com/a/b/pull/1"});
        d.record(&ev, 7, now);
        d.forget(&json!("e1"));
        let open = json!({"src": "agent", "evt": "event_action", "id": "e1", "action": "open", "url": "https://github.com/a/b/pull/1"});
        assert!(d.take(&open, 7, now).is_none());
    }

    use super::*;

    fn cfg(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[tokio::test]
    async fn detail_action_skips_failed_sends_and_omits_bad_links() {
        let t = crate::talk::tests::agent("event-undelivered", false).await;
        let extra = json!({"url":format!("https://example.org/{}", "x".repeat(200)), "body":"One line"});
        let ev = crate::events::notify_with(&t.st, "release", "V2", "", &extra).await.unwrap();
        assert!(ev.get("url").is_none());
        let gen_before = t.st.link.generation();
        t.fake.set_busy(10, "write failed");
        let ev = crate::events::notify_with(&t.st, "release", "V3", "", &json!({"url":"https://example.org"}))
            .await
            .unwrap();
        let read = json!({"id":ev["id"],"action":"read"});
        assert!(t.st.home.deliveries.lock().await.take(&read, gen_before, std::time::Instant::now()).is_none());
    }

    #[test]
    fn detail_actions_expire_and_evict_old_deliveries() {
        let now = Instant::now();
        let mut history = Deliveries::default();
        let open = |id| json!({"id":id,"action":"open","url":"https://example.org/details"});
        for id in 0..=DELIVERY_LIMIT {
            history.record(&json!({"id":id.to_string(),"url":"https://example.org/details"}), 2, now);
        }
        assert_eq!(history.0.len(), DELIVERY_LIMIT);
        assert!(history.take(&open("0"), 2, now).is_none());
        assert!(history.take(&open("1"), 3, now).is_none());
        assert!(history.take(&open("1"), 2, now + DELIVERY_TTL).is_none());
        assert!(history.take(&open("2"), 2, now).is_some());
        assert!(history.take(&open("2"), 2, now).is_none());

        history.record(&json!({"id":"same","url":"https://example.org/old","body":"old"}), 2, now);
        history.record(&json!({"id":"same","url":"https://example.org/new","body":"new"}), 2, now);
        let old = json!({"id":"same","action":"open","url":"https://example.org/old"});
        let forged = json!({"id":"same","action":"read","text":"from stick"});
        assert!(history.take(&old, 2, now).is_none());
        assert!(history.take(&forged, 2, now).is_none());
        assert!(matches!(history.take(&json!({"id":"same","action":"read"}), 2, now),
                          Some(DetailAction::Read(text)) if text == "new"));
    }

    #[test]
    fn detail_event_urls_require_an_exact_safe_destination() {
        for raw in [
            "file:///tmp/a",
            "https:",
            "http:///a",
            "https://a:b@example.org/",
            "https://@example.org",
            "https://example.org/a b",
            "https://example.org/a\nb",
            "https://example.org/a\\b",
            " https://example.org",
            "https://example.org ",
        ] {
            assert!(!valid_event_url(raw), "{raw}");
        }
        let prefix = "https://example.org/";
        let longest = format!("{prefix}{}", "a".repeat(200 - prefix.len()));
        assert!(valid_event_url(&longest));
        assert!(!valid_event_url(&(longest + "x")));
        assert!(valid_event_url("https://example.org/one%20two?issue=4#notes"));
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
        assert_eq!(
            (&s["pr_merged"], &s["release"], &s["ci_fixed"]),
            (&json!("applause"), &json!("party_horn"), &json!("cheer"))
        );
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
        assert_eq!(
            bad(json!({"event_sounds": {"pr_merged": "tada", "ci_failed": "off", "release": ""}}), Some(&tunes), &none),
            ""
        );
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "anything_newer"}}), None, &none), "");
        assert!(bad(json!({"event_sounds": {"pr_merged": "coins"}}), Some(&tunes), &none).contains("no sound 'coins'"));
        let saved = cfg(json!({"event_sounds": {"pr_merged": "coins"}}));
        assert_eq!(bad(json!({"event_sounds": {"pr_merged": "coins"}}), Some(&tunes), &saved), ""); // kept as it was
        // the page sends every kind: a default the (older) stick lacks may stay too
        assert_eq!(
            bad(json!({"event_sounds": {"pr_merged": "applause", "ci_failed": "tada"}}), Some(&tunes), &none),
            ""
        );
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
