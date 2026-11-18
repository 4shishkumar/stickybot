---
type: memory
description: "Firmware updates: device/session-owned BLE transfers with resume, cancel and rollback; board/chip/layout validation for S3 and Plus2; separate USB parts preserve settings and bonds; unknown or incompatible bundles are unavailable."
---
# Firmware Update

**Domain**: link

## Overview

The Mac app updates the stick over the Mac link (stick firmware 1.8.0 and later), with USB
(esptool) as the fallback and the recovery path. The message tables and error codes are in
`PROTOCOL.md` "Firmware update over Bluetooth"; the receiver's state machine is
`m5-stick-s3/src/logic/ota_rx.*`, the sender is `desktop/crates/stick-link/src/ota.rs` (and
`mac-agent/stick_agent/ota.py`). Board-aware selection and pre-write checks are implemented in
the Rust app; the Python updater retains its legacy single-device implementation.

## Requirements

### Requirement: Settings and pairings survive every update
Over Bluetooth only the app image (`app.bin`) is sent, and the stick writes it to its other OTA
slot. Over USB the release's separate parts are written at their manifest offsets. No update
writes the settings storage (NVS, 0x9000–0xDFFF), and a single merged flash image MUST NOT be
used: its padding would cover NVS and wipe the pairings.

### Requirement: Board identity is checked before writing
Rust canonicalizes `m5stick-s3` / `m5-sticks3` to S3 (`esp32s3`, silicon ID 9) and
`m5-stickc-plus2` to Plus2 (`esp32`, ID 0). Unknown models and incompatible bundles are unavailable.
The manifest board must match, its chip must agree when supplied, and part names, offsets, sizes
and duplicates are validated. App and bootloader image headers must target the board's silicon.
S3 bootloader offset is `0x0`, Plus2 `0x1000`; partition table, boot_app0 and app offsets are
`0x8000`, `0xE000` and `0x10000` for both.

`make_release.py --board` selects either canonical board and validates app/bootloader silicon
before writing the bundle. Local version folders may contain separate board-ID subfolders.
Bundle selection filters by compatibility before choosing a version; missing Plus2 firmware
never offers an S3 image. Normal USB flashing queries `@state` once on the selected port and validates the bundle and
parts before reset/write. Explicit USB recovery chooses a physical port and board model; esptool
verifies the matching ROM chip before writing when firmware cannot answer. Recovery preserves
all bundle/layout/hash/NVS checks. Passive enumeration never probes generic WCH bridges. After
an S3 touch, serial mismatch fallback accepts only one newly appeared bootloader with the old
app port gone; pre-existing or ambiguous bootloaders are refused.

### Requirement: Jobs retain device and transfer ownership
Each Rust device runtime owns its firmware jobs. Changing the dashboard view or default cannot
retarget a running job. In shared mode the bulk receiver belongs to its authenticated peer and
session generation. Other hosts cannot cancel, write or claim it. Disconnect revokes the lease;
the same saved host can renew it through a matching fresh-session begin/put within the receiver
resume window. Stale binary packets cannot attach to a later session.

### Requirement: The transfer is windowed, ordered and resumable
After `begin` (size, sha256, version) the stick answers `ready` with an offset, a window (16384
bytes) and a chunk size. Packets are `[offset u32 LE][bytes]`, taken only in order; the stick acks
every 4 KB, answers a `gap` when a packet was lost (the sender rewinds to that offset), and
ignores duplicates. The sender keeps at most one window unacknowledged.

#### Scenario: The link drops mid-transfer
- **GIVEN** a transfer is under way
- **WHEN** the Bluetooth link drops and comes back within 3 minutes
- **THEN** a `begin` with the same size and sha256 answers `ready` with `"resumed":true` and the offset to continue from; shared mode also requires the same bonded host to claim a fresh session lease

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
**Why**: Transfer writes require an encrypted paired peer and the active transfer lease, and the Mac only sends what matches the release manifest.
**Rejected**: Shipping without the read-back check; relying on the transfer's acks alone.
*Introduced by*: predates fab; see `docs/ARCHITECTURE.md` "Security model"

### Fail closed on model and layout
**Decision**: The Rust updater requires a recognized board and matching image/layout before either USB or BLE writes.
**Why**: Several connected boards can share a version while requiring different silicon and bootloader offsets.
**Rejected**: Selecting the newest bundle without board filtering or guessing S3 when model detection fails.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`PROTOCOL.md` ("Firmware update over Bluetooth"), `docs/ARCHITECTURE.md` (flow 7),
`m5-stick-s3/tools/make_release.py`, `mac-agent/stick_agent/flasher.py`,
`desktop/crates/stickybot-server/src/{firmware,flasher,releases}.rs`, `docs/MULTI_DEVICE.md`. Related:
[Mac link](/link/mac-link.md), [releases](/delivery/releases.md).
