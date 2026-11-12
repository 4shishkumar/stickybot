//! A calendar in memory, so the server's logic is tested (and the demo shown) without EventKit:
//! no permission prompt, no real events. Clones share their state.

use crate::{Access, CalendarInfo, CalendarSource, Event};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    access: Option<Access>,
    /// what the user answers when asked (while the access is `NotAsked`)
    answer: Option<Access>,
    calendars: Vec<CalendarInfo>,
    events: Vec<Event>,
    requests: usize,
    reads: usize,
    fail: Option<String>,
    listeners: Vec<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone, Default)]
pub struct FakeCalendar {
    s: Arc<Mutex<State>>,
}

impl FakeCalendar {
    /// Full access, no calendars, no events.
    pub fn new() -> Self {
        let f = Self::default();
        f.set_access(Access::Full);
        f
    }

    /// A Mac with no Calendar to read (not macOS): the permission is "restricted".
    pub fn unavailable() -> Self {
        let f = Self::default();
        f.set_access(Access::Restricted);
        f
    }

    /// A plausible day around `now` (unix seconds) for the demo and the screenshots: two calendars,
    /// a stand-up in 12 minutes, a review with a Zoom link in its notes, a declined planning
    /// meeting, an all-day event and a 1:1 later. Generic names only.
    pub fn sample(now: i64) -> Self {
        const MIN: i64 = 60;
        const HOUR: i64 = 60 * MIN;
        let f = Self::new();
        f.set_calendars(vec![
            CalendarInfo {
                id: "work".into(),
                title: "Work".into(),
                color: "#4a7bd0".into(),
                source: "Example account".into(),
            },
            CalendarInfo { id: "home".into(), title: "Home".into(), color: "#3aa76d".into(), source: "iCloud".into() },
        ]);
        let ev = |id: &str, cal: &str, title: &str, start: i64, mins: i64| Event {
            id: id.into(),
            calendar_id: cal.into(),
            title: title.into(),
            start,
            end: start + mins * MIN,
            ..Event::default()
        };
        f.set_events(vec![
            Event {
                url: "https://meet.google.com/abc-defg-hij".into(),
                ..ev("standup", "work", "Standup", now + 12 * MIN, 15)
            },
            Event {
                notes: "Agenda in the doc.\nJoin: https://example.zoom.us/j/123456789".into(),
                attendees: vec!["Teammate A".into(), "Teammate B".into()],
                ..ev("review", "work", "Design review", now + 90 * MIN, 60)
            },
            Event { declined: true, ..ev("planning", "work", "Planning", now + 2 * HOUR, 45) },
            Event { location: "Room 2".into(), ..ev("one-on-one", "work", "1:1 with a teammate", now + 4 * HOUR, 30) },
            Event { all_day: true, ..ev("offsite", "home", "Team offsite", now - 12 * HOUR, 24 * 60) },
        ]);
        f
    }

    pub fn set_access(&self, a: Access) {
        self.s.lock().unwrap().access = Some(a);
    }

    /// What the user will answer to the permission question (default: allow).
    pub fn set_answer(&self, a: Access) {
        self.s.lock().unwrap().answer = Some(a);
    }

    pub fn set_calendars(&self, c: Vec<CalendarInfo>) {
        self.s.lock().unwrap().calendars = c;
    }

    pub fn set_events(&self, e: Vec<Event>) {
        self.s.lock().unwrap().events = e;
    }

    pub fn add_event(&self, e: Event) {
        self.s.lock().unwrap().events.push(e);
    }

    /// Every read fails with this text until `None` (a calendar database that cannot be read).
    pub fn fail_with(&self, why: Option<&str>) {
        self.s.lock().unwrap().fail = why.map(str::to_string);
    }

    /// How many times the events were read (`events` calls).
    pub fn reads(&self) -> usize {
        self.s.lock().unwrap().reads
    }

    /// How many times the permission was asked for.
    pub fn requests(&self) -> usize {
        self.s.lock().unwrap().requests
    }

    /// Tells every listener the calendar changed (what EventKit's change notification does).
    pub fn fire_change(&self) {
        let listeners = self.s.lock().unwrap().listeners.clone();
        for l in listeners {
            l();
        }
    }
}

impl CalendarSource for FakeCalendar {
    fn access(&self) -> Access {
        self.s.lock().unwrap().access.unwrap_or(Access::NotAsked)
    }

    fn request_access(&self) -> Result<Access, String> {
        let mut s = self.s.lock().unwrap();
        s.requests += 1;
        if s.access.unwrap_or(Access::NotAsked) == Access::NotAsked {
            s.access = Some(s.answer.unwrap_or(Access::Full));
        }
        Ok(s.access.unwrap_or(Access::NotAsked))
    }

    fn calendars(&self) -> Result<Vec<CalendarInfo>, String> {
        let s = self.s.lock().unwrap();
        match &s.fail {
            Some(why) => Err(why.clone()),
            None => Ok(s.calendars.clone()),
        }
    }

    fn events(&self, from: i64, to: i64) -> Result<Vec<Event>, String> {
        let mut s = self.s.lock().unwrap();
        s.reads += 1;
        if let Some(why) = &s.fail {
            return Err(why.clone());
        }
        // an event is in the range when it overlaps it; one with no length counts at its start
        Ok(s.events.iter().filter(|e| e.start < to && (e.end > from || e.start >= from)).cloned().collect())
    }

    fn on_change(&self, f: Box<dyn Fn() + Send + Sync>) {
        self.s.lock().unwrap().listeners.push(Arc::from(f));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn asking_once_settles_the_answer() {
        let f = FakeCalendar::default();
        assert_eq!(f.access(), Access::NotAsked);
        f.set_answer(Access::Denied);
        assert_eq!(f.request_access(), Ok(Access::Denied));
        // macOS does not ask again: the answer stands
        f.set_answer(Access::Full);
        assert_eq!(f.request_access(), Ok(Access::Denied));
        assert_eq!((f.access(), f.requests()), (Access::Denied, 2));
        let ok = FakeCalendar::default();
        assert_eq!(ok.request_access(), Ok(Access::Full));
    }

    #[test]
    fn events_in_a_range_overlap_it() {
        let f = FakeCalendar::new();
        let ev = |id: &str, start, end| Event { id: id.into(), start, end, ..Event::default() };
        f.set_events(vec![ev("before", 0, 100), ev("inside", 150, 160), ev("across", 190, 260), ev("after", 300, 400)]);
        let ids: Vec<String> = f.events(100, 250).unwrap().into_iter().map(|e| e.id).collect();
        assert_eq!(ids, ["inside", "across"]);
    }

    #[test]
    fn changes_reach_every_listener_and_failures_are_words() {
        let f = FakeCalendar::new();
        let n = Arc::new(AtomicUsize::new(0));
        for _ in 0..2 {
            let n = n.clone();
            f.on_change(Box::new(move || {
                n.fetch_add(1, Ordering::Relaxed);
            }));
        }
        f.fire_change();
        assert_eq!(n.load(Ordering::Relaxed), 2);
        f.fail_with(Some("the calendar database is locked"));
        assert_eq!(f.events(0, 10).unwrap_err(), "the calendar database is locked");
        assert!(f.calendars().is_err());
        f.fail_with(None);
        assert!(f.events(0, 10).is_ok());
    }

    #[test]
    fn the_sample_day_has_what_the_demo_shows() {
        let now = 1_800_000_000;
        let f = FakeCalendar::sample(now);
        assert_eq!(f.access(), Access::Full);
        assert_eq!(f.calendars().unwrap().len(), 2);
        let events = f.events(now - 86_400, now + 86_400).unwrap();
        let titles: Vec<&str> = events.iter().map(|e| e.title.as_str()).collect();
        assert!(titles.contains(&"Standup") && titles.contains(&"Design review"), "{titles:?}");
        assert!(events.iter().any(|e| e.all_day) && events.iter().any(|e| e.declined));
    }
}
