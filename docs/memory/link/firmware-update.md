---
type: memory
description: "Firmware update over Bluetooth: only the app image goes to the other OTA slot, in windowed packets with acks, gaps, resume and cancel; sha256 checked on both sides; a new image confirms itself or rolls back."
---
# Firmware Update

**Domain**: link

## Overview

The Mac app updates the stick over the Mac link (stick firmware 1.8.0 and later), with USB
(esptool) as the fallback and the recovery path. The message tables and error codes are in
`PROTOCOL.md` "Firmware update over Bluetooth"; the receiver's state machine is
`m5-stick-s3/src/logic/ota_rx.*`, the sender is `desktop/crates/stick-link/src/ota.rs` (and
`mac-agent/stick_agent/ota.py`).

## Requirements

### Requirement: Settings and pairings survive every update
Over Bluetooth only the app image (`app.bin`) is sent, and the stick writes it to its other OTA
slot. Over USB the release's separate parts are written at their manifest offsets. No update
writes the settings storage (NVS, 0x9000–0xDFFF), and a single merged flash image MUST NOT be
used: its padding would cover NVS and wipe the pairings.

### Requirement: The transfer is windowed, ordered and resumable
After `begin` (size, sha256, version) the stick answers `ready` with an offset, a window (16384
bytes) and a chunk size. Packets are `[offset u32 LE][bytes]`, taken only in order; the stick acks
every 4 KB, answers a `gap` when a packet was lost (the sender rewinds to that offset), and
ignores duplicates. The sender keeps at most one window unacknowledged.

#### Scenario: The link drops mid-transfer
- **GIVEN** a transfer is under way
- **WHEN** the Bluetooth link drops and comes back within 3 minutes
- **THEN** a `begin` with the same size and sha256 answers `ready` with `"resumed":true` and the offset to continue from

### Requirement: Either side can cancel, and nothing changes
`{"ota":{"cancel":1}}` from the Mac, or B on the stick's update screen, ends the transfer. The
running firmware and the boot slot are untouched.

### Requirement: The stick refuses when it should not update
`begin` is refused with a reason when the stick is busy (dictation, talk, approval, pairing, menu,
a game, a remote mode, IR, a face install), when the battery is below 30 % and not on USB power,
when the running image is still pending, and for an older or equal version without `force`.

### Requirement: The image is verified before it boots, and proves itself after
The app checks `app.bin` against the release manifest's sha256. The stick reads the written image
back from flash, compares its SHA-256 with `begin.sha256`, and validates the image before making
it the boot slot. The new image boots pending and confirms itself after 25 s of running with
Bluetooth up under the loop watchdog; any reset before that boots the previous firmware.

#### Scenario: A new image hangs at start
- **GIVEN** a new image was written and set to boot
- **WHEN** it hangs or crashes within its first 25 s
- **THEN** the watchdog resets the stick and the bootloader starts the previous firmware

## Design Decisions

### Reuse one receiver for firmware and faces
**Decision**: Face packages use the firmware update's receiver and characteristic with a file sink.
**Why**: Windows, gaps and resume are already handled and unit-tested natively; one bulk path means one set of flow-control bugs.
**Rejected**: A second transfer protocol for small files.
*Introduced by*: predates fab; see `docs/faces/FORMAT.md` section 1

### Images are hash-checked, not signed
**Decision**: Integrity rests on the encrypted paired link, the manifest's sha256 and the stick's read-back; signature hooks exist (`ota.verify_signature()`, before `commit()` in `ota.cpp`) and are empty.
**Why**: The link only accepts the paired Mac, and the Mac only sends what matches the release manifest.
**Rejected**: Shipping without the read-back check; relying on the transfer's acks alone.
*Introduced by*: predates fab; see `docs/ARCHITECTURE.md` "Security model"

## Sources

`PROTOCOL.md` ("Firmware update over Bluetooth"), `docs/ARCHITECTURE.md` (flow 7),
`m5-stick-s3/tools/make_release.py`, `mac-agent/stick_agent/flasher.py`. Related:
[Mac link](/link/mac-link.md), [releases](/delivery/releases.md).
