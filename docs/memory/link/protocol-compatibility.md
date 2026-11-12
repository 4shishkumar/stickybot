---
type: memory
description: "How a newer app works with older firmware and the reverse: unknown messages and fields are ignored, features are detected from @state keys and hello flags, PROTOCOL.md changes with every message."
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
| stick, in `@state` | `usbMounted` | wireless mic behaviour on USB |
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

### Requirement: Each message documents its fallback
Every message in `PROTOCOL.md` states the firmware or app version that introduced it and what an
older peer does when it receives it. A change to a message MUST update `PROTOCOL.md` in the same
pull request as the code on both sides.

### Requirement: Release manifests stay readable by older apps
`manifest.json` additions are new keys (`apps` beside the firmware keys); older apps read only the
keys they know.

## Design Decisions

### Capability flags instead of a protocol version number
**Decision**: Features are announced one by one (`@state` keys, hello flags); the protocol's version number stays at 1.
**Why**: Features arrive independently on each side, and a flag lets each one degrade on its own with a clear message.
**Rejected**: A single version number to compare, which forces both sides to know every version's feature set and breaks when a feature is back-ported or turned off.
*Introduced by*: predates fab; see `PROTOCOL.md`

## Sources

`PROTOCOL.md`, `docs/CONTRIBUTING.md`, `.github/pull_request_template.md`. Related:
[Mac link](/link/mac-link.md), [face packages](/faces/face-packages.md).
