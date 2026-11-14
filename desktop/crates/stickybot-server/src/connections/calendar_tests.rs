//! The Calendar connection end to end on the fake stick and `FakeCalendar`: the API, connecting,
//! the cache, the reminder, the card, voice, the stick tools and what stays out of the logs. Nothing
//! here can reach the real Calendar: `AppState` with the fake stick never builds an EventKit source.

use super::*;
use crate::Options;
use crate::commands::Action;
use crate::talk::tests::{FakeBrain, T, agent, frames};
use axum::body::Body;
use axum::extract::Request;
use chrono::{NaiveDateTime, TimeZone};
use http_body_util::BodyExt;
use macos_calendar::fake::FakeCalendar;
use stick_link::{Backend, fake::FakeStick};
use tower::ServiceExt;

/// India's offset: half an hour, so a zone mix-up cannot hide.
const OFFSET: i32 = 5 * 3600 + 1800;

fn at(s: &str) -> i64 {
    let tz = FixedOffset::east_opt(OFFSET).unwrap();
    let n = NaiveDateTime::parse_from_str(&format!("{s}:00"), "%Y-%m-%d %H:%M:%S").unwrap();
    tz.from_local_datetime(&n).unwrap().timestamp()
}

/// 10:18 on the 12th: the stand-up is 12 minutes away.
fn morning() -> i64 {
    at("2026-11-12 10:18")
}

fn meeting(id: &str, title: &str, start: &str, end: &str) -> Event {
    Event {
        id: id.into(),
        calendar_id: "work".into(),
        title: title.into(),
        start: at(start),
        end: at(end),
        ..Event::default()
    }
}

fn calendars() -> Vec<CalendarInfo> {
    vec![
        CalendarInfo {
            id: "work".into(),
            title: "Work".into(),
            color: "#4a7bd0".into(),
            source: "Example account".into(),
        },
        CalendarInfo { id: "home".into(), title: "Home".into(), color: "#3aa76d".into(), source: "iCloud".into() },
    ]
}

/// A day: stand-up with a Meet link, a review with a Zoom link in its notes, a 1:1 with no link, a
/// declined meeting and an all-day event.
fn day() -> Vec<Event> {
    let mut standup = meeting("standup", "Standup", "2026-11-12 10:30", "2026-11-12 11:00");
    standup.url = "https://meet.google.com/abc-defg-hij".into();
    let mut review = meeting("review", "Design review", "2026-11-12 14:00", "2026-11-12 15:00");
    review.notes = "Join: https://example.zoom.us/j/123456789".into();
    let one = Event {
        location: "Room 2".into(),
        ..meeting("one", "1:1 with a teammate", "2026-11-12 16:00", "2026-11-12 16:30")
    };
    let declined = Event { declined: true, ..meeting("declined", "Planning", "2026-11-12 12:00", "2026-11-12 12:45") };
    let offsite = Event { all_day: true, ..meeting("offsite", "Team offsite", "2026-11-12 00:00", "2026-11-12 23:59") };
    let tomorrow =
        Event { calendar_id: "home".into(), ..meeting("tomorrow", "Dentist", "2026-11-13 09:00", "2026-11-13 10:00") };
    vec![review, one, standup, declined, offsite, tomorrow]
}

struct W {
    t: T,
    cal: FakeCalendar,
    app: Router,
}

impl W {
    async fn new(name: &str) -> Self {
        let t = agent(name, false).await;
        let cal = FakeCalendar::new();
        cal.set_calendars(calendars());
        cal.set_events(day());
        t.st.calendar.set_source(Arc::new(cal.clone()));
        t.st.calendar.set_clock(morning(), OFFSET);
        let app = crate::api::router(t.st.clone());
        W { t, cal, app }
    }

    fn st(&self) -> &Arc<AppState> {
        &self.t.st
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut b = Request::builder().method(method).uri(path).header("host", "localhost:8670");
        if method != "GET" {
            b = b.header("x-stick-token", self.st().token.as_str());
        }
        if body.is_some() {
            b = b.header("content-type", "application/json");
        }
        let req = b.body(body.map(|v| Body::from(v.to_string())).unwrap_or_default()).unwrap();
        let resp = self.app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    async fn connect(&self) -> Value {
        let (s, v) = self.call("POST", "/api/connections/calendar/connect", None).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v
    }

    /// What was sent to the stick, as JSON lines.
    fn sent(&self) -> Vec<Value> {
        String::from_utf8(self.t.fake.writes().concat())
            .unwrap()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    fn reminders(&self) -> Vec<Value> {
        self.sent()
            .into_iter()
            .filter_map(|m| m.get("event").cloned())
            .filter(|e| e["kind"] == "meeting_soon")
            .collect()
    }
}

// --- the list and the guards ----------------------------------------------------------------------

#[tokio::test]
async fn a_new_install_has_the_calendar_off_with_the_default_settings() {
    let w = W::new("cal-list").await;
    let (s, v) = w.call("GET", "/api/connections", None).await;
    assert_eq!(s, StatusCode::OK);
    let list = v["connections"].as_array().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(
        list[0],
        json!({"id": "calendar", "title": "Calendar", "status": "off", "detail": "Not connected", "permission": "",
               "settings": {"on": false, "calendars": [], "skip_all_day": true, "skip_declined": true, "alert_min": 5}})
    );
    // nothing was read or asked while it is off
    assert_eq!(w.cal.requests(), 0);
    assert!(w.st().calendar.private_strings().is_empty());
}

#[tokio::test]
async fn the_routes_sit_behind_the_same_host_and_token_guards() {
    let w = W::new("cal-guard").await;
    for (method, path) in [
        ("PUT", "/api/connections/calendar"),
        ("POST", "/api/connections/calendar/connect"),
        ("POST", "/api/connections/calendar/disconnect"),
    ] {
        let req = Request::builder()
            .method(method)
            .uri(path)
            .header("host", "localhost:8670")
            .body(Body::from("{}"))
            .unwrap();
        let bad_token = w.app.clone().oneshot(req).await.unwrap().status();
        assert_eq!(bad_token, StatusCode::FORBIDDEN, "{method} {path}");
    }
    let req =
        Request::builder().uri("/api/connections").header("host", "evil.example:8670").body(Body::empty()).unwrap();
    assert_eq!(w.app.clone().oneshot(req).await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(w.cal.requests(), 0);
}

#[tokio::test]
async fn the_connections_page_is_served_to_the_rust_app_and_added_to_the_nav() {
    let w = W::new("cal-assets").await;
    let get = |path: &str| {
        let req =
            Request::builder().uri(path.to_string()).header("host", "localhost:8670").body(Body::empty()).unwrap();
        let app = w.app.clone();
        async move {
            let resp = app.oneshot(req).await.unwrap();
            let (status, ctype) =
                (resp.status(), resp.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()));
            let text = String::from_utf8(resp.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
            (status, ctype.unwrap_or_default(), text)
        }
    };
    for (path, must_have) in [
        ("/static-rs/connections.js", "export function render"),
        ("/static-rs/connections_logic.js", "export function permissionHelp"),
    ] {
        let (status, ctype, text) = get(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(ctype.starts_with("text/javascript"), "{path}: {ctype}");
        assert!(text.contains(must_have), "{path}");
    }
    // the shared page loads rust.js, which puts Connections in the nav before Feedback
    let (_, _, page) = get("/").await;
    assert!(page.contains("/static-rs/rust.js"));
    let (_, _, rust) = get("/static-rs/rust.js").await;
    assert!(
        rust.contains(r#"stickyAddPage("connections", "Connections", connections"#) && rust.contains(r#""feedback");"#),
        "{rust}"
    );
    // every control the card promises is in the page's code
    let (_, _, page_js) = get("/static-rs/connections.js").await;
    for control in [
        "Connect",
        "Disconnect",
        "calCalendars",
        "Skip all-day events",
        "Skip meetings I declined",
        "Remind me",
        "calToday",
        "Send a test alert",
        "More connections are coming.",
        "/api/connections/calendar/calendars",
        "/api/events/test",
    ] {
        assert!(page_js.contains(control), "{control}");
    }
    // the Python app does not serve it: the legacy dashboard files are untouched by this change
    assert!(!include_str!("../../../../../mac-agent/stick_agent/dashboard.html").contains("connections"));
}

// --- connect, permission, settings, disconnect ------------------------------------------------------

#[tokio::test]
async fn connecting_asks_for_permission_once_reads_and_adds_the_voice_phrases_once() {
    let w = W::new("cal-connect").await;
    w.cal.set_access(Access::NotAsked);
    let v = w.connect().await;
    assert_eq!((v["status"].as_str(), v["permission"].as_str()), (Some("connected"), Some("")));
    assert_eq!(v["detail"], "2 calendars · next at 10:30");
    assert_eq!(v["settings"]["on"], true);
    assert_eq!(w.cal.requests(), 1);
    assert_eq!(w.st().cfg.lock().unwrap()["connections"]["calendar"]["on"], true);
    // again: the permission is settled, so macOS is not asked again; nothing doubles
    w.connect().await;
    assert_eq!(w.cal.requests(), 1);
    let phrases = |w: &W| {
        let doc: Value = serde_json::from_str(&std::fs::read_to_string(&w.st().commands.path).unwrap()).unwrap();
        doc["commands"].as_array().unwrap().iter().filter(|c| c.get("calendar").is_some()).count()
    };
    assert_eq!(phrases(&w), 3);
    assert_eq!(w.st().cfg.lock().unwrap()["calendar_voice_added"], true);
    // taken off by the person, they stay off after the next connect
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&w.st().commands.path).unwrap()).unwrap();
    doc["commands"].as_array_mut().unwrap().retain(|c| c.get("calendar").is_none());
    std::fs::write(&w.st().commands.path, doc.to_string()).unwrap();
    w.call("POST", "/api/connections/calendar/disconnect", None).await;
    w.connect().await;
    assert_eq!(phrases(&w), 0);
}

#[tokio::test]
async fn permission_that_was_not_given_reads_as_needs_permission_and_is_not_asked_for_again() {
    for (access, word, text) in [
        (Access::Denied, "denied", "StickyBot isn't allowed to read your calendar"),
        (Access::Restricted, "restricted", "this Mac doesn't allow StickyBot to read the calendar"),
        (Access::WriteOnly, "write_only", "StickyBot can only add events: it needs Full Access to read them"),
    ] {
        let w = W::new(&format!("cal-denied-{word}")).await;
        w.cal.set_access(access);
        let v = w.connect().await;
        assert_eq!((v["status"].as_str(), v["permission"].as_str()), (Some("needs_permission"), Some(word)));
        assert_eq!(v["detail"], text);
        assert_eq!(w.cal.requests(), 0, "{word}: macOS does not ask again; System Settings does");
        // no phrases are added for a Calendar that cannot be read, and the data routes say why
        assert_eq!(w.st().cfg.lock().unwrap()["calendar_voice_added"], false);
        for path in ["/api/connections/calendar/calendars", "/api/connections/calendar/events"] {
            let (s, e) = w.call("GET", path, None).await;
            assert_eq!((s, e["error"].as_str()), (StatusCode::CONFLICT, Some(NOT_CONNECTED)), "{path}");
        }
        assert!(connection(w.st())["settings"]["on"] == true, "still switched on: it only waits for the permission");
    }
    // asked now and the answer is no
    let w = W::new("cal-asked-no").await;
    w.cal.set_access(Access::NotAsked);
    w.cal.set_answer(Access::Denied);
    let v = w.connect().await;
    assert_eq!((v["status"].as_str(), v["permission"].as_str()), (Some("needs_permission"), Some("denied")));
    assert_eq!(w.cal.requests(), 1);
    // the person allows it in System Settings and presses Connect again: no second question, it reads
    w.cal.set_access(Access::Full);
    let v = w.connect().await;
    assert_eq!(v["status"], "connected");
    assert_eq!(w.cal.requests(), 1);
}

#[tokio::test]
async fn settings_are_saved_in_any_subset_and_bad_ones_are_refused_in_words() {
    let w = W::new("cal-settings").await;
    w.connect().await;
    let reads = w.cal.reads();
    let (s, v) = w.call("PUT", "/api/connections/calendar", Some(json!({"alert_min": 10}))).await;
    tokio::time::sleep(Duration::from_millis(200)).await; // a second, background read would land by now
    assert_eq!(w.cal.reads(), reads + 1, "one save, one read of the calendar");
    assert_eq!(
        (s, v["settings"]["alert_min"].clone(), v["settings"]["on"].clone()),
        (StatusCode::OK, json!(10), json!(true))
    );
    let (_, v) =
        w.call("PUT", "/api/connections/calendar", Some(json!({"calendars": ["work"], "skip_all_day": false}))).await;
    assert_eq!(
        v["settings"],
        json!({"on": true, "calendars": ["work"], "skip_all_day": false, "skip_declined": true, "alert_min": 10})
    );
    // saved: the config carries it
    assert_eq!(w.st().cfg.lock().unwrap()["connections"]["calendar"]["calendars"], json!(["work"]));
    let before = w.st().cfg.lock().unwrap().clone();
    for (body, words) in [
        (json!({"alert_min": 500}), "alert_min must be 0-120 minutes"),
        (json!({"on": "yes"}), "on must be true or false"),
        (json!({"calendars": "work"}), "calendars must be a list of calendar ids"),
        (json!({"colour": "red"}), "unknown setting 'colour'"),
    ] {
        let (s, v) = w.call("PUT", "/api/connections/calendar", Some(body.clone())).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
        assert!(v["error"].as_str().unwrap().starts_with(words), "{body}: {v}");
        assert_eq!(*w.st().cfg.lock().unwrap(), before, "nothing is saved: {body}");
    }
    let (s, v) = w.call("PUT", "/api/connections/calendar", None).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("send the settings as a JSON object")));
    // the calendars choice changes what is read: only the work calendar's next meeting
    let (_, v) = w.call("PUT", "/api/connections/calendar", Some(json!({"calendars": ["home"]}))).await;
    assert_eq!(v["detail"], "1 calendar · next Fri 09:00");
}

#[tokio::test]
async fn the_same_rules_hold_through_put_config_and_a_partial_change_keeps_the_rest() {
    let w = W::new("cal-putconfig").await;
    w.connect().await;
    let (s, v) = w.call("PUT", "/api/config", Some(json!({"connections": {"calendar": {"alert_min": 500}}}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("alert_min must be 0-120 minutes")));
    let (s, v) = w.call("PUT", "/api/config", Some(json!({"connections": {"github": {}}}))).await;
    assert_eq!((s, v["error"].as_str()), (StatusCode::BAD_REQUEST, Some("connections: no connection 'github'")));
    let (s, _) = w.call("PUT", "/api/config", Some(json!({"connections": "x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let (s, _) = w.call("PUT", "/api/config", Some(json!({"calendar_voice_added": "x"}))).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    // one setting of one connection: the others (and "on") are not reset
    let (s, v) = w.call("PUT", "/api/config", Some(json!({"connections": {"calendar": {"alert_min": 15}}}))).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v["connections"]["calendar"],
        json!({"on": true, "calendars": [], "skip_all_day": true, "skip_declined": true, "alert_min": 15})
    );
}

#[tokio::test]
async fn a_saved_config_with_partial_or_malformed_calendar_settings_reads_as_defaults() {
    for (name, saved) in [
        ("partial", json!({"connections": {"calendar": {"on": true}}})),
        ("junk", json!({"connections": {"calendar": {"on": "yes", "alert_min": "soon", "calendars": 5}}})),
        ("not-an-object", json!({"connections": 5})),
        ("calendar-not-an-object", json!({"connections": {"calendar": []}})),
        ("missing", json!({})),
    ] {
        let home = std::env::temp_dir().join(format!("stickybot-rs-cal-saved-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("config.json"), saved.to_string()).unwrap();
        let opts = Options { port: 8670, home, fake_stick: true, import_from: None };
        let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap();
        let s = settings(&st);
        let want = if name == "partial" { Settings { on: true, ..Settings::default() } } else { Settings::default() };
        assert_eq!(s, want, "{name}");
        let c = connection(&st); // no panic, and the common shape
        assert!(["off", "connected", "needs_permission"].contains(&c["status"].as_str().unwrap()), "{name}: {c}");
    }
}

#[tokio::test]
async fn disconnecting_forgets_the_events_the_reminders_and_takes_the_card_off() {
    let w = W::new("cal-disconnect").await;
    w.st().calendar.set_clock(at("2026-11-12 10:27"), OFFSET);
    w.connect().await;
    assert_eq!(w.reminders().len(), 1, "the stand-up is 3 minutes away: reminded once");
    assert!(!w.st().calendar.private_strings().is_empty());
    assert!(meeting_card(w.st()).is_some_and(|m| m.pinned));
    let (s, v) = w.call("POST", "/api/connections/calendar/disconnect", None).await;
    assert_eq!((s, v["status"].as_str(), v["settings"]["on"].clone()), (StatusCode::OK, Some("off"), json!(false)));
    assert!(w.st().calendar.private_strings().is_empty(), "the cache is empty");
    assert!(w.st().calendar.cache.lock().unwrap().fired.is_empty());
    assert!(meeting_card(w.st()).is_none());
    let (s, _) = w.call("GET", "/api/connections/calendar/events", None).await;
    assert_eq!(s, StatusCode::CONFLICT);
    // the stick was told: the last home card it got is not the meeting
    let last_home = w.sent().into_iter().rev().find_map(|m| m.get("home").cloned()).unwrap();
    assert_ne!(last_home["k"], "meeting", "{last_home}");
    // the voice phrases stay (they answer "not connected"), and nothing fires while it is off
    w.st().calendar.set_clock(at("2026-11-12 10:28"), OFFSET);
    refresh(w.st()).await;
    tick(w.st()).await;
    assert_eq!(w.reminders().len(), 1);
    // connecting again starts clean: the same occurrence is reminded again
    w.connect().await;
    assert_eq!(w.reminders().len(), 2);
}

// --- calendars and the day's events -----------------------------------------------------------------

#[tokio::test]
async fn the_calendars_route_lists_them_with_what_is_selected() {
    let w = W::new("cal-calendars").await;
    w.connect().await;
    let (s, v) = w.call("GET", "/api/connections/calendar/calendars", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        v["calendars"],
        json!([{"id": "work", "title": "Work", "color": "#4a7bd0", "source": "Example account", "selected": true},
               {"id": "home", "title": "Home", "color": "#3aa76d", "source": "iCloud", "selected": true}])
    );
    w.call("PUT", "/api/connections/calendar", Some(json!({"calendars": ["home"]}))).await;
    let (_, v) = w.call("GET", "/api/connections/calendar/calendars", None).await;
    assert_eq!(v["calendars"][0]["selected"], false);
    assert_eq!(v["calendars"][1]["selected"], true);
}

#[tokio::test]
async fn the_events_route_lists_a_local_day_sorted_with_the_documented_fields() {
    let w = W::new("cal-events").await;
    w.connect().await;
    let (s, v) = w.call("GET", "/api/connections/calendar/events?day=today", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["day"], "2026-11-12");
    let titles: Vec<&str> = v["events"].as_array().unwrap().iter().map(|e| e["title"].as_str().unwrap()).collect();
    // all-day and declined are off by default; the rest by start time
    assert_eq!(titles, ["Standup", "Design review", "1:1 with a teammate"]);
    assert_eq!(
        v["events"][0],
        json!({"id": "standup", "title": "Standup", "start": at("2026-11-12 10:30"), "end": at("2026-11-12 11:00"),
               "from": "10:30", "to": "11:00", "all_day": false, "declined": false, "now": false,
               "calendar": "Work", "color": "#4a7bd0", "where": "", "join": "Google Meet"})
    );
    assert_eq!((v["events"][1]["join"].as_str(), v["events"][2]["where"].as_str()), (Some("Zoom"), Some("Room 2")));
    // the default day is today; tomorrow is loaded too
    let (_, d) = w.call("GET", "/api/connections/calendar/events", None).await;
    assert_eq!(d, v);
    let (_, t) = w.call("GET", "/api/connections/calendar/events?day=tomorrow", None).await;
    assert_eq!(
        (t["day"].as_str(), t["events"][0]["title"].as_str(), t["events"][0]["calendar"].as_str()),
        (Some("2026-11-13"), Some("Dentist"), Some("Home"))
    );
    let (_, t) = w.call("GET", "/api/connections/calendar/events?day=2026-11-13", None).await;
    assert_eq!(t["events"].as_array().unwrap().len(), 1);
    // outside what is loaded, or not a day: refused in words
    for day in ["2026-11-14", "2026-11-11", "yesterday", "13-11-2026"] {
        let (s, e) = w.call("GET", &format!("/api/connections/calendar/events?day={day}"), None).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{day}");
        assert!(e["error"].as_str().unwrap().starts_with("day must be today, tomorrow or YYYY-MM-DD"), "{e}");
    }
    // the switches show the all-day and declined ones, marked
    w.call("PUT", "/api/connections/calendar", Some(json!({"skip_all_day": false, "skip_declined": false}))).await;
    let (_, v) = w.call("GET", "/api/connections/calendar/events", None).await;
    let flags: Vec<(String, bool, bool)> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            (
                e["title"].as_str().unwrap().to_string(),
                e["all_day"].as_bool().unwrap(),
                e["declined"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(flags[0], ("Team offsite".to_string(), true, false));
    assert!(flags.contains(&("Planning".to_string(), false, true)));
    assert_eq!(v["events"][0]["from"], "", "no clock times for an all-day event");
    // "now" marks the meeting that is on
    w.st().calendar.set_clock(at("2026-11-12 10:40"), OFFSET);
    let (_, v) = w.call("GET", "/api/connections/calendar/events", None).await;
    let running: Vec<&str> = v["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["now"] == true)
        .map(|e| e["title"].as_str().unwrap())
        .collect();
    assert_eq!(running, ["Standup"]);
}

// --- failing and changing sources ---------------------------------------------------------------------

#[tokio::test]
async fn a_source_that_fails_reads_as_an_error_in_words_not_a_crash() {
    let w = W::new("cal-error").await;
    w.connect().await;
    w.cal.fail_with(Some("the calendar database is locked"));
    refresh(w.st()).await;
    let v = connection(w.st());
    assert_eq!((v["status"].as_str(), v["detail"].as_str()), (Some("error"), Some("the calendar database is locked")));
    for path in ["/api/connections/calendar/events", "/api/connections/calendar/calendars"] {
        let (s, e) = w.call("GET", path, None).await;
        assert_eq!(
            (s, e["error"].as_str()),
            (StatusCode::BAD_GATEWAY, Some("the calendar database is locked")),
            "{path}"
        );
    }
    // the list still answers; the voice says it cannot
    let (s, list) = w.call("GET", "/api/connections", None).await;
    assert_eq!((s, list["connections"][0]["status"].as_str()), (StatusCode::OK, Some("error")));
    w.cal.fail_with(None);
    refresh(w.st()).await;
    assert_eq!(connection(w.st())["status"], "connected");
}

#[tokio::test]
async fn a_change_in_the_calendar_shows_up_in_the_next_read() {
    let w = W::new("cal-change").await;
    w.connect().await;
    let (_, before) = w.call("GET", "/api/connections/calendar/events", None).await;
    assert_eq!(before["events"].as_array().unwrap().len(), 3);
    w.cal.add_event(meeting("new", "Added later", "2026-11-12 17:00", "2026-11-12 17:30"));
    // the source tells the app, the loop reads again (the loop is `spawn`)
    spawn(w.st().clone());
    for _ in 0..40 {
        w.cal.fire_change();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let (_, after) = w.call("GET", "/api/connections/calendar/events", None).await;
        if after["events"].as_array().unwrap().len() == 4 {
            return;
        }
    }
    panic!("the new event never showed up");
}

#[tokio::test]
async fn refreshing_reads_only_what_the_window_holds_and_keeps_nothing_on_disk() {
    let w = W::new("cal-window").await;
    w.cal.add_event(meeting("far", "Far away", "2026-11-20 10:00", "2026-11-20 11:00"));
    w.connect().await;
    let titles = w.st().calendar.private_strings().join("|");
    assert!(!titles.contains("Far away"), "only today and tomorrow are read: {titles}");
    // nothing about the meetings is written under the app's home (events.jsonl has the reminders only)
    let home = &w.st().paths.home;
    for entry in std::fs::read_dir(home).unwrap().flatten() {
        let p = entry.path();
        if p.is_file() {
            let text = std::fs::read_to_string(&p).unwrap_or_default();
            assert!(!text.contains("Design review") && !text.contains("meet.google.com"), "{}", p.display());
        }
    }
}

// --- the reminder --------------------------------------------------------------------------------------

#[tokio::test]
async fn the_reminder_goes_to_the_stick_once_and_is_a_listed_kind() {
    let w = W::new("cal-alert").await;
    w.st().calendar.set_clock(at("2026-11-12 10:26"), OFFSET);
    w.connect().await;
    let reminders = w.reminders();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0]["title"], "Standup in 4 min");
    assert_eq!(reminders[0]["sub"], "10:30 · Google Meet");
    assert_eq!((reminders[0]["level"].as_str(), reminders[0]["icon"].as_str()), (Some("headsup"), Some("bell")));
    // the checks go on every minute: still one
    for minute in ["10:27", "10:28", "10:29"] {
        w.st().calendar.set_clock(at(&format!("2026-11-12 {minute}")), OFFSET);
        refresh(w.st()).await;
    }
    tick(w.st()).await;
    assert_eq!(w.reminders().len(), 1);
    // it is in the Events page's list with its level
    let (_, kinds) = w.call("GET", "/api/events/kinds", None).await;
    assert_eq!(
        kinds["kinds"]["meeting_soon"],
        json!({"default": "headsup", "icon": "bell", "title": "Standup in 5 min"})
    );
    assert_eq!(kinds["rules"]["meeting_soon"], "headsup");
    // "Send a test alert" on the Connections page is the Events page's sample
    let (_, v) = w.call("POST", "/api/events/test", Some(json!({"kind": "meeting_soon"}))).await;
    assert_eq!(
        (v["event"]["kind"].as_str(), v["event"]["title"].as_str()),
        (Some("meeting_soon"), Some("Standup in 5 min"))
    );
}

#[tokio::test]
async fn no_reminder_for_all_day_declined_alert_min_zero_off_or_a_stick_that_is_away() {
    // all-day and declined events in the window
    let w = W::new("cal-alert-no").await;
    let start = "2026-11-12 10:30";
    w.cal.set_events(vec![
        Event { all_day: true, ..meeting("a", "All day", start, "2026-11-12 11:00") },
        Event { declined: true, ..meeting("d", "Declined", start, "2026-11-12 11:00") },
        Event { canceled: true, ..meeting("c", "Canceled", start, "2026-11-12 11:00") },
    ]);
    w.st().calendar.set_clock(at("2026-11-12 10:27"), OFFSET);
    w.connect().await;
    assert!(w.reminders().is_empty());
    // alert_min 0
    let w = W::new("cal-alert-zero").await;
    w.st().calendar.set_clock(at("2026-11-12 10:27"), OFFSET);
    w.call("PUT", "/api/connections/calendar", Some(json!({"alert_min": 0}))).await;
    w.connect().await;
    assert!(w.reminders().is_empty());
    // off: even with events in the window
    let w = W::new("cal-alert-off").await;
    w.st().calendar.set_clock(at("2026-11-12 10:27"), OFFSET);
    refresh(w.st()).await;
    tick(w.st()).await;
    assert!(w.reminders().is_empty());
    // the stick is away: not sent and not used up, so it goes out when the stick is back in time
    let w = W::new("cal-alert-away").await;
    w.connect().await; // 10:18: nothing due yet
    w.t.st.link.set_target(None);
    let mut s = w.t.st.link.watch_state();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.wait_for(|s| *s != stick_link::LinkState::Connected)).await;
    w.st().calendar.set_clock(at("2026-11-12 10:27"), OFFSET);
    refresh(w.st()).await;
    assert!(w.reminders().is_empty());
    assert!(w.st().calendar.cache.lock().unwrap().fired.is_empty());
}

// --- the card ------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_home_card_follows_the_calendar_only_while_it_is_connected() {
    let w = W::new("cal-card").await;
    assert!(meeting_card(w.st()).is_none(), "off");
    w.st().calendar.set_clock(at("2026-11-12 10:00"), OFFSET);
    w.connect().await;
    let m = meeting_card(w.st()).unwrap();
    assert_eq!(m.card, json!({"k": "meeting", "a": "Standup", "b": "in 30 min · 10:30"}));
    assert!(!m.pinned, "30 min is outside the 15 that pin it...");
    // ...at 10:16 it is inside: pinned, and the stick is told right away
    w.st().calendar.set_clock(at("2026-11-12 10:16"), OFFSET);
    tick(w.st()).await;
    assert!(meeting_card(w.st()).unwrap().pinned);
    let card = crate::home::home_card(w.st());
    assert_eq!(card, json!({"k": "meeting", "a": "Standup", "b": "in 14 min · 10:30"}));
    let home_pushes: Vec<Value> = w.sent().into_iter().filter_map(|m| m.get("home").cloned()).collect();
    assert_eq!(home_pushes.last().unwrap()["a"], "Standup");
    // when the permission is withdrawn the card goes
    w.cal.set_access(Access::Denied);
    assert!(meeting_card(w.st()).is_none());
    assert_ne!(crate::home::home_card(w.st())["k"], "meeting");
}

// --- voice, without the model -----------------------------------------------------------------------------

fn said(t: &T, text: &str) {
    *t.said.lock().unwrap() = Ok(text.into());
}

async fn ask(t: &T, text: &str) -> (String, String) {
    said(t, text);
    let before = t.fake.screens().len();
    t.fake.talk(&frames(1.5));
    for _ in 0..500 {
        let screens = t.fake.screens();
        if let Some((state, _, line)) = screens.last().filter(|_| screens.len() > before)
            && matches!(state.as_str(), "done" | "error" | "cancelled")
        {
            return (state.clone(), line.clone());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the stick was never told how {text:?} ended");
}

/// The talk path with a connected fake calendar, the brain watched.
async fn voice_setup(name: &str) -> (T, FakeCalendar, Arc<std::sync::Mutex<Vec<String>>>) {
    let t = agent(name, true).await;
    let cal = FakeCalendar::new();
    cal.set_calendars(calendars());
    cal.set_events(day());
    t.st.calendar.set_source(Arc::new(cal.clone()));
    t.st.calendar.set_clock(morning(), OFFSET);
    let brain = FakeBrain::new(&["the model answered"]);
    let asked = brain.asked.clone();
    t.st.set_brain(Arc::new(brain));
    connect(&t.st).await.unwrap();
    (t, cal, asked)
}

#[tokio::test]
async fn the_voice_phrases_match_without_the_model() {
    let cmds = voice_commands();
    let kind =
        |text: &str| crate::commands::match_in(&cmds, text).map(|(c, _)| c["calendar"].as_str().unwrap().to_string());
    for (text, want) in [
        ("What's my next meeting?", "next"),
        ("what is my next meeting", "next"),
        ("When is my next meeting?", "next"),
        ("next meeting", "next"),
        ("What's my day like?", "today"),
        ("what does my day look like", "today"),
        ("agenda", "today"),
        ("My agenda", "today"),
        ("What's on my calendar", "today"),
        ("Join my meeting", "join"),
        ("join the meeting.", "join"),
        ("join meeting", "join"),
    ] {
        assert_eq!(kind(text).as_deref(), Some(want), "{text}");
    }
    for text in ["what's the weather", "join the standup channel", "turn off the tv", "open my calendar app"] {
        assert_eq!(kind(text), None, "{text}");
    }
    // they do not step on the TV phrases
    assert!(crate::commands::match_in(&crate::ir::voice_commands(), "next meeting").is_none());
}

#[tokio::test]
async fn asking_for_the_next_meeting_and_the_day_is_answered_from_the_calendar() {
    let (t, _cal, asked) = voice_setup("cal-voice").await;
    assert_eq!(ask(&t, "What's my next meeting?").await, ("done".into(), "Standup in 12 min, at 10:30.".into()));
    assert_eq!(
        ask(&t, "What's my day like?").await,
        ("done".into(), "3 meetings today. Next: Standup at 10:30, then Design review at 14:00.".into())
    );
    assert!(asked.lock().unwrap().is_empty(), "the model was not asked");
    assert_eq!(t.st.status()["stats"]["quick"], 2);
    // the Activity log is a file: it has the question, not the meetings
    let history = std::fs::read_to_string(t.st.paths.history()).unwrap();
    assert!(history.contains("Calendar: next meeting") && history.contains("(from your calendar)"), "{history}");
    for private in ["Standup", "Design review", "10:30"] {
        assert!(!history.contains(private), "{private} in {history}");
    }
}

#[tokio::test]
async fn joining_opens_the_link_of_the_meeting_on_now_or_the_next_one() {
    let (t, cal, asked) = voice_setup("cal-join").await;
    assert_eq!(ask(&t, "Join my meeting").await, ("done".into(), "Opened Google Meet for Standup.".into()));
    assert_eq!(*t.ran.lock().unwrap(), vec![Action::Open(vec!["https://meet.google.com/abc-defg-hij".into()])]);
    // the stand-up is over, the review (a Zoom link in its notes) is next
    t.st.calendar.set_clock(at("2026-11-12 11:30"), OFFSET);
    refresh(&t.st).await;
    assert_eq!(ask(&t, "join the meeting").await, ("done".into(), "Opened Zoom for Design review.".into()));
    assert_eq!(t.ran.lock().unwrap()[1], Action::Open(vec!["https://example.zoom.us/j/123456789".into()]));
    // after the last link there is nothing to open
    t.st.calendar.set_clock(at("2026-11-12 15:30"), OFFSET);
    refresh(&t.st).await;
    assert_eq!(ask(&t, "join meeting").await, ("error".into(), "No meeting link found.".into()));
    assert_eq!(t.ran.lock().unwrap().len(), 2, "nothing was opened");
    // a link in the notes to some other site is not a meeting link
    cal.set_events(vec![Event {
        notes: "Agenda: https://docs.example.com/d/1 and https://evil.example/join".into(),
        ..meeting("x", "Sync", "2026-11-12 15:45", "2026-11-12 16:15")
    }]);
    refresh(&t.st).await;
    assert_eq!(ask(&t, "join my meeting").await, ("error".into(), "No meeting link found.".into()));
    assert!(asked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn not_connected_the_phrase_says_so_instead_of_asking_the_model() {
    let (t, cal, asked) = voice_setup("cal-voice-off").await;
    disconnect(&t.st).await.unwrap();
    for text in ["What's my next meeting?", "What's my day like?", "Join my meeting"] {
        assert_eq!(ask(&t, text).await, ("error".into(), NOT_CONNECTED.into()), "{text}");
    }
    // permission taken away while it is on: the same words
    connect(&t.st).await.unwrap();
    cal.set_access(Access::Denied);
    assert_eq!(ask(&t, "agenda").await, ("error".into(), NOT_CONNECTED.into()));
    assert!(asked.lock().unwrap().is_empty());
    assert!(t.ran.lock().unwrap().is_empty());
}

// --- the stick tools, with no stick ------------------------------------------------------------------------

async fn no_stick(name: &str) -> (Arc<AppState>, FakeCalendar, Arc<std::sync::Mutex<Vec<Action>>>) {
    let home = std::env::temp_dir().join(format!("stickybot-rs-cal-nostick-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let opts = Options { port: 8670, home, fake_stick: true, import_from: None };
    let st = AppState::new(&opts, Backend::Fake(FakeStick::new())).unwrap(); // the link is never started: no stick
    let ran = Arc::new(std::sync::Mutex::new(Vec::new()));
    let r = ran.clone();
    st.set_runner(Arc::new(move |a: &Action| {
        r.lock().unwrap().push(a.clone());
        String::new()
    }));
    let cal = FakeCalendar::new();
    cal.set_calendars(calendars());
    cal.set_events(day());
    st.calendar.set_source(Arc::new(cal.clone()));
    st.calendar.set_clock(morning(), OFFSET);
    (st, cal, ran)
}

#[tokio::test]
async fn the_calendar_tools_answer_without_a_stick_and_say_when_calendar_is_off() {
    let (st, _cal, ran) = no_stick("tools").await;
    assert!(!st.link.connected());
    let call = |name: &'static str, args: Value| {
        let st = st.clone();
        async move { crate::mcp::call(&st, name, &args, "Agent").await }
    };
    // off: tool errors in words (the stick tools would say "stick not connected")
    for name in ["calendar_next", "calendar_today", "calendar_join"] {
        let (text, err, _) = call(name, json!({})).await;
        assert!(err && text == NOT_CONNECTED, "{name}: {text}");
    }
    assert_eq!(call("notify", json!({"text": "hi"})).await.0, "stick not connected");
    connect(&st).await.unwrap();
    assert_eq!(call("calendar_next", json!({})).await, ("Standup in 12 min, at 10:30.".to_string(), false, None));
    let (today, err, _) = call("calendar_today", json!({})).await;
    assert!(!err);
    assert_eq!(
        today,
        "3 events today:\n10:30-11:00 Standup (Google Meet)\n14:00-15:00 Design review (Zoom)\n16:00-16:30 1:1 with a teammate (Room 2)"
    );
    let (tomorrow, _, _) = call("calendar_today", json!({"day": "tomorrow"})).await;
    assert_eq!(tomorrow, "1 event tomorrow:\n09:00-10:00 Dentist");
    assert!(
        call("calendar_today", json!({"day": "friday"})).await.1,
        "a day that is not today / tomorrow is a tool error"
    );
    let (joined, err, _) = call("calendar_join", json!({})).await;
    assert_eq!((joined.as_str(), err), ("Opened Google Meet for Standup.", false));
    assert_eq!(*ran.lock().unwrap(), vec![Action::Open(vec!["https://meet.google.com/abc-defg-hij".into()])]);
    // an empty day, and the cap on lines
    let (st2, cal2, _) = no_stick("tools-many").await;
    connect(&st2).await.unwrap();
    cal2.set_events(
        (0..15)
            .map(|i| {
                meeting(
                    &format!("m{i}"),
                    &format!("Meeting {i:02}"),
                    &format!("2026-11-12 {:02}:00", 8 + i / 2),
                    &format!("2026-11-12 {:02}:30", 8 + i / 2),
                )
            })
            .collect(),
    );
    refresh(&st2).await;
    let text = crate::mcp::call(&st2, "calendar_today", &json!({}), "Agent").await.0;
    assert_eq!(text.lines().count(), 1 + 12 + 1, "{text}");
    assert!(text.starts_with("15 events today:") && text.ends_with("+ 3 more"), "{text}");
    cal2.set_events(vec![]);
    refresh(&st2).await;
    assert_eq!(
        crate::mcp::call(&st2, "calendar_today", &json!({}), "Agent").await.0,
        "Nothing on your calendar today."
    );
}

// --- meeting text stays out of the logs ----------------------------------------------------------------------------

/// Keeps every record at `info` and above, to look for meeting text in them.
struct Capture(std::sync::Mutex<Vec<String>>);

impl log::Log for Capture {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Debug
    }
    fn log(&self, r: &log::Record) {
        if r.level() <= log::Level::Info {
            self.0.lock().unwrap().push(format!("{}: {}", r.level(), r.args()));
        }
    }
    fn flush(&self) {}
}

static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));

#[tokio::test]
async fn no_meeting_text_reaches_an_info_log_line() {
    let _ = log::set_logger(&CAPTURE);
    log::set_max_level(log::LevelFilter::Debug);
    let (t, cal, _asked) = voice_setup("cal-privacy").await;
    let secret = "Quarterly numbers 7f3a";
    let mut e = meeting("secret", secret, "2026-11-12 10:30", "2026-11-12 11:00");
    e.attendees = vec!["Teammate Zed".into()];
    e.location = "Room 9 7f3a".into();
    e.url = "https://meet.google.com/zzz-yyyy-xxx".into();
    cal.add_event(e);
    // refresh, the card, the reminder (4 min before), voice, tools, the API: every path that touches it
    t.st.calendar.set_clock(at("2026-11-12 10:26"), OFFSET);
    refresh(&t.st).await;
    let _ = crate::home::home_card(&t.st);
    assert!(String::from_utf8_lossy(&t.fake.writes().concat()).contains(secret), "the stick gets the title");
    ask(&t, "What's my next meeting?").await;
    ask(&t, "What's my day like?").await;
    ask(&t, "Join my meeting").await;
    let _ = crate::mcp::call(&t.st, "calendar_today", &json!({}), "Agent").await;
    let _ = crate::mcp::call(&t.st, "calendar_next", &json!({}), "Agent").await;
    let _ = crate::events::test(&t.st, "meeting_soon", None, None).await;
    let lines = CAPTURE.0.lock().unwrap().clone();
    for private in ["7f3a", "Teammate", "zzz-yyyy", "Standup", "Design review", "meet.google.com", "Room 9"] {
        let leaks: Vec<&String> = lines.iter().filter(|l| l.contains(private)).collect();
        assert!(leaks.is_empty(), "{private} in the log: {leaks:?}");
    }
    // the reminder's own line is the kind and the level, nothing else
    assert!(lines.iter().any(|l| l.ends_with("event meeting_soon -> headsup")), "{lines:?}");
    // and the diagnostics take them out of anything that slipped through
    let known = crate::feedback::Known::of(&t.st, None);
    for private in [secret, "Teammate Zed", "Room 9 7f3a", "https://meet.google.com/zzz-yyyy-xxx"] {
        assert!(known.meetings.iter().any(|m| m == private), "{private} not known: {:?}", known.meetings);
        assert!(!crate::feedback::redact(&format!("something {private} else"), &known).contains(private));
    }
}
