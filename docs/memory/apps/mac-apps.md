---
type: memory
description: "The Mac apps: StickyBot.app in Rust (Tauri 2, axum, ~/.stickybot-rs, com.stickybot.rs) that ships, the legacy Python app (~/.stick-agent), the dashboard files both serve, ports 8660 and 8670, ad-hoc signing, macOS permissions and the signed-in Calendar entitlement."
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

### Requirement: Tests run without hardware
`cargo test --workspace` and the Python test suite use a fake stick, an in-memory Core Audio, a
fake Claude and temporary homes. Tests against real Core Audio or a real stick are opt-in.

## Design Decisions

### Ad-hoc signing, with permissions re-asked per version
**Decision**: Release builds are signed ad hoc by GitHub Actions (no Apple Developer ID, not notarized).
**Why**: No Apple certificate has to live in CI; only the built-in `GITHUB_TOKEN` is used. [INFERRED] The ad-hoc signature still uses the hardened runtime, so the entitlements an API needs (the calendars one) are signed in and checked by `build-app.sh`.
**Rejected**: A Developer ID signature with notarization, which needs a paid certificate held as a CI secret. [INFERRED] What ad hoc costs: macOS sees each version as a new app, so Bluetooth, System Audio Recording and Calendar are asked again after every update, and the first open needs right-click → Open. A local build can use a stable identity (`STICKYBOT_SIGNING_IDENTITY`) to keep its permissions.
*Introduced by*: predates fab; see `README.md` "Install the Mac app"

### Claude Code is the default brain
**Decision**: Voice requests go to a long-lived headless Claude Code session; approvals reach the stick through Claude Code's hooks; an OpenRouter model is the optional other brain with only the stick tools.
**Why**: Claude Code brings a proven agent loop with tools, permissions and hooks, MCP, sessions and project instructions, and runs on the user's own Claude Code login with no API key.
**Rejected**: Running the tool loop in the app against a model API, which needs an API key and rebuilds tools, permissions and sessions; and a process per request as the default, which adds start-up time to every request.
*Introduced by*: predates fab; see `docs/ARCHITECTURE.md` "Why Claude Code as the brain"

## Sources

`desktop/README.md`, `desktop/SWITCHOVER.md`, `docs/ARCHITECTURE.md`, `README.md`,
`mac-agent/README.md`. Related: [Mac link](/link/mac-link.md), [wireless mic](/link/wireless-mic.md),
[releases](/delivery/releases.md).
