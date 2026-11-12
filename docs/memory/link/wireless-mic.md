---
type: memory
description: "The wireless mic: holding A away from USB streams the stick's mic to the Mac, which plays it into a per-hold Core Audio input device that a dictation app records from; prepare/ready handshake, tap or BlackHole carrier."
---
# Wireless Mic

**Domain**: link

## Overview

Held away from a computer's USB, button A makes the stick's microphone reach a dictation app
(Wispr Flow) on the Mac: the stick streams audio over the Mac link and the app plays it into a
Core Audio input device that exists only around the hold. Plugged into a computer, the stick is a
USB microphone and nothing goes over Bluetooth. Details: `PROTOCOL.md` section 5 (wispr messages,
"The mic device"), `docs/ARCHITECTURE.md` flow 4, `desktop/crates/macos-audio/`.

## Requirements

### Requirement: The key waits for the device
When the app's hello said `wmic` and `wready`, A down sends `prepare` with a press id and starts
streaming 20 ms IMA-ADPCM frames, but holds the key. The app creates the device, waits the switch
time if it just created it, and answers `ready` with the same id. The stick presses the key on
`ready`, or after 800 ms without one. A released before that sends no key.

### Requirement: The device exists only around a hold
The app creates the aggregate device `<stick>-bt-mic` per hold and removes it after the tail
(and while another process still reads it, at most 4 s). It MUST be removed at once when the stick
disconnects or reports `usb mounted`, when the feature is turned off, and on exit; at start the
app removes any left over, matching only its own UID and tap name.

#### Scenario: The stick is plugged in during a hold
- **GIVEN** a hold is streaming over Bluetooth
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

## Sources

`PROTOCOL.md` (section 5), `docs/ARCHITECTURE.md` (flow 4), `desktop/README.md`,
`desktop/crates/macos-audio/src/`, `desktop/crates/stickybot-server/src/wmic.rs`,
`m5-stick-s3/src/logic/wispr.*`. Related: [Mac link](/link/mac-link.md), [Mac apps](/apps/mac-apps.md).
