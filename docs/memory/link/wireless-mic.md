---
type: memory
description: "Wireless dictation to the selected Mac: per-hold Core Audio input, prepare/ready handshake, tap or BlackHole carrier, one stream per Mac, source-session ownership and shared-mode USB audio suppression."
---
# Wireless Mic

**Domain**: link

## Overview

Button A streams the stick's microphone over the Mac link to a dictation app (Wispr Flow) on
the selected Mac, through a Core Audio input device that exists only around the hold. Legacy S3
mode uses its USB mic when mounted by a computer. Shared mode uses Bluetooth even with a cable
attached; Plus2 has no native USB mic. Details: `PROTOCOL.md` section 5 (wispr messages,
"The mic device"), `docs/ARCHITECTURE.md` flow 4, `desktop/crates/macos-audio/`.

## Requirements

### Requirement: Audio retains its selected host and device
Firmware binds mic/audio notifications to the selected authenticated host session. Shared mode
disables USB audio forwarding and reports USB mic mounting/streaming false. No USB-host
association mechanism is implemented.
Focus changes refuse while recording/playback is busy, and disconnect/session retirement cancels
the owner. Replies and readiness messages retain the source device/session.

The Rust app has per-device wireless-mic state and a shared audio-owner guard. Only one stick
can dictate through the Mac audio device at a time; a competing prepare receives a busy response
("Another stick is dictating"). Disconnect, cancellation and cleanup release only the owning
device's lease. The speech model/worker is shared; dictation streams are not mixed.

### Requirement: The key waits for the device
When the app's hello said `wmic` and `wready`, A down sends `prepare` with a press id and starts
streaming 20 ms IMA-ADPCM frames, but holds the key. The app creates the device, waits the switch
time if it just created it, and answers `ready` with the same id. The stick presses the key on
`ready`, or after 800 ms without one. A released before that sends no key.

### Requirement: The device exists only around a hold
The app creates the aggregate device `<stick>-bt-mic` per hold and removes it after the tail
(and while another process still reads it, at most 4 s). It MUST be removed at once when the stick
disconnects or reports `usb mounted` in legacy mode, when the feature is turned off, and on exit.
At start the app removes any left over, matching only its own UID and tap name.

#### Scenario: The stick is plugged in during a hold
- **GIVEN** a legacy-mode S3 hold is streaming over Bluetooth
- **WHEN** the stick reports `usb mounted`
- **THEN** the app removes the device at once and the dictation app goes back to its usual mic

### Requirement: Audio only goes out through a checked carrier
The carrier is an Apple audio tap of the app's own process (macOS 14.2+, needs System Audio
Recording) or BlackHole 2ch. The tap is used only after an inaudible check (a −80 dBFS tone through
a test-named device) heard the tone, and audio is played only while the app's tap exists, so the
stick's mic never comes out of the speakers. With no working carrier the hello says `wmic:false`
and the stick presses the key without streaming.

### Requirement: Lost audio is silence, not delay
Frames carry a sequence number and their own decoder state. The player orders them, plays from a
short jitter buffer, and replaces a lost frame with 20 ms of silence. The app does not transcribe
this audio.

## Design Decisions

### A device that appears, instead of a permanent virtual mic
**Decision**: The mic device is created for each hold and removed after it.
**Why**: The dictation app picks the highest-ranked mic present and re-checks when the device list changes; a device that exists only during a hold is used then, and the user's usual mic the rest of the time.
**Rejected**: A permanent device (it would be picked for every dictation, including ones without the stick) and setting the dictation app to BlackHole itself.
*Introduced by*: predates fab; see `docs/ARCHITECTURE.md` flow 4

### Bluetooth headphones are not the tap's clock
**Decision**: The Rust app clocks the aggregate by the default output unless that is a Bluetooth device, and then by the built-in output.
**Why**: With Bluetooth headphones as the clock the aggregate's input never runs and audio reaches the tap about a second late.
**Rejected**: Always using the default output.
*Introduced by*: predates fab; see `desktop/README.md` "Wireless mic and Bluetooth output"

### Use wireless audio for shared hosts
**Decision**: Shared mode suppresses USB mic forwarding and uses the selected host's Bluetooth session.
**Why**: USB mounted/power state cannot prove which Mac is on the cable.
**Rejected**: Sending speech to the cable whenever USB is mounted, independently of input focus.
*Introduced by*: 261115-wjoy-multi-mac-device-connections

## Sources

`PROTOCOL.md` (section 5), `docs/ARCHITECTURE.md` (flow 4), `desktop/README.md`,
`desktop/crates/macos-audio/src/`, `desktop/crates/stickybot-server/src/wmic.rs`,
`m5-stick-s3/src/logic/wispr.*`. Related: [Mac link](/link/mac-link.md), [Mac apps](/apps/mac-apps.md).
