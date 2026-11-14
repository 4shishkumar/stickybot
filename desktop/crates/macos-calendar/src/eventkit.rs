//! The Mac's Calendar through EventKit: a thin converter from EventKit objects into the plain
//! structs of [`crate`], and nothing else. It reads; there is no call here that saves, changes or
//! removes anything.
//!
//! **Never run by tests or by the demo** (they use [`crate::fake::FakeCalendar`]): creating an event
//! store and asking for access shows the macOS permission prompt and reads the real calendar. The
//! store is made on first use, so an app with Calendar switched off never creates one.
//!
//! **The store is replaced once the permission is granted.** A store made before the answer (it has to
//! exist to ask) can keep returning no calendars after the grant, so [`request_access`] swaps in a
//! fresh one as soon as the access is `Full`. Observers are not tied to a store (they watch
//! `EKEventStoreChangedNotification` with a nil object, i.e. any store in the process), so the
//! change callback keeps working across the swap. They are removed in `Drop`.
//!
//! Every call that talks to EventKit runs in an autorelease pool: they run on blocking-pool threads,
//! which have none of their own, and EventKit hands back autoreleased objects.
//!
//! macOS 14 replaced `requestAccessToEntityType:` with `requestFullAccessToEventsWithCompletion:`
//! (the app's minimum is 13, so both are used, the new one when the OS has it). The permission needs
//! `NSCalendarsFullAccessUsageDescription` / `NSCalendarsUsageDescription` in Info.plist and, with the
//! hardened runtime, the `com.apple.security.personal-information.calendars` entitlement.

use crate::{Access, CalendarInfo, CalendarSource, Event};
use block2::RcBlock;
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol, ProtocolObject};
use objc2::sel;
use objc2_core_graphics::CGColor;
use objc2_event_kit::{
    EKAuthorizationStatus, EKCalendar, EKEntityType, EKEvent, EKEventStatus, EKEventStore,
    EKEventStoreChangedNotification, EKParticipantStatus,
};
use objc2_foundation::{NSDate, NSError, NSNotification, NSNotificationCenter};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

/// How long [`CalendarSource::request_access`] waits for the person to answer the macOS prompt. A
/// prompt left open longer keeps waiting inside macOS; the answer then arrives as a change
/// notification and the status is read again.
const ANSWER_WAIT: Duration = Duration::from_secs(120);

/// Shown for a calendar whose colour cannot be read (a neutral grey).
const NO_COLOR: &str = "#8a8a8a";

/// At most this many attendee names per event: a company-wide invite has hundreds and the names
/// are only used to keep them out of logs, not to show.
const MAX_ATTENDEES: usize = 50;

/// EventKit objects are reference counted Objective-C objects and are not marked thread safe. Every
/// use below happens behind a [`Mutex`] (one thread at a time), which is what makes moving them
/// between threads sound.
struct Hold<T>(T);
// SAFETY: see the type's doc: access is serialised by the mutexes that own every `Hold`.
unsafe impl<T> Send for Hold<T> {}
// SAFETY: same.
unsafe impl<T> Sync for Hold<T> {}

/// What `addObserverForName` returns: kept so the observer stays registered.
type Observer = Retained<ProtocolObject<dyn NSObjectProtocol>>;

pub struct EventKitCalendar {
    store: Mutex<Option<Hold<Retained<EKEventStore>>>>,
    /// whether the held store was made while access was already `Full` (only changed under the
    /// store's lock): one made earlier is replaced the first time a caller sees `Full`
    made_with_access: AtomicBool,
    /// the notification observers, kept so they stay registered for the life of the app
    observers: Mutex<Vec<Hold<Observer>>>,
}

impl Default for EventKitCalendar {
    fn default() -> Self {
        Self::new()
    }
}

impl EventKitCalendar {
    /// Nothing is created or asked yet.
    pub fn new() -> Self {
        Self { store: Mutex::new(None), made_with_access: AtomicBool::new(false), observers: Mutex::new(Vec::new()) }
    }

    fn store_guard(&self) -> MutexGuard<'_, Option<Hold<Retained<EKEventStore>>>> {
        let mut g = self.store.lock().unwrap_or_else(|e| e.into_inner());
        // A store made before the grant may keep returning nothing, however late the answer came
        // (even after `request_access` stopped waiting): the first caller that sees `Full` gets a new one.
        let full = status() == Access::Full;
        if g.is_none() || (full && !self.made_with_access.load(Ordering::Relaxed)) {
            // SAFETY: `new` on a class with no preconditions
            *g = Some(Hold(unsafe { EKEventStore::new() }));
            self.made_with_access.store(full, Ordering::Relaxed);
        }
        g
    }

    /// Runs `f` with the event store, one caller at a time.
    fn with_store<R>(&self, f: impl FnOnce(&EKEventStore) -> R) -> R {
        let g = self.store_guard();
        let Some(store) = g.as_ref() else { unreachable!("made above") };
        f(&store.0)
    }
}

fn map_status(s: EKAuthorizationStatus) -> Access {
    match s {
        EKAuthorizationStatus::NotDetermined => Access::NotAsked,
        EKAuthorizationStatus::Restricted => Access::Restricted,
        EKAuthorizationStatus::Denied => Access::Denied,
        EKAuthorizationStatus::WriteOnly => Access::WriteOnly,
        // FullAccess, and `Authorized` before macOS 14 (the same number)
        EKAuthorizationStatus::FullAccess => Access::Full,
        _ => Access::Denied,
    }
}

fn status() -> Access {
    // SAFETY: a class method that only reads the permission; it never asks
    map_status(unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) })
}

fn need_full(a: Access) -> Result<(), String> {
    match a {
        Access::Full => Ok(()),
        Access::NotAsked => Err("StickyBot hasn't asked for access to your calendar yet".into()),
        Access::WriteOnly => {
            Err("StickyBot only has permission to add events; it needs Full Access to read them".into())
        }
        Access::Denied | Access::Restricted => Err("StickyBot isn't allowed to read your calendar".into()),
    }
}

fn hex(cal: &EKCalendar) -> String {
    // SAFETY: a getter; the colour is read through CoreGraphics' own accessors
    let Some(c) = (unsafe { cal.CGColor() }) else { return NO_COLOR.into() };
    let n = CGColor::number_of_components(Some(&c));
    let p = CGColor::components(Some(&c));
    if p.is_null() || n == 0 {
        return NO_COLOR.into();
    }
    // SAFETY: CoreGraphics says the pointer holds `n` components
    let v = unsafe { std::slice::from_raw_parts(p, n) };
    let byte = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    match v {
        [r, g, b, ..] => format!("#{:02x}{:02x}{:02x}", byte(*r), byte(*g), byte(*b)),
        [grey, ..] => format!("#{0:02x}{0:02x}{0:02x}", byte(*grey)),
        [] => NO_COLOR.into(),
    }
}

fn info(cal: &EKCalendar) -> CalendarInfo {
    // SAFETY: getters
    unsafe {
        CalendarInfo {
            id: cal.calendarIdentifier().to_string(),
            title: cal.title().to_string(),
            color: hex(cal),
            source: cal.source().map(|s| s.title().to_string()).unwrap_or_default(),
        }
    }
}

fn convert(e: &EKEvent) -> Event {
    // SAFETY: getters on an event the store just returned
    unsafe {
        let attendees = e.attendees();
        let declined = attendees.as_ref().is_some_and(|a| {
            a.iter().any(|p| p.isCurrentUser() && p.participantStatus() == EKParticipantStatus::Declined)
        });
        let names = attendees
            .map(|a| a.iter().take(MAX_ATTENDEES).filter_map(|p| p.name().map(|n| n.to_string())).collect())
            .unwrap_or_default();
        Event {
            id: e.eventIdentifier().unwrap_or_else(|| e.calendarItemIdentifier()).to_string(),
            calendar_id: e.calendar().map(|c| c.calendarIdentifier().to_string()).unwrap_or_default(),
            title: e.title().to_string(),
            start: e.startDate().timeIntervalSince1970() as i64,
            end: e.endDate().timeIntervalSince1970() as i64,
            all_day: e.isAllDay(),
            declined,
            canceled: e.status() == EKEventStatus::Canceled,
            location: e.location().map(|s| s.to_string()).unwrap_or_default(),
            notes: e.notes().map(|s| s.to_string()).unwrap_or_default(),
            url: e.URL().and_then(|u| u.absoluteString()).map(|s| s.to_string()).unwrap_or_default(),
            attendees: names,
        }
    }
}

impl CalendarSource for EventKitCalendar {
    fn access(&self) -> Access {
        status()
    }

    fn request_access(&self) -> Result<Access, String> {
        autoreleasepool(|_| self.ask())
    }

    fn calendars(&self) -> Result<Vec<CalendarInfo>, String> {
        autoreleasepool(|_| self.list_calendars())
    }

    fn events(&self, from: i64, to: i64) -> Result<Vec<Event>, String> {
        autoreleasepool(|_| self.list_events(from, to))
    }

    fn on_change(&self, f: Box<dyn Fn() + Send + Sync>) {
        autoreleasepool(|_| self.observe(f))
    }
}

impl EventKitCalendar {
    fn ask(&self) -> Result<Access, String> {
        if status() != Access::NotAsked {
            return Ok(status()); // macOS asks once; the answer stands (the person changes it in System Settings)
        }
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let done = RcBlock::new(move |_granted: Bool, err: *mut NSError| {
            // the answer is read back from the permission itself; only a failure matters here
            // SAFETY: EventKit passes nil or a valid error for the duration of the call
            let failure = unsafe { err.as_ref() }.map(|e| e.localizedDescription().to_string());
            let _ = tx.send(failure.map_or(Ok(()), Err));
        });
        // the store is used outside its lock: the answer can take minutes and reads must not wait for it
        let store = self.store_guard().as_ref().map(|s| s.0.clone()).ok_or("no calendar store")?;
        // SAFETY: the block pointer stays valid for the call, EventKit copies the block it keeps
        unsafe {
            if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
                store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&done));
            } else {
                #[allow(deprecated)] // macOS 13: the only call it has
                store.requestAccessToEntityType_completion(EKEntityType::Event, RcBlock::as_ptr(&done));
            }
        }
        let answer = rx.recv_timeout(ANSWER_WAIT);
        drop(store);
        // granted: swap the store now (see `store_guard`), so it is there for the change notification
        drop(self.store_guard());
        match answer {
            Ok(Err(why)) if status() == Access::NotAsked => Err(format!("macOS couldn't ask: {why}")),
            Ok(_) => Ok(status()),
            Err(_) => Err("macOS is still waiting for your answer to the Calendar question".into()),
        }
    }

    fn list_calendars(&self) -> Result<Vec<CalendarInfo>, String> {
        need_full(status())?;
        Ok(self.with_store(|s| {
            // SAFETY: a read-only query
            let all = unsafe { s.calendarsForEntityType(EKEntityType::Event) };
            all.iter().map(|c| info(&c)).collect()
        }))
    }

    fn list_events(&self, from: i64, to: i64) -> Result<Vec<Event>, String> {
        need_full(status())?;
        Ok(self.with_store(|s| {
            let (start, end) =
                (NSDate::dateWithTimeIntervalSince1970(from as f64), NSDate::dateWithTimeIntervalSince1970(to as f64));
            // SAFETY: a read-only query with a predicate made by the store itself (nil = all calendars)
            let found = unsafe {
                let predicate = s.predicateForEventsWithStartDate_endDate_calendars(&start, &end, None);
                s.eventsMatchingPredicate(&predicate)
            };
            found.iter().map(|e| convert(&e)).collect()
        }))
    }

    fn observe(&self, f: Box<dyn Fn() + Send + Sync>) {
        // the store has to exist for the notification to be posted
        drop(self.store_guard());
        let block = RcBlock::new(move |_n: NonNull<NSNotification>| f());
        // SAFETY: the name is EventKit's own constant; nil object = any store in this process; nil queue =
        // the thread that posts (the callback only wakes the refresh loop)
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(EKEventStoreChangedNotification),
                None::<&AnyObject>,
                None,
                &block,
            )
        };
        self.observers.lock().unwrap_or_else(|e| e.into_inner()).push(Hold(token));
    }
}

impl Drop for EventKitCalendar {
    /// Takes the observers out of the notification center. The app keeps one source for its whole life,
    /// so this only matters for a source made and dropped (a test, a tool); it keeps a dropped source
    /// from leaving callbacks behind.
    fn drop(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        for Hold(token) in self.observers.get_mut().unwrap_or_else(|e| e.into_inner()).drain(..) {
            // SAFETY: the token is what `addObserverForName` returned for this center
            let object: &AnyObject = (*token).as_ref();
            unsafe { center.removeObserver(object) };
        }
    }
}
