use super::*;
use chrono::{Duration, FixedOffset, NaiveDateTime};

/// The Mac's zone in most tests: +05:30, a half-hour zone, so a mix-up between zones shows.
fn ist() -> FixedOffset {
    FixedOffset::east_opt(5 * 3600 + 1800).unwrap()
}

fn utc() -> FixedOffset {
    FixedOffset::east_opt(0).unwrap()
}

fn la() -> FixedOffset {
    FixedOffset::west_opt(8 * 3600).unwrap()
}

/// `2026-11-12 10:30` in `tz` as unix seconds.
fn at(tz: &FixedOffset, s: &str) -> i64 {
    let n = NaiveDateTime::parse_from_str(&format!("{s}:00"), "%Y-%m-%d %H:%M:%S").unwrap();
    tz.from_local_datetime(&n).unwrap().timestamp()
}

fn ev(id: &str, title: &str, start: i64, end: i64) -> Event {
    Event { id: id.into(), calendar_id: "work".into(), title: title.into(), start, end, ..Event::default() }
}

fn on() -> Settings {
    Settings { on: true, ..Settings::default() }
}

const MIN: i64 = 60;

// --- settings -----------------------------------------------------------------------------------

#[test]
fn settings_default_to_off_with_a_five_minute_reminder() {
    let d = Settings::default();
    assert_eq!(
        d.to_json(),
        json!({"on": false, "calendars": [], "skip_all_day": true, "skip_declined": true, "alert_min": 5})
    );
    assert_eq!(Settings::from_value(&Value::Null), d);
}

#[test]
fn a_saved_file_with_missing_or_malformed_keys_reads_as_defaults() {
    let s = Settings::from_value(&json!({"on": true, "alert_min": 500, "calendars": "work", "skip_all_day": "no"}));
    assert_eq!(s, Settings { on: true, ..Settings::default() });
    let s = Settings::from_value(&json!({"calendars": ["a", 3, "b"], "alert_min": 10, "skip_declined": false}));
    assert_eq!(s.calendars, ["a", "b"]);
    assert_eq!((s.alert_min, s.skip_declined, s.skip_all_day), (10, false, true));
    assert_eq!(Settings::from_value(&json!([1, 2])), Settings::default());
    assert_eq!(Settings::from_value(&json!({"alert_min": -1})).alert_min, 5);
}

#[test]
fn settings_are_checked_in_words() {
    let bad = |v: Value| invalid_settings(v.as_object().unwrap());
    assert_eq!(bad(json!({})), "");
    assert_eq!(
        bad(json!({"on": true, "calendars": ["a"], "skip_all_day": false, "skip_declined": true, "alert_min": 0})),
        ""
    );
    assert_eq!(bad(json!({"alert_min": 120})), "");
    assert_eq!(bad(json!({"alert_min": 500})), "alert_min must be 0-120 minutes");
    assert_eq!(bad(json!({"alert_min": -1})), "alert_min must be 0-120 minutes");
    assert_eq!(bad(json!({"alert_min": "5"})), "alert_min must be 0-120 minutes");
    assert_eq!(bad(json!({"alert_min": 2.5})), "alert_min must be 0-120 minutes");
    assert_eq!(bad(json!({"on": "yes"})), "on must be true or false");
    assert_eq!(bad(json!({"skip_all_day": 1})), "skip_all_day must be true or false");
    assert_eq!(bad(json!({"skip_declined": null})), "skip_declined must be true or false");
    assert!(bad(json!({"calendars": "work"})).starts_with("calendars must be a list of calendar ids"));
    assert!(bad(json!({"calendars": [1]})).starts_with("calendars must be a list"));
    let many: Vec<String> = (0..101).map(|i| format!("c{i}")).collect();
    assert!(bad(json!({"calendars": many})).starts_with("calendars must be a list"));
    assert!(bad(json!({"calendars": ["x".repeat(201)]})).starts_with("calendars must be a list"));
    assert_eq!(bad(json!({"calendars": ["x".repeat(200)]})), "");
    let unknown = bad(json!({"colour": "red"}));
    assert!(unknown.starts_with("unknown setting 'colour'") && unknown.contains("alert_min"), "{unknown}");
    // a body can name any subset; the rest stays
    let s = Settings { on: true, calendars: vec!["a".into()], ..Settings::default() };
    let merged = s.with(json!({"alert_min": 10}).as_object().unwrap());
    assert_eq!((merged.on, merged.calendars.clone(), merged.alert_min), (true, vec!["a".to_string()], 10));
}

// --- which events count ---------------------------------------------------------------------------

#[test]
fn canceled_events_and_other_calendars_never_count() {
    let tz = ist();
    let now = at(&tz, "2026-11-12 09:00");
    let mut canceled = ev("c", "Canceled", at(&tz, "2026-11-12 09:10"), at(&tz, "2026-11-12 09:40"));
    canceled.canceled = true;
    let mut other = ev("o", "Other calendar", at(&tz, "2026-11-12 09:20"), at(&tz, "2026-11-12 09:50"));
    other.calendar_id = "home".into();
    let mine = ev("m", "Mine", at(&tz, "2026-11-12 09:30"), at(&tz, "2026-11-12 10:00"));
    let events = vec![canceled, other, mine];
    assert_eq!(next(&events, &on(), now).unwrap().title, "Other calendar");
    let only_work = Settings { calendars: vec!["work".into()], ..on() };
    assert_eq!(next(&events, &only_work, now).unwrap().title, "Mine");
    let titles = |s: &Settings| agenda_titles(&today(&events, s, &tz, now));
    assert_eq!(titles(&on()), ["Other calendar", "Mine"]);
    assert_eq!(titles(&only_work), ["Mine"]);
    let none_chosen = Settings { calendars: vec!["nope".into()], ..on() };
    assert!(next(&events, &none_chosen, now).is_none() && titles(&none_chosen).is_empty());
}

fn agenda_titles(v: &[&Event]) -> Vec<String> {
    v.iter().map(|e| e.title.clone()).collect()
}

#[test]
fn all_day_and_declined_never_count_as_a_meeting_whatever_the_switches_say() {
    let tz = ist();
    let now = at(&tz, "2026-11-12 09:00");
    let mut all_day = ev("a", "Offsite", at(&tz, "2026-11-12 00:00"), at(&tz, "2026-11-12 23:59"));
    all_day.all_day = true;
    let mut declined = ev("d", "Declined", at(&tz, "2026-11-12 09:05"), at(&tz, "2026-11-12 09:35"));
    declined.declined = true;
    let real = ev("r", "Real", at(&tz, "2026-11-12 09:30"), at(&tz, "2026-11-12 10:00"));
    let events = vec![all_day, declined, real];
    for (skip_all_day, skip_declined) in [(true, true), (false, false)] {
        let s = Settings { skip_all_day, skip_declined, ..on() };
        assert_eq!(next(&events, &s, now).unwrap().title, "Real");
        assert!(current(&events, &s, now).is_none(), "the all-day event is not 'now'");
        assert_eq!(current(&events, &s, at(&tz, "2026-11-12 09:10")), None);
        assert!(due_alerts(&events, &s, at(&tz, "2026-11-12 09:03"), &HashMap::new()).is_empty());
    }
    // with a switch off they only show in the day's list
    let show = Settings { skip_all_day: false, skip_declined: false, ..on() };
    assert_eq!(agenda_titles(&today(&events, &show, &tz, now)), ["Offsite", "Declined", "Real"]);
    assert_eq!(agenda_titles(&today(&events, &on(), &tz, now)), ["Real"]);
    let only_declined = Settings { skip_all_day: true, skip_declined: false, ..on() };
    assert_eq!(agenda_titles(&today(&events, &only_declined, &tz, now)), ["Declined", "Real"]);
}

// --- now and next -----------------------------------------------------------------------------------

#[test]
fn overlapping_meetings_while_one_runs_next_is_still_the_next_start() {
    let tz = ist();
    let long = ev("long", "Workshop", at(&tz, "2026-11-12 10:00"), at(&tz, "2026-11-12 12:00"));
    let inside = ev("inside", "Quick sync", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"));
    let later = ev("later", "Later", at(&tz, "2026-11-12 13:00"), at(&tz, "2026-11-12 13:30"));
    let events = vec![later.clone(), inside.clone(), long.clone()];
    let now = at(&tz, "2026-11-12 10:10");
    assert_eq!(current(&events, &on(), now).unwrap().title, "Workshop");
    assert_eq!(next(&events, &on(), now).unwrap().title, "Quick sync");
    // both are on: the one that started last
    let now = at(&tz, "2026-11-12 10:45");
    assert_eq!(current(&events, &on(), now).unwrap().title, "Quick sync");
    assert_eq!(next(&events, &on(), now).unwrap().title, "Later");
    assert!(next(&events, &on(), at(&tz, "2026-11-12 13:00")).is_none());
}

#[test]
fn back_to_back_meetings_the_first_is_over_at_the_instant_the_second_starts() {
    let tz = ist();
    let first = ev("a", "First", at(&tz, "2026-11-12 10:00"), at(&tz, "2026-11-12 10:30"));
    let second = ev("b", "Second", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"));
    let events = vec![second, first];
    let before = at(&tz, "2026-11-12 10:30") - 1;
    assert_eq!(current(&events, &on(), before).unwrap().title, "First");
    assert_eq!(next(&events, &on(), before).unwrap().title, "Second");
    let at_end = at(&tz, "2026-11-12 10:30");
    assert_eq!(current(&events, &on(), at_end).unwrap().title, "Second");
    assert!(next(&events, &on(), at_end).is_none());
    // the start is "now", the end is not
    assert_eq!(current(&events, &on(), at(&tz, "2026-11-12 10:00")).unwrap().title, "First");
    assert!(current(&events, &on(), at(&tz, "2026-11-12 11:00")).is_none());
}

#[test]
fn meetings_starting_together_the_shorter_one_is_next_then_the_title() {
    let tz = ist();
    let start = at(&tz, "2026-11-12 14:00");
    let events = vec![
        ev("1", "Zed", start, start + 60 * MIN),
        ev("2", "Brief", start, start + 15 * MIN),
        ev("3", "Alpha", start, start + 15 * MIN),
    ];
    assert_eq!(next(&events, &on(), at(&tz, "2026-11-12 09:00")).unwrap().title, "Alpha");
}

#[test]
fn recurring_occurrences_share_an_id_but_each_counts_on_its_own() {
    let tz = ist();
    let daily = |day: &str| {
        ev("standup", "Standup", at(&tz, &format!("2026-11-{day} 10:00")), at(&tz, &format!("2026-11-{day} 10:15")))
    };
    let events = vec![daily("12"), daily("13"), daily("14")];
    let now = at(&tz, "2026-11-12 10:20"); // today's is over
    assert_eq!(next(&events, &on(), now).unwrap().start, at(&tz, "2026-11-13 10:00"));
    assert_eq!(today(&events, &on(), &tz, now).len(), 1);
    // a reminder for today's occurrence does not hide tomorrow's
    let mut fired = HashMap::new();
    let first = at(&tz, "2026-11-12 10:00");
    fired.insert(("standup".to_string(), first), first + 15 * MIN);
    assert!(due_alerts(&events, &on(), at(&tz, "2026-11-12 09:57"), &fired).is_empty());
    assert_eq!(due_alerts(&events, &on(), at(&tz, "2026-11-13 09:57"), &fired).len(), 1);
}

// --- days, zones --------------------------------------------------------------------------------------

#[test]
fn an_event_made_in_another_zone_shows_at_its_local_time_on_the_local_day() {
    // 23:30 on the 12th in Los Angeles is 07:30 UTC on the 13th and 13:00 on the 13th in India
    let call = ev("c", "Late call", at(&la(), "2026-11-12 23:30"), at(&la(), "2026-11-13 00:30"));
    let events = vec![call];
    let day = |tz: &FixedOffset, d: &str| today(&events, &on(), tz, at(tz, &format!("{d} 08:00")));
    assert_eq!(day(&la(), "2026-11-12").len(), 1);
    assert_eq!(day(&la(), "2026-11-13").len(), 1, "it runs past midnight there: both days");
    assert_eq!(day(&utc(), "2026-11-12").len(), 0);
    assert_eq!(day(&utc(), "2026-11-13").len(), 1);
    assert_eq!(day(&ist(), "2026-11-13").len(), 1);
    assert_eq!(hhmm(&la(), events[0].start), "23:30");
    assert_eq!(hhmm(&utc(), events[0].start), "07:30");
    assert_eq!(hhmm(&ist(), events[0].start), "13:00");
    assert_eq!(date_of(&ist(), events[0].start), NaiveDate::from_ymd_opt(2026, 11, 13));
    assert_eq!(date_of(&la(), events[0].start), NaiveDate::from_ymd_opt(2026, 11, 12));
}

#[test]
fn a_multi_day_event_is_on_each_day_it_covers_and_a_day_has_exact_edges() {
    let tz = ist();
    let trip = ev("trip", "Conference", at(&tz, "2026-11-11 18:00"), at(&tz, "2026-11-13 12:00"));
    let ends_at_midnight = ev("e", "Ends at midnight", at(&tz, "2026-11-11 22:00"), at(&tz, "2026-11-12 00:00"));
    let starts_at_midnight = ev("s", "Starts at midnight", at(&tz, "2026-11-13 00:00"), at(&tz, "2026-11-13 01:00"));
    let events = vec![trip, ends_at_midnight, starts_at_midnight];
    let on_day = |d: &str| {
        let date = NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap();
        agenda_titles(&day(&events, &on(), &tz, date))
    };
    assert_eq!(on_day("2026-11-11"), ["Conference", "Ends at midnight"]);
    assert_eq!(on_day("2026-11-12"), ["Conference"]);
    assert_eq!(on_day("2026-11-13"), ["Conference", "Starts at midnight"]);
    assert!(on_day("2026-11-14").is_empty());
    let (from, to) = day_bounds(&tz, NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()).unwrap();
    assert_eq!((from, to - from), (at(&tz, "2026-11-12 00:00"), 86_400));
}

#[test]
fn an_all_day_event_lists_on_its_own_day_only() {
    let tz = ist();
    let mut off = ev("o", "Offsite", at(&tz, "2026-11-12 00:00"), at(&tz, "2026-11-12 23:59") + 59);
    off.all_day = true;
    let events = vec![off];
    let show = Settings { skip_all_day: false, ..on() };
    let d = |d: u32| agenda_titles(&day(&events, &show, &tz, NaiveDate::from_ymd_opt(2026, 11, d).unwrap()));
    assert_eq!((d(11), d(12), d(13)), (Vec::<String>::new(), vec!["Offsite".to_string()], Vec::<String>::new()));
    assert!(day(&events, &on(), &tz, NaiveDate::from_ymd_opt(2026, 11, 12).unwrap()).is_empty());
}

#[test]
fn the_day_is_sorted_by_start_then_title() {
    let tz = ist();
    let s = at(&tz, "2026-11-12 10:00");
    let events = vec![
        ev("3", "Zed", s + 60 * MIN, s + 90 * MIN),
        ev("2", "Beta", s, s + 30 * MIN),
        ev("1", "Alpha", s, s + 30 * MIN),
    ];
    assert_eq!(agenda_titles(&today(&events, &on(), &tz, s)), ["Alpha", "Beta", "Zed"]);
}

// --- the home card -------------------------------------------------------------------------------------

fn card_at(events: &[Event], tz: &FixedOffset, now: &str) -> Option<MeetingCard> {
    meeting_card(events, &on(), tz, at(tz, now))
}

fn texts(c: &MeetingCard) -> (String, String) {
    (c.card["a"].as_str().unwrap().to_string(), c.card["b"].as_str().unwrap().to_string())
}

#[test]
fn the_card_says_how_far_off_and_when() {
    let tz = ist();
    let events = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    let c = card_at(&events, &tz, "2026-11-12 10:18").unwrap();
    assert_eq!(c.card["k"], "meeting");
    assert_eq!(texts(&c), ("Standup".into(), "in 12 min · 10:30".into()));
    // minutes round up: 11 min 20 s is "in 12 min", under a minute is "in 1 min"
    let now = at(&tz, "2026-11-12 10:18") + 40;
    assert_eq!(texts(&meeting_card(&events, &on(), &tz, now).unwrap()).1, "in 12 min · 10:30");
    assert_eq!(
        texts(&meeting_card(&events, &on(), &tz, at(&tz, "2026-11-12 10:30") - 20).unwrap()).1,
        "in 1 min · 10:30"
    );
    // 89 min is still minutes; from 90 min it is hours, to the nearest
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 09:01").unwrap()).1, "in 89 min · 10:30");
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 09:00").unwrap()).1, "in 2 h · 10:30");
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 08:00").unwrap()).1, "in 3 h · 10:30");
    // running
    let running = card_at(&events, &tz, "2026-11-12 10:45").unwrap();
    assert_eq!(texts(&running), ("Standup".into(), "now · until 11:00".into()));
    assert!(card_at(&events, &tz, "2026-11-12 11:00").is_none(), "over");
}

#[test]
fn the_card_is_pinned_from_fifteen_minutes_before_until_five_after() {
    let tz = ist();
    let events = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:30"))];
    let pinned = |now: &str| card_at(&events, &tz, now).map(|c| c.pinned);
    assert_eq!(pinned("2026-11-12 10:14"), Some(false));
    assert_eq!(pinned("2026-11-12 10:15"), Some(true));
    assert_eq!(pinned("2026-11-12 10:29"), Some(true));
    assert_eq!(pinned("2026-11-12 10:30"), Some(true)); // just started
    assert_eq!(pinned("2026-11-12 10:34"), Some(true));
    assert_eq!(pinned("2026-11-12 10:35"), Some(false)); // five minutes in: back to the rotation
    assert_eq!(pinned("2026-11-12 11:00"), Some(false));
    // it is not urgent: orange is for what waits for a button
    assert!(card_at(&events, &tz, "2026-11-12 10:20").unwrap().card.get("u").is_none());
}

#[test]
fn the_next_meeting_wins_over_a_long_one_that_is_ending_when_it_is_close() {
    let tz = ist();
    let events = vec![
        ev("long", "Workshop", at(&tz, "2026-11-12 09:00"), at(&tz, "2026-11-12 11:00")),
        ev("next", "Retro", at(&tz, "2026-11-12 10:50"), at(&tz, "2026-11-12 11:30")),
    ];
    let c = card_at(&events, &tz, "2026-11-12 10:40").unwrap();
    assert_eq!((texts(&c).0.as_str(), c.pinned), ("Retro", true));
    // a meeting that just started beats the next one
    let events = vec![
        ev("a", "Just started", at(&tz, "2026-11-12 10:00"), at(&tz, "2026-11-12 10:30")),
        ev("b", "Soon", at(&tz, "2026-11-12 10:10"), at(&tz, "2026-11-12 10:40")),
    ];
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 10:02").unwrap()).0, "Just started");
}

#[test]
fn the_card_shows_a_meeting_today_or_within_eight_hours_only() {
    let tz = ist();
    let late = vec![ev("l", "Evening review", at(&tz, "2026-11-12 19:00"), at(&tz, "2026-11-12 20:00"))];
    // a long gap, but still today: shown in the rotation
    assert_eq!(texts(&card_at(&late, &tz, "2026-11-12 07:00").unwrap()).1, "in 12 h · 19:00");
    // tomorrow, more than 8 h away: not shown
    let tomorrow = vec![ev("t", "Early start", at(&tz, "2026-11-13 09:00"), at(&tz, "2026-11-13 10:00"))];
    assert!(card_at(&tomorrow, &tz, "2026-11-12 18:00").is_none());
    // tomorrow but close enough to matter (after midnight)
    let after_midnight = vec![ev("m", "Night call", at(&tz, "2026-11-13 02:00"), at(&tz, "2026-11-13 03:00"))];
    assert_eq!(texts(&card_at(&after_midnight, &tz, "2026-11-12 22:00").unwrap()).1, "in 4 h · 02:00");
    assert!(card_at(&[], &tz, "2026-11-12 22:00").is_none());
}

#[test]
fn a_title_is_cut_and_made_stick_safe() {
    let tz = ist();
    let long = "Quarterly planning and roadmap review with the whole platform group";
    let events = vec![ev("l", long, at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    let a = texts(&card_at(&events, &tz, "2026-11-12 10:20").unwrap()).0;
    assert_eq!(a.chars().count(), 40);
    assert!(a.ends_with('…') && a.starts_with("Quarterly planning and roadmap review"), "{a}");
    // letters the stick's font lacks become plain ones; symbols it lacks go; spaces are tidied
    let events = vec![ev("c", "  Café ☕  review  ", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 10:20").unwrap()).0, "Cafe review");
    let events = vec![ev("e", "   ", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    assert_eq!(texts(&card_at(&events, &tz, "2026-11-12 10:20").unwrap()).0, "Meeting");
}

#[test]
fn declined_all_day_and_canceled_events_never_make_a_card() {
    let tz = ist();
    let mut declined = ev("d", "Declined", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"));
    declined.declined = true;
    let mut all_day = ev("a", "All day", at(&tz, "2026-11-12 00:00"), at(&tz, "2026-11-12 23:59"));
    all_day.all_day = true;
    let mut canceled = ev("c", "Canceled", at(&tz, "2026-11-12 10:35"), at(&tz, "2026-11-12 11:00"));
    canceled.canceled = true;
    assert!(card_at(&[declined, all_day, canceled], &tz, "2026-11-12 10:20").is_none());
}

// --- the reminder ---------------------------------------------------------------------------------------

#[test]
fn a_reminder_is_due_from_alert_min_before_the_start_until_the_start() {
    let tz = ist();
    let events = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    let due = |now: &str| due_alerts(&events, &on(), at(&tz, now), &HashMap::new()).len();
    assert_eq!(due("2026-11-12 10:24"), 0);
    assert_eq!(due("2026-11-12 10:25"), 1);
    assert_eq!(due("2026-11-12 10:29"), 1);
    assert_eq!(due("2026-11-12 10:30"), 0, "at the start it is on, not coming up");
    let ten = Settings { alert_min: 10, ..on() };
    assert_eq!(due_alerts(&events, &ten, at(&tz, "2026-11-12 10:21"), &HashMap::new()).len(), 1);
    // a meeting made or moved into the window is reminded once, at that point
    let moved = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:27"), at(&tz, "2026-11-12 11:00"))];
    assert_eq!(due_alerts(&moved, &on(), at(&tz, "2026-11-12 10:26"), &HashMap::new()).len(), 1);
}

#[test]
fn a_reminder_fires_once_per_occurrence() {
    let tz = ist();
    let events = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"))];
    let mut fired = HashMap::new();
    let now = at(&tz, "2026-11-12 10:27");
    let first = due_alerts(&events, &on(), now, &fired);
    assert_eq!(first.len(), 1);
    fired.insert((first[0].id.clone(), first[0].start), first[0].end);
    assert!(due_alerts(&events, &on(), now + 30, &fired).is_empty());
    assert!(due_alerts(&events, &on(), now + 120, &fired).is_empty());
    // forgotten once the meeting is over, not before
    prune_fired(&mut fired, at(&tz, "2026-11-12 10:59"));
    assert_eq!(fired.len(), 1);
    prune_fired(&mut fired, at(&tz, "2026-11-12 11:00"));
    assert!(fired.is_empty());
    // a meeting moved to another time is another occurrence
    let moved = vec![ev("s", "Standup", at(&tz, "2026-11-12 10:35"), at(&tz, "2026-11-12 11:00"))];
    fired.insert(("s".to_string(), at(&tz, "2026-11-12 10:30")), at(&tz, "2026-11-12 11:00"));
    assert_eq!(due_alerts(&moved, &on(), at(&tz, "2026-11-12 10:31"), &fired).len(), 1);
}

#[test]
fn no_reminder_for_all_day_declined_canceled_alert_min_zero_or_off() {
    let tz = ist();
    let start = at(&tz, "2026-11-12 10:30");
    let now = at(&tz, "2026-11-12 10:27");
    let mut all_day = ev("a", "All day", start, start + 30 * MIN);
    all_day.all_day = true;
    let mut declined = ev("d", "Declined", start, start + 30 * MIN);
    declined.declined = true;
    let mut canceled = ev("c", "Canceled", start, start + 30 * MIN);
    canceled.canceled = true;
    let normal = ev("n", "Normal", start, start + 30 * MIN);
    let none = HashMap::new();
    assert!(due_alerts(&[all_day, declined, canceled], &on(), now, &none).is_empty());
    let events = vec![normal];
    assert_eq!(due_alerts(&events, &on(), now, &none).len(), 1);
    assert!(due_alerts(&events, &Settings { alert_min: 0, ..on() }, now, &none).is_empty());
    assert!(due_alerts(&events, &Settings::default(), now, &none).is_empty(), "off");
    assert!(due_alerts(&events, &Settings { calendars: vec!["home".into()], ..on() }, now, &none).is_empty());
}

#[test]
fn the_reminder_text() {
    let tz = ist();
    let mut e = ev("s", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"));
    e.url = "https://meet.google.com/abc-defg-hij".into();
    let now = at(&tz, "2026-11-12 10:25");
    assert_eq!(alert_text(&e, &tz, now), ("Standup in 5 min".into(), "10:30 · Google Meet".into()));
    assert_eq!(alert_text(&e, &tz, now + 70).0, "Standup in 4 min");
    assert_eq!(alert_text(&e, &tz, e.start - 59).0, "Standup starts now");
    // no link: the room; nothing: just the time
    e.url.clear();
    e.location = "Room 2\n1 Example Street".into();
    assert_eq!(alert_text(&e, &tz, now).1, "10:30 · Room 2");
    e.location.clear();
    assert_eq!(alert_text(&e, &tz, now).1, "10:30");
    // a long title is cut so "in 5 min" is never the part that is lost
    e.title = "Quarterly planning and roadmap review with the whole platform group".into();
    let (title, _) = alert_text(&e, &tz, now);
    assert!(title.chars().count() <= 40 && title.ends_with("… in 5 min"), "{title}");
}

// --- what is said -----------------------------------------------------------------------------------------

#[test]
fn saying_the_next_meeting() {
    let tz = ist();
    let events = vec![
        ev("a", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00")),
        ev("b", "Design review", at(&tz, "2026-11-12 14:00"), at(&tz, "2026-11-12 15:00")),
    ];
    let say = |now: &str| say_next(&events, &on(), &tz, at(&tz, now));
    assert_eq!(say("2026-11-12 10:18"), "Standup in 12 min, at 10:30.");
    assert_eq!(say("2026-11-12 09:00"), "Standup in 2 h, at 10:30.");
    assert_eq!(say("2026-11-12 10:40"), "Standup is on now, until 11:00. Next: Design review at 14:00.");
    assert_eq!(say("2026-11-12 14:10"), "Design review is on now, until 15:00.");
    assert_eq!(say("2026-11-12 15:00"), "Nothing else today.");
    // tomorrow's first meeting is not "today"
    let tomorrow = vec![ev("t", "Early", at(&tz, "2026-11-13 09:00"), at(&tz, "2026-11-13 09:30"))];
    assert_eq!(say_next(&tomorrow, &on(), &tz, at(&tz, "2026-11-12 18:00")), "Nothing else today.");
    assert_eq!(say_next(&[], &on(), &tz, 0), "Nothing else today.");
}

#[test]
fn saying_the_day() {
    let tz = ist();
    let mut events = vec![
        ev("a", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00")),
        ev("b", "Design review", at(&tz, "2026-11-12 14:00"), at(&tz, "2026-11-12 15:00")),
        ev("c", "1:1 with a teammate", at(&tz, "2026-11-12 16:00"), at(&tz, "2026-11-12 16:30")),
    ];
    let say = |events: &[Event], now: &str| say_today(events, &on(), &tz, at(&tz, now));
    assert_eq!(
        say(&events, "2026-11-12 09:00"),
        "3 meetings today. Next: Standup at 10:30, then Design review at 14:00."
    );
    assert_eq!(say(&events, "2026-11-12 14:30"), "3 meetings today. Next: 1:1 with a teammate at 16:00.");
    assert_eq!(say(&events, "2026-11-12 16:10"), "3 meetings today. 1:1 with a teammate is on now, until 16:30.");
    assert_eq!(say(&events, "2026-11-12 17:00"), "3 meetings today. None left.");
    assert_eq!(say(&events[..1], "2026-11-12 09:00"), "1 meeting today. Next: Standup at 10:30.");
    assert_eq!(say(&[], "2026-11-12 09:00"), "Nothing on your calendar today.");
    // declined and all-day events are not meetings
    events[0].declined = true;
    let mut all_day = ev("d", "Offsite", at(&tz, "2026-11-12 00:00"), at(&tz, "2026-11-12 23:59"));
    all_day.all_day = true;
    events.push(all_day);
    assert_eq!(
        say(&events, "2026-11-12 09:00"),
        "2 meetings today. Next: Design review at 14:00, then 1:1 with a teammate at 16:00."
    );
}

#[test]
fn a_day_listing_for_a_tool() {
    let tz = ist();
    let mut a = ev("a", "Standup", at(&tz, "2026-11-12 10:30"), at(&tz, "2026-11-12 11:00"));
    a.url = "https://example.zoom.us/j/123456789".into();
    let mut b = ev("b", "Design review", at(&tz, "2026-11-12 14:00"), at(&tz, "2026-11-12 15:00"));
    b.location = "Room 2".into();
    b.declined = true;
    let mut c = ev("c", "Offsite", at(&tz, "2026-11-12 00:00"), at(&tz, "2026-11-12 23:59"));
    c.all_day = true;
    assert_eq!(day_line(&a, &tz), "10:30-11:00 Standup (Zoom)");
    assert_eq!(day_line(&b, &tz), "14:00-15:00 Design review (Room 2) [declined]");
    assert_eq!(day_line(&c, &tz), "all day Offsite");
}

// --- meeting links ------------------------------------------------------------------------------------------

fn link(f: impl FnOnce(&mut Event)) -> Option<(&'static str, String)> {
    let mut e = Event::default();
    f(&mut e);
    join_link(&e)
}

#[test]
fn every_provider_is_recognised() {
    let url = |u: &str| link(|e| e.url = u.into()).map(|(p, _)| p);
    assert_eq!(url("https://meet.google.com/abc-defg-hij"), Some("Google Meet"));
    assert_eq!(url("https://meet.google.com/lookup/abc123?authuser=0"), Some("Google Meet"));
    assert_eq!(url("https://zoom.us/j/123456789?pwd=x"), Some("Zoom"));
    assert_eq!(url("https://example.zoom.us/j/123456789"), Some("Zoom"));
    assert_eq!(url("https://example.zoom.us/my/someroom"), Some("Zoom"));
    assert_eq!(url("https://us02web.zoom.us/w/123456789"), Some("Zoom"));
    assert_eq!(url("https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc/0?context=x"), Some("Microsoft Teams"));
    assert_eq!(url("https://teams.microsoft.com/meet/123456789"), Some("Microsoft Teams"));
    assert_eq!(url("https://teams.live.com/meet/123456789"), Some("Microsoft Teams"));
    assert_eq!(url("https://example.webex.com/example/j.php?MTID=abc"), Some("Webex"));
    assert_eq!(url("https://webex.com/meet/someone"), Some("Webex"));
    // the host is not case sensitive
    assert_eq!(url("https://MEET.Google.com/abc-defg-hij"), Some("Google Meet"));
}

#[test]
fn links_are_looked_for_in_the_url_then_the_location_then_the_notes() {
    let meet = "https://meet.google.com/abc-defg-hij";
    let zoom = "https://example.zoom.us/j/123456789";
    let teams = "https://teams.microsoft.com/meet/123456789";
    assert_eq!(link(|e| e.url = zoom.into()).unwrap().1, zoom);
    assert_eq!(link(|e| e.location = meet.into()).unwrap(), ("Google Meet", meet.to_string()));
    assert_eq!(link(|e| e.notes = format!("Agenda\nJoin: {teams}\nThanks")).unwrap().0, "Microsoft Teams");
    // the first field that has one wins
    let both = link(|e| {
        e.url = zoom.into();
        e.location = meet.into();
        e.notes = teams.into();
    });
    assert_eq!(both.unwrap().0, "Zoom");
    let loc_notes = link(|e| {
        e.location = meet.into();
        e.notes = zoom.into();
    });
    assert_eq!(loc_notes.unwrap().0, "Google Meet");
    // a docs link before the conference link in the same notes is passed over
    let notes = link(|e| {
        e.notes = "Doc: https://docs.example.com/d/1 and call https://example.zoom.us/j/55 please".to_string()
    });
    assert_eq!(notes.unwrap(), ("Zoom", "https://example.zoom.us/j/55".to_string()));
    assert!(link(|_| {}).is_none());
}

#[test]
fn punctuation_brackets_and_html_around_a_link_are_tolerated() {
    let want = Some(("Google Meet", "https://meet.google.com/abc-defg-hij".to_string()));
    for notes in [
        "Join https://meet.google.com/abc-defg-hij.",
        "Join (https://meet.google.com/abc-defg-hij)",
        "Join <https://meet.google.com/abc-defg-hij>",
        "<a href=\"https://meet.google.com/abc-defg-hij\">Join the call</a>",
        "<p>https://meet.google.com/abc-defg-hij</p>",
        "Join: https://meet.google.com/abc-defg-hij, then wait",
        "https://meet.google.com/abc-defg-hij;",
        "line one\r\nhttps://meet.google.com/abc-defg-hij\r\nline three",
    ] {
        assert_eq!(link(|e| e.notes = notes.into()), want, "{notes}");
    }
}

#[test]
fn only_https_links_on_known_conference_hosts_are_ever_meeting_links() {
    for refused in [
        "http://meet.google.com/abc-defg-hij",
        "http://example.zoom.us/j/123456789",
        "https://example.com/meet/abc",
        "https://docs.google.com/document/d/1",
        "https://meet.google.com.evil.example/abc-defg-hij",
        "https://evil-meet.google.com.example.net/abc",
        "https://notzoom.us/j/123",
        "https://zoom.us.example.com/j/123",
        "https://meet.google.com@evil.example/abc-defg-hij",
        "https://user:pass@meet.google.com/abc-defg-hij",
        "https://meet.google.com:8443/abc-defg-hij",
        "https://meet.google.com/",
        "https://meet.google.com",
        "https://zoom.us/pricing",
        "https://zoom.us/j/",
        "https://teams.microsoft.com/",
        "https://teams.microsoft.com/l/message/123",
        "https://www.microsoft.com/meet/1",
        "https://webex.com.example.org/x/y",
        "ftp://meet.google.com/abc",
        "javascript:alert(1)",
        "file:///etc/passwd",
        "meet.google.com/abc-defg-hij",
    ] {
        for field in 0..3 {
            let found = link(|e| match field {
                0 => e.url = refused.into(),
                1 => e.location = refused.into(),
                _ => e.notes = format!("see {refused} for details"),
            });
            assert!(found.is_none(), "{refused} (field {field}) gave {found:?}");
        }
    }
}

#[test]
fn joining_picks_the_meeting_on_now_then_the_next_one_with_a_link_today() {
    let tz = ist();
    let with_link = |id: &str, title: &str, start: &str, end: &str, host: &str| {
        let mut e = ev(id, title, at(&tz, start), at(&tz, end));
        e.url = format!("https://{host}");
        e
    };
    let a = with_link("a", "Standup", "2026-11-12 10:00", "2026-11-12 10:30", "meet.google.com/abc-defg-hij");
    let b = with_link("b", "Review", "2026-11-12 14:00", "2026-11-12 15:00", "example.zoom.us/j/55");
    let no_link = ev("n", "Hallway chat", at(&tz, "2026-11-12 11:00"), at(&tz, "2026-11-12 11:30"));
    let events = vec![b.clone(), no_link.clone(), a.clone()];
    let pick = |now: &str| join_choice(&events, &on(), &tz, at(&tz, now)).map(|(e, p, _)| (e.title.clone(), p));
    assert_eq!(pick("2026-11-12 10:10"), Some(("Standup".into(), "Google Meet")));
    assert_eq!(pick("2026-11-12 09:50"), Some(("Standup".into(), "Google Meet")));
    // the running one has no link: the next one that has
    assert_eq!(pick("2026-11-12 11:10"), Some(("Review".into(), "Zoom")));
    assert_eq!(pick("2026-11-12 15:10"), None);
    // two on: the one that started last
    let mut x = with_link("x", "Late joiner", "2026-11-12 10:05", "2026-11-12 10:25", "example.webex.com/w/1");
    x.title = "Late joiner".into();
    let both = vec![a, x];
    let now = at(&tz, "2026-11-12 10:10");
    assert_eq!(join_choice(&both, &on(), &tz, now).unwrap().0.title, "Late joiner");
    // tomorrow's is not joined today; a declined one never
    let mut declined =
        with_link("d", "Declined", "2026-11-12 10:00", "2026-11-12 11:00", "meet.google.com/zzz-zzzz-zzz");
    declined.declined = true;
    let tomorrow = with_link("t", "Tomorrow", "2026-11-13 10:00", "2026-11-13 11:00", "meet.google.com/ttt-tttt-ttt");
    assert!(join_choice(&[declined, tomorrow], &on(), &tz, now).is_none());
}

#[test]
fn joining_reaches_a_meeting_starting_within_fifteen_minutes_across_midnight() {
    let tz = ist();
    let mut late = ev("m", "Midnight call", at(&tz, "2026-11-13 00:05"), at(&tz, "2026-11-13 00:35"));
    late.url = "https://meet.google.com/abc-defg-hij".into();
    let events = vec![late];
    let pick = |now: &str| join_choice(&events, &on(), &tz, at(&tz, now)).map(|(e, _, _)| e.title.clone());
    assert_eq!(pick("2026-11-12 23:55"), Some("Midnight call".into()));
    assert_eq!(pick("2026-11-12 23:50"), Some("Midnight call".into()), "exactly 15 min ahead");
    assert_eq!(pick("2026-11-12 23:49"), None, "16 min ahead, and tomorrow");
}

#[test]
fn a_one_minute_reminder_cannot_fall_between_two_ticks() {
    // the check runs every ALERT_TICK_S; whatever the phase, a reminder with a 1 minute lead is due at
    // some tick, and only at one tick once it is recorded as fired
    const { assert!(ALERT_TICK_S * 2 <= 60) } // at least two checks fit in the shortest window
    let tz = ist();
    let start = at(&tz, "2026-11-12 10:30");
    let events = vec![ev("s", "Standup", start, start + 30 * MIN)];
    let s = Settings { alert_min: 1, ..on() };
    for phase in 0..ALERT_TICK_S {
        let mut fired = HashMap::new();
        let mut reminded = 0;
        let mut now = start - 5 * MIN + phase;
        while now < start + MIN {
            let due = due_alerts(&events, &s, now, &fired);
            reminded += due.len();
            for e in due {
                fired.insert((e.id.clone(), e.start), e.end);
            }
            now += ALERT_TICK_S;
        }
        assert_eq!(reminded, 1, "phase {phase}");
    }
}

/// A made-up zone that is UTC-5 until a clock change on 2026-03-08 and UTC-4 after it: the clocks
/// jump forward over `[gap_hour:00, gap_hour+1:00)` local, so that hour does not exist.
#[derive(Clone, Copy, Debug)]
struct Springs {
    gap_hour: u32,
}

impl Springs {
    const BEFORE: i32 = -5 * 3600;
    const AFTER: i32 = -4 * 3600;

    /// the instant of the change, UTC
    fn change(&self) -> i64 {
        FixedOffset::east_opt(Self::BEFORE)
            .unwrap()
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 3, 8).unwrap().and_hms_opt(self.gap_hour, 0, 0).unwrap(),
            )
            .unwrap()
            .timestamp()
    }

    fn at_utc(&self, utc: i64) -> FixedOffset {
        FixedOffset::east_opt(if utc >= self.change() { Self::AFTER } else { Self::BEFORE }).unwrap()
    }
}

impl TimeZone for Springs {
    type Offset = FixedOffset;

    fn from_offset(offset: &FixedOffset) -> Self {
        let _ = offset;
        Springs { gap_hour: 2 }
    }

    fn offset_from_local_date(&self, d: &NaiveDate) -> chrono::LocalResult<FixedOffset> {
        self.offset_from_local_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
    }

    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> chrono::LocalResult<FixedOffset> {
        let gap_start = NaiveDate::from_ymd_opt(2026, 3, 8).unwrap().and_hms_opt(self.gap_hour, 0, 0).unwrap();
        let gap_end = gap_start + Duration::hours(1);
        if *local < gap_start {
            chrono::LocalResult::Single(FixedOffset::east_opt(Self::BEFORE).unwrap())
        } else if *local >= gap_end {
            chrono::LocalResult::Single(FixedOffset::east_opt(Self::AFTER).unwrap())
        } else {
            chrono::LocalResult::None
        }
    }

    fn offset_from_utc_date(&self, d: &NaiveDate) -> FixedOffset {
        self.offset_from_utc_datetime(&d.and_hms_opt(12, 0, 0).unwrap())
    }

    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
        self.at_utc(utc.and_utc().timestamp())
    }
}

#[test]
fn a_day_with_a_clock_change_is_23_hours_and_events_keep_their_local_times() {
    let day = |tz: &Springs, d: u32| day_bounds(tz, NaiveDate::from_ymd_opt(2026, 3, d).unwrap()).unwrap();
    // the clocks skip 02:00-03:00: the 8th has 23 hours, its neighbours 24
    let z = Springs { gap_hour: 2 };
    let (from, to) = day(&z, 8);
    assert_eq!(to - from, 23 * 3600);
    assert_eq!(day(&z, 7).1 - day(&z, 7).0, 24 * 3600);
    assert_eq!(day(&z, 9).1 - day(&z, 9).0, 24 * 3600);
    assert_eq!((day(&z, 7).1, day(&z, 8).1), (from, day(&z, 9).0), "the days follow each other");
    // an hour after the change is 03:xx local, not 02:xx
    let after = z.change() + 30 * MIN;
    assert_eq!(hhmm(&z, after), "03:30");
    assert_eq!(hhmm(&z, z.change() - 30 * MIN), "01:30");
    let meeting = ev("m", "Early call", after, after + 30 * MIN);
    assert_eq!(today(std::slice::from_ref(&meeting), &on(), &z, after).len(), 1);
    // a day whose midnight does not exist starts at the first hour that does (01:00), and is 23 hours
    let z = Springs { gap_hour: 0 };
    let (from, to) = day(&z, 8);
    assert_eq!(from, z.change());
    assert_eq!(to - from, 23 * 3600);
    assert_eq!(hhmm(&z, from), "01:00");
    let first = ev("f", "First hour", from + 30 * MIN, from + 60 * MIN);
    assert_eq!(today(&[first], &on(), &z, from + 5 * MIN).len(), 1);
    // an event the evening before is not in it
    let before = ev("b", "Evening", from - 90 * MIN, from - 30 * MIN);
    assert!(today(&[before], &on(), &z, from + 5 * MIN).is_empty());
}
