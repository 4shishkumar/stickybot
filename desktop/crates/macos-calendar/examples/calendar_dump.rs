//! Checks EventKit on a real Mac, by hand. Never run by tests or CI.
//!
//! ```text
//! cargo run -p macos-calendar --example calendar_dump -- --status   # the permission; never asks
//! cargo run -p macos-calendar --example calendar_dump -- --list     # the calendars (needs Full Access)
//! cargo run -p macos-calendar --example calendar_dump -- --today    # asks for permission if needed, then today's events
//! ```
//!
//! `--today` is the only one that can show the macOS permission prompt. When run from a terminal the
//! permission belongs to the terminal app (Terminal, iTerm...), not to StickyBot. It only reads; titles
//! are printed because this is you looking at your own calendar.

#[cfg(target_os = "macos")]
fn main() {
    use macos_calendar::{Access, CalendarSource};

    let arg = std::env::args().nth(1).unwrap_or_default();
    if !matches!(arg.as_str(), "--status" | "--list" | "--today") {
        // usage first and nothing else: no event store is made for --help
        println!("calendar_dump: look at the Mac's Calendar the way StickyBot does (read-only)\n");
        println!("  --status   the Calendar permission (does not ask)");
        println!("  --list     the calendars: id, title, colour, account (needs Full Access)");
        println!("  --today    today's events (asks for permission if it has not been asked yet)");
        println!("  --help     this text");
        return;
    }
    let cal = macos_calendar::eventkit::EventKitCalendar::new();
    match arg.as_str() {
        "--status" => println!("{:?}", cal.access()),
        "--list" => match cal.calendars() {
            Ok(list) => list.iter().for_each(|c| println!("{}  {}  {}  {}", c.id, c.title, c.color, c.source)),
            Err(e) => eprintln!("{e}"),
        },
        _ => today(&cal),
    }

    fn today(cal: &macos_calendar::eventkit::EventKitCalendar) {
        use chrono::{Duration, Local, TimeZone};
        if cal.access() == Access::NotAsked {
            println!("asking macOS for Calendar access: answer the prompt...");
        }
        match cal.request_access() {
            Ok(Access::Full) => {}
            Ok(other) => {
                return eprintln!("no Full Access ({other:?}): System Settings > Privacy & Security > Calendars");
            }
            Err(e) => return eprintln!("{e}"),
        }
        let midnight = Local::now().date_naive().and_hms_opt(0, 0, 0).expect("midnight");
        let Some(from) = Local.from_local_datetime(&midnight).earliest() else {
            return eprintln!("no local midnight today");
        };
        let to = from + Duration::days(1);
        let at = |t: i64| Local.timestamp_opt(t, 0).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
        match cal.events(from.timestamp(), to.timestamp()) {
            Ok(mut events) => {
                events.sort_by_key(|e| (e.start, e.title.clone()));
                for e in &events {
                    let flags = [(e.all_day, "all-day"), (e.declined, "declined"), (e.canceled, "canceled")]
                        .iter()
                        .filter(|(on, _)| *on)
                        .map(|(_, w)| *w)
                        .collect::<Vec<_>>()
                        .join(", ");
                    println!("{}-{}  {}  [{}]", at(e.start), at(e.end), e.title, flags);
                }
                println!("{} event(s) today", events.len());
            }
            Err(e) => eprintln!("{e}"),
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("calendar_dump needs macOS (EventKit)");
}
