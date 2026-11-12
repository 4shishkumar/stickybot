---
type: memory
description: "The Mac link: a Nordic UART service on the stick's HID connection carrying JSON lines, audio and bulk transfers; presence by hello and bye; one central at a time."
---
# Mac Link

**Domain**: link

## Overview

The stick is a Bluetooth keyboard / mouse (HID) and, on the same connection, offers the Mac link:
a Nordic UART service the Mac app uses for messages, audio and bulk transfers. The full wire
format is `PROTOCOL.md` section 5; this file keeps the facts that shape every change to it.

## Requirements

### Requirement: One service, five characteristics
The Mac link is service `6e400001-b5a3-f393-e0a9-e50e24dcca9e`. Its characteristics are RX
(`…0002…`, Mac → stick, JSON lines), TX (`…0003…`, stick → Mac, JSON lines by notify, in pieces of
at most 180 bytes), Audio (`…0004…`, stick → Mac, IMA-ADPCM frames), Audio in (`…0005…`, Mac →
stick, sound clips) and OTA data (`…0006…`, Mac → stick, firmware and face packets). Writes MUST
need the paired, encrypted link. Mac programs find the stick by the service UUID in its
advertisement and MUST stay with the one stick the user picked.

### Requirement: Messages are JSON lines tagged by source
Each message is one compact JSON object per `\n`-terminated line. Messages for the Mac app carry
`"src":"agent"`; Claude Code permission prompts and questions carry `"src":"cli"`. A line can span
several notifications and a notification can carry several lines, so both sides MUST reassemble by
line. Settings commands travel as `{"proto":"@state","id":…}` and are answered by id.

### Requirement: Presence is the hello, not the connection
The app sends a hello at least every 10 s (the apps send one every 8 s, carrying the time, the
weather and the home card). The stick treats the app as present for 25 s after a hello. The app
sends `{"bye":true}` before it pauses, picks another stick or quits, and the stick then shows the
app offline at once.

#### Scenario: The app quits while the keyboard stays connected
- **GIVEN** the stick is paired and the app is running
- **WHEN** the app quits
- **THEN** macOS keeps the Bluetooth connection up for the keyboard, the app's `bye` tells the
  stick it has gone, and a stick that never got a `bye` shows the app offline 25 s after the last hello

### Requirement: One central, one bulk transfer
The stick accepts one Bluetooth central. An app MUST NOT connect while another app has the stick
(the Rust app asks the Python app's status before every connect attempt and waits). One bulk
transfer owns `…0006…` at a time: a firmware `begin` during a face install is refused `busy`, and a
face `put` during a firmware update is refused `ota`.

### Requirement: Writes are paced and retried
JSON writes are with response, in pieces of at most 180 bytes; a write refused with ATT
"Insufficient Resources" (the stick is sending a long reply) MUST be retried. Bulk packets are
write-without-response and MUST wait for the system's ready-to-send signal.

## Design Decisions

### The Mac link shares the HID connection
**Decision**: The Mac link is a GATT service on the same connection as the keyboard, not a second connection or Wi-Fi.
**Why**: One pairing, one encrypted link, and it works wherever the keyboard works, with no network setup.
**Rejected**: A Wi-Fi link (needs credentials and a network; the stick's Wi-Fi is only on for the settings page) and a second Bluetooth identity (the stick takes one central).
*Introduced by*: predates fab; see `PROTOCOL.md` section 5

### The Rust app talks to CoreBluetooth directly
**Decision**: The Rust app uses its own CoreBluetooth layer through `objc2`, behind `Connector` / `Connection` traits, and discovers only the Mac link service.
**Why**: It needs the ATT error code of a failed write, the ready-to-send signal for write-without-response and the maximum write length. Discovering only the Mac link service leaves macOS's own HID and Battery services alone.
**Rejected**: btleplug, which reports only successful writes and exposes none of the pacing calls.
*Introduced by*: predates fab; see `desktop/README.md` "Why CoreBluetooth via objc2, not btleplug"

## Sources

`PROTOCOL.md` (section 5), `docs/ARCHITECTURE.md`, `desktop/crates/stick-link/src/proto.rs`,
`desktop/crates/stick-link/src/link.rs`, `m5-stick-s3/src/buddy.cpp`. Related:
[protocol compatibility](/link/protocol-compatibility.md), [firmware update](/link/firmware-update.md).
