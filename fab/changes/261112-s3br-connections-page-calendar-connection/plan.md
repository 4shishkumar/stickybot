# Plan: Connections page and the Calendar connection

**Change**: 261112-s3br-connections-page-calendar-connection
**Intake**: `intake.md`

## Requirements

`intake.md` § What Changes holds the exact values (structs, config, texts, routes, link hosts);
the requirements below are the rules review checks. Where they differ, the intake's detail fills
in, the requirement wins.

### Calendar source: the `macos-calendar` crate

#### R1: The Mac's Calendar sits behind a read-only trait with a fake
A crate `desktop/crates/macos-calendar` MUST define `CalendarSource` (access, request_access,
calendars, events(from, to), on_change), the `Access`, `CalendarInfo` and `Event` types, a
`FakeCalendar`, and an EventKit implementation compiled on macOS only. No function in the crate
SHALL create, change or delete calendar data.

- **GIVEN** the workspace's tests
- **WHEN** `cargo test --workspace` runs
- **THEN** no `EKEventStore` is created and no permission prompt can appear
- **AND** every server test uses `FakeCalendar`

- **GIVEN** macOS 14 or later and access not asked yet
- **WHEN** `EventKitCalendar::request_access` runs
- **THEN** it calls `requestFullAccessToEventsWithCompletion` (macOS 13: `requestAccessToEntityType`)
- **AND** returns the resulting `Access`

#### R2: A manual check tool
`cargo run -p macos-calendar --example calendar_dump -- --status | --list | --today` SHALL let a
person check EventKit on a real Mac; `--help` (and no arguments) MUST print usage without creating
an event store.

- **GIVEN** the example
- **WHEN** it runs with `--help`
- **THEN** it prints usage and exits 0 without touching EventKit

### Bundle: permission strings and entitlement

#### R3: The bundle can ask for the Calendar
The bundle's Info.plist MUST carry `NSCalendarsFullAccessUsageDescription` and
`NSCalendarsUsageDescription`; the bundle MUST be signed with the
`com.apple.security.personal-information.calendars` entitlement (through
`bundle.macOS.entitlements` in `tauri.conf.json`), and `desktop/scripts/build-app.sh` MUST fail
when the signed app lacks it.

- **GIVEN** `desktop/scripts/build-app.sh` (also what CI and the release workflow run)
- **WHEN** it builds the app
- **THEN** `codesign -d --entitlements -` on the app lists the calendars entitlement
- **AND** the script exits non-zero if it does not

### Server: connections and the Calendar connection

#### R4: A common connection shape and its list
`GET /api/connections` MUST answer `{"connections": [{id, title, status, detail, settings}]}` with
`status` one of `off`, `needs_permission`, `connected`, `error`; today the list holds `calendar`.

- **GIVEN** a new install
- **WHEN** `GET /api/connections` is called
- **THEN** the calendar connection is `off` with the default settings
  (`on` false, `calendars` [], `skip_all_day` true, `skip_declined` true, `alert_min` 5)

#### R5: Connect, settings, disconnect
`POST /api/connections/calendar/connect` MUST set `on`, ask the source for access when it was not
asked yet, refresh, and answer the connection; denied / restricted / write-only access MUST read
as `needs_permission` with a `permission` field. `PUT /api/connections/calendar` MUST accept any
subset of the settings and refuse bad ones with words (also through `PUT /api/config`).
`POST …/disconnect` MUST set `on` false and forget the cached events and fired alerts.

- **GIVEN** a fake calendar whose access is `Denied`
- **WHEN** connect is called
- **THEN** the status is `needs_permission`, `permission` is `denied`, and `request_access` was not called again
- **GIVEN** `{"alert_min": 500}`
- **WHEN** it is PUT
- **THEN** the answer is a 400 with `alert_min must be 0-120 minutes` and nothing is saved

#### R6: Calendars and the day's events
`GET /api/connections/calendar/calendars` MUST list `{id, title, color, source, selected}`;
`GET /api/connections/calendar/events?day=today|tomorrow|YYYY-MM-DD` MUST list that local day's
events (`from` / `to` as `HH:MM`, `all_day`, `declined`, `now`, `calendar`, `color`, `where`,
`join`). Both answer an error in words when the connection is not connected.

- **GIVEN** a connected fake calendar with three events today
- **WHEN** `…/events?day=today` is called
- **THEN** three events come back sorted by start

#### R7: Next, now and today are chosen by fixed rules
Selection MUST be pure functions of the events, the settings, `now` and the time zone: canceled
events and events outside the chosen calendars are dropped; all-day and declined events are
dropped when their switch is on and are never "next", "now" or alerted; "now" is
`start <= now < end`; "next" is the earliest later start; "today" is every event overlapping the
local day, so a multi-day event shows on each day and each occurrence of a recurring event counts
on its own.

- **GIVEN** two overlapping meetings and one starting when the first ends
- **WHEN** `now` is the first one's end
- **THEN** the first is no longer "now" and the back-to-back one is
- **GIVEN** an event created in another time zone
- **WHEN** today's list is built for the Mac's zone
- **THEN** it shows at its local time on the correct local day

#### R8: Refresh on change and on a timer
While `on`, the cached events MUST be refreshed at connect, on a settings change, when the source
reports a change, and every 60 s; events stay in memory only.

- **GIVEN** a connected fake calendar
- **WHEN** an event is added and `fire_change()` is called
- **THEN** the next read of the agenda includes it

#### R9: The home card
The next meeting MUST join the home card rotation (starts today or within 8 h, or runs now) as
`a` = title, `b` = `in 12 min · 10:30` / `in 2 h · 14:00` / `now · until 11:00`, and MUST be
shown ahead of the rotation from 15 min before the start to 5 min after it, below approvals,
questions and a session's line. Text MUST be stick-safe and within the card's limits. No message
or field is added to the protocol.

- **GIVEN** a meeting starting in 12 min and a fresh last answer
- **WHEN** the card is built
- **THEN** it is the meeting card, not the rotation's
- **GIVEN** an approval waiting
- **WHEN** the card is built
- **THEN** the approval is shown

#### R10: The `meeting_soon` alert
A new event kind `meeting_soon` (default `headsup`, icon `bell`) MUST be listed with the other
kinds and fired once per occurrence when `now` enters `[start - alert_min, start)`; never for
all-day or declined events, never when `alert_min` is 0 or the connection is off; quiet hours
follow the existing rule.

- **GIVEN** a meeting in 4 min, `alert_min` 5
- **WHEN** the check runs twice
- **THEN** one `meeting_soon` event goes to the stick
- **AND** `GET /api/events/kinds` lists `meeting_soon`

#### R11: Join links
`join_link` MUST find a Google Meet, Zoom, Microsoft Teams or Webex **https** link in the event's
URL, then location, then notes, and nothing else; joining opens the link of the meeting running
now, else the next one within 15 min, else the next one today with a link.

- **GIVEN** an event whose notes hold a docs link and a Zoom link
- **WHEN** `join_link` runs
- **THEN** the Zoom link is returned with provider `Zoom`
- **GIVEN** only an `http://` or non-conference link
- **THEN** nothing is returned

#### R12: Voice without the model
Quick commands with `"calendar": "next" | "today" | "join"` MUST be answered in `talk.rs` without
the brain, with one short line; the phrases are added to `commands.json` once at first connect
(`calendar_voice_added`); when not connected the line says so. The Python defaults are untouched.

- **GIVEN** a connected fake calendar and the transcript "what's my next meeting"
- **WHEN** the talk runs
- **THEN** the reply is via `quick`, names the meeting and its time, and the brain is not called
- **GIVEN** "join my meeting" with a Meet link on the running meeting
- **THEN** the runner gets `Open([url])` and the reply says what it opened

#### R13: Tools for the brain
The in-app MCP server MUST offer `calendar_next`, `calendar_today` and `calendar_join`; they
answer short text, work without the stick, and are tool errors when Calendar is not connected;
`calendar_join` follows `open_url`'s approval rule.

- **GIVEN** `tools/list`
- **THEN** the three tools are listed
- **GIVEN** the connection off
- **WHEN** `calendar_next` is called
- **THEN** the result has `isError` and says Calendar isn't connected

#### R14: Meeting details stay private
No meeting title, attendee, location, note or link SHALL appear in a log line at `info` or above
(the `meeting_soon` event's info line carries no text). `feedback::Known` MUST hold the cached
meetings' titles, attendees, locations and links, and `redact` MUST replace them.

- **GIVEN** a cached meeting "Quarterly numbers" and a log line containing it
- **WHEN** the diagnostics are built
- **THEN** the title does not appear in them

### Dashboard

#### R15: The Connections page
The Rust app MUST serve `assets/connections.js` and add a "Connections" page to the nav through
`window.stickyAddPage`. Its Calendar card MUST show the status, Connect / Disconnect, the
calendars (checkboxes with colour dots), skip all-day, skip declined, the reminder minutes
(the dashboard's dropdown), a Today preview and "Send a test alert"; when access was denied it
MUST say how to allow it in System Settings → Privacy & Security → Calendars. Saving MUST NOT
re-render or scroll the page. A muted "More connections are coming." line follows the card. Pure
helpers MUST be testable with `node --test`.

- **GIVEN** the dashboard in the Rust app
- **WHEN** the nav renders
- **THEN** "Connections" is there and opens the Calendar card
- **GIVEN** the Python app
- **THEN** nothing changes (it does not serve the asset)

### Docs

#### R16: People can read what changed
`CHANGELOG.md` `[Unreleased]` MUST gain a user-facing line; `README.md` and `desktop/README.md`
MUST say what the Calendar permission is for, what is read, that nothing is written or leaves the
Mac, and how to check it by hand; any added third-party crate MUST be recorded where the repo
records them.

- **GIVEN** the changelog
- **THEN** `[Unreleased]` → Added mentions Connections and the Calendar

### Non-Goals

- Firmware, `PROTOCOL.md`, the Python app's behaviour: unchanged.
- Writing to calendars; reminders (EKReminder); other calendar providers' APIs.
- GitHub and Claude Code as connections (later; only the muted line now).

### Design Decisions

#### EventKit behind a trait in its own crate
**Decision**: `macos-calendar` holds the trait, the types, a fake and the EventKit glue; all
choosing and wording is pure code in `stickybot-server/src/connections/agenda.rs`.
**Why**: the logic is tested in seconds without the OS (constitution V), and nothing in tests can
raise a permission prompt.
**Rejected**: EventKit calls inside the server module (untestable, and a test could touch the
real Calendar); the Google Calendar API (OAuth, tokens, one provider).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

#### The card and the alert reuse existing messages
**Decision**: the meeting card is a `home` card and the reminder an `event` of a new kind.
**Why**: every released firmware already shows both; nothing to detect, nothing to update.
**Rejected**: a new `meeting` message (a protocol change and a firmware release for the same pixels).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

#### The calendars entitlement is signed in
**Decision**: `entitlements.plist` with `com.apple.security.personal-information.calendars`,
applied by Tauri's signing step; `build-app.sh` checks it.
**Why**: the bundle is signed ad hoc with the hardened runtime, under which EventKit is refused
without that entitlement.
**Rejected**: turning the hardened runtime off (weaker app for one permission).
*Introduced by*: 261112-s3br-connections-page-calendar-connection

#### Only known meeting hosts are opened
**Decision**: join opens https links on Meet / Zoom / Teams / Webex hosts only.
**Why**: event notes are text other people wrote; opening any link in them from a voice command
would be a way to make the Mac open an arbitrary page.
**Rejected**: "first URL in the event".
*Introduced by*: 261112-s3br-connections-page-calendar-connection

## Tasks

### Phase 1: Setup

- [x] T001 Create `desktop/crates/macos-calendar` (Cargo.toml with `objc2-event-kit` matching the workspace's objc2 family, macOS-only deps under `[target.'cfg(target_os = "macos")'.dependencies]`; add to the workspace in `desktop/Cargo.toml`): `src/lib.rs` with `Access`, `CalendarInfo`, `Event`, `CalendarSource`, module docs <!-- R1 -->
- [x] T002 `desktop/crates/macos-calendar/src/fake.rs`: `FakeCalendar` (settable access / calendars / events, request counter, `fire_change`, `sample(now)`) with unit tests <!-- R1 -->
- [x] T003 Config: `connections.calendar` defaults and `calendar_voice_added` in `desktop/crates/stickybot-server/src/config.rs` `defaults()`; validation hooked into `config::invalid` <!-- R5 -->

### Phase 2: Core Implementation

- [x] T004 `desktop/crates/macos-calendar/src/eventkit.rs`: `EventKitCalendar` (status, request on 14+ / 13, calendars with colours, events in a range incl. the current user's declined status and attendees, change notification); compile-checked only, never run <!-- R1 --> <!-- rework: store made before the grant must be recreated/reset; autorelease pools; observer note -->
- [x] T005 `desktop/crates/macos-calendar/examples/calendar_dump.rs` (`--status`, `--list`, `--today`, `--help`); run only `--help` <!-- R2 -->
- [x] T006 `desktop/crates/stickybot-server/src/connections/agenda.rs` + `agenda_tests.rs`: settings struct + parsing/validation, filtering, now / next / today, card text + priority window, alert decision, spoken lines (next, today), join-link detection and join choice; tests for time zones, all-day, declined, canceled, overlapping, back-to-back, multi-day, recurring occurrences, card text, links <!-- R7 --> <!-- rework: join_choice: next within 15 min across midnight; DST-changing zone test for day bounds -->
- [x] T007 `desktop/crates/stickybot-server/src/connections/calendar.rs`: the connection state on `AppState` (source, cache, error, fired set), status / detail, refresh (blocking calls in `spawn_blocking`), change subscription, the 60 s loop with the alert check and card push; `connections/mod.rs`: the common shape; `lib.rs` wiring (fake source with `--fake-stick` / tests, real EventKit otherwise on macOS) <!-- R8 --> <!-- rework: subscribe/on_change off the async runtime; alert check often enough for a 1-min reminder; one refresh per save -->
- [x] T008 API in `connections/mod.rs` + `calendar.rs`, merged in `api.rs`: list, connect, settings, disconnect, calendars, events; tests in `connections/calendar_tests.rs` (incl. denied permission, bad settings, not connected) <!-- R4 R5 R6 -->
- [x] T009 Home card: meeting candidate + priority in `desktop/crates/stickybot-server/src/home.rs` (`CardInput` gains the meeting card); check the firmware's card code for unknown `k` and choose `cal` or an existing kind; tests <!-- R9 -->
- [x] T010 Alert: `meeting_soon` in `events::KINDS` (+ `event_sounds` default none), info log line without text for it, fired from the calendar loop once per occurrence; tests (once, not for all-day / declined / 0 / off, listed in kinds) <!-- R10 R14 -->
- [x] T011 Voice: `calendar` action in `talk.rs` (like `ir`), phrases + one-time add to `commands.json` (`calendar_voice_added`) in `calendar.rs`; join through the `commands::Runner`; tests (next, today, join, not connected) <!-- R11 R12 --> <!-- rework: answer_today: remove the unused tomorrow branch (zero call sites, duplicates day_lines) -->
- [x] T012 MCP: `calendar_next`, `calendar_today`, `calendar_join` in `desktop/crates/stickybot-server/src/mcp.rs` (definitions, approval list, handlers, the OpenRouter tool list if shared); tests <!-- R13 -->
- [x] T013 Privacy: `Known.meetings` + `[meeting]` in `feedback.rs` `redact`, `calendar` in the features line; tests in `feedback_tests.rs` <!-- R14 --> <!-- rework: redact meeting strings of 3 chars (whole-word for short ones) -->

### Phase 3: Integration & Edge Cases

- [x] T014 Bundle: usage strings in `desktop/crates/stickybot-app/Info.plist`, `desktop/crates/stickybot-app/entitlements.plist`, `bundle.macOS.entitlements` in `tauri.conf.json`, the entitlement check in `desktop/scripts/build-app.sh` (shellcheck-clean); pass the EventKit source from `stickybot-app`; if `cargo tauri build` is practical, build once (never launch) and confirm the entitlement in the signature <!-- R3 --> <!-- rework: build-app.sh entitlement check without a pipe under pipefail -->
- [x] T015 Dashboard: `desktop/crates/stickybot-server/assets/connections.js` (+ an import-free helpers module), registered in `assets/rust.js`, served in `api.rs` `static-rs`; a `node --test` suite for the helpers under `mac-agent/tests/js/`; API test that the assets are served <!-- R15 --> <!-- rework: hide the detail line when the permission box shows -->
- [x] T016 Screenshots of the Connections page (off, permission denied, connected with sample meetings; light and dark if the dashboard has both) with headless Chromium against the headless server on a free port with `--fake-stick` and a temporary home, or a small fake server; saved to the session scratchpad `connections-page/`; every browser and server started is stopped <!-- R15 -->

### Phase 4: Polish

- [x] T017 Docs: `CHANGELOG.md` `[Unreleased]`, `README.md`, `desktop/README.md` (permission, what is read, the real-Mac checklist, `calendar_dump`), `docs/ARCHITECTURE.md` if it lists crates / permissions, `THIRD_PARTY_NOTICES.md` if it lists Rust crates <!-- R16 -->
- [x] T018 Full run: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check` on touched crates if the repo keeps it clean (in `desktop/`); `python -m pytest` and `python -m ruff check .` (in `mac-agent/`); report the counts <!-- R16 -->

## Execution Order

- T001 → T002 → T006 / T007; T004 and T005 need T001 only.
- T007 blocks T008–T012; T006 blocks T007, T009–T012.
- T015 needs T008; T016 needs T015.

## Acceptance

### Functional Completeness

- [x] A-001 R1: `macos-calendar` exists with the trait, types, `FakeCalendar` and a macOS-only EventKit implementation; no write path to calendars exists anywhere in the diff
- [x] A-002 R2: `calendar_dump` has `--status`, `--list`, `--today`, and `--help` prints usage without creating an event store
- [x] A-003 R3: Info.plist has both usage strings; `entitlements.plist` + `bundle.macOS.entitlements` are set; `build-app.sh` fails without the entitlement
- [x] A-004 R4: `GET /api/connections` answers the common shape with the four statuses
- [x] A-005 R5: connect / settings / disconnect work and validate as specified, including through `PUT /api/config`
- [x] A-006 R6: the calendars and events routes answer the documented fields
- [x] A-007 R7: selection is pure (takes `now` and a zone) and implements every rule
- [x] A-008 R8: refresh happens at connect, on settings change, on the source's change callback and on the 60 s timer; events are not written to disk
- [x] A-009 R9: the meeting card joins the rotation and is prioritised in the 15 min / 5 min window below approvals, questions and session lines
- [x] A-010 R10: `meeting_soon` is a listed kind (headsup, bell) and fires once per occurrence
- [x] A-011 R11: join links are limited to https Meet / Zoom / Teams / Webex hosts in the URL → location → notes order
- [x] A-012 R12: the three voice intents answer without the brain; phrases are added once; the Python defaults file is unchanged
- [x] A-013 R13: the three MCP tools exist, error when not connected, and `calendar_join` follows `open_url`'s approval rule
- [x] A-014 R14: no info-level log line carries meeting text; `redact` removes cached meeting titles, attendees, locations and links
- [x] A-015 R15: the Connections page is served, in the nav, and has every control listed
- [x] A-016 R16: changelog, README, desktop/README (and notices if applicable) are updated

### Scenario Coverage

- [x] A-017 R7: tests cover time zones, all-day, declined, canceled, overlapping, back-to-back, multi-day and recurring occurrences
- [x] A-018 R9: tests cover the card texts (`in N min · HH:MM`, hours, `now · until`), the title cut, stick-safe text and the priority window
- [x] A-019 R10: a test shows the alert firing exactly once and not for all-day / declined / `alert_min` 0 / off
- [x] A-020 R11: tests cover each provider, each field, refusal of other and http links
- [x] A-021 R5: API tests cover denied permission, bad settings (words in the error), disconnect clearing the cache
- [x] A-022 R12: talk tests cover next, today, join and the not-connected line
- [x] A-023 R14: a feedback test shows a meeting title and link redacted
- [x] A-024 R15: a `node --test` suite covers the page's helpers and runs with `python -m pytest` in `mac-agent/`

### Edge Cases & Error Handling

- [x] A-025 R5: a saved config with a partial or malformed `connections.calendar` reads as defaults without a panic
- [x] A-026 R6: a failing source (fetch error) reads as status `error` with words, and the routes answer an error, not a 500 panic
- [x] A-027 R9: no meeting, or the connection off → the card and rotation behave exactly as before
- [x] A-028 R1: `cargo test --workspace` creates no EventKit object (the real source is only constructed outside tests / `--fake-stick`)

### Code Quality

- [x] A-029 Pattern consistency: new Rust follows the neighbours (`//!` docs saying what and why, traits + fakes, `serde_json::Value` shapes, error words), JS follows `assets/feedback.js`
- [x] A-030 No unnecessary duplication: reuses `stick_text`, `events::notify`, `commands::Runner`, the API's guards and helpers, the dashboard's components (re-review: the unused `tomorrow` branch of `answer_today` is gone; `day_lines` is the one day listing)
- [x] A-031 Logic kept away from the OS: `eventkit.rs` only converts objects; all rules are in pure code with tests
- [x] A-032 Constants have names, units and reasons (window lengths, limits, intervals); errors reach the person as words
- [x] A-033 No real-looking names, emails, meeting titles of real people, or keys in code, tests, fixtures, screenshots or log lines
- [x] A-034 Changelog line is written for the person using StickyBot; versions untouched; `Cargo.lock` changed only by cargo
- [x] A-035 Lints: `cargo clippy --workspace --all-targets -- -D warnings` and `ruff check` are clean; all suites pass (the one port-8660 hooks test excepted when the user's app holds the port)

### Security

- [x] A-036 R11: nothing but an allow-listed https conference link can be opened from event text
- [x] A-037 R4: the new routes sit behind the same token / Host guards as the rest of the API

### Compatibility

- [x] A-038: no message, field or command between the Mac and the stick changed; `PROTOCOL.md` untouched; the card kind chosen is one every released firmware draws
- [x] A-039: the Python app's behaviour and files under `mac-agent/stick_agent/` are unchanged (apart from nothing); its tests pass

### Real Stick Check

- [x] A-040: **N/A**: for the agent — recorded as open for a person: the meeting card and the `meeting_soon` alert on a real stick, and the permission prompt + EventKit + entitlement on a real signed bundle; the PR carries `needs-hardware` and lists the steps

## Notes

- Check items as you review: `- [x]`
- All acceptance items must pass before `/fab-continue` (hydrate)
- If an item is not applicable, mark checked and prefix with **N/A**: `- [x] A-NNN **N/A**: {reason}`

## Deletion Candidates

None — this change adds new functionality without making existing code redundant (re-review: the two earlier candidates, the `tomorrow` branch of `answer_today` and the double read on a settings save, were removed / fixed in rework cycle 1)

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Confident | Pure rules live in `connections/agenda.rs` (server), not in the crate | Keeps the crate a thin OS port like `macos-audio`; rules need `stick_text` and config | S:70 R:90 A:80 D:70 |
| 2 | Confident | "Send a test alert" uses the existing `POST /api/events/test` with kind `meeting_soon` | `KINDS` carries the sample; no new endpoint | S:75 R:95 A:85 D:80 |
| 3 | Confident | Settings validation is one function used by both PUT routes | One rule, two doors | S:70 R:90 A:85 D:80 |
| 4 | Tentative | Card kind `cal` vs an existing kind is decided in T009 from the firmware source | Wire-compatible either way | S:45 R:90 A:60 D:50 |
| 5 | Confident | JS helper tests live in `mac-agent/tests/js/` (where pytest runs `node --test`) and import the asset by relative path | The only JS test runner in the repo | S:65 R:90 A:75 D:65 |

| 6 | Confident | Card kind sent is `"meeting"`, not `"cal"`: released firmware (v1.8.0 on, checked in `ui.cpp` `cardIcon` / `kindColor` at v1.8.0, v2.3.2 and v2.5.0) already draws `meeting` with a calendar icon and the amber tint; an unknown kind such as `cal` would draw as a plain grey dot card (`hello` branch). The card stays non-urgent (`u` unset) | Existing kind gives the right icon on every released stick with no firmware or protocol change; resolves Tentative #4 | S:90 R:90 A:90 D:80 |
| 7 | Confident | `eventkit` source is built inside `stickybot-server` (`macos_calendar::system()` in `AppState::new`, lazily: no store until Calendar is on), not passed from `stickybot-app`; `stickybot-app` needed only Info.plist, entitlements and tauri.conf changes | One code path for the app and `stickybot-headless`; `--fake-stick` and tests pick the fake in the same place | S:75 R:85 A:80 D:70 |
| 8 | Confident | Reminder is held back while the stick is not connected (not marked fired), so it still goes out if the stick returns inside the window | A reminder nobody could see is not a reminder; fired map stays in memory | S:70 R:90 A:75 D:70 |
| 9 | Confident | Meeting text is also kept out of `history.jsonl` (voice answers log `(from your calendar)`) and out of the MCP log / activity feed result for the three calendar tools | Brief: nothing about meetings on disk except the fired `meeting_soon` line; history.jsonl is a file | S:75 R:90 A:80 D:75 |
| 10 | Confident | `calendar_next` / `calendar_today` are in `FEEDBACK_TOOLS` (pre-allowed for the voice session); `calendar_join` is in `MAC_TOOLS` (asks, like `open_url`) | Read-only on a calendar the person connected vs. opening a link | S:70 R:85 A:75 D:65 |
| 11 | Tentative | The shared Python pages (Events labels, Quick commands action list) are not edited: the Events page shows the new kind through a Rust-only relabel in `rust.js`; Quick commands shows the three calendar phrases as a generic row (action select reads "Open a link", empty target) | A-039 keeps `mac-agent/stick_agent/` unchanged | S:60 R:85 A:60 D:50 |
| 12 | Confident | `THIRD_PARTY_NOTICES.md` and `docs/ARCHITECTURE.md` are unchanged: the notices list no Rust crates (objc2-event-kit is MIT/Zlib/Apache like the objc2 crates already used) and ARCHITECTURE lists no crates or per-feature permissions | Task 17 says "if it lists them" | S:75 R:90 A:80 D:80 |
| 13 | Confident | Fetch window is today and tomorrow (start of today to the end of tomorrow, local), i.e. 48 h from local midnight; `--fake-stick` sample day is anchored to process start time and can spill past midnight late in the evening | Keeps "today" / "tomorrow" / next-meeting rules; sample is demo-only | S:70 R:90 A:75 D:70 |

13 assumptions (0 certain, 11 confident, 2 tentative).
