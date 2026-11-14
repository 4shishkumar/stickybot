# Intake: Connections page and the Calendar connection

**Change**: 261112-s3br-connections-page-calendar-connection
**Created**: 2026-11-12

## Origin

One-shot brief from the owner (no back-and-forth; where the brief left a choice open it is decided
below and recorded in `## Assumptions`):

> Build the first "Connection" for StickyBot — Calendar from the Mac's own Calendar (EventKit).
> **Connections page + Calendar connection (Mac app only; NO firmware change).** Source: macOS
> Calendar through EventKit (objc2 / objc2-event-kit in the Rust app, `desktop/`; the macOS glue in
> its own small crate or module with a trait so the server logic is testable with a fake calendar).
> macOS 14+: `requestFullAccessToEvents`; add `NSCalendarsFullAccessUsageDescription` (and the
> pre-14 key) to the bundle's Info.plist; the release build is ad-hoc signed with the hardened
> runtime flag, so add the `com.apple.security.personal-information.calendars` entitlement if
> EventKit needs it (make `scripts/build-app.sh` and CI builds apply it). Never write to calendars.
> Server: a `connections` module with a small common shape and the Calendar connection (config
> `connections.calendar`), refresh on EventKit's change notification plus a timer, "next meeting"
> and "today"; a home card; a `meeting_soon` event; voice quick commands and `calendar_next` /
> `calendar_today` / `calendar_join` tools on the in-app MCP server; an API under
> `/api/connections`; privacy (titles / attendees / links never in info logs, redacted in
> feedback). Dashboard: a new Connections page (Rust-only asset) with a Calendar card. The legacy
> Python app gets nothing new. Docs, tests, screenshots, a small manual check tool.

Hard limits from the brief: do not launch the app, do not touch ports 8660 / 8670, the stick,
`~/.stickybot-rs`, `~/.claude*`, the Keychain or the user's real Calendar; nothing may trigger a
Calendar permission prompt during this work; no secrets, no personal or employer names.

## Why

1. **The pain**: the stick sits on the desk next to the keyboard and already pages its owner for
   approvals, PRs and finished Claude tasks, but it knows nothing about the day. People miss the
   start of a meeting while heads-down, and "what's next?" means opening the Calendar app.
2. **Without it**: the stick stays a developer-only gadget, and every later integration (GitHub
   and Claude Code are to become "connections" too) has no common place in the app or the
   dashboard: each would invent its own settings card, status wording and API shape.
3. **Why this way**: the Mac's own Calendar (EventKit) already holds every account the user added
   to macOS (iCloud, Google, Exchange), so there is no OAuth, no token to store and no network
   call: one macOS permission and read-only access. A small common "connection" shape
   (id, title, status, detail, settings) now means the next ones are a card each. Rejected: the
   Google Calendar API (OAuth client, tokens in the Keychain, one provider only) and AppleScript
   to Calendar.app (slow, launches the app, needs Automation permission).

## What Changes

Mac app (Rust, `desktop/`) and the dashboard asset it serves only. **No firmware change, no new or
changed message between the Mac and the stick** (`PROTOCOL.md` stays as it is): the card goes out
as the existing `home` card and the alert as the existing `event`.

### 1. A small crate for the Mac's Calendar: `desktop/crates/macos-calendar`

Like `macos-audio`: the OS behind a trait, a fake for every test.

- `lib.rs` (`//!` doc: what it is, read-only by design):
  - `enum Access { NotAsked, Denied, Restricted, WriteOnly, Full }` (EventKit's
    `EKAuthorizationStatus`; pre-14 `Authorized` maps to `Full`).
  - `struct CalendarInfo { id, title, color: String /* "#rrggbb" */, source: String /* account */ }`
  - `struct Event { id, calendar_id, title, start: i64, end: i64 /* unix seconds */, all_day: bool,
    declined: bool /* the current user's participant status is Declined */, canceled: bool,
    location: String, notes: String, url: String, attendees: Vec<String> /* display names */ }`.
    A recurring meeting arrives as one `Event` per occurrence (EventKit expands them in
    `eventsMatchingPredicate`), all with the same `id`: an occurrence is keyed by `(id, start)`.
  - `trait CalendarSource: Send + Sync { fn access(&self) -> Access; fn request_access(&self) ->
    Result<Access, String> /* blocks until the user answers */; fn calendars(&self) ->
    Result<Vec<CalendarInfo>, String>; fn events(&self, from: i64, to: i64) ->
    Result<Vec<Event>, String>; fn on_change(&self, f: Box<dyn Fn() + Send + Sync>); }`
  - **No write method exists on the trait or anywhere in the crate** (read-only is structural).
- `fake.rs`: `FakeCalendar` (in memory: settable access, calendars, events, a counter of
  `request_access` calls, `fire_change()`), plus `FakeCalendar::sample(now)` with a plausible day
  (generic titles such as "Standup", "Design review", "1:1"; no real names) for
  `--fake-stick` and the screenshots.
- `eventkit.rs` (`#[cfg(target_os = "macos")]`): `EventKitCalendar` over `objc2-event-kit`
  (`EKEventStore`): `authorizationStatusForEntityType(.event)`; on macOS 14+
  `requestFullAccessToEventsWithCompletion`, before that `requestAccessToEntityType:completion:`
  (the app's minimum is macOS 13); `calendarsForEntityType`, `predicateForEventsWithStartDate:
  endDate:calendars:` + `eventsMatchingPredicate`; observes `EKEventStoreChangedNotification`.
  Thin: it only converts objects to the structs above. Versions must match the workspace's
  existing `objc2` family.
- `examples/calendar_dump.rs`: `cargo run -p macos-calendar --example calendar_dump -- --status`
  (no prompt), `--list` (calendars), `--today` (asks for permission if needed, prints today's
  events). For the owner to check EventKit on a real Mac later. **Never run during this change**
  except `--help` (it must not create an event store for `--help`).
- Licence: `objc2-event-kit` (and anything else added) recorded in `THIRD_PARTY_NOTICES.md` if
  that file lists Rust crates of this kind.

### 2. Bundle: usage strings and the entitlement

- `desktop/crates/stickybot-app/Info.plist` (merged into the bundle by `cargo tauri build`): add
  `NSCalendarsFullAccessUsageDescription` (macOS 14+) and `NSCalendarsUsageDescription` (macOS 13),
  both: "StickyBot reads your calendar to show your next meeting on the stick and remind you
  before it starts. It never changes your calendar."
- **Entitlement finding** (checked on the installed build: `codesign -dv` shows
  `flags=0x10002(adhoc,runtime)` and no entitlements): the bundle is signed ad hoc **with the
  hardened runtime**. Under the hardened runtime, Apple's "Calendars Entitlement"
  (`com.apple.security.personal-information.calendars`) is required for EventKit; without it the
  request is refused without a prompt. Add `desktop/crates/stickybot-app/entitlements.plist` with
  that one key and point `bundle.macOS.entitlements` in `tauri.conf.json` at it. `scripts/
  build-app.sh` and both workflows build through `cargo tauri build` with that config, so they
  apply it; `build-app.sh` additionally prints / checks the entitlement after signing
  (`codesign -d --entitlements -`) so a build without it fails loudly. Bluetooth and the audio tap
  need no hardened-runtime entitlement (they work today without one), so nothing else is added.
  If a local `cargo tauri build` is practical during apply, build once and verify the entitlement
  is in the signature (do not launch the app).

### 3. Server: `desktop/crates/stickybot-server/src/connections/`

- `mod.rs`: the common shape and the list.
  ```json
  {"id": "calendar", "title": "Calendar", "status": "off|needs_permission|connected|error",
   "detail": "3 calendars · next at 10:30", "settings": { … }}
  ```
  `GET /api/connections` → `{"connections": [ … ]}` (today: one). Routes merged into the API
  router like `feedback::routes()` (same token / Host guards as the rest of the API).
- `agenda.rs` (pure, no I/O, no clock: every function takes `now` and a time zone / offset):
  filtering, next / today selection, the card, the alert decision, spoken answers, join links.
- `calendar.rs`: the connection's state (`AppState.calendar`): the `Arc<dyn CalendarSource>`, the
  cached events, the last error, the fired-alert set; refresh; the API handlers; voice; tools.

**Config** `connections.calendar` in `config.json` (defaults in `config::defaults()`):
```json
"connections": {"calendar": {"on": false, "calendars": [], "skip_all_day": true,
                             "skip_declined": true, "alert_min": 5}}
```
`calendars`: ids, empty = all. `alert_min`: 0 = no alert, at most 120. Validation (used by
`PUT /api/connections/calendar` and by `config::invalid` for `PUT /api/config`): `on`,
`skip_all_day`, `skip_declined` booleans; `calendars` a list of at most 100 strings of at most
200 characters; `alert_min` an integer 0–120; unknown keys refused with words
("alert_min must be 0-120 minutes"). A saved file with missing keys reads as the defaults.

**Status**: `off` (`on` false) · `needs_permission` (`on` true, access not `Full`; `detail` and a
`permission` field say which: `not_asked`, `denied`, `restricted`, `write_only`) · `connected`
(`on` true, `Full`) · `error` (the last fetch failed; `detail` has the words).

**Refresh**: on connect, on a settings change, on EventKit's change notification (`on_change`),
and on a timer (every 60 s; it also drives the alert check and pushes the card when it changed).
The fetch window is from the start of today to 48 h ahead (local time), kept in memory only
(nothing about meetings is written to disk except the fired `meeting_soon` line in
`events.jsonl`, like every event). EventKit calls run in `spawn_blocking`.

**Selection rules** (`agenda.rs`):
- Dropped always: canceled events; events not in the chosen calendars (when the list isn't empty).
- Dropped when `skip_all_day` / `skip_declined` (defaults true): all-day events / events the user
  declined. All-day and declined events never alert and are never the "next meeting", whatever
  the two switches say (with a switch off they only show in "today").
- **Now**: timed events with `start <= now < end`. **Next**: the earliest timed event with
  `start > now` (ties: the shorter one, then the title). Overlapping and back-to-back meetings:
  while one runs, "next" is still the next start; an event ending exactly when another starts
  is not "now" any more at that instant.
- **Today**: events that overlap the local day `[00:00, 24:00)` in the Mac's time zone, sorted by
  start then title; a multi-day event shows on each day it covers; times are shown in local time
  (`HH:MM`, 24 h), so an event created in another zone shows at its local instant.

**Home card** (joins `home::card`'s rotation; text made stick-safe by `home_card`, 60 / 140):
- A candidate in the rotation when the next meeting starts today or within 8 h, or one runs now:
  `{"k": "cal", "a": "Standup", "b": "in 12 min · 10:30"}`; from 90 min up `"in 2 h · 14:00"`;
  running: `"now · until 11:00"`. The title is cut to 40 characters with `…`.
- **Priority**: from 15 min before the start until 5 min after it, the card is pinned ahead of
  the rotation (after approvals, questions and a session's pinned / progress line, which stay on
  top). Not urgent (`u` unset): orange stays for things that wait for a button.
- `k`: the apply stage checks the firmware's card drawing for what an unknown `k` does. If every
  released firmware draws an unknown kind like a plain card, send `"cal"`; otherwise send an
  existing kind (`"hello"`). Either way no firmware change and no protocol change.
  <!-- assumed: card kind "cal" vs an existing kind — depends on how released firmware draws an unknown kind; checked at apply -->

**Alert**: a new event kind in `events::KINDS`:
`("meeting_soon", "headsup", "bell", "Standup in 5 min", "10:30 - Google Meet")`. It appears on the
Events page with its level and sound like the others (the page builds its list from
`/api/events/kinds`), and "Send a test alert" on the Connections page uses the existing
`POST /api/events/test {"kind": "meeting_soon"}`. Fired once per occurrence (`(id, start)`, kept in
memory, pruned after the meeting ends) when `now` is within `[start - alert_min, start)`; title
`"<title> in N min"` (or `"<title> starts now"` under a minute), sub `"HH:MM · <where>"` where
`<where>` is the conference provider ("Google Meet"), else the location, else nothing. Quiet
hours: the existing rule (shown, silent). Not for all-day or declined events; `alert_min: 0` =
never. A meeting created or moved inside the window still alerts once.

**Join links** (`agenda::join_link(event) -> Option<(provider, url)>`): looks at the event's URL,
then its location, then its notes, and takes the first **https** link whose host is a known
conference service: Google Meet (`meet.google.com/…`), Zoom (`*.zoom.us/j/…`, `/my/…`, `/w/…`),
Microsoft Teams (`teams.microsoft.com/l/meetup-join/…`, `teams.microsoft.com/meet/…`,
`teams.live.com/meet/…`), Webex (`*.webex.com/…`). Nothing else is ever opened (a link in the
notes to some other site is not a meeting link). Trailing punctuation, angle brackets and HTML
around the link are tolerated.

**Voice, without the model** (the repo's mechanism: a `commands.json` entry with its own key, like
`{"ir": {…}}` handled in `talk.rs`): entries with `"calendar": "next" | "today" | "join"`:
- Next meeting: "what's my next meeting", "what is my next meeting", "when is my next meeting",
  "next meeting" → `"Standup in 12 min, at 10:30."` / `"Standup is on now, until 11:00."` /
  `"Nothing else today."`
- Today: "what's my day like", "what does my day look like", "agenda", "my agenda",
  "what's on my calendar" → `"3 meetings today. Next: Standup at 10:30, then Design review at
  14:00."` (at most two named; short enough for the stick and for reading aloud).
- Join: "join my meeting", "join the meeting", "join meeting" → opens the link of the meeting
  running now (the one that started last), else of the next one starting within 15 min, else of
  the next one today that has a link, through the existing `commands::Runner`
  (`Action::Open`), and says `"Opened Google Meet for Standup."`; `"No meeting link found."` when
  there is none.
- Added to the user's `commands.json` once, the first time Calendar is connected (config
  `calendar_voice_added`, like `ir_voice_added`), so they show on the Quick commands page and can
  be edited or removed. The Python app's `defaults/commands.json` is **not** touched. When
  Calendar isn't connected a matched phrase answers "Calendar isn't connected. Open Connections
  in the dashboard."

**Tools for the brain** (the in-app MCP server, `mcp.rs`; also in `openai_tools()` if that list
is derived from the same definitions): `calendar_next` (no arguments), `calendar_today`
(`day`: `today` | `tomorrow`, default today; lists times, titles, where; at most 12 lines),
`calendar_join` (opens the link as above; it changes something on the Mac, so it follows
`open_url`'s approval rule). All three answer with a tool error
("Calendar isn't connected…") when the connection is off or lacks permission. They work without
the stick (reading the calendar needs no stick), except that `calendar_join` needs nothing either.

**API** (all under the existing guards; errors as `{"error": "<words>"}`):
| Method, path | Does |
|---|---|
| `GET /api/connections` | the list (above) |
| `POST /api/connections/calendar/connect` | sets `on`, asks macOS for permission if not asked yet (waits for the answer), refreshes, adds the voice phrases once; answers the connection |
| `PUT /api/connections/calendar` | settings (any subset), validated; answers the connection |
| `POST /api/connections/calendar/disconnect` | `on` false, forgets the cached events and fired alerts, takes the card off; answers the connection |
| `GET /api/connections/calendar/calendars` | `{"calendars": [{"id", "title", "color", "source", "selected"}]}` |
| `GET /api/connections/calendar/events?day=today` | `{"day": "2026-11-12", "events": [{"id", "title", "start", "end", "from": "10:30", "to": "11:00", "all_day", "declined", "now", "calendar", "color", "where", "join": "Google Meet" or ""}]}`; `day`: `today`, `tomorrow` or `YYYY-MM-DD` within the fetch window |

With `--fake-stick` (the headless binary, the tests) the source is `FakeCalendar::sample`: no
EventKit object is created, so tests and the fake dashboard never touch the real Calendar.

**Privacy**:
- Titles, attendees, locations, notes and links never appear in a log line at `info` or above.
  `events::notify` logs titles today; for `meeting_soon` its info line is
  `event meeting_soon -> headsup` only (the text at `debug`). Refresh logs counts only.
- `feedback::Known` gains `meetings` (titles, attendee names, locations and links of the cached
  events), replaced by `[meeting]` in `redact`, with a test. The diagnostics' feature list may say
  `calendar` (on / off), nothing more.
- The cached events live in memory only; nothing is sent anywhere except to the stick (card,
  alert) and to the brain when it calls a calendar tool.

### 4. Dashboard: the Connections page (`desktop/crates/stickybot-server/assets/connections.js`)

A Rust-only asset like `assets/feedback.js`: served from `/static-rs/`, added to the nav by
`assets/rust.js` through `window.stickyAddPage` (before Feedback). It follows the dashboard's
design language (the cards, buttons, the custom dropdown style, saving in place with no page
jump or re-render of the whole page).

The Calendar card:
- a status pill (Off / Needs permission / Connected / Error) and the detail line;
- **Connect** (off or needs permission) / **Disconnect** (connected);
- when permission was denied or restricted: "StickyBot isn't allowed to read your calendar. Open
  System Settings → Privacy & Security → Calendars, turn on StickyBot (Full Access), then press
  Connect again." (with a button that opens that settings pane through the app if such an
  endpoint exists; otherwise the words alone);
- "Calendars": a checkbox per calendar with its colour dot and account name; none ticked = all;
- "Skip all-day events", "Skip meetings I declined" (toggles); "Remind me" (a dropdown: Off, 1,
  2, 5, 10, 15 minutes before);
- "Today": the preview list (time, colour dot, title, a "Join" tag when a link was found, "now"
  marked), "Nothing today" when empty;
- "Send a test alert".
Below the card, muted: "More connections are coming." (nothing for GitHub or Claude Code yet).
Pure helpers (status label, event row text, the settings diff) sit in a small import-free module
so `node --test` can cover them.

### 5. Docs

- `CHANGELOG.md` `[Unreleased]` → Added, written for the user.
- `README.md` and `desktop/README.md`: the Calendar permission, what is read and that nothing is
  written or leaves the Mac; the real-Mac checklist gets Calendar steps; `calendar_dump`.
- `docs/ARCHITECTURE.md` only if it lists crates / permissions (add the crate and the permission).
- `PROTOCOL.md`: untouched (no message changes).
- `docs/memory/` at hydrate.

### 6. Tests (all without EventKit, a stick, the network or a real Claude)

`FakeCalendar` → next / today selection (time zones incl. an event created in another zone,
all-day, declined, canceled, overlapping, back-to-back, multi-day, recurring occurrences with the
same id); card text and its priority window; the alert fires once (and not for all-day / declined
/ `alert_min` 0 / off); join-link detection (each provider, URL vs location vs notes, non-meeting
links refused, http refused); the API (list, connect incl. denied permission, settings, bad
settings, disconnect, calendars, events); voice quick commands; the MCP tools; redaction; config
validation; the JS helpers. Run: `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` (in `desktop/`); `python -m pytest` and
`python -m ruff check .` (in `mac-agent/`, which also runs the JS tests).

### 7. Screenshots

The Connections page (off, needs permission / denied, connected with a day of sample meetings)
rendered headless against a fake server, saved outside the repo and referenced in the PR.

## Affected Memory

- `apps/connections`: (new) the connection shape, the Calendar connection (EventKit behind a trait, config, selection rules, card, alert, voice, tools, API, privacy) and the Connections page.
- `apps/mac-apps`: (modify) the new crate, the Calendar permission, and the hardened-runtime entitlement next to the signing notes.

## Impact

- New crate `desktop/crates/macos-calendar` (+ `objc2-event-kit` dependency, `Cargo.lock`).
- `desktop/crates/stickybot-server`: new `src/connections/` (+ tests), `lib.rs` (`AppState`),
  `api.rs` (routes, the asset), `home.rs` (card candidate + priority, the timer), `events.rs`
  (kind, private log line), `talk.rs` + `commands.rs` (the `calendar` action), `mcp.rs` (three
  tools), `config.rs` (defaults, validation), `feedback.rs` (+ tests), `assets/rust.js`,
  `assets/connections.js`.
- `desktop/crates/stickybot-app`: `Info.plist`, `entitlements.plist`, `tauri.conf.json`, wiring
  the real EventKit source; `desktop/scripts/build-app.sh`.
- `mac-agent/`: no behaviour change; possibly one JS test file under `mac-agent/tests/js/` (where
  the dashboard's `node --test` suites live). Its tests stay green.
- Docs: `CHANGELOG.md`, `README.md`, `desktop/README.md`, `THIRD_PARTY_NOTICES.md`.
- Compatibility: nothing new on the wire; older firmware shows the card and the alert like any
  other. `needs-hardware`: the card and alert on a real stick, and EventKit + the permission
  prompt + the entitlement on a real signed bundle, are a person's checks.

## Open Questions

None blocking: the brief answers the product questions; the remaining choices are graded below.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Mac app (Rust) + its dashboard asset only; no firmware, no PROTOCOL.md change; Python app unchanged | Stated in the brief; the card and alert reuse `home` and `event` | S:95 R:85 A:95 D:95 |
| 2 | Certain | EventKit through `objc2-event-kit` in its own crate `macos-calendar` behind a `CalendarSource` trait with a `FakeCalendar`; read-only (no write API exists) | Brief + constitution V + the `macos-audio` precedent | S:95 R:80 A:90 D:90 |
| 3 | Certain | Config `connections.calendar` {on, calendars, skip_all_day (true), skip_declined (true), alert_min (5, 0 = off)}; statuses off / needs_permission / connected / error; the six API routes | Given verbatim in the brief | S:95 R:80 A:90 D:95 |
| 4 | Confident | Add the `com.apple.security.personal-information.calendars` entitlement via `bundle.macOS.entitlements`; build-app.sh verifies it | The installed build is `adhoc,runtime`; Apple lists the Calendars entitlement under hardened-runtime resource access; one-line revert if a real-Mac check disagrees | S:80 R:85 A:75 D:80 |
| 5 | Confident | Card priority window: 15 min before the start to 5 min after; placed after approvals, questions and a session's pinned line; not urgent (no orange) | Brief says "priority within 15 min"; orange means "press a button" in this app; easy to tune | S:70 R:90 A:70 D:65 |
| 6 | Confident | Card joins the rotation when the next meeting is today or within 8 h; texts "in 12 min · 10:30", "in 2 h · 14:00", "now · until 11:00"; 24 h times | Brief gives the first form; the rest follows it; trivially changed | S:70 R:95 A:70 D:65 |
| 7 | Confident | `meeting_soon`: level headsup, icon `bell`, fired once per occurrence (id + start) kept in memory; a restart inside the window may alert once more | Brief gives level / icon / once; persisting fired ids would put meeting ids on disk for little gain | S:80 R:85 A:75 D:70 |
| 8 | Confident | Voice: `commands.json` entries with a `calendar` key (the `ir` precedent), added once at first connect (`calendar_voice_added`); Python defaults untouched | The repo's existing no-model mechanism; brief says the Python app gets nothing new | S:75 R:85 A:80 D:70 |
| 9 | Confident | Join opens only https links on known Meet / Zoom / Teams / Webex hosts, order URL → location → notes; `calendar_join` follows `open_url`'s approval rule | Brief lists the providers and "prefer the conference URL"; never opening arbitrary links from notes is the safe reading | S:80 R:85 A:80 D:75 |
| 10 | Confident | Privacy: `meeting_soon` info log line carries no text; `Known.meetings` → `[meeting]` in feedback redaction; events cached in memory only | Brief + constitution IV | S:85 R:85 A:85 D:80 |
| 11 | Confident | With `--fake-stick` / in tests the source is `FakeCalendar::sample`; no EventKit object is ever created there | Constitution "Do Not Touch What Is Running"; the `FakeSystem` precedent | S:80 R:90 A:85 D:80 |
| 12 | Confident | Connections page is `assets/connections.js` registered from `assets/rust.js` before Feedback; test alert reuses `POST /api/events/test` | Brief names the mechanism; reuse over a new endpoint | S:85 R:90 A:85 D:75 |
| 13 | Tentative | Card kind `"cal"` if unknown kinds draw as a plain card on released firmware, else an existing kind | Needs a look at the firmware's card code during apply; either choice is wire-compatible | S:45 R:90 A:60 D:50 |
| 14 | Confident | Fetch window: start of today to +48 h, refreshed on change notification + every 60 s | Covers "today", "tomorrow" and the next meeting; brief asks for notification + timer | S:70 R:95 A:75 D:70 |

14 assumptions (3 certain, 10 confident, 1 tentative, 0 unresolved).
