---
type: memory
description: "The Mac apps: StickyBot.app in Rust (Tauri 2, axum, ~/.stickybot-rs, com.stickybot.rs) that ships, the legacy Python app (~/.stick-agent), the dashboard files both serve, ports 8660 and 8670, ad-hoc signing, in-app updates and macOS permissions."
---
# Mac Apps

**Domain**: apps

## Overview

Two apps can drive the stick. `desktop/` is StickyBot.app, a Rust workspace (Tauri 2 menu bar
app, axum server, CoreBluetooth, Core Audio, sherpa-onnx speech to text) and the app each release
ships. `mac-agent/` is the older Python app, installed by `install.sh`, of which the Rust app is
a module-by-module port. Crate layout, environment variables and the real-stick checklist are in
`desktop/README.md`; moving from one app to the other is `desktop/SWITCHOVER.md`.

## Requirements

### Requirement: One dashboard, served by both apps
The dashboard is `mac-agent/stick_agent/dashboard.html` and the vanilla JS modules in
`mac-agent/stick_agent/static/`. The Rust server serves those same files (from disk in debug
builds, embedded in release builds). A dashboard change therefore changes both apps, and pages
MUST cope with an endpoint one app lacks (the API helper resolves with `missing: true`).

### Requirement: Each app keeps to its own home
The Rust app's settings, token, models and data live in `~/.stickybot-rs/`; the Python app's in
`~/.stick-agent/`. The Rust app only reads the Python app's folder (to import settings) and never
changes it.

### Requirement: Port and name depend on what else is installed
On a Mac with no Python app the Rust app is StickyBot on port 8660 from its first start. Next to
the Python app it runs on port 8670 and waits while that app has the stick, until the user
switches over; after that it serves 8660 with the Python app's token so hooks, stick tools and
bookmarks keep working.

#### Scenario: Both apps installed, not switched over
- **GIVEN** the Python app is connected to the stick
- **WHEN** the Rust app wants to connect
- **THEN** it asks the Python app's status on 8660, shows "Waiting: the Python agent has the stick" and tries again every 3 s

### Requirement: The dashboard API is local and token-guarded
The server listens on 127.0.0.1 only, checks the `Host` header, and needs the token for changes.
The stick tools (`/mcp`) need the same token as a bearer token. API keys are kept in the login
Keychain, never in config files, logs or API replies.

### Requirement: Nothing asks for a permission before it is needed
Bluetooth is first touched on the first scan or connect; System Audio Recording only for the
wireless mic; no Microphone access at all (the stick's mic listens). The speech model (487 MB) is
downloaded on first start into the app's home and is never part of the bundle.

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
- **GIVEN** an install has downloaded and verified while a firmware update, a voice request, a permission prompt or stick question, a wireless-mic hold or a face install is running
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
**Why**: No Apple certificate has to live in CI; the release workflow uses the built-in `GITHUB_TOKEN` and, on tag pushes only, the update-signing secrets `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. [INFERRED]
**Rejected**: A Developer ID signature with notarization, which needs a paid certificate held as a CI secret. [INFERRED] What ad hoc costs: macOS sees each version as a new app, so Bluetooth and System Audio Recording are asked again after every update, and the first open needs right-click → Open. A local build can use a stable identity (`STICKYBOT_SIGNING_IDENTITY`) to keep its permissions.
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

## Sources

`desktop/README.md`, `desktop/SWITCHOVER.md`, `docs/ARCHITECTURE.md`, `README.md`,
`mac-agent/README.md`, `desktop/crates/stickybot-server/src/selfupdate.rs`. Related: [Mac link](/link/mac-link.md), [wireless mic](/link/wireless-mic.md),
[releases](/delivery/releases.md).
