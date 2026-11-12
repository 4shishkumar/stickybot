//! What the Calendar connection decides, as plain functions: which events count, what is on now, what
//! is next, what today holds, the home card, the reminder, the lines read aloud and the meeting link.
//!
//! No I/O and no clock: every function takes the events, the settings, `now` (unix seconds) and the
//! Mac's time zone, so each rule is tested with made-up days (`agenda_tests.rs`). The OS side is
//! `macos-calendar`; `calendar.rs` joins the two. Times are shown in the Mac's own zone, 24 hour
//! (`10:30`), so an event made in another zone shows at its local instant.
//!
//! An occurrence of a recurring meeting is told apart by `(id, start)`: EventKit gives every
//! occurrence the same `id`.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Timelike};
use macos_calendar::Event;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// How often the cached events are checked for a due reminder, in seconds. It must be well under
/// the shortest lead time the settings allow (1 minute), or a tick could land just before the window
/// opens and the next one just after it closes and the reminder would never go out; 15 s leaves four
/// chances. A check reads nothing from the Calendar.
pub const ALERT_TICK_S: i64 = 15;

/// A meeting that has been reminded about: `(event id, start)`.
pub type Key = (String, i64);

/// Longest reminder lead time the settings accept, in minutes (a reminder two hours ahead is a
/// plan, not a heads-up).
pub const ALERT_MAX_MIN: i64 = 120;
/// Most calendar ids the settings keep (a person has a handful, a shared account a few dozen).
const MAX_CALENDARS: usize = 100;
/// Longest calendar id accepted, in characters (EventKit's are UUIDs, 36).
const MAX_ID_CHARS: usize = 200;

/// The card joins the rotation when the next meeting is today or at most this far ahead, in seconds
/// (8 h: a meeting after a long gap is not "what's on my mind").
const CARD_AHEAD_S: i64 = 8 * 3600;
/// The card is put ahead of the rotation from this long before a start, in seconds (15 min: time to
/// finish a thought, find the link, refill the water).
const PIN_BEFORE_S: i64 = 15 * 60;
/// ...until this long after the start, in seconds (5 min: late arrivals still see which meeting).
const PIN_AFTER_S: i64 = 5 * 60;
/// From this long ahead, in minutes, the card says hours ("in 2 h") instead of minutes.
const HOURS_FROM_MIN: i64 = 90;
/// A title is cut to this many characters on the card (the stick's card line is 60; leaves room).
const CARD_TITLE_MAX: usize = 40;
/// The reminder's title ("Standup in 5 min") must fit the event title the stick shows, in characters.
const ALERT_TITLE_MAX: usize = 40;
/// A meeting starting within this long after `now` can be joined even across midnight, in seconds
/// (a 00:05 call joined at 23:55).
const JOIN_SOON_S: i64 = 15 * 60;
/// How many meetings the spoken day names after the count.
const SPOKEN_NAMED: usize = 2;

/// The Calendar connection's settings (`connections.calendar` in the config).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    pub on: bool,
    /// calendar ids; empty = every calendar
    pub calendars: Vec<String>,
    pub skip_all_day: bool,
    pub skip_declined: bool,
    /// reminder lead time in minutes; 0 = no reminder
    pub alert_min: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { on: false, calendars: Vec::new(), skip_all_day: true, skip_declined: true, alert_min: 5 }
    }
}

const KEYS: &[&str] = &["on", "calendars", "skip_all_day", "skip_declined", "alert_min"];

impl Settings {
    /// From the saved value: a missing or malformed key reads as its default, never as an error.
    pub fn from_value(v: &Value) -> Self {
        let d = Self::default();
        let b = |k: &str, d: bool| v.get(k).and_then(Value::as_bool).unwrap_or(d);
        Self {
            on: b("on", d.on),
            calendars: v
                .get("calendars")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
                .unwrap_or(d.calendars),
            skip_all_day: b("skip_all_day", d.skip_all_day),
            skip_declined: b("skip_declined", d.skip_declined),
            alert_min: v
                .get("alert_min")
                .and_then(Value::as_i64)
                .filter(|n| (0..=ALERT_MAX_MIN).contains(n))
                .unwrap_or(d.alert_min),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"on": self.on, "calendars": self.calendars, "skip_all_day": self.skip_all_day,
               "skip_declined": self.skip_declined, "alert_min": self.alert_min})
    }

    /// These settings with the keys of a (checked) body on top.
    pub fn with(&self, body: &Map<String, Value>) -> Self {
        let mut merged = self.to_json();
        if let Some(m) = merged.as_object_mut() {
            m.extend(body.iter().map(|(k, v)| (k.clone(), v.clone())));
        }
        Self::from_value(&merged)
    }
}

/// What is wrong with a body of calendar settings (any subset of them), in words, or `""`.
pub fn invalid_settings(body: &Map<String, Value>) -> String {
    if let Some(k) = body.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return format!("unknown setting '{k}' (the settings are: {})", KEYS.join(", "));
    }
    for k in ["on", "skip_all_day", "skip_declined"] {
        if body.get(k).is_some_and(|v| !v.is_boolean()) {
            return format!("{k} must be true or false");
        }
    }
    if let Some(c) = body.get("calendars") {
        let ok = c.as_array().is_some_and(|a| {
            a.len() <= MAX_CALENDARS
                && a.iter().all(|id| id.as_str().is_some_and(|s| s.chars().count() <= MAX_ID_CHARS))
        });
        if !ok {
            return format!(
                "calendars must be a list of calendar ids (at most {MAX_CALENDARS}, each up to {MAX_ID_CHARS} characters)"
            );
        }
    }
    if body.get("alert_min").is_some_and(|n| !n.as_i64().is_some_and(|n| (0..=ALERT_MAX_MIN).contains(&n))) {
        return format!("alert_min must be 0-{ALERT_MAX_MIN} minutes");
    }
    String::new()
}

// --- which events count ----------------------------------------------------------------------

fn chosen(e: &Event, s: &Settings) -> bool {
    !e.canceled && (s.calendars.is_empty() || s.calendars.contains(&e.calendar_id))
}

/// A meeting you could be in or be reminded about: timed, not declined, not canceled, in a chosen
/// calendar. All-day and declined events are never one, whatever the two switches say.
fn is_meeting(e: &Event, s: &Settings) -> bool {
    chosen(e, s) && !e.all_day && !e.declined
}

/// The events to list (today's preview, the day's events): chosen calendars, not canceled, and
/// without the all-day / declined ones when their switch is on.
pub fn listed<'a>(events: &'a [Event], s: &Settings) -> Vec<&'a Event> {
    events
        .iter()
        .filter(|e| chosen(e, s) && !(s.skip_all_day && e.all_day) && !(s.skip_declined && e.declined))
        .collect()
}

fn meetings<'a>(events: &'a [Event], s: &Settings) -> impl Iterator<Item = &'a Event> {
    events.iter().filter(|e| is_meeting(e, s))
}

/// The meeting on now (`start <= now < end`): when several overlap, the one that started last. An
/// event ending exactly when another starts is no longer on at that instant.
pub fn current<'a>(events: &'a [Event], s: &Settings, now: i64) -> Option<&'a Event> {
    meetings(events, s)
        .filter(|e| e.start <= now && now < e.end)
        .max_by(|a, b| a.start.cmp(&b.start).then(b.title.cmp(&a.title)))
}

/// The next meeting: the earliest start after `now` (ties: the shorter one, then the title). While
/// one is on, this is still the next start, not the one on now.
pub fn next<'a>(events: &'a [Event], s: &Settings, now: i64) -> Option<&'a Event> {
    meetings(events, s).filter(|e| e.start > now).min_by(|a, b| {
        a.start.cmp(&b.start).then((a.end - a.start).cmp(&(b.end - b.start))).then(a.title.cmp(&b.title))
    })
}

// --- days and times --------------------------------------------------------------------------

fn at<Tz: TimeZone>(tz: &Tz, ts: i64) -> Option<DateTime<Tz>> {
    tz.timestamp_opt(ts, 0).single()
}

/// `10:30` in the zone.
pub fn hhmm<Tz: TimeZone>(tz: &Tz, ts: i64) -> String {
    at(tz, ts).map(|d| format!("{:02}:{:02}", d.hour(), d.minute())).unwrap_or_default()
}

/// The local date of an instant.
pub fn date_of<Tz: TimeZone>(tz: &Tz, ts: i64) -> Option<NaiveDate> {
    at(tz, ts).map(|d| d.date_naive())
}

/// `[00:00, 24:00)` of a local date as unix seconds. A zone whose clocks skip midnight starts the
/// day at the first hour that exists.
pub fn day_bounds<Tz: TimeZone>(tz: &Tz, date: NaiveDate) -> Option<(i64, i64)> {
    let start = |d: NaiveDate| {
        (0..4).find_map(|h| tz.from_local_datetime(&d.and_hms_opt(h, 0, 0)?).earliest()).map(|t| t.timestamp())
    };
    Some((start(date)?, start(date + Duration::days(1))?))
}

/// The events of a local day, sorted by start then title: every event overlapping `[00:00, 24:00)`,
/// so a multi-day event is on each day it covers.
pub fn day<'a, Tz: TimeZone>(events: &'a [Event], s: &Settings, tz: &Tz, date: NaiveDate) -> Vec<&'a Event> {
    let Some((from, to)) = day_bounds(tz, date) else { return Vec::new() };
    let mut v: Vec<&Event> = listed(events, s)
        .into_iter()
        .filter(|e| e.start < to && (e.end > from || (e.start >= from && e.end <= e.start)))
        .collect();
    v.sort_by(|a, b| a.start.cmp(&b.start).then(a.title.cmp(&b.title)));
    v
}

pub fn today<'a, Tz: TimeZone>(events: &'a [Event], s: &Settings, tz: &Tz, now: i64) -> Vec<&'a Event> {
    date_of(tz, now).map(|d| day(events, s, tz, d)).unwrap_or_default()
}

/// `12 min` / `2 h`: how far ahead, rounded up to a minute below an hour and a half, else to the
/// nearest hour.
fn ahead(secs: i64) -> String {
    let min = (secs + 59) / 60;
    if min < HOURS_FROM_MIN { format!("{} min", min.max(1)) } else { format!("{} h", (min + 30) / 60) }
}

fn cut(s: &str, n: usize) -> String {
    let t = crate::talk::stick_text(s);
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= n { t } else { format!("{}…", t.chars().take(n - 1).collect::<String>().trim_end()) }
}

fn title_of(e: &Event, n: usize) -> String {
    let t = cut(&e.title, n);
    if t.is_empty() { "Meeting".into() } else { t }
}

// --- the home card ---------------------------------------------------------------------------

/// The meeting's card for the stick's home screen.
#[derive(Debug, Clone, PartialEq)]
pub struct MeetingCard {
    /// `{"k","a","b"}`
    pub card: Value,
    /// from 15 min before a start until 5 min after it: shown ahead of the rotation
    pub pinned: bool,
}

/// The card kind the stick draws with a calendar icon: every released firmware (1.8.0 on) knows
/// `meeting`, and one that did not would draw it as a plain card.
pub const CARD_KIND: &str = "meeting";

fn card(a: String, b: String, pinned: bool) -> MeetingCard {
    MeetingCard { card: json!({"k": CARD_KIND, "a": a, "b": b}), pinned }
}

/// The meeting card now, or `None`: nothing to show when no meeting is on, or the next is neither
/// today nor within 8 h.
///
/// A meeting that just started (up to 5 min) or one starting within 15 min is pinned; one that has
/// been on a while, or starts later, only joins the rotation.
pub fn meeting_card<Tz: TimeZone>(events: &[Event], s: &Settings, tz: &Tz, now: i64) -> Option<MeetingCard> {
    let running = current(events, s, now);
    let upcoming = next(events, s, now);
    let running_card =
        |e: &Event, pinned| card(title_of(e, CARD_TITLE_MAX), format!("now · until {}", hhmm(tz, e.end)), pinned);
    let upcoming_card = |e: &Event, pinned| {
        card(title_of(e, CARD_TITLE_MAX), format!("in {} · {}", ahead(e.start - now), hhmm(tz, e.start)), pinned)
    };
    if let Some(e) = running.filter(|e| now - e.start < PIN_AFTER_S) {
        return Some(running_card(e, true));
    }
    if let Some(e) = upcoming.filter(|e| e.start - now <= PIN_BEFORE_S) {
        return Some(upcoming_card(e, true));
    }
    if let Some(e) = running {
        return Some(running_card(e, false));
    }
    let e = upcoming?;
    let soon = e.start - now <= CARD_AHEAD_S || date_of(tz, e.start) == date_of(tz, now);
    soon.then(|| upcoming_card(e, false))
}

// --- the reminder ----------------------------------------------------------------------------

/// The meetings to remind about now: `alert_min` before the start up to the start, once per
/// occurrence (`fired` holds the ones already done). Never all-day or declined, never with
/// `alert_min` 0. A meeting created or moved into the window is reminded once, at that point.
pub fn due_alerts<'a>(events: &'a [Event], s: &Settings, now: i64, fired: &HashMap<Key, i64>) -> Vec<&'a Event> {
    if !s.on || s.alert_min <= 0 {
        return Vec::new();
    }
    let lead = s.alert_min * 60;
    let mut due: Vec<&Event> = meetings(events, s)
        .filter(|e| e.start - lead <= now && now < e.start && !fired.contains_key(&(e.id.clone(), e.start)))
        .collect();
    due.sort_by(|a, b| a.start.cmp(&b.start).then(a.title.cmp(&b.title)));
    due
}

/// Forgets the reminders of meetings that are over (the map's value is the meeting's end).
pub fn prune_fired(fired: &mut HashMap<Key, i64>, now: i64) {
    fired.retain(|_, end| *end > now);
}

/// `("Standup in 5 min", "10:30 · Google Meet")`: the title is cut so the "in 5 min" is never lost.
pub fn alert_text<Tz: TimeZone>(e: &Event, tz: &Tz, now: i64) -> (String, String) {
    let secs = e.start - now;
    let when = if secs < 60 { " starts now".to_string() } else { format!(" in {} min", ((secs + 30) / 60).max(1)) };
    let title = format!("{}{when}", title_of(e, ALERT_TITLE_MAX.saturating_sub(when.chars().count())));
    let place =
        join_link(e).map(|(p, _)| p.to_string()).unwrap_or_else(|| cut(e.location.lines().next().unwrap_or(""), 40));
    let sub = if place.is_empty() { hhmm(tz, e.start) } else { format!("{} · {place}", hhmm(tz, e.start)) };
    (title, sub)
}

// --- what is said ----------------------------------------------------------------------------

/// "Standup in 12 min, at 10:30." / "Standup is on now, until 11:00." / "Nothing else today."
pub fn say_next<Tz: TimeZone>(events: &[Event], s: &Settings, tz: &Tz, now: i64) -> String {
    let (running, upcoming) = (current(events, s, now), next(events, s, now));
    let today_next = upcoming.filter(|e| date_of(tz, e.start) == date_of(tz, now));
    let next_line = |e: &Event| format!("{} in {}, at {}.", title_of(e, 60), ahead(e.start - now), hhmm(tz, e.start));
    match (running, today_next) {
        (Some(r), Some(n)) => {
            format!(
                "{} is on now, until {}. Next: {} at {}.",
                title_of(r, 60),
                hhmm(tz, r.end),
                title_of(n, 60),
                hhmm(tz, n.start)
            )
        }
        (Some(r), None) => format!("{} is on now, until {}.", title_of(r, 60), hhmm(tz, r.end)),
        (None, Some(n)) => next_line(n),
        (None, None) => "Nothing else today.".into(),
    }
}

/// "3 meetings today. Next: Standup at 10:30, then Design review at 14:00."
pub fn say_today<Tz: TimeZone>(events: &[Event], s: &Settings, tz: &Tz, now: i64) -> String {
    let day = today(events, s, tz, now);
    let all: Vec<&&Event> = day.iter().filter(|e| is_meeting(e, s)).collect();
    if all.is_empty() {
        return "Nothing on your calendar today.".into();
    }
    let count = format!("{} meeting{} today.", all.len(), if all.len() == 1 { "" } else { "s" });
    let upcoming: Vec<String> = all
        .iter()
        .filter(|e| e.start > now)
        .take(SPOKEN_NAMED)
        .map(|e| format!("{} at {}", title_of(e, 60), hhmm(tz, e.start)))
        .collect();
    if !upcoming.is_empty() {
        return format!("{count} Next: {}.", upcoming.join(", then "));
    }
    match current(events, s, now) {
        Some(r) => format!("{count} {} is on now, until {}.", title_of(r, 60), hhmm(tz, r.end)),
        None => format!("{count} None left."),
    }
}

/// One line per event for a tool: `10:30-11:00 Standup (Google Meet)`; `all day` for an all-day one.
pub fn day_line<Tz: TimeZone>(e: &Event, tz: &Tz) -> String {
    let when = if e.all_day { "all day".to_string() } else { format!("{}-{}", hhmm(tz, e.start), hhmm(tz, e.end)) };
    let place =
        join_link(e).map(|(p, _)| p.to_string()).unwrap_or_else(|| cut(e.location.lines().next().unwrap_or(""), 40));
    let mut line = format!("{when} {}", title_of(e, 80));
    if !place.is_empty() {
        line.push_str(&format!(" ({place})"));
    }
    if e.declined {
        line.push_str(" [declined]");
    }
    line
}

// --- meeting links ---------------------------------------------------------------------------

/// The provider's name when `u` is a meeting link on a known conference host (https only), else
/// `None`. Anything else is never a meeting link: notes are text other people wrote.
fn provider(u: &url::Url) -> Option<&'static str> {
    if u.scheme() != "https" || u.port().is_some() || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str()?.to_ascii_lowercase();
    let path = u.path();
    let under = |domain: &str| host == domain || host.ends_with(&format!(".{domain}"));
    let has_more = path.len() > 1;
    if host == "meet.google.com" && has_more {
        Some("Google Meet")
    } else if under("zoom.us") && ["/j/", "/my/", "/w/"].iter().any(|p| path.starts_with(p) && path.len() > p.len()) {
        Some("Zoom")
    } else if (host == "teams.microsoft.com"
        && (path.starts_with("/l/meetup-join/") || path.starts_with("/meet/"))
        && has_more)
        || (host == "teams.live.com" && path.starts_with("/meet/") && has_more)
    {
        Some("Microsoft Teams")
    } else if under("webex.com") && has_more {
        Some("Webex")
    } else {
        None
    }
}

/// Every `https://…` / `http://…` in a text, with the punctuation and markup around it dropped.
fn urls(text: &str) -> impl Iterator<Item = url::Url> + '_ {
    static RE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r#"(?i)https?://[^\s<>"'\\]+"#).expect("the link pattern"));
    RE.find_iter(text).filter_map(|m| {
        let raw = m.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '}', '>']);
        url::Url::parse(raw).ok()
    })
}

/// The meeting link of an event: the first conference link in its URL, then its location, then its
/// notes. `(provider, https url)`.
pub fn join_link(e: &Event) -> Option<(&'static str, String)> {
    [e.url.as_str(), e.location.as_str(), e.notes.as_str()]
        .into_iter()
        .flat_map(urls)
        .find_map(|u| provider(&u).map(|p| (p, u.to_string())))
}

/// The meeting to join: the one on now (the one that started last), else the next ones today (or
/// starting within 15 min, across midnight) in order, the first of them that has a link. Not
/// declined, not all-day. (A meeting on now without a link is passed over for the next one that
/// has one.)
pub fn join_choice<'a, Tz: TimeZone>(
    events: &'a [Event],
    s: &Settings,
    tz: &Tz,
    now: i64,
) -> Option<(&'a Event, &'static str, String)> {
    let mut running: Vec<&Event> = meetings(events, s).filter(|e| e.start <= now && now < e.end).collect();
    running.sort_by(|a, b| b.start.cmp(&a.start).then(a.title.cmp(&b.title)));
    let mut later: Vec<&Event> = meetings(events, s)
        .filter(|e| e.start > now && (date_of(tz, e.start) == date_of(tz, now) || e.start - now <= JOIN_SOON_S))
        .collect();
    later.sort_by(|a, b| a.start.cmp(&b.start).then(a.title.cmp(&b.title)));
    running.into_iter().chain(later).find_map(|e| join_link(e).map(|(p, u)| (e, p, u)))
}

#[cfg(test)]
#[path = "agenda_tests.rs"]
mod tests;
