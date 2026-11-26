---
type: memory
description: "S3 and Plus2 firmware: bounded authenticated host sessions and input picker, button gestures, Launcher favorites and agent navigation, legacy pairing and write-only per-host passwords, safe diagnostics, hardware capabilities, native tests, flash/Bluetooth budgets and firmware versioning."
---
# Stick Firmware

**Domain**: firmware

## Overview

`m5-stick-s3/` supplies shared C++17 firmware for M5StickS3 (ESP32-S3, 8 MB flash/8 MB PSRAM)
and M5StickC Plus2 (ESP32-PICO-V3-02, 8 MB flash/2 MB PSRAM). PlatformIO/pioarduino runs Arduino
core 3.x, M5Unified, NimBLE-Arduino and ArduinoJson. `board.h` selects capabilities: S3 has native
USB mic/TinyUSB and IR reception; Plus2 has a USB serial bridge and no IR receiver. The separate
XIAO C3 prototype retains its HID-only scope. Layout/tests are in `m5-stick-s3/README.md`; build
settings are in `m5-stick-s3/platformio.ini`.

## Requirements

### Requirement: Logic is hardware-free and tested natively
Code in `m5-stick-s3/src/logic/` uses no Arduino or hardware API and is compiled for the host by
the `native` PlatformIO env with Unity suites in `m5-stick-s3/test/test_*/`
(`pio test -e native`). Hardware-facing modules (`ui.cpp`, `buddy.cpp`, `ota.cpp`, `hid.cpp`) are
thin layers over it. Physical behavior requires a real-stick check before release. The
production TeeLog/command response paths also compile with fake Arduino/Wi-Fi sinks in native tests.

### Requirement: Tests give the same result under gcc and clang
CI runs the native tests with gcc on Linux (the stick's compiler family); developers run them
with clang on macOS. Code MUST NOT depend on unspecified behaviour: one side effect per
statement, no two random draws or reads in one expression.

#### Scenario: Two random draws in one call
- **GIVEN** a function call whose arguments each draw from the random generator
- **WHEN** it is compiled with gcc and with clang
- **THEN** the draws happen in different orders and the same seed gives different results, so the draws are written on separate lines

### Requirement: The app image stays within the flash budget
The firmware MUST be at most 85 % of the app slot (3,342,336 bytes). CI reads the build's
`Flash:` line and fails above the limit. The slot size is fixed by the partition table
(`default_8MB.csv` on S3 and `partitions_8MB.csv` on Plus2: two 3.2 MB OTA slots and a
1.5 MB file system for faces and IR remotes).

### Requirement: Replies stay short
NimBLE is built with 48 msys blocks. A long reply to the Mac competes with key presses and
audio for them, so `@state` MUST stay small and lists are paged (`@faces` answers 6 per page).
Both boards configure three simultaneous connections and nine stored bonds; the authenticated
host registry capacity is three. Each host has bounded RX assembly with a 4096-byte line cap;
overflow drops the whole line. Host lists, prompt slots and event routes remain bounded.

### Requirement: Stored state survives updates
Settings (combo, name, Mac slots, sounds, brightness, shared-mode selection, host metadata and
passwords) live in NVS; faces and IR remotes live in LittleFS. Updates preserve both. Legacy
mode retains the slot identities and global password. Shared mode uses the slot selected on
enablement, retains bonds and requires explicit password assignment per authenticated host; it
does not copy the global password to each Mac. Passwords remain write-only, absent from readback
and diagnostic output.

### Global button mappings

StickS3 and StickC Plus2 persist four bindings together in one NVS record. Double B defaults to
Unlock Mac, triple B to Launcher, A+B to reopen the last reply, and hold B to Talk. On screens
with global controls, physical dispatch and hint labels use the same binding table. Press B
then add A before the hold threshold, or press both together. The chord owns both releases.
Triple B wakes a sleeping display. Modal screens keep their own controls.

A discrete gesture can open any Launcher tile, start Talk, trigger a named Mac action or do
nothing. Discrete Talk records until the next B press or its 30-second limit. Hold B can
instead run Dictation, with release ending the hold. Named Mac actions go only to the selected
live peer advertising `buttonActions` in its hello. [Controls](../../CONTEXT-ACTIONS.md#buttons--gestures) · [Wire format](../../../PROTOCOL.md#button-bindings)

### Launcher favorites and agent navigation

Each stick stores its visible tile order and favorites together. The grid places favorites first,
preserves the configured order within each group and selects the first favorite when opened.
With an empty favorites list, the Launcher resumes at the last visible tile.

The selected Mac can open a built-in tile from Home or the Launcher grid while the agent
is thinking. Recording, pairing, approvals, transfers and other busy screens retain control.
Air mouse and Media consume the first A click to enable motion controls after an agent opens
them. [Controls](../../CONTEXT-ACTIONS.md#launcher-favorites-and-voice-shortcuts)

### Requirement: Focus and resource ownership survive concurrent peers
`logic/hosts.*` owns identity, validation, heartbeat, generation leases and transfer reclaim rules.
The vendored HID driver tracks authentication, subscriptions and connection parameters per peer.
Only the selected host receives keyboard/mouse/media/unlock/voice input. The Mac picker shows
short labels (stable disambiguation for duplicates), the active label and offline state; prompts
show their origin. Local selection releases held input, while remote selection needs physical
confirmation. Busy operations block switches. Disconnect clears held reports and leaves focus
offline; reconnect never steals focus. See [Mac link](/link/mac-link.md).

### Requirement: Diagnostics expose fixed events, not payloads
`TeeLog::event` accepts the fixed `diagnostics::Event` catalogue; `count` accepts typed counters
and unsigned numeric values. USB and up to three TCP viewers receive these safe status/counter
lines. Raw `Print` writes, including print/println/printf and fragmented bytes, fail closed to
a fixed omitted-payload marker in both modes. Vendored HID diagnostics use a payload-free callback
and do not write to Serial directly.

`protocol::serialLine` receives a requester-owned `Print&`: USB commands answer on USB, TCP
commands on that client. Machine replies remain `@` + JSON + CRLF; human setting commands return
result JSON, with readable help/show output. Intentional responses can contain requested data;
other viewers do not receive it. Host labels, prompts/options/hints, event text, raw JSON,
user-provided identifiers and secrets are excluded from unsolicited diagnostics.

### Requirement: Hardware evidence is distinct from build evidence
The two-Mac/two-board physical and screenshot checks in `docs/MULTI_DEVICE.md` are pending.
The multi-device draft retains `needs-hardware`; native/fake tests and successful S3/Plus2 builds
do not certify Bluetooth/audio, displays, sleep/wake, USB/TCP isolation or OTA behavior on devices.

### Requirement: The firmware has its own version
`FW_VERSION` in `m5-stick-s3/src/config.h` changes only when the firmware changes, and a feature
the Mac must detect is also reported in `@state`.

### Requirement: Text fits the fonts
Text drawn on the stick uses ASCII plus `°’‘“”–—…•·×`; the Mac transliterates anything else.
Fonts (`fonts_vlw.cpp`) and sound clips (`sounds_clips.cpp`) are generated by
`tools/make_fonts.py` and `tools/make_sounds.py`.

## Design Decisions

### HID changes need a re-pair on macOS
**Decision**: A change to the HID report map or the set of GATT services is treated as a user-visible event: the changelog and the PR say that a stick which stops connecting must be forgotten in the Mac's Bluetooth settings and paired again.
**Why**: macOS caches a paired device's GATT handles, and a changed table leaves the Mac link's characteristics at stale handles.
**Rejected**: Changing the services without telling the user, which leaves a stick that pairs but never gets its Mac link back.
*Introduced by*: predates fab; see `desktop/README.md` "Stick 2.5.0" checklist

### pioarduino and pinned display libraries
**Decision**: The build uses the pioarduino platform (Arduino core 3.x) with M5GFX, M5Unified and M5PM1 pinned to tested versions.
**Why**: NimBLE 2.x and the HID library need core 3.x, and unpinned upstream changes have broken a release build.
**Rejected**: M5Stack's documented `espressif32@6.x` platform (core 2.x).
*Introduced by*: predates fab; see `m5-stick-s3/platformio.ini`

### Fixed diagnostic vocabulary at the sink
**Decision**: Firmware emits safe event enums and typed counters; arbitrary Print bytes are suppressed at TeeLog itself.
**Why**: Any payload-bearing call site can otherwise leak another host's data through USB or TCP, including fragmented writes.
**Rejected**: Filtering selected message producers or suppressing output only in shared mode.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`m5-stick-s3/README.md`, `m5-stick-s3/platformio.ini`, `.github/workflows/ci.yml` (firmware job),
`docs/DEVELOPMENT.md`, `CHANGELOG.md`, `m5-stick-s3/src/{netlog,protocol,hid,buddy,mic}.cpp`,
`m5-stick-s3/src/logic/{hosts,diagnostics}.h`, `docs/MULTI_DEVICE.md`. Related: [firmware update](/link/firmware-update.md),
[CI and review](/delivery/ci-and-review.md).
