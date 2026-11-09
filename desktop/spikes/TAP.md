# Spike 2: the "wireless mic" tap from Rust

**Verdict: it works.** The Rust port creates the same process tap and aggregate device as
`mac-agent/stick_agent/stick_mic_device.py`, the inaudible -80 dBFS probe comes through it
at the expected level, and both objects are destroyed cleanly, including on Ctrl-C, on
the watchdog and after a crash (the next run cleans up). No hand-written FFI was needed:
`objc2-core-audio` 0.3.2 binds `CATapDescription` and the tap functions.

Tested on macOS 27.2 (Apple Silicon, default output "MacBook Pro Speakers"), Rust 1.99.0,
objc2 0.6.5, objc2-core-audio 0.3.2, cpal 0.18.2.

## What's here

| Path | What |
|---|---|
| `desktop/Cargo.toml` | the Cargo workspace (Tauri crates will join it later) |
| `crates/macos-audio/src/naming.rs` | `slug`, `name_for`, the UID / tap / check-device suffixes (tested against the Python behaviour) |
| `crates/macos-audio/src/plist.rs` | the aggregate description as plain Rust data, then a `CFDictionary` (tests: same keys as Python, the keys match the CoreAudio constants, CF round trip) |
| `crates/macos-audio/src/level.rs` | the probe sine, the meter and the "ok / failed" rule (same thresholds as Python), resampling |
| `crates/macos-audio/src/hal.rs` | HAL property helpers: devices, taps, default output, PID → process object, create / destroy aggregate |
| `crates/macos-audio/src/tap.rs` | `CATapDescription` → `AudioHardwareCreateProcessTap` |
| `crates/macos-audio/src/mic.rs` | `TapMic` (tap + aggregate), a process-wide registry of live objects, `destroy_all_live()`, `cleanup_stale()` |
| `crates/macos-audio/examples/tapmic.rs` | the self-check / demo binary |

`cargo test` runs 13 unit tests (naming, the description dictionary, the meter and the
probe rule). They never touch audio hardware.

## Same behaviour as the Python version

| | Python (`stick_mic_device.py`) | Rust |
|---|---|---|
| Tap | `CATapDescription initStereoMixdownOfProcesses:[own process object]` | same, via the objc2 binding |
| Mute | `setMuteBehavior: 1` (CATapMuted) | `setMuteBehavior(CATapMuteBehavior::Muted)`. Read back through the `isMuted` getter = **1** |
| Public | `setPrivate: NO` | `setPrivate(false)`. Read back `isPrivate` = **false** |
| Tap name | `<uid>.tap` | same (`com.stickkey.spike-rust-mic.tap`) |
| Aggregate | `name, uid, private=0, stacked=0, master=<default output uid>, subdevices=[{uid}], taps=[{uid: <tap UUID>, drift: 1}], tapautostart=1` | identical (unit-tested key for key) |
| Device name | `<slug>-bt-mic` / `stickybot-bt-mic` | `name_for()` is the same. The spike uses **`spike-rust-mic`** / `com.stickkey.spike-rust-mic` so it can never touch the agent's device |
| Probe | `-check` device, UID `.check`, tap `.check.tap`; 440 Hz at 1e-4 (-80 dBFS); ok if the tone is heard and rms ≥ ¼ of a sine's rms; ≤ 1.5 s, stops after 60 ms of tone | same names, same numbers and the same rule |
| HAL run loop | `kAudioHardwarePropertyRunLoop = NULL` | same |
| Cleanup | destroy the device, then the tap; `cleanup_stale()` by UID / tap name | same, plus a registry so Ctrl-C or the watchdog destroys exactly what this process still owns |
| Gate | `wispr_mic.py` writes silence unless `tap_live` | the output callback writes silence unless the tap is live, and silence wins on Ctrl-C or the watchdog *before* anything is destroyed |

One difference: Python's probe plays into the *aggregate's* output through an IOProc on the
aggregate. The Rust probe does what `wispr_mic.py` does for real audio: it plays into the
**default output** with cpal and reads the **aggregate's input** with cpal. That checks the
same path the stick audio will take.

## Measurements (three probe runs)

| | run 1 | run 2 | run 3 |
|---|---|---|---|
| create tap + aggregate | 45.7 ms | 60.1 ms | 58.8 ms |
| measured rms on the device input | 6.66e-5 = **-83.5 dBFS** | 6.03e-5 = -84.4 dBFS | 6.32e-5 = -84.0 dBFS |
| expected (a -80 dBFS sine) | 7.07e-5 = -83.0 dBFS | | |
| peak | -80.0 dBFS | -80.0 dBFS | -80.0 dBFS |
| tone first heard after | 21 ms of input | 21 ms | 21 ms |
| whole measurement | 137 ms | 220 ms | 211 ms |
| destroy (device + tap) | 2.0 ms | 1.6 ms | 2.7 ms |

The input was 2 channels at 48 kHz. All three runs gave the verdict OK. The device
count was 6 before and 6 after, and there were 0 process taps before and 0 after.

What else was verified:
- **Ctrl-C**: `tapmic --show 6` with SIGINT after 2 s printed "destroyed 2 device/tap
  object(s)". Afterwards no spike device or tap was listed.
- **Watchdog**: `tapmic --show 10 --timeout 3` exited with 124 after destroying both
  objects. Nothing was left.
- **Crash**: after SIGKILL, the aggregate *and* the public tap were **still there**, as in
  Python. The next run (`--cleanup`) removed them by UID and tap name. The device count
  went from 7 to 6 and the tap count from 1 to 0.
- The agent's devices (`com.stickkey.wireless-mic*`) were never touched. Their UIDs can't
  match the spike's (there's a unit test for that).

## Findings worth knowing for the real port

- **No raw FFI was needed.** `objc2-core-audio` 0.3.2 (features `AudioHardware`, `objc2`,
  `objc2-foundation`) has `CATapDescription` with every init method, every setter and getter,
  `CATapMuteBehavior`, `AudioHardwareCreateProcessTap` / `DestroyProcessTap`, and all the
  `kAudioAggregateDevice…` and `kAudioSubTap…` keys. Small wrinkles:
  - `kAudioHardwarePropertyRunLoop` is only in the bindings' "deprecated" feature, so it is
    defined locally as `'rnlp'`.
  - `AudioHardwareCreateProcessTap` takes a `*mut u32` out pointer, while the other functions
    take `NonNull`.
  - The `CFDictionary` is built with `objc2-core-foundation`, which is safe and needs no manual
    `CFRelease`.
- **Check that the class exists first.** objc2 panics on a missing class, the way Python
  would abort on an unknown selector. `tap::available()` checks macOS ≥ 14.2 and
  `AnyClass::get("CATapDescription")` before sending any message.
- **Object IDs are per process.** The same aggregate was device 165 / tap 164 inside the
  process that made it, but 117 / 53 when another process listed it. Never hand an
  `AudioObjectID` across processes (for example from a sidecar to the Tauri core). Use the
  UID or the tap name.
- **Destroy returns before the device list catches up.** A just-destroyed aggregate was
  still listed, with its UID, for about 7 ms. Leak checks (and "is it gone?" logic before
  re-creating the device) must poll briefly.
- **cpal can open the aggregate's input directly.** Its `DeviceId` on macOS is the Core Audio
  UID, so finding the device by UID is exact.
  - Pitfall: cpal 0.18 has its own loopback feature. Opening an *input* stream on an
    output-only device makes cpal create its own private, *unmuted* tap of that device.
    Always look the aggregate up by UID and fail if it isn't there. Never fall back to "the
    default device".
- **Aggregates and public taps outlive the process.** The Tauri app needs `cleanup_stale()`
  at startup and on quit, and `destroy_all_live()` in its signal and panic paths, exactly as
  the agent has them.

## TCC: who gets "System Audio Recording"

macOS gives the permission (`kTCCServiceAudioCapture`) to the **responsible process**, not
necessarily the binary that creates the tap. Until it is granted, a tap delivers silence
**and does not mute**. That is why nothing loud is ever played before the -80 dBFS probe
has heard itself.

- **From a terminal**, the responsible process is the terminal app (Terminal, iTerm, Warp,
  VS Code…). Every binary it starts uses *its* permission, and there is no separate
  `tapmic` entry. This spike ran as tmux → zsh → claude → zsh → tapmic. The tmux server is
  daemonized, so responsibility goes to the app that started it. That app already had the
  permission, so the probe passed with **no prompt**. A terminal without the permission
  gets the prompt (or silence) on the first run. The watchdog (`--timeout`, 20 s by
  default) stops a run stuck waiting on it, and the run is never retried.
- **The Python agent under launchd** is responsible for itself, which is why its hint
  says "Python" when StickyBot.app isn't set up.
- **A Tauri `.app`** is its own responsible process, so the grant is tied to the bundle's
  code signature (its designated requirement). Implications:
  - **A stable signing identity is required.** An ad-hoc signature (the default for unsigned
    or `tauri build` without an identity) has a designated requirement based on the cdhash,
    which changes with every build. After each update the old grant stops matching: the
    tap goes silent and stops muting. The probe catches this, but users would have to
    re-grant after every release. Sign every build, dev builds included, with the same
    Developer ID (or at least one stable self-signed certificate on the dev machine).
  - **`tauri dev` behaves differently from the release app.** It runs the binary from
    `target/` outside a bundle, so the permission is the terminal's. A run that works in
    `tauri dev` proves nothing about the shipped app's grant.
  - **Sidecars and LaunchAgents.** A sidecar binary that the `.app` launches normally still
    counts as the app. One started by a LaunchAgent plist is responsible for itself and
    needs its own grant, like Python today. Keep the tap in the app's main process.
  - **Moving from the Python agent means a new grant.** Users will be asked again, or must
    flip the switch in Privacy & Security, when they move to the Tauri app. The old
    "Python" or "StickyBot" entry doesn't carry over.
  - **Not verified:** the bundled app likely needs `NSAudioCaptureUsageDescription` in its
    `Info.plist` (the purpose string for audio capture). Whether it is required, and what
    happens without it (silent denial or a crash), still has to be tested in the Tauri
    spike. No hardened-runtime entitlement is known to be needed for taps
    (`com.apple.security.device.audio-input` is for microphones). Also not verified.
- **Never use `tccutil reset`** to test this on a machine where the agent runs. It would
  also revoke the agent's grant.

## The audible test (for a human — not run by the agent)

This plays speech into `spike-rust-mic`. If everything works, you **hear nothing** from
the speakers and Wispr Flow types the sentence.

```sh
cd <repo>/desktop
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"   # Homebrew's rustup is keg-only
cargo build --example tapmic                         # or: cargo +1.99.0 build --example tapmic

# 0. the probe alone (inaudible); must end with "verdict: OK" and "clean"
./target/debug/examples/tapmic

# 1. optional: make "spike-rust-mic" appear for 60 s (nothing is played), so you can pick it
#    in Wispr Flow → Settings → Microphone (it disappears again afterwards)
./target/debug/examples/tapmic --show 60

# 2. the real test: probe → device → re-probe → countdown → speech → destroy
./target/debug/examples/tapmic --speech --delay 10
#    during the 10 s countdown: pick "spike-rust-mic" in Wispr Flow (the device exists
#    only now) and start a dictation (hands-free mode, or hold the shortcut until it's done).
#    Optional: --text "your own sentence", or --tone for 2 s of 440 Hz at -20 dBFS.
```

What to check:
1. **Nothing comes out of the speakers or headphones.** Muting works.
2. Wispr Flow types: "This is the StickyBot wireless mic test. If Wispr Flow typed this
   sentence, the Rust audio tap works."
3. `tapmic` prints "the device heard it: rms … dBFS" at a sane speech level (about -30 to
   -15 dBFS), then "destroyed" and "clean: no spike device or tap left".

Ctrl-C at any point silences the output first and then removes the device. If a run is
killed hard, `./target/debug/examples/tapmic --cleanup` removes what it left behind.

## Not verified / remaining risks

- **The loud path (`--speech`, `--tone`) was compiled but never run by the agent**, by
  design. Whether Wispr hears it, and the real-world mute, are still unverified. The mute is
  only inferred: the probe heard the tap, so the permission is granted, so `CATapMuted`
  applies. Python relies on the same inference.
- **Wispr's device switching** (ranking, the device appearing and disappearing around a
  hold, `LEAD_MS` and `TAIL_MS`) wasn't exercised. Only the device itself was.
- **Only the built-in speakers were tested as the clock/sub-device.** Bluetooth outputs
  (AirPods: sample-rate switches, HFP) and a default output that changes while the device
  exists are untested. `measure()` refuses to play if the default output changed since
  the device was made.
- **Only Apple Silicon, macOS 27.2, and a terminal with the permission already granted were
  tested.** The first-run prompt path, a denied permission and macOS 14.2–15 weren't
  exercised. The code handles them: a silent tap fails the probe, and macOS < 14.2 or a
  missing class is reported.
- **The cpal output stream doesn't stream yet.** It plays a pre-rendered buffer. The real
  port still needs `wispr_mic.py`'s jitter buffer, sequencer and resampler, and the "open
  only around a hold" lifecycle in tap mode.
- **Toolchain:** Homebrew's rustup isn't on `PATH`, and its `stable` toolchain reports
  "missing manifest" for `rustc --version`, though builds still worked. The pinned `1.99.0`
  toolchain is complete. Nothing was installed or changed.
