//! The Mac's Calendar as StickyBot sees it: calendars and events, read-only.
//!
//! macOS keeps every account the user added (iCloud, Google, Exchange...) in one place that
//! EventKit reads, so there is no sign-in, no token and no network call here: one macOS permission
//! ("Full Access" to Calendars) and that is all.
//!
//! **Read-only by design**: [`CalendarSource`] has no way to create, change or delete anything, and
//! nothing else in this crate does either. A later change that wants a write must add one on
//! purpose and say why.
//!
//! The OS sits behind the trait like `macos-audio`'s Core Audio does: `eventkit::EventKitCalendar`
//! (macOS only) only converts EventKit objects into the plain structs below, and
//! [`fake::FakeCalendar`] stands in for it in every test, so no test can show a permission prompt or
//! read a real calendar. All the choosing (what is "next", what is "today") lives in
//! `stickybot-server`.

pub mod fake;

#[cfg(target_os = "macos")]
pub mod eventkit;

use std::sync::Arc;

/// Whether this app may read the Calendar (EventKit's `EKAuthorizationStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// the user was never asked
    NotAsked,
    Denied,
    /// a profile or Screen Time stops this Mac from allowing it
    Restricted,
    /// macOS 14+: the user allowed adding events only, which is not enough to read them
    WriteOnly,
    /// allowed to read (EventKit's "authorized" before macOS 14)
    Full,
}

impl Access {
    /// The word the API and the dashboard use for what is missing; `""` when nothing is.
    pub fn permission(self) -> &'static str {
        match self {
            Access::NotAsked => "not_asked",
            Access::Denied => "denied",
            Access::Restricted => "restricted",
            Access::WriteOnly => "write_only",
            Access::Full => "",
        }
    }
}

/// One calendar (a list of events) with the account it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarInfo {
    pub id: String,
    pub title: String,
    /// `#rrggbb`
    pub color: String,
    /// the account's name ("iCloud", "Google"...)
    pub source: String,
}

/// One event, or one *occurrence* of a recurring one: EventKit expands a recurring meeting into one
/// event per occurrence, all with the same `id`, so an occurrence is told apart by `(id, start)`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    /// unix seconds
    pub start: i64,
    /// unix seconds; an all-day event ends at the end of its last day
    pub end: i64,
    pub all_day: bool,
    /// the current user's own answer is "declined"
    pub declined: bool,
    pub canceled: bool,
    pub location: String,
    pub notes: String,
    /// the event's own URL field (many invites put the conference link there)
    pub url: String,
    /// display names
    pub attendees: Vec<String>,
}

/// Where calendars and events come from: EventKit on a Mac, [`fake::FakeCalendar`] in tests and in
/// the demo. Reading only.
pub trait CalendarSource: Send + Sync {
    /// The current permission. Never asks the user.
    fn access(&self) -> Access;
    /// Asks macOS for permission if it was never asked (the prompt shows) and waits for the
    /// answer; returns the resulting access. Blocks, so call it off the async runtime.
    fn request_access(&self) -> Result<Access, String>;
    /// The calendars the user can see.
    fn calendars(&self) -> Result<Vec<CalendarInfo>, String>;
    /// Events overlapping `from..to` (unix seconds) in every calendar, recurring ones expanded.
    fn events(&self, from: i64, to: i64) -> Result<Vec<Event>, String>;
    /// Calls `f` whenever the calendar database changes (an event added, moved, answered...).
    /// May be called from any thread; it only says "look again", not what changed.
    fn on_change(&self, f: Box<dyn Fn() + Send + Sync>);
}

/// The real source: EventKit on a Mac. Nothing is created or asked until it is used. Elsewhere
/// there is no Calendar to read, which it says in words.
pub fn system() -> Arc<dyn CalendarSource> {
    #[cfg(target_os = "macos")]
    {
        Arc::new(eventkit::EventKitCalendar::new())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Arc::new(fake::FakeCalendar::unavailable())
    }
}
