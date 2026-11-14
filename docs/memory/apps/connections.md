---
type: memory
description: "The Connections page and the Calendar connection in the Rust app: read-only EventKit behind a trait with a fake, settings, refresh and reminder timing, the home meeting card, the meeting_soon event, join links, voice phrases, stick tools, and what stays out of logs and diagnostics."
---
# Connections

**Domain**: apps

## Overview

A connection is a place StickyBot reads from to know about the person's day. Today there is one, the Mac's Calendar, read through EventKit with a single macOS permission and no sign-in or network call. Connections exist only in the Rust app: `desktop/crates/macos-calendar` is the OS port, `desktop/crates/stickybot-server/src/connections/` holds the shape, the rules (`agenda.rs`, pure) and the state and plumbing (`calendar.rs`), and `assets/connections.js` is the dashboard's Connections page. The Python app does not serve the page. See [Mac apps](/apps/mac-apps.md) for the bundle, signing and permissions.

## Requirements

### Requirement: The Calendar sits behind a read-only trait with a fake
`macos_calendar::CalendarSource` offers `access`, `request_access`, `calendars`, `events(from, to)` and `on_change`; nothing in the crate creates, changes or deletes calendar data. `access` never asks; `request_access` asks once when access was never asked (macOS 14+: `requestFullAccessToEventsWithCompletion`, macOS 13: `requestAccessToEntityType`) and waits up to 120 s for the answer. `Access` is `NotAsked`, `Denied`, `Restricted`, `WriteOnly` or `Full`; only `Full` can read. The EventKit implementation compiles on macOS only, makes its event store on first use, swaps in a fresh store once access becomes `Full`, and returns each occurrence of a recurring event as its own `Event` (told apart by `(id, start)`). `FakeCalendar` stands in everywhere else; `FakeCalendar::sample` is the demo day.

The source is chosen in `AppState::new`: the fake with `--fake-stick`, otherwise `macos_calendar::system()`, so the app and `stickybot-headless` share one path and no test or demo can reach the real Calendar. `cargo run -p macos-calendar --example calendar_dump -- --status | --list | --today` checks EventKit by hand on a real Mac; `--help` creates no store.

### Requirement: Connections share one shape and one list
`GET /api/connections` answers `{"connections": [{id, title, status, detail, settings, ...}]}` with `status` one of `off`, `needs_permission`, `connected`, `error`; a connection's own fields (Calendar's `permission`: `""`, `not_asked`, `denied`, `restricted`, `write_only`) sit beside them. Settings live in the config under `connections.<id>`; `PUT /api/config` accepts `connections` (an unknown connection or a non-object is refused in words) and merges it one connection deep, so naming one setting does not reset the others. A config change re-reads the Calendar unless it came through the connection's own route, which reads itself.

### Requirement: Calendar settings and routes
The settings are `on` (false), `calendars` (ids; empty means all), `skip_all_day` (true), `skip_declined` (true) and `alert_min` (5; 0 to 120, 0 means no reminder). One validator serves both `PUT /api/connections/calendar` and `PUT /api/config`; bad input is a 400 in words (`alert_min must be 0-120 minutes`) and nothing is saved. Malformed saved values read as their defaults.

- `POST /api/connections/calendar/connect` sets `on`, asks macOS only when access was never asked (never again after a no), reads, subscribes to changes and adds the voice phrases once; denied, restricted and write-only access read as `needs_permission` with the matching `permission`.
- `POST /api/connections/calendar/disconnect` sets `on` false, forgets the cached events and the reminders already sent, and takes the card off the stick.
- `GET /api/connections/calendar/calendars` lists `{id, title, color, source, selected}`.
- `GET /api/connections/calendar/events?day=today|tomorrow|YYYY-MM-DD` lists that local day's events (`from` / `to` as `HH:MM`, `all_day`, `declined`, `now`, `calendar`, `color`, `where`, `join`). Only today and tomorrow are loaded; another date is a 400 saying so. Not connected is a 409, a failed last read a 502.

#### Scenario: Access was denied
- **GIVEN** a source whose access is `Denied`
- **WHEN** connect is called
- **THEN** the status is `needs_permission`, `permission` is `denied`, and `request_access` is not called again

### Requirement: Events are cached in memory and refreshed on a schedule
While `on`, the events of the window from local midnight today to the end of tomorrow are read at connect, on a settings change, when EventKit reports a change (after a 700 ms settle, since one edit posts several notifications) and every 60 s. Events live in memory only and are replaced on each read; a failed read keeps the error for the status. Every 15 s a tick checks the cache, not EventKit, for a due reminder and for a change of the pinned card, and pushes the home card when the pinned card changed.

### Requirement: What counts is decided by pure functions
`agenda.rs` takes the events, the settings, `now` and a time zone, and does no I/O. Canceled events and events outside the chosen calendars are dropped; all-day and declined events are listed only when their switch is off and are never "next", "now" or reminded about. "Now" is `start <= now < end` (the one that started last when several overlap); "next" is the earliest later start; "today" is every event overlapping the local day, so a multi-day event shows on each day. Times are shown 24 hour in the Mac's zone.

### Requirement: The next meeting is a home card
The card kind is `meeting` (every released firmware from 1.8.0 draws it with a calendar icon), `a` is the title cut to 40 characters, `b` is `in 12 min · 10:30`, `in 2 h · 14:00` or `now · until 11:00`. It joins the home rotation when a meeting is on now, or the next starts today or within 8 h. From 15 min before a start until 5 min after it, the card is pinned ahead of the rotation, below approvals, questions and a Claude session's line; it is not urgent (no `u`). No message or field is added to the protocol.

### Requirement: A meeting_soon event reminds once
The event kind `meeting_soon` (default level `headsup`, icon `bell`) fires once per occurrence when `now` enters `[start - alert_min, start)`, as `Standup in 5 min` with `10:30 · Google Meet` (or the first line of the location) beneath. It never fires for all-day or declined events, with `alert_min` 0, or with the connection off, and quiet hours follow the existing rule. While the stick is not connected the reminder is held back and not marked as sent, so it still goes out if the stick returns inside the window; sent reminders are remembered in memory until the meeting ends. The dashboard's "Send a test alert" uses `POST /api/events/test` with this kind.

### Requirement: Join links open only known meeting services
`join_link` returns the first Google Meet, Zoom, Microsoft Teams or Webex link found in the event's URL, then its location, then its notes, and only over `https` without a port or credentials. Joining opens the link of the meeting on now, else of a later one today (or starting within 15 min, across midnight) in order, the first that has a link; with none, the answer is `No meeting link found.` The link opens through the same runner as quick commands.

### Requirement: Voice answers come from the Calendar, not the model
Quick commands with `"calendar": "next" | "today" | "join"` are answered in `talk.rs` without the brain, with one short line, and the line is spoken. The default phrases ("what's my next meeting", "what's my day like", "join my meeting" and variants) are added to `commands.json` once at the first connect (config flag `calendar_voice_added`) so they show on the Quick commands page and can be edited or removed. The Python app's defaults are not touched. When not connected the line says Calendar isn't connected.

### Requirement: The brain has three calendar tools
The in-app MCP server offers `calendar_next`, `calendar_today` (today or tomorrow, at most 12 lines) and `calendar_join`. They answer short text, work without the stick, and are tool errors when Calendar is not connected. `calendar_next` and `calendar_today` are pre-allowed for the voice session; `calendar_join` is in `MAC_TOOLS` and asks for approval like `open_url`.

### Requirement: Meeting details stay private
No meeting title, attendee, location, note or link appears in a log line at `info` or above: the `meeting_soon` kind is private, so the log keeps the kind and level and the text only at `debug`. The text still reaches the stick, the Events page and `events.jsonl`. `history.jsonl` records voice answers and calendar tool results as `(from your calendar)`, and the MCP log and activity feed do the same. `feedback::Known` holds the cached meetings' titles, attendees, locations and links so `redact` replaces them in diagnostics, which also report `calendar=on|off`.

### Requirement: The Connections page
`assets/connections.js` is served by the Rust app and adds "Connections" to the nav (before Feedback) through `window.stickyAddPage`. The Calendar card shows the status, Connect and Disconnect, the calendars with colour dots, skip all-day, skip declined, the reminder minutes, a Today preview and "Send a test alert"; with denied access it says how to allow it in System Settings → Privacy & Security → Calendars. Saving does not re-render or scroll the page. A muted line says more connections are coming. `rust.js` relabels the `meeting_soon` kind as "Meeting reminder" on the shared Events page, which is not edited; the three calendar phrases show on the shared Quick commands page as a generic row. Pure helpers are tested with `node --test` from `mac-agent/tests/js/`.

## Design Decisions

### EventKit behind a trait in its own crate
**Decision**: `macos-calendar` holds the trait, the plain types, a fake and the EventKit glue; all choosing and wording is pure code in `connections/agenda.rs`.
**Why**: The logic is tested in seconds without the OS, and nothing in a test can raise a permission prompt or read a real calendar.
**Rejected**: EventKit calls inside the server module (untestable, and a test could touch the real Calendar); the Google Calendar API (OAuth, tokens, one provider).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

### The card and the alert reuse existing messages
**Decision**: The meeting card is a `home` card of the existing `meeting` kind and the reminder an `event` of a new kind.
**Why**: Every released firmware already shows both, so nothing needs detecting or updating, and the protocol is unchanged.
**Rejected**: A new `meeting` message (a protocol change and a firmware release for the same pixels); a new card kind such as `cal` (an unknown kind draws as a plain grey dot).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

### Alerts run against the cache, reads run slower
**Decision**: Reminders and the card are checked every 15 s against cached events; EventKit is read every 60 s or on its change notification.
**Why**: A 1 minute lead time must never fall between two checks, and a check that reads nothing is free.
**Rejected**: Reading EventKit at the alert cadence (needless work) and checking once a minute (a tick can miss a short window).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

### Only known meeting hosts are opened
**Decision**: Join opens https links on Meet, Zoom, Teams and Webex hosts only.
**Why**: Event notes are text other people wrote; opening any link in them from a voice command would let a calendar invite make the Mac open an arbitrary page.
**Rejected**: Opening the first URL in the event.
*Introduced by*: 261112-s3br-connections-page-calendar-connection

### Meeting text stays out of files and logs
**Decision**: Titles and the rest are kept in memory, shown to the stick and to the brain on request, and replaced by `(from your calendar)` in history, the MCP log and the activity feed.
**Why**: The app runs on work machines; the only file that may hold a meeting's text is the `events.jsonl` line of a fired reminder.
**Rejected**: Logging results for debugging (a log file would hold the person's day).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

## Sources

`desktop/crates/macos-calendar/`, `desktop/crates/stickybot-server/src/connections/`, `desktop/crates/stickybot-server/assets/connections.js`, `desktop/README.md`. Related: [Mac apps](/apps/mac-apps.md), [Mac link](/link/mac-link.md).
