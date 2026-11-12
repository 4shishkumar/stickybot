//! The Calendar connection: the Mac's Calendar (EventKit, through `macos-calendar`) read-only, kept
//! fresh in memory, and what the rest of the app does with it: the home card, the `meeting_soon`
//! reminder, voice phrases, the stick tools and the API under `/api/connections/calendar`.
//!
//! The rules are in `agenda.rs` (pure, tested). This file holds the state and the plumbing: the
//! source (EventKit, or `FakeCalendar` with the fake stick and in tests, so nothing there can reach
//! the real Calendar), the cached events, the last error and the reminders already sent.
//!
//! **Privacy**: events live in memory only (the next 48 h, replaced on every refresh); titles,
//! attendees, locations and links are never logged at `info` or above, and the diagnostics
//! (`feedback.rs`) take them out through [`private_strings`]. They leave this Mac only to the stick
//! (the card, the reminder) and to the brain when it calls a calendar tool.
//!
//! **Refresh**: when connected, when the settings change, when EventKit says the calendar changed,
//! and every 60 s (which also drives the reminder and the card).

use super::Status;
use super::agenda::{self, Key, MeetingCard, Settings};
use crate::AppState;
use crate::commands::Action;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use chrono::{Duration as Days, FixedOffset, NaiveDate};
use macos_calendar::{Access, CalendarInfo, CalendarSource, Event};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the events are read again, the reminder checked and the card kept right, in seconds. A
/// minute is the finest a "starts in N min" text needs, and cheap: one query of two days.
pub const REFRESH_S: u64 = 60;
/// After EventKit says "changed", wait this long, in milliseconds, before reading: one edit in the
/// Calendar app posts several notifications in a burst.
const CHANGE_SETTLE_MS: u64 = 700;
/// How many days from the start of today the events are read: today and tomorrow.
const WINDOW_DAYS: i64 = 2;
/// Most lines a tool or the preview lists.
const MAX_LINES: usize = 12;
/// Shown to a person and to the brain when Calendar is off or lacks its permission.
pub const NOT_CONNECTED: &str = "Calendar isn't connected. Open Connections in the dashboard.";

/// The Mac's time zone: the system's, or a fixed offset in tests.
#[derive(Clone, Copy)]
enum Zone {
    Local,
    Fixed(FixedOffset),
}

/// Runs `$body` with `$tz` bound to the zone, whichever kind it is (chrono's functions are generic).
macro_rules! zoned {
    ($cal:expr, $tz:ident => $body:expr) => {
        match $cal.zone() {
            Zone::Local => {
                let $tz = chrono::Local;
                $body
            }
            Zone::Fixed(o) => {
                let $tz = o;
                $body
            }
        }
    };
}

#[derive(Default)]
struct Cache {
    events: Vec<Event>,
    calendars: Vec<CalendarInfo>,
    /// the range the events were read for (unix seconds)
    window: (i64, i64),
    /// why the last read failed, or empty
    error: String,
    /// what macOS said when asking for permission failed or is still open, or empty
    note: String,
    /// meetings already reminded about -> when they end
    fired: HashMap<Key, i64>,
    /// the meeting card as last pushed to the stick (only a pinned one matters): its title
    pushed: Option<String>,
}

/// What a clock says: the system's, or a fixed time and zone offset (tests).
#[derive(Clone, Copy)]
enum Clock {
    System,
    Fixed { now: i64, offset_s: i32 },
}

pub struct Calendar {
    source: Mutex<Arc<dyn CalendarSource>>,
    cache: Mutex<Cache>,
    subscribed: AtomicBool,
    changed: Arc<tokio::sync::Notify>,
    refreshing: tokio::sync::Mutex<()>,
    clock: Mutex<Clock>,
}

impl Calendar {
    pub fn new(source: Arc<dyn CalendarSource>) -> Self {
        Self {
            source: Mutex::new(source),
            cache: Mutex::new(Cache::default()),
            subscribed: AtomicBool::new(false),
            changed: Arc::new(tokio::sync::Notify::new()),
            refreshing: tokio::sync::Mutex::new(()),
            clock: Mutex::new(Clock::System),
        }
    }

    pub fn source(&self) -> Arc<dyn CalendarSource> {
        self.source.lock().unwrap().clone()
    }

    /// Swaps the source (tests); it is subscribed to again when next needed.
    pub fn set_source(&self, s: Arc<dyn CalendarSource>) {
        *self.source.lock().unwrap() = s;
        self.subscribed.store(false, Ordering::Relaxed);
        *self.cache.lock().unwrap() = Cache::default();
    }

    /// Fixes the clock and the zone (tests): `now` unix seconds, the zone's offset east of UTC.
    pub fn set_clock(&self, now: i64, offset_s: i32) {
        *self.clock.lock().unwrap() = Clock::Fixed { now, offset_s };
    }

    fn now(&self) -> i64 {
        match *self.clock.lock().unwrap() {
            Clock::System => crate::hub::now() as i64,
            Clock::Fixed { now, .. } => now,
        }
    }

    fn zone(&self) -> Zone {
        match *self.clock.lock().unwrap() {
            Clock::Fixed { offset_s, .. } => FixedOffset::east_opt(offset_s).map_or(Zone::Local, Zone::Fixed),
            Clock::System => Zone::Local,
        }
    }

    /// Forgets everything read (off, disconnected).
    fn clear(&self) {
        let mut c = self.cache.lock().unwrap();
        *c = Cache { note: String::new(), ..Cache::default() };
    }

    fn subscribe(&self) {
        if !self.subscribed.swap(true, Ordering::Relaxed) {
            let changed = self.changed.clone();
            self.source().on_change(Box::new(move || changed.notify_one()));
        }
    }

    /// The titles, attendees, locations and links of the cached events: what the diagnostics must
    /// not carry out of the Mac.
    pub fn private_strings(&self) -> Vec<String> {
        let c = self.cache.lock().unwrap();
        let mut out = Vec::new();
        for e in &c.events {
            out.extend([e.title.clone(), e.location.clone(), e.url.clone()]);
            out.extend(e.attendees.iter().cloned());
            if let Some((_, link)) = agenda::join_link(e) {
                out.push(link);
            }
        }
        out.retain(|s| !s.trim().is_empty());
        out.sort();
        out.dedup();
        out
    }
}

/// The settings now (`connections.calendar`); anything missing or malformed reads as its default.
pub fn settings(st: &AppState) -> Settings {
    let cfg = st.cfg.lock().unwrap();
    Settings::from_value(cfg.get("connections").and_then(|c| c.get("calendar")).unwrap_or(&Value::Null))
}

fn save_settings(st: &AppState, s: &Settings) -> Result<(), String> {
    let mut calendar = Map::new();
    calendar.insert("calendar".into(), s.to_json());
    st.update_config([("connections".to_string(), Value::Object(calendar))].into_iter().collect()).map(|_| ())
}

/// Connected: on, and macOS lets us read.
fn connected(st: &AppState) -> Option<Settings> {
    let s = settings(st);
    (s.on && st.calendar.source().access() == Access::Full).then_some(s)
}

fn permission_words(a: Access) -> &'static str {
    match a {
        Access::NotAsked => "macOS hasn't asked yet: press Connect",
        Access::Denied => "StickyBot isn't allowed to read your calendar",
        Access::Restricted => "this Mac doesn't allow StickyBot to read the calendar",
        Access::WriteOnly => "StickyBot can only add events: it needs Full Access to read them",
        Access::Full => "",
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The connection in the common shape, with `permission` beside it (`""`, `not_asked`, `denied`,
/// `restricted`, `write_only`).
pub fn connection(st: &AppState) -> Value {
    let s = settings(st);
    let cal = &st.calendar;
    let mk = |status, detail: &str, permission: &str| {
        super::shape("calendar", "Calendar", status, detail, s.to_json(), json!({"permission": permission}))
    };
    if !s.on {
        return mk(Status::Off, "Not connected", "");
    }
    let access = cal.source().access();
    if access != Access::Full {
        let note = cal.cache.lock().unwrap().note.clone();
        let detail =
            if access == Access::NotAsked && !note.is_empty() { note } else { permission_words(access).to_string() };
        return mk(Status::NeedsPermission, &detail, access.permission());
    }
    let (events, calendars, error, now) = {
        let c = cal.cache.lock().unwrap();
        (c.events.clone(), c.calendars.clone(), c.error.clone(), cal.now())
    };
    if !error.is_empty() {
        return mk(Status::Error, &error, "");
    }
    let count = if s.calendars.is_empty() {
        calendars.len()
    } else {
        calendars.iter().filter(|c| s.calendars.contains(&c.id)).count()
    };
    let next = zoned!(cal, tz => match agenda::next(&events, &s, now) {
        Some(e) if agenda::date_of(&tz, e.start) == agenda::date_of(&tz, now) => format!("next at {}", agenda::hhmm(&tz, e.start)),
        Some(e) => format!("next {}", chrono::TimeZone::timestamp_opt(&tz, e.start, 0).single().map(|d| d.format("%a %H:%M").to_string()).unwrap_or_default()),
        None => "nothing coming up".to_string(),
    });
    mk(Status::Connected, &format!("{} · {next}", plural(count, "calendar", "calendars")), "")
}

// --- reading and the loop --------------------------------------------------------------------

/// What one read of the source returns: the calendars and the events (`None`: no permission).
type Read = (Vec<CalendarInfo>, Vec<Event>);

/// Reads the calendar again and runs the reminder / card check. Off: forgets everything.
pub async fn refresh(st: &Arc<AppState>) {
    let cal = &st.calendar;
    let _one_at_a_time = cal.refreshing.lock().await;
    let s = settings(st);
    if !s.on {
        cal.clear();
        return;
    }
    let source = cal.source();
    let window = zoned!(cal, tz => agenda::date_of(&tz, cal.now())
        .and_then(|d| Some((agenda::day_bounds(&tz, d)?.0, agenda::day_bounds(&tz, d + Days::days(WINDOW_DAYS - 1))?.1))));
    let Some((from, to)) = window else { return };
    let read = tokio::task::spawn_blocking(move || -> Result<Option<Read>, String> {
        if source.access() != Access::Full {
            return Ok(None);
        }
        Ok(Some((source.calendars()?, source.events(from, to)?)))
    })
    .await
    .unwrap_or_else(|_| Err("reading the calendar stopped unexpectedly".into()));
    {
        let mut c = cal.cache.lock().unwrap();
        match read {
            Ok(Some((calendars, events))) => {
                log::debug!("calendar: {} events in {} calendars", events.len(), calendars.len());
                *c = Cache {
                    calendars,
                    events,
                    window: (from, to),
                    fired: std::mem::take(&mut c.fired),
                    pushed: c.pushed.take(),
                    ..Cache::default()
                };
            }
            Ok(None) => {
                let note = std::mem::take(&mut c.note);
                *c = Cache { note, ..Cache::default() };
            }
            Err(e) => {
                log::warn!("calendar: can't read it: {e}");
                c.error = e;
            }
        }
    }
    tick(st).await;
}

/// Sends the due reminders and keeps the stick's card right. Needs no read: it works on what is cached.
pub async fn tick(st: &Arc<AppState>) {
    let Some(s) = connected(st) else { return };
    let cal = &st.calendar;
    let now = cal.now();
    // a reminder the stick cannot show is not "sent": it still goes out if the stick is back in time
    let due: Vec<(String, String)> = if st.link.connected() {
        let mut c = cal.cache.lock().unwrap();
        agenda::prune_fired(&mut c.fired, now);
        let due: Vec<Event> = agenda::due_alerts(&c.events, &s, now, &c.fired).into_iter().cloned().collect();
        for e in &due {
            c.fired.insert((e.id.clone(), e.start), e.end);
        }
        zoned!(cal, tz => due.iter().map(|e| agenda::alert_text(e, &tz, now)).collect())
    } else {
        Vec::new()
    };
    for (title, sub) in due {
        crate::events::notify(st, "meeting_soon", &title, &sub, None, None, None).await;
    }
    let pinned = meeting_card(st).filter(|m| m.pinned).map(|m| m.card["a"].as_str().unwrap_or("").to_string());
    let changed = {
        let mut c = cal.cache.lock().unwrap();
        let changed = c.pushed != pinned;
        c.pushed = pinned;
        changed
    };
    if changed {
        crate::home::push_home(st).await; // the card, now and not with the next hello
    }
}

/// The meeting card for the home screen, `None` when Calendar is off or has nothing to show.
pub fn meeting_card(st: &AppState) -> Option<MeetingCard> {
    let s = connected(st)?;
    let cal = &st.calendar;
    let (events, now) = (cal.cache.lock().unwrap().events.clone(), cal.now());
    zoned!(cal, tz => agenda::meeting_card(&events, &s, &tz, now))
}

/// The loop: reads now, then every minute or when the Calendar says it changed.
pub fn spawn(st: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            if settings(&st).on {
                st.calendar.subscribe();
                refresh(&st).await;
            }
            let changed = st.calendar.changed.clone();
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(REFRESH_S)) => {}
                _ = changed.notified() => tokio::time::sleep(Duration::from_millis(CHANGE_SETTLE_MS)).await,
            }
        }
    });
}

/// The settings changed (a PUT of the config): read again, in the background.
pub fn settings_changed(st: &Arc<AppState>) {
    let st = st.clone();
    if let Ok(h) = tokio::runtime::Handle::try_current() {
        h.spawn(async move { refresh(&st).await });
    }
}

// --- connect, disconnect -----------------------------------------------------------------------

/// Switches Calendar on, asks macOS for permission if it was never asked (waits for the answer),
/// reads, and adds the voice phrases once. Never asks again after a "no": that is System Settings'.
pub async fn connect(st: &Arc<AppState>) -> Result<(), String> {
    save_settings(st, &Settings { on: true, ..settings(st) })?;
    let source = st.calendar.source();
    let access = {
        let source = source.clone();
        tokio::task::spawn_blocking(move || source.access()).await.unwrap_or(Access::Denied)
    };
    if access == Access::NotAsked {
        let asked = tokio::task::spawn_blocking(move || source.request_access())
            .await
            .unwrap_or_else(|_| Err("macOS didn't answer".into()));
        st.calendar.cache.lock().unwrap().note = asked.err().unwrap_or_default();
    }
    st.calendar.subscribe();
    refresh(st).await;
    if st.calendar.source().access() == Access::Full {
        ensure_voice_commands(st);
    }
    Ok(())
}

/// Switches Calendar off: forgets the events and the reminders and takes the card off the stick.
pub async fn disconnect(st: &Arc<AppState>) -> Result<(), String> {
    save_settings(st, &Settings { on: false, ..settings(st) })?;
    st.calendar.clear();
    crate::home::push_home(st).await;
    Ok(())
}

// --- voice -----------------------------------------------------------------------------------

/// The default phrases (`commands.json`): "calendar": next | today | join.
pub fn voice_commands() -> Vec<Value> {
    vec![
        json!({"name": "Calendar: next meeting", "calendar": "next",
               "phrases": ["what's my next meeting", "what is my next meeting", "when is my next meeting", "next meeting"]}),
        json!({"name": "Calendar: my day", "calendar": "today",
               "phrases": ["what's my day like", "what does my day look like", "agenda", "my agenda", "what's on my calendar"]}),
        json!({"name": "Calendar: join the meeting", "calendar": "join",
               "phrases": ["join my meeting", "join the meeting", "join meeting"]}),
    ]
}

/// Adds the Calendar phrases to the person's commands.json once, the first time Calendar is
/// connected, so they show on the Quick commands page and can be edited or removed (the Python app's
/// defaults are not touched). Returns whether it ran.
pub fn ensure_voice_commands(st: &AppState) -> bool {
    if st.cfg.lock().unwrap().get("calendar_voice_added").and_then(Value::as_bool).unwrap_or(false) {
        return false;
    }
    let path = &st.commands.path;
    let Some(Value::Object(mut doc)) = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok())
    else {
        return false;
    };
    if let Some(list) = doc.entry("commands").or_insert_with(|| json!([])).as_array_mut()
        && !list.iter().any(|c| c.get("calendar").is_some())
    {
        list.extend(voice_commands());
        match serde_json::to_string_pretty(&doc) {
            Ok(text) => {
                if let Err(e) = std::fs::write(path, text) {
                    log::warn!("commands.json: {e}");
                }
            }
            Err(e) => log::warn!("commands.json: {e}"),
        }
    }
    let changes: Map<String, Value> = [("calendar_voice_added".to_string(), json!(true))].into_iter().collect();
    if let Err(e) = st.update_config(changes) {
        log::warn!("calendar: {e}");
    }
    true
}

/// What the person hears for a quick command's `"calendar": "next" | "today" | "join"`: `(ok, line)`.
pub async fn voice(st: &Arc<AppState>, what: &str) -> (bool, String) {
    let result = match what {
        "next" => answer_next(st),
        "today" => answer_today(st, false),
        "join" => open_join(st).await,
        _ => Err("I don't know that calendar question.".to_string()),
    };
    match result {
        Ok(line) => (true, line),
        Err(e) => (false, e),
    }
}

fn snapshot(st: &AppState) -> Result<(Settings, Vec<Event>, i64), String> {
    let s = connected(st).ok_or(NOT_CONNECTED)?;
    let (events, now) = (st.calendar.cache.lock().unwrap().events.clone(), st.calendar.now());
    Ok((s, events, now))
}

/// "Standup in 12 min, at 10:30."
pub fn answer_next(st: &AppState) -> Result<String, String> {
    let (s, events, now) = snapshot(st)?;
    Ok(zoned!(st.calendar, tz => agenda::say_next(&events, &s, &tz, now)))
}

/// A day's meetings: the short spoken line, or (for a tool) one line per event, at most 12.
pub fn answer_today(st: &AppState, tomorrow: bool) -> Result<String, String> {
    let (s, events, now) = snapshot(st)?;
    zoned!(st.calendar, tz => {
        if !tomorrow {
            return Ok(agenda::say_today(&events, &s, &tz, now));
        }
        let date = agenda::date_of(&tz, now).map(|d| d + Days::days(1)).ok_or("no date")?;
        Ok(lines(&agenda::day(&events, &s, &tz, date), &tz, "tomorrow"))
    })
}

/// A tool's listing of a day.
pub fn day_lines(st: &AppState, tomorrow: bool) -> Result<String, String> {
    let (s, events, now) = snapshot(st)?;
    zoned!(st.calendar, tz => {
        let mut date = agenda::date_of(&tz, now).ok_or("no date")?;
        if tomorrow {
            date += Days::days(1);
        }
        let day = agenda::day(&events, &s, &tz, date);
        Ok(lines(&day, &tz, if tomorrow { "tomorrow" } else { "today" }))
    })
}

fn lines<Tz: chrono::TimeZone>(day: &[&Event], tz: &Tz, word: &str) -> String {
    if day.is_empty() {
        return format!("Nothing on your calendar {word}.");
    }
    let mut out: Vec<String> = day.iter().take(MAX_LINES).map(|e| agenda::day_line(e, tz)).collect();
    if day.len() > MAX_LINES {
        out.push(format!("+ {} more", day.len() - MAX_LINES));
    }
    format!("{} {word}:\n{}", plural(day.len(), "event", "events"), out.join("\n"))
}

/// Opens the meeting link (only a known conference service's, https) of the meeting on now, else the
/// next with one today, through the same `open` runner as quick commands.
pub async fn open_join(st: &Arc<AppState>) -> Result<String, String> {
    let (s, events, now) = snapshot(st)?;
    let found =
        zoned!(st.calendar, tz => agenda::join_choice(&events, &s, &tz, now).map(|(e, p, u)| (e.title.clone(), p, u)));
    let Some((title, provider, url)) = found else { return Err("No meeting link found.".into()) };
    let runner = st.runner();
    let _ = tokio::task::spawn_blocking(move || runner(&Action::Open(vec![url]))).await;
    let title: String = crate::talk::stick_text(&title).chars().take(40).collect();
    Ok(format!("Opened {provider} for {}.", if title.trim().is_empty() { "your meeting" } else { title.trim() }))
}

// --- the API -----------------------------------------------------------------------------------

pub fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/connections", get(list_api))
        .route("/api/connections/calendar", put(settings_api))
        .route("/api/connections/calendar/connect", post(connect_api))
        .route("/api/connections/calendar/disconnect", post(disconnect_api))
        .route("/api/connections/calendar/calendars", get(calendars_api))
        .route("/api/connections/calendar/events", get(events_api))
}

fn answer(status: StatusCode, v: Value) -> Response {
    (status, axum::Json(v)).into_response()
}

fn fail(status: StatusCode, words: &str) -> Response {
    answer(status, json!({"error": words}))
}

async fn list_api(State(st): State<Arc<AppState>>) -> axum::Json<Value> {
    axum::Json(super::list(&st))
}

async fn settings_api(State(st): State<Arc<AppState>>, body: Bytes) -> Response {
    let Ok(Value::Object(body)) = serde_json::from_slice::<Value>(&body) else {
        return fail(StatusCode::BAD_REQUEST, "send the settings as a JSON object");
    };
    let bad = agenda::invalid_settings(&body);
    if !bad.is_empty() {
        return fail(StatusCode::BAD_REQUEST, &bad);
    }
    if let Err(e) = save_settings(&st, &settings(&st).with(&body)) {
        return fail(StatusCode::INTERNAL_SERVER_ERROR, &e);
    }
    refresh(&st).await;
    answer(StatusCode::OK, connection(&st))
}

async fn connect_api(State(st): State<Arc<AppState>>) -> Response {
    match connect(&st).await {
        Ok(()) => answer(StatusCode::OK, connection(&st)),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

async fn disconnect_api(State(st): State<Arc<AppState>>) -> Response {
    match disconnect(&st).await {
        Ok(()) => answer(StatusCode::OK, connection(&st)),
        Err(e) => fail(StatusCode::INTERNAL_SERVER_ERROR, &e),
    }
}

/// Why the data routes cannot answer: not connected (409), or the last read failed (502).
fn unavailable(st: &AppState) -> Option<Response> {
    if connected(st).is_none() {
        return Some(fail(StatusCode::CONFLICT, NOT_CONNECTED));
    }
    let error = st.calendar.cache.lock().unwrap().error.clone();
    (!error.is_empty()).then(|| fail(StatusCode::BAD_GATEWAY, &error))
}

async fn calendars_api(State(st): State<Arc<AppState>>) -> Response {
    if let Some(r) = unavailable(&st) {
        return r;
    }
    let s = settings(&st);
    let source = st.calendar.source();
    match tokio::task::spawn_blocking(move || source.calendars()).await {
        Ok(Ok(list)) => {
            let calendars: Vec<Value> = list
                .iter()
                .map(|c| {
                    json!({"id": c.id, "title": c.title, "color": c.color, "source": c.source,
                                "selected": s.calendars.is_empty() || s.calendars.contains(&c.id)})
                })
                .collect();
            answer(StatusCode::OK, json!({"calendars": calendars}))
        }
        Ok(Err(e)) => fail(StatusCode::BAD_GATEWAY, &e),
        Err(_) => fail(StatusCode::INTERNAL_SERVER_ERROR, "reading the calendar stopped unexpectedly"),
    }
}

#[derive(serde::Deserialize)]
struct DayQuery {
    day: Option<String>,
}

/// `today`, `tomorrow` or `YYYY-MM-DD`.
fn parse_day(word: &str, today: NaiveDate) -> Option<NaiveDate> {
    match word {
        "" | "today" => Some(today),
        "tomorrow" => Some(today + Days::days(1)),
        w => NaiveDate::parse_from_str(w, "%Y-%m-%d").ok(),
    }
}

async fn events_api(State(st): State<Arc<AppState>>, Query(q): Query<DayQuery>) -> Response {
    if let Some(r) = unavailable(&st) {
        return r;
    }
    let Some(s) = connected(&st) else { return fail(StatusCode::CONFLICT, NOT_CONNECTED) };
    let cal = &st.calendar;
    let (events, calendars, window, now) = {
        let c = cal.cache.lock().unwrap();
        (c.events.clone(), c.calendars.clone(), c.window, cal.now())
    };
    zoned!(cal, tz => {
        let word = q.day.as_deref().unwrap_or("today");
        let wrong = "day must be today, tomorrow or YYYY-MM-DD (only today and tomorrow are loaded)";
        let Some(date) = agenda::date_of(&tz, now).and_then(|t| parse_day(word, t)) else { return fail(StatusCode::BAD_REQUEST, wrong) };
        let covered = agenda::day_bounds(&tz, date).is_some_and(|(from, to)| window.0 <= from && to <= window.1);
        if !covered {
            return fail(StatusCode::BAD_REQUEST, wrong);
        }
        let rows: Vec<Value> = agenda::day(&events, &s, &tz, date)
            .into_iter()
            .map(|e| {
                let cal = calendars.iter().find(|c| c.id == e.calendar_id);
                let running = e.start <= now && now < e.end && !e.all_day && !e.declined;
                json!({"id": e.id, "title": e.title, "start": e.start, "end": e.end,
                       "from": if e.all_day { String::new() } else { agenda::hhmm(&tz, e.start) },
                       "to": if e.all_day { String::new() } else { agenda::hhmm(&tz, e.end) },
                       "all_day": e.all_day, "declined": e.declined, "now": running,
                       "calendar": cal.map(|c| c.title.as_str()).unwrap_or(""), "color": cal.map(|c| c.color.as_str()).unwrap_or(""),
                       "where": e.location.lines().next().unwrap_or("").chars().take(80).collect::<String>(),
                       "join": agenda::join_link(e).map(|(p, _)| p).unwrap_or("")})
            })
            .collect();
        answer(StatusCode::OK, json!({"day": date.format("%Y-%m-%d").to_string(), "events": rows}))
    })
}

#[cfg(test)]
#[path = "calendar_tests.rs"]
mod tests;
