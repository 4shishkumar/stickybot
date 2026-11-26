---
type: memory
description: "Rust StickyBot.app and the legacy Python app: independent device sessions, scoped APIs/MCP/history/jobs, default routing, host labels, Mac keyboard push-to-talk and reply panel, shared Mac services, source-bound approvals, shared dashboard compatibility, stick volume controls, button gestures, Launcher favorites and stick app shortcuts, permissions and in-app updates."
---
# Mac Apps

**Domain**: apps

## Overview

Two apps can drive the stick. `desktop/` is StickyBot.app, a Rust workspace (Tauri 2 menu bar
app, axum server, CoreBluetooth, Core Audio, sherpa-onnx speech to text) and the app each release
ships; its `macos-calendar` crate reads the Mac's Calendar (see [connections](/apps/connections.md)). `mac-agent/` is the older Python app, installed by `install.sh`, of which the Rust app is
a module-by-module port. Crate layout, environment variables and the real-stick checklist are in
`desktop/README.md`; moving from one app to the other is `desktop/SWITCHOVER.md`.

## Requirements

### Requirement: One dashboard, served by both apps
The dashboard is `mac-agent/stick_agent/dashboard.html` and the vanilla JS modules in
`mac-agent/stick_agent/static/`. The Rust server serves those same files (from disk in debug
builds, embedded in release builds). A dashboard change therefore changes both apps, and pages
MUST cope with an endpoint one app lacks (the API helper resolves with `missing: true`).

### Requirement: Menu and dashboard control the stick speaker
Both Mac apps provide a 0–100% slider and a speaker button in the menu bar and on the dashboard's
Sounds and Stick pages. The native menus use Apple’s speaker-with-waves and slashed-speaker symbols.
The dashboard draws matching states. The tooltip and accessible label say “Mute stick speaker”
when audible and “Unmute stick speaker” when muted. The Rust menu controls the default stick. The
dashboard controls the stick selected in that view. Muting with the button or by sliding to zero preserves the stick's
exact saved volume (0–255). Unmute restores that value, or uses 140 if the saved value is zero.

Sound requests to each stick run one at a time. The Rust menu and dashboard coalesce drag updates
and send the final level on release. If another control changes the volume while the thumb is held
still, release displays the latest hardware report.

### Buttons & gestures

Both apps expose **Settings → Your stick → Buttons & gestures** when the selected Bluetooth
stick reports its four bindings in `@state.buttons`. Save submits the entire table with the
responding peer's `device_address` as `expected_address`. The editor displays the stick's
acknowledged bindings. A disconnected or newly selected peer requires a fresh read.

A physical press can open an installed Mac app by exact name or run one uniquely named saved
quick command that needs no arguments. The app checks the emitting peer's live binding and
keeps execution on that connection. The physical gesture supplies intent for that action.
Agent-initiated tool calls retain the broker's approval checks. [Protocol](../../../PROTOCOL.md#button-bindings)

### Launcher editor and stick app shortcuts

The shared Launcher editor saves favorites on the selected stick. When the layout came from an
identified Bluetooth peer, both apps bind saves and resets to that address, including tiles-only
edits on older firmware. A lost Bluetooth reply requires a fresh read.

Both MCP servers expose `stick_open(app)` for built-in stick tiles. The Rust Assistant profile
includes it, and the local brain derives its tool list from that profile. A voice request uses
its originating connection and turn. Opening a stick tile needs no approval. Success requires
the stick to acknowledge the requested tile. [Protocol](../../../PROTOCOL.md#launcher-favorites-and-app-navigation)

### Requirement: Each app keeps to its own home
The Rust app's settings, token, models and data live in `~/.stickybot-rs/`; the Python app's in
`~/.stick-agent/`. The Rust app only reads the Python app's folder (to import settings) and never
changes it.

### Requirement: Port and name depend on what else is installed
On a Mac with no Python app the Rust app is StickyBot on port 8660 from its first start. Next to
the Python app it runs on port 8670 and waits for any target that Python owns, until the user
switches over; after that it serves 8660 with the Python app's token so hooks, stick tools and
bookmarks keep working.

#### Scenario: Both apps installed, not switched over
- **GIVEN** the Python app is connected to one configured stick
- **WHEN** the Rust app wants that target
- **THEN** it checks Python status on 8660 and waits/retries for that target; other configured sticks can connect independently

### Requirement: Each configured device has an independent runtime
`devices::DeviceManager` retains a root `primary` session and independent child `AppState` sessions.
Legacy `stick` config migrates into the device list without losing preferences. Discovery does not
enroll a target; the user explicitly adds it. CoreBluetooth shares one central and serializes scans,
with routes keyed by peripheral. Each runtime owns its link, capability/host state, recording,
brain/conversation, reply, firmware jobs, face operations, reconnect and cancellation workers.
Home rotation, pins/progress, reply cache and world-send cache are per-device; one device cannot
show or clear another's display state. Pause/remove affects only the selected runtime. Removal
leaves the hardware's settings and Bluetooth bonds intact.

Credentials, installation identity, speech model/worker, Calendar, weather/feed/GitHub/pet
providers, command store, USB discovery and app updates are shared Mac services. Provider polling
and the speech worker start once; child devices observe their shared results through local display
state. Authoritative configuration writes go through the root, with per-device overrides for voice,
wireless mic, brain and related settings. Only one wireless dictation stream owns the Mac audio
device at a time. See [wireless mic](/link/wireless-mic.md).

### Requirement: Targets are explicit and stable for the whole operation
`GET /api/devices` lists configured devices, default ID and local host metadata. `POST /api/devices`
adds `{address,name}`; DELETE `/api/devices/{id}` removes, POST `.../default` selects the persisted
default and POST `.../pause` with `{paused}` pauses/resumes. `/api/devices/{id}/api/...` runs the
existing API against that device. Unknown IDs fail explicitly. Legacy unscoped routes, hooks and
tray actions use the persisted default; selecting a dashboard view does not change it.

MCP tools accept optional `device_id`; omitted IDs use the default, and explicit targets retain
scope in spawned calls. Background notification policy does not override explicit API/MCP targets,
including `primary`. Operations and asynchronous job polling retain their captured device and
connection generation. SSE events carry `device_id`; late results from a different viewed device
do not repaint the active view. Activity rows carry `device_id`; event rows also retain destination
`device_ids`. Activity queries filter by source device and event-history queries by source or
destination. Legacy rows without a source ID belong to `primary`.

### Requirement: Talk from the Mac keyboard
In StickyBot.app, **Settings → Speech & replies → Talk from Mac keyboard** enables a Mac-wide
shortcut. It starts disabled. The suggested chord is Control–Shift–Space. A conflicting replacement
leaves the working chord registered. Hold the chord in any app and speak; release to send. A press
shorter than 0.35 s is a tap and records nothing: a single tap shows a hint (at most three per
session), and a second tap within 0.5 s locks hands-free listening until the next press sends.
Capture is limited to 60 seconds, with a sound and a countdown for the last 10. Esc belongs to StickyBot only while
the panel is up: it cancels a recording or a running request and closes an answer or error (with
the chord held too, never Command–Option–Esc). After an answer, Esc returns to the front app after
15 s even if the panel stays; sleep and screen lock discard it.
A recording whose loudest moment stays below the silence floor is not sent ("Didn't hear anything
from <microphone>"). If the microphone disconnects, a recording of a second or more is still sent.
Talk from the stick uses its own microphone, including with the default hold B binding.

**Microphone:** the system default unless the user picks an input (saved by Core Audio UID as
`mac_voice_input`). A saved microphone that is not connected falls back to the default and Settings
says so. Bluetooth headset microphones carry a quality note. **Test** opens the chosen input for six
seconds and shows its level; nothing is recorded. Start/stop sounds (`mac_voice_sounds`) are on by
default.

The native panel leaves focus in the front app. Its 46 pt StickyBot icon pulses with a teal glow
beside the 13 pt heading during active stages, and live level bars show while listening. Reduce
Motion uses a steady glow. The same panel shows the transcript, thinking, approval waits and the
answer as available. While a request runs, its × cancels it (only a keyboard-started request; a
stick turn is the stick's to cancel). The full reply stays readable until dismissed or the next
question. Errors and hints fade after a few seconds, except a microphone-permission error, which
offers **Open Privacy Settings**. Pressing the chord while a request runs shows the panel instead
of starting another. Read-aloud, when enabled, plays keyboard replies on the Mac.

Key-down pins the default device for the turn. Speech and the configured local or remote brain use
that device's settings. Each device accepts one owner at a time. Competing input reports busy, and
stale stick events cannot take over a subsequent turn. Keyboard answers stay in the Mac panel and
out of the stick's reply history. Ordinary questions work with the stick disconnected. Tools that
require stick approval still require the broker's exact, single-use approval from the pinned stick;
those actions cannot proceed while it is offline. (261122-k4gw-mac-talk-shortcut)

### Requirement: People recognize the Mac without using its name as identity
A persisted `host.json` holds the installation ID and custom short label. ComputerName comes through
the injectable `System` interface, filtered to at most 80 characters with a safe `Mac` fallback.
`GET /api/host` returns host metadata; `PUT /api/host` with `{label}` saves at most 12 printable
ASCII characters. Empty override derives an ASCII label from the full name (Unicode normalization,
filtered/truncated, with `Mac` fallback). The next hello propagates edits without changing pairing.
Firmware retains offline labels and supplies a disambiguated `display` label for duplicate names.
Names never authenticate a peer; [Mac link](/link/mac-link.md) describes the bond/session binding.

Devices shows model, battery, connection and host state, with add/remove/pause/default/view actions,
shared-mode enablement, remote focus confirmation and This Mac short-name editing. Per-device pages
keep settings, faces, sounds, IR and firmware scoped and capability-gated. The Python app remains
single-device and explains unavailable multi-device controls when the endpoints are absent.

### Requirement: Approvals resolve once within their captured destinations
The shared approvals coordinator records an owning device and destination device IDs. Permission
prompts resend and clear on those devices' current connections across reconnects. The first valid
decision resolves once; unrelated devices and stale connection work cannot decide a prompt.
Dashboard reads, answers and busy checks are owner/destination-scoped. Questions retain their
source generation for initial send, resend and cleanup; disconnect/generation changes cancel them.
Voice replies retain the originating device/session even if the view/default changes.

Config `notification_devices` and `approval_devices` accept `null` for default, an array of configured
IDs for explicit destinations, or `[]` for no hardware destinations. Devices edits this policy
through `/api/devices/primary/api/config`, independently of the viewed/default device. Notifications
are scoped to their destination and approvals do not appear on unrelated home cards. Empty or
unresolvable approval lists return immediately. Device removal prunes both routing lists and
refreshes child snapshots. Invalid saved entries are skipped with metadata-only warnings; a
bad entry or excess capacity does not stop healthy sessions from starting.

### Requirement: The dashboard API is local and token-guarded
The server listens on 127.0.0.1 only, checks the `Host` header, and needs the token for changes.
The stick tools (`/mcp`) need the same token as a bearer token. API keys are kept in the login
Keychain, never in config files, logs or API replies.

### Requirement: Nothing asks for a permission before it is needed
Bluetooth is first touched on the first scan or connect; System Audio Recording only for the
wireless mic; Calendar only when the person presses Connect on the Connections page (no event
store exists before that); no Microphone access at all (the stick's mic listens). The speech model
(487 MB) is downloaded on first start into the app's home and is never part of the bundle.

### Requirement: The Calendar permission is declared and signed in
`desktop/crates/stickybot-app/Info.plist` carries `NSCalendarsFullAccessUsageDescription` (asked
on macOS 14 and later) and `NSCalendarsUsageDescription` (macOS 13), both saying the app reads the
calendar to show the next meeting and remind, and never changes it. The bundle is signed with the
hardened runtime and the `com.apple.security.personal-information.calendars` entitlement, given in
`crates/stickybot-app/entitlements.plist` and applied through `bundle.macOS.entitlements` in
`tauri.conf.json`. `desktop/scripts/build-app.sh` (what CI and the release workflow run) reads the
signed app's entitlements with `codesign -d --entitlements` and exits non-zero when the calendars
entitlement is missing. The EventKit code itself is in the `macos-calendar` crate, which
`stickybot-server` builds into `AppState`; the app crate needs only these bundle files. What
Calendar does is in [connections](/apps/connections.md).

### Requirement: StickyBot.app installs its own updates
The update state machine, busy gate, channel to endpoint rule and API live in
`stickybot-server/src/selfupdate.rs` behind an `Installer` trait (no Tauri type in the server
crate; the headless binary has `NoInstaller` and says only the menu bar app updates itself, with
the release page as the way out). `stickybot-app/src/updater.rs` implements it with
`tauri-plugin-updater`: the signature is checked against the public key in `tauri.conf.json`
(`plugins.updater.pubkey`, `requireSignedVersion`) before anything is replaced, https only. It
reports in words when the app is not in a `.app`, runs translocated, or its folder is not
writable (no admin prompt).

`/api/update` reports `state` (`idle | checking | available | downloading | installing |
restarting | error`), `progress {bytes,total}`, `waiting`, `install_error {kind,message}` with
kinds `signature | network | not-writable | no-update | other`, `updated {from,to}`, `auto` and
`install.can_update` / `why` / `download`; the older fields are kept, and each state change is a
hub `update` event (progress at most about 5 a second). `POST /api/update/install` answers 202 and
starts, or 409 with words (an install already running, no app update, a release that is not newer
than this app, no `latest.json`, installer unavailable); `POST /api/update/cancel` drops a waiting
install; `POST /api/update/agent` behaves as install. All need the API token. The config key
`update_auto` (default `false`) lets the check loop install an available update when the gate is
open, no stick button was pressed for 10 min (counted from the later of the last button and app
start), `update_check` is on and that version was not tried in the last 24 h; quiet hours do not
block it.

#### Scenario: A verified update while the stick is busy
- **GIVEN** an install has downloaded and verified while any configured device has a firmware update, voice request, permission prompt/question, wireless-mic hold or face install running
- **WHEN** the state is read
- **THEN** it is `available` with `waiting: "busy — will install when idle: <reason>"`, nothing is replaced, and the app re-checks every 5 s for at most 30 min and continues by itself when idle (paused is not busy)

#### Scenario: Around the restart
- **GIVEN** the gate is open
- **WHEN** the install replaces the app
- **THEN** the stick is notified "Updating StickyBot…" and `pending {from,to,at}` is written to `updates.json`, the busy gate is looked at again, `bye` is sent and the app relaunches
- **AND** on the next start with that version the app emits `updated`, notifies the stick "Updated to vX" once and clears `pending`; a pending version that did not take is reported once as an error

The menu reads "Install update vX and restart…" when the app can install in place (a click
starts it) and "Update available: vX — download…" (release page) otherwise. The dashboard's
`updates.js` shows the banner and Settings card (notes, Install and restart, progress, waiting
with Cancel, errors with Try again and the download page, a one-time "Updated to vX", the
`update_auto` toggle) only when `/api/update` has `state`; without it (the Python app, older Rust
apps) the old card and `/api/update/agent` path behave as before. The stick sees only ordinary
notify messages; the protocol is unchanged.

### Requirement: Tests run without hardware
`cargo test --workspace` and the Python test suite use a fake stick, an in-memory Core Audio, a
fake Claude and temporary homes. Tests against real Core Audio or a real stick are opt-in.

## Design Decisions

### Ad-hoc signing, with permissions re-asked per version
**Decision**: Release builds are signed ad hoc by GitHub Actions (no Apple Developer ID, not notarized).
**Why**: No Apple certificate has to live in CI; the release workflow uses the built-in `GITHUB_TOKEN` and, on tag pushes only, the update-signing secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. [INFERRED] The ad-hoc signature still uses the hardened runtime, so the entitlements an API needs (the calendars one) are signed in and checked by `build-app.sh`.
**Rejected**: A Developer ID signature with notarization, which needs a paid certificate held as a CI secret. [INFERRED] What ad hoc costs: macOS sees each version as a new app, so Bluetooth, System Audio Recording and Calendar are asked again after every update, and the first open needs right-click → Open. A local build can use a stable identity (`STICKYBOT_SIGNING_IDENTITY`) to keep its permissions.
*Introduced by*: predates fab; see `README.md` "Install the Mac app"

### Tauri's updater plugin behind a server-side trait
**Decision**: `tauri-plugin-updater` does the download, signature check and bundle swap inside `stickybot-app`; `stickybot-server/src/selfupdate.rs` owns the state machine, gate, endpoint and API behind `Installer`, with a fake for tests. The minisign update key, not Apple signing, is the trust anchor.
**Why**: The server crate has no Tauri (headless binary, tests with fakes); the plugin already does the dangerous parts and is what the Tauri CLI's artifacts are made for.
**Rejected**: An own updater in the server (own tar extraction, swap and signature code to maintain); calling the plugin from the dashboard's JS (the page is also served over HTTP to browsers and has no Tauri IPC).
*Introduced by*: 261112-xkmx-in-app-updates

### The gate sits between "verified" and "replace"
**Decision**: The update is downloaded and verified at once; waiting for the stick to be free happens only before replacing and relaunching.
**Why**: A download disturbs nothing; the restart is what would cut a talk, a prompt or a firmware update.
**Rejected**: Refusing the click while busy (the person would have to come back and try again).
*Introduced by*: 261112-xkmx-in-app-updates

### No endpoint override in release builds
**Decision**: In release builds the update endpoint comes only from the compiled-in repo, the channel and a validated tag; debug and test builds accept `STICKYBOT_RS_UPDATE_ENDPOINT` only for an `http://127.0.0.1` / `http://localhost` URL.
**Why**: No config file or environment value may point the updater at another feed.
**Rejected**: A configurable feed URL.
*Introduced by*: 261112-xkmx-in-app-updates

### Claude Code is the default brain
**Decision**: Voice requests go to a long-lived headless Claude Code session; approvals reach the stick through Claude Code's hooks; an OpenRouter model is the optional other brain with only the stick tools.
**Why**: Claude Code brings a proven agent loop with tools, permissions and hooks, MCP, sessions and project instructions, and runs on the user's own Claude Code login with no API key.
**Rejected**: Running the tool loop in the app against a model API, which needs an API key and rebuilds tools, permissions and sessions; and a process per request as the default, which adds start-up time to every request.
*Introduced by*: predates fab; see `docs/ARCHITECTURE.md` "Why Claude Code as the brain"

### Share providers while keeping device work local
**Decision**: Each device owns its runtime, display caches and jobs while Mac providers and approval coordination are shared.
**Why**: A disconnected device or changed view must not retarget work; Calendar, credentials and audio resources belong to the Mac.
**Rejected**: One mutable current-device runtime, or duplicating Mac polling/model services per stick.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

### Capture the destination and connection generation
**Decision**: Asynchronous replies, approval destinations, question resend/cleanup and jobs retain their originating scope.
**Why**: A reused device connection or a changed default cannot make a stale response affect another operation.
**Rejected**: Resolving a global selected device again when a callback finishes.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`desktop/README.md`, `desktop/SWITCHOVER.md`, `docs/ARCHITECTURE.md`, `README.md`,
`mac-agent/README.md`, `desktop/crates/stickybot-server/src/{lib,devices,hosts,api,approvals,selfupdate}.rs`,
`desktop/crates/stickybot-server/src/talk.rs`, `desktop/crates/stickybot-app/src/mac_voice.{rs,m}`,
`docs/MULTI_DEVICE.md`. Related: [Mac link](/link/mac-link.md), [wireless mic](/link/wireless-mic.md),
[releases](/delivery/releases.md).
