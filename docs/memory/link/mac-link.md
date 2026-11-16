---
type: memory
description: "The Mac link: Nordic UART over the HID connection, opt-in three-host sessions, authenticated routing, selected input focus, per-host presence, bounded resources and requester-only replies."
---
# Mac Link

**Domain**: link

## Overview

The stick is a Bluetooth keyboard / mouse (HID) and, on the same connection, offers the Mac link:
a Nordic UART service the Mac app uses for messages, audio and bulk transfers. The full wire
format is `PROTOCOL.md` sections 5–6; this file keeps the facts that shape every change to it.

## Requirements

### Requirement: One service, five characteristics
The Mac link is service `6e400001-b5a3-f393-e0a9-e50e24dcca9e`. Its characteristics are RX
(`…0002…`, Mac → stick, JSON lines), TX (`…0003…`, stick → Mac, JSON lines by notify, in pieces of
at most 180 bytes), Audio (`…0004…`, stick → Mac, IMA-ADPCM frames), Audio in (`…0005…`, Mac →
stick, sound clips) and OTA data (`…0006…`, Mac → stick, firmware and face packets). Writes MUST
need the paired, encrypted link. Mac programs find sticks by the service UUID in advertisements
and connect only to explicitly configured targets. The Rust app maintains an independent link for each configured device.

### Requirement: Messages are JSON lines tagged by source
Each message is one compact JSON object per `\n`-terminated line. Messages for the Mac app carry
`"src":"agent"`; Claude Code permission prompts and questions carry `"src":"cli"`. A line can span
several notifications and a notification can carry several lines, so both sides MUST reassemble by
line. Settings commands travel as `{"proto":"@state","id":…}` and are answered by id. Firmware keeps
a separate bounded RX inbox and line assembler per peer; an overlong line is dropped in full.

### Requirement: Authenticated sessions own messages
Firmware advertises stable physical `deviceId`, canonical `board`, `multiHost:1` and `maxHosts:3`.
On capable firmware the Rust app sends `host:{id,name,label,session}` in hello. A host installation
ID, physical device ID and CoreBluetooth peripheral UUID are distinct identities. Names and labels
are display metadata; the encrypted bonded BLE peer owns its host ID. Shared-mode firmware refuses identity
collisions, invalid names and sessions. Legacy hellos bypass the shared registry. Timed-out
capability negotiation retries with bounded backoff; a definite legacy response stops negotiation. Shared-mode application writes carry matching `hs`;
discovery queries (`@state`, `@hosts`, `@keys`) and the handshake permit initial negotiation.

Connection generations invalidate old requests and resource leases. TX, audio and HID notifications
target one connection explicitly; private characteristic data is notify-only, unavailable through
a shared read. Replies, approval decisions and cancellation retain their originating peer/session.

### Requirement: Presence is the hello, not the connection
The app sends a hello at least every 10 s (the apps send one every 8 s, carrying the time, the
weather and the home card). The stick treats the app as present for 25 s after a hello. The app
sends `{"bye":true}` before its device session pauses or quits, and the stick then shows that
app offline at once. Shared hosts have independent heartbeats; `@hosts` / `evt:hosts` distinguish
BLE `online`, app presence `app` and selected `active`, and retain offline labels.

#### Scenario: The app quits while the keyboard stays connected
- **GIVEN** the stick is paired and the app is running
- **WHEN** the app quits
- **THEN** macOS keeps the Bluetooth connection up for the keyboard, the app's `bye` tells the
  stick it has gone, and a stick that never got a `bye` shows the app offline 25 s after the last hello

### Requirement: Shared mode is explicit and preserves legacy pairing
Legacy mode accepts one central using the selected saved slot. `@hosts shared=1` opts into up
to three hosts using the current slot's persistent BLE identity; `shared=0` returns to legacy
mode. Saved slots and bonds are retained. Macs paired to other slot identities may need guided
re-pairing with the shared identity. Old apps require legacy mode. The Rust app checks Python
coexistence for the particular target, leaving unrelated devices available. Shared records can
be removed with `@hosts forget=<host-id|all>` or the confirmed **Forget saved Macs** action.
Legacy slots and Bluetooth bonds remain intact. Forgetting a host clears its saved credentials
and restarts the stick after acknowledgement so new installation identities can enroll.

### Requirement: One selected host receives input
Keyboard, mouse, media, unlock and new voice input go to the selected host. The stick picker
selects a connected host directly; a remote `hosts.select` request requires confirmation on the
stick. Switching releases held input first and refuses while exclusive activity is busy. Reconnect
does not change focus; losing the selected host stops input without selecting another Mac.
Prompts and events display their origin host, independently of input focus.

### Requirement: Physical resources have bounded owners
Microphone, speaker and bulk transfers have one owner each, bound to peer/session generation.
Another host cannot replace or cancel the operation. Firmware keeps bounded per-host prompt slots
and event routes; expiry/disconnect retires ownership. One transfer owns `…0006…`: firmware and
face transfers exclude one another. A resumable transfer retains its bonded host identity, and
only that host can reclaim it with a fresh-session begin/put.

### Requirement: Diagnostics do not broadcast private payloads
USB/TCP diagnostics contain fixed safe events and numeric counters. Host names, prompts, replies,
events, settings values and raw Mac-link JSON do not reach these broadcast sinks. Explicit USB
and TCP commands return data only to their requester; BLE responses remain peer-targeted. The
implementation boundary is documented in [stick firmware](/firmware/stick-firmware.md).

### Requirement: Writes are paced and retried
JSON writes are with response, in pieces of at most 180 bytes; a write refused with ATT
"Insufficient Resources" (the stick is sending a long reply) MUST be retried. Bulk packets are
write-without-response and MUST wait for the system's ready-to-send signal.

## Design Decisions

### The Mac link shares the HID connection
**Decision**: The Mac link is a GATT service on the same connection as the keyboard, not a second connection or Wi-Fi.
**Why**: One pairing, one encrypted link, and it works wherever the keyboard works, with no network setup.
**Rejected**: A Wi-Fi link (needs credentials and a network; the stick's Wi-Fi is only on for the settings page) and a separate Bluetooth identity for the Mac link.
*Introduced by*: predates fab; see `PROTOCOL.md` section 5

### The Rust app talks to CoreBluetooth directly
**Decision**: The Rust app uses its own CoreBluetooth layer through `objc2`, behind `Connector` / `Connection` traits, and discovers only the Mac link service.
**Why**: It needs the ATT error code of a failed write, the ready-to-send signal for write-without-response and the maximum write length. Discovering only the Mac link service leaves macOS's own HID and Battery services alone.
**Rejected**: btleplug, which reports only successful writes and exposes none of the pacing calls.
*Introduced by*: predates fab; see `desktop/README.md` "Why CoreBluetooth via objc2, not btleplug"

### Bind authority to the bond and session
**Decision**: Host IDs and labels accompany an authenticated peer; asynchronous operations retain a generation lease.
**Why**: A display name or reused connection handle cannot authorize a delayed answer or transfer packet.
**Rejected**: Routing private traffic by a label, an arbitrary first peer, or the current input selection.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`PROTOCOL.md` (sections 5–6), `docs/ARCHITECTURE.md`, `desktop/crates/stick-link/src/proto.rs`,
`desktop/crates/stick-link/src/link.rs`, `m5-stick-s3/src/buddy.cpp`,
`m5-stick-s3/src/logic/hosts.*`, `docs/MULTI_DEVICE.md`. Related:
[protocol compatibility](/link/protocol-compatibility.md), [firmware update](/link/firmware-update.md).
