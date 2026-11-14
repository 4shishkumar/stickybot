# Intake: M5StickC Plus2 board bring-up

**Change**: 261113-lll6-stickc-plus2-board-bringup
**Created**: 2026-11-13

## Origin

> "i have connected a new stickc, we need to flash our firmware to this, but have to check its spec first and make changes needed" — then, after the spec check: "yes start with step 1 and 2".

Conversational. The connected stick was probed read-only with esptool: **ESP32-PICO-V3-02** (classic ESP32, rev 3.1), 8 MB flash, embedded PSRAM, CH9102 USB serial (VID:PID 1A86:55D4) — an **M5StickC Plus2**. The user approved steps 1 and 2 of a four-step port:

1. Board layer + new build target, compiling, unsupported parts switched off.
2. Flash over USB; bring up screen, buttons, Bluetooth pairing and the Mac link.
3. (later) battery, sound, mic, IR send, memory tuning.
4. (later) release + Mac app shipping two firmware files and picking by board.

## Why

The firmware builds for one board (M5StickS3, ESP32-S3) and reaches StickS3-only hardware directly: the PM1 power chip (charging, LED, rails, power off), TinyUSB (USB microphone, native USB serial), S3 pin numbers (IR G46/G42, wake pin), and 8 MB of PSRAM. A second stick model can't run it. A small board layer keeps the StickS3 behavior exactly as is while letting other boards switch parts off, and is also the base for the planned roadmap of other ESP32 boards.

## What Changes

### Board layer
- `src/board.h` + `src/board_sticks3.cpp` + `src/board_stickc_plus2.cpp` (each compiled only for its board via a `BOARD_*` build flag). Functions cover only what differs:
  - `board::begin()` (after `M5.begin`): S3 turns the charger on; Plus2 holds power (G4 high; M5Unified already does it, keep explicit).
  - `board::speakerAmp(bool)`: PM1 gpio3 on S3; no-op on Plus2.
  - `board::statusLed(bool)`: PM1 LED on S3; no-op on Plus2 for now (red LED shares G19 with IR).
  - `board::chargerKick()` / `board::chargerControl` capability: S3 only.
  - `board::prepareStandby()`: S3 turns the L3B rail off via PM1; Plus2 nothing. `board::WAKE_PIN` per board (Plus2: G37, button A, RTC-capable, external pull-up, so no internal pull).
  - `board::powerOff()`: `M5.Power.powerOff()` on both (Plus2 releases HOLD).
  - Capability constants: `HAS_USB_MIC`, `HAS_IR_RX`, `HAS_IR_TX`, `HAS_CHARGER_CONTROL`, `HAS_VBUS_SENSE`.
- `STICK_BOARD` in `config.h` becomes per board: `"m5-sticks3"` (unchanged) and `"m5-stickc-plus2"`. It is already sent in the hello line, so the app can tell boards apart later.

### Parts switched off on the Plus2 (this change)
- **USB microphone**: `mic.cpp`'s TinyUSB audio card and `lib/StickUSBAudio` are S3-only (`lib_ignore` on the Plus2 env); `usbStreaming()`/`usbMounted()` return false. The Bluetooth mic path stays.
- **IR**: receive off (no receiver). Send is left compiled but untested (G19); if RMT on classic ESP32 needs more than a pin change, the whole IR app is hidden on the Plus2 and noted for step 3.
- **Charger control / status LED / VBUS**: off; battery level from `M5.Power.getBatteryVoltage()` only, charging state unknown (reported as not charging).
- PSRAM: allocation call sites already use `heap_caps_malloc(MALLOC_CAP_SPIRAM…)`; where they'd fail without PSRAM they must fail cleanly. No tuning in this change.

### Build
- `platformio.ini`: new `[env:stickc_plus2]` (board `m5stick-c` or `pico32` with 8 MB flash, `default_8MB.csv`, PSRAM enabled, no USB CDC flags, normal serial upload at 1500000 baud, `-D BOARD_STICKC_PLUS2`). `[env:sticks3]` gets `-D BOARD_STICKS3` and is otherwise unchanged.
- `Makefile`: `make build-plus2`, `make flash-plus2 PORT=…`.
- CI `firmware` job: also builds `stickc_plus2` with the same ≤85 % flash limit.

### Bring-up (step 2, on the real stick, done by the main session, never a subagent)
- Flash over USB serial (normal esptool auto-reset; never a merged/factory image, app + bootloader + partitions written the standard PlatformIO way).
- Check on the device: screen draws, A/B buttons, Bluetooth advertising and pairing, the Mac app link (hello with `board: m5-stickc-plus2`).

### Not in this change
Battery gauge polish, buzzer clip quality, mic tuning, IR send verification, memory tuning, release tooling and Mac app firmware selection per board (the app must not OTA StickS3 firmware onto a Plus2: until step 4, that guard is "don't press update on the Plus2"; if cheap, the firmware refuses an OTA image built for another board).

## Affected Memory

- `firmware/stick-firmware`: (modify) board layer, supported boards, what is off on the Plus2
- `delivery/ci-and-review`: (modify) firmware job builds both boards

## Impact

`m5-stick-s3/platformio.ini`, `src/config.h`, new `src/board*.{h,cpp}`, `src/main.cpp` (standby, setup), `src/battery.cpp`, `src/sound.cpp`, `src/mic.cpp`, `src/protocol.cpp` (PM1 debug commands), `src/ir/pins.h` + `ir.cpp`/`rmt.cpp`, `Makefile`, `.github/workflows/ci.yml`, `CHANGELOG.md`. No Mac app or protocol changes. StickS3 binary should be functionally identical.

## Open Questions

- Does the RMT-based IR code build and behave on classic ESP32 without changes? (decided at build time; fallback is hiding IR on the Plus2)
- Does the full firmware fit the Plus2's internal RAM with Wi-Fi + BLE + 2 MB PSRAM? (found during bring-up; tuning is step 3)

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | The device is an M5StickC Plus2 | esptool read: ESP32-PICO-V3-02, 8 MB flash, CH9102 | S:95 R:90 A:95 D:95 |
| 2 | Certain | Scope is steps 1 and 2 only | Discussed — user said "yes start with step 1 and 2" | S:95 R:85 A:90 D:95 |
| 3 | Confident | Compile-time board selection with one .cpp per board, not runtime detection | Different chip families can't share a binary; simplest | S:80 R:70 A:90 D:85 |
| 4 | Confident | USB mic, IR receive, charger control off on the Plus2 | Hardware lacks native USB, IR receiver and a power chip | S:85 R:80 A:90 D:90 |
| 5 | Confident | Same partition table (default_8MB) and OTA layout | Same 8 MB flash; firmware is 2.5 MB of a 3.2 MB slot | S:80 R:75 A:85 D:85 |
| 6 | Confident | Board id string `m5-stickc-plus2` in the existing hello `board` field | Field already exists as `m5-sticks3`; no protocol change | S:80 R:80 A:85 D:80 |
| 7 | Tentative | IR send stays compiled on the Plus2 if RMT builds unchanged, else IR hidden | Unknown until built; either is reversible | S:60 R:85 A:60 D:60 |
| 8 | Confident | Flashing is done only by the main session over USB serial, standard images | Standing rule: subagents never flash; never merged/factory images | S:90 R:60 A:95 D:90 |

8 assumptions (2 certain, 5 confident, 1 tentative, 0 unresolved).
