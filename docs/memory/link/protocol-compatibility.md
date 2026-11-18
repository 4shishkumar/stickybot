---
type: memory
description: "Capability-gated compatibility between apps and firmware: legacy slots and Python support, shared-host handshake and identities, board-specific updates, unchanged machine framing and requester-only command replies."
---
# Protocol Compatibility

**Domain**: link

## Overview

The Mac app and the stick firmware update separately, so any pair of versions can meet.
`PROTOCOL.md` is the contract; this file states the rules that keep old and new working together.

## Requirements

### Requirement: Unknown input is ignored
Each side MUST ignore messages and fields it does not know, without an error shown to the user.
Additions go in new fields or new messages; the meaning of an existing field MUST NOT change.

### Requirement: Features are detected, never assumed
The app MUST use a stick feature only when the stick reports it, and the stick MUST use an app
feature only when the app's hello says so.

| Reported by | Signal | Gates |
|---|---|---|
| stick, in `@state` | `"ota":1`, `otaPending` | firmware update over Bluetooth |
| stick, in `@state` | `"pkgFaces":{"n","fmt"}` | `faces` messages and `@faces` |
| stick, in `@state` | `"ir":{…}` | `@ir` and the IR remote |
| stick, in `@state` | `usbMounted` | wireless mic behaviour on USB in legacy mode |
| stick, in `@state` | `multiHost:1`, `maxHosts:3` | host hello extension, shared-mode controls and bounded host list |
| stick, in `@state` | `deviceId`, `board` | physical identity and board-compatible firmware selection |
| app, in the hello | `"wmic":true` | the stick streams the wireless mic |
| app, in the hello | `"wready":true` | the stick sends `prepare` and waits for `ready` |
| app, in the hello | `"speak":true` | the stick offers "read aloud" in the reply view |

#### Scenario: A newer app meets firmware without face packages
- **GIVEN** a stick whose `@state` has no `pkgFaces`
- **WHEN** the user opens the faces section of the dashboard
- **THEN** the app sends no `faces` message or `@faces` to it and says which firmware is needed

#### Scenario: Newer firmware meets an app that never says `wmic`
- **GIVEN** an app whose hello carries no `wmic`
- **WHEN** A is held away from USB
- **THEN** the stick presses the key and streams nothing

### Requirement: Shared mode has an explicit compatibility boundary
New firmware defaults to legacy slots and preserves their identities/bonds. A new Rust app reads
`@state` before sending the host extension and works with older firmware through the single-app
messages. An old app works with new firmware in legacy mode; shared-mode mutations require the
authenticated host/session handshake and otherwise answer `session_required` with upgrade/legacy
guidance. Unknown fields remain ignorable; they do not grant permission to bypass session checks.

Python remains a single-device app. The shared dashboard checks for the device-manager endpoints
and explains unavailable controls when they are absent. Its updater retains its legacy behavior;
board-aware mixed-device updates belong to the Rust app. XIAO C3 remains HID-only, with no Mac-link
extension. S3/Plus2 features follow reported hardware capabilities, including USB mic and IR.

### Requirement: Command framing and diagnostic privacy are distinct
S3/Plus2 machine responses retain exact `@` + JSON + CRLF framing. USB replies go only to USB;
TCP replies go only to the requesting client. Human setting commands acknowledge success/failure
with result JSON on the same requester; help and show retain readable output. Fixed diagnostics
can fan out, but omit private payloads in both legacy and shared mode.

### Requirement: Each message documents its fallback
Every message in `PROTOCOL.md` states the firmware or app version that introduced it and what an
older peer does when it receives it. A change to a message MUST update `PROTOCOL.md` in the same
pull request as the code on both sides.

### Requirement: Release manifests stay readable by older apps
`manifest.json` additions are new keys (`apps` beside the firmware keys); older apps read only the
keys they know. Rust accepts the S3 alias `m5stick-s3` as `m5-sticks3`; it rejects unknown or
incompatible boards/chips/layouts instead of treating a missing model as an S3 default. See
[firmware update](/link/firmware-update.md).

## Design Decisions

### Capability flags instead of a protocol version number
**Decision**: Features are announced one by one (`@state` keys, hello flags); the protocol's version number stays at 1.
**Why**: Features arrive independently on each side, and a flag lets each one degrade on its own with a clear message.
**Rejected**: A single version number to compare, which forces both sides to know every version's feature set and breaks when a feature is back-ported or turned off.
*Introduced by*: predates fab; see `PROTOCOL.md`

### Preserve slots behind opt-in sharing
**Decision**: Shared mode uses the active slot identity and retains all saved slots and bonds.
**Why**: Firmware and apps update independently, and existing pairings remain usable in legacy mode.
**Rejected**: An automatic identity reset, bond wipe or shared-mode enrollment on upgrade.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`PROTOCOL.md`, `docs/CONTRIBUTING.md`, `.github/pull_request_template.md`. Related:
[Mac link](/link/mac-link.md), [face packages](/faces/face-packages.md).
