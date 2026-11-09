# StickyBot desktop (Rust + Tauri 2)

The rewrite of StickyBot's Mac app (`mac-agent/`, Python) in Rust. **Phase 1** runs *next to* the
Python agent and replaces nothing: its own port (**8670**, Python: 8660), its own folder
(**`~/.stickybot-rs/`**, never `~/.stick-agent/`), its own bundle id (**`com.stickybot.rs`**).

What it does today: the Bluetooth link to the stick, the dashboard (the Python agent's own page)
served from Rust, a menu bar app, and (Phase 2) **hold B to talk** (on-device speech to text, quick
commands, Claude Code streamed to the stick), Claude Code **approvals** on the stick, **sound clips**,
the **firmware update over Bluetooth** and the **wireless Wispr mic** (hold A: the stick's mic reaches
Wispr Flow over Bluetooth, through a Core Audio tap).

## Run it

Needs Rust (stable, edition 2024) and, for the app bundle, the Tauri CLI
(`cargo install tauri-cli --version "^2" --locked`).

```sh
cd desktop

# the dashboard + link without the menu bar app, with a stick in memory (no Bluetooth at all)
cargo run -p stickybot-server --bin stickybot-headless -- --fake-stick
open http://localhost:8670/#stick

# the menu bar app (tray icon → Open dashboard); add STICKYBOT_RS_FAKE_STICK=1 for the fake stick
cd crates/stickybot-app && cargo tauri dev

# the bundle: target/release/bundle/macos/StickyBot (Rust).app
cd crates/stickybot-app && cargo tauri build --bundles app

cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stick-link --example scan        # read-only scan for sticks (never connects)

# the wireless mic end to end with real Core Audio (a temporary tap + aggregate, inaudible; see Testing)
STICKYBOT_RS_AUDIO_E2E=1 cargo test -p stickybot-server --test wmic_e2e -- --nocapture
```

Environment: `STICKYBOT_RS_PORT` (8670), `STICKYBOT_RS_HOME` (`~/.stickybot-rs`),
`STICKYBOT_RS_FAKE_STICK=1`, `STICKYBOT_RS_PYTHON_PORT` (8660), `STICKYBOT_RS_ALLOW_SHARED=1`
(see *Safety*), `RUST_LOG=debug` (every message from the stick is logged at debug),
`STICKYBOT_RS_MODELS` (`<home>/models`, the speech model), `STICKYBOT_RS_NO_STT=1` (don't download /
load it), `STICKYBOT_RS_FIRMWARE` (a firmware bundle folder), `STICKYBOT_RS_SOUNDS`
(`<home>/sounds`, sound packs), `STICKYBOT_RS_AUDIO_E2E=1` (runs the wireless-mic test against real
Core Audio).

The first start downloads the speech model (Parakeet TDT 0.6b v3 int8 ONNX, 487 MB, from
sherpa-onnx's GitHub release; SHA-256 checked, resumable) into `~/.stickybot-rs/models/` and
loads it in ~0.7 s; `GET /api/speech` and `speech` events show the progress.

## Layout

| Crate | What |
|---|---|
| `crates/stick-link` | The Bluetooth link (port of `link.py`). `proto.rs`: the wire format, **no I/O** (compact ASCII JSON lines, 180-byte writes, line decoder, hello / `proto` / sound messages, the "Insufficient Resource" retry rule). `link.rs`: reconnect loop, hello every 8 s, `@…` requests answered by id (5 s), message + audio streams (and both in order: `incoming()`), clips (`send_audio`), OTA packets, pause, re-target, a connect gate. `adpcm.rs`: the stick's IMA-ADPCM frames. `ota.rs`: the windowed firmware transfer (`ota.py`). `corebt.rs`: CoreBluetooth through `objc2-core-bluetooth`, with write-without-response pacing. `fake.rs`: a stick in memory (answers `@state`, `@keys`, settings, sounds; holds B with any audio; plays clips; takes firmware updates; test knobs for busy / silent / absent / drops / lost packets / rollback). `examples/scan.rs`. |
| `crates/stickybot-server` | axum: the dashboard page + `static/` from `mac-agent/stick_agent/` (rust-embed: read from disk in debug builds, embedded in release), the API slice, config + token in `~/.stickybot-rs/`, the SSE event feed, the Python-agent check. `talk.rs` (hold B: core.py's flow), `speech.rs` (model download + STT worker), `commands.rs` (quick commands), `brain.rs` (Claude Code session), `approvals.rs`, `clips.rs`, `firmware.rs`, `wmic.rs` (the wireless Wispr mic: core.py's wispr parts + `Holds`). Bin `stickybot-headless`. Tests `tests/talk_e2e.rs`, `tests/wmic_e2e.rs`. |
| `crates/stickybot-app` | Tauri 2 shell: tray icon (status line, Open dashboard, Open in browser, Quit), webview window on `http://localhost:8670/`, no Dock icon (`ActivationPolicy::Accessory` + `LSUIElement`). |
| `crates/macos-audio` | The wireless mic's audio side (`spikes/TAP.md`). Plain Rust, tested anywhere: `carrier.rs` (`pick_carrier`: tap / BlackHole), `device.rs` (`StickMicDevice`: the aggregate + tap, `recording()`, the -80 dBFS check, cleanup by *our* UID only), `jitter.rs` (sequencer, jitter buffer, resampler), `player.rs` (`WisprPlayer`: the stick's audio into the carrier, gated on the tap), `fake.rs` (Core Audio and outputs in memory), `naming.rs`. macOS: `system.rs` (the HAL + cpal backends), `hal.rs`, `tap.rs`, `mic.rs` (the live-object registry). `examples/tapmic.rs` (the spike's self-check). |
| `crates/stt` | Speech to text: sherpa-onnx Parakeet TDT 0.6b v3 with word hints (hotwords) by default (`spikes/STT.md`; whisper behind `--features whisper`, FluidAudio behind `fluid`). `examples/hotwords.rs`. |

### Why CoreBluetooth via objc2, not btleplug

btleplug 0.11.8 (the current release) is the obvious choice, but on macOS its
`peripheral:didWriteValueForCharacteristic:error:` only reports *successful* writes: a write that
fails (e.g. ATT 0x11 "Insufficient Resources", which the stick returns while a long reply is going
out; link.py retries it) never completes. It also doesn't expose `canSendWriteWithoutResponse` /
`peripheralIsReadyToSendWriteWithoutResponse` or `maximumWriteValueLength`, which the Bluetooth
firmware update and sound clips need for pacing (Phase 2). Our own CoreBluetooth layer is ~500
lines with the delegate in `define_class!`, all calls on one serial dispatch queue, and gives the
ATT error code directly (`CBATTErrorDomain` code 17). It's behind the `Connector` / `Connection`
traits, so the link, the API and all tests run against the fake.

The central manager is created lazily (first scan or first connect), so nothing asks macOS for
Bluetooth until it's needed.

## What's ported (Phase 1)

| Endpoint | Notes |
|---|---|
| `GET /`, `GET /static/*` | the Python agent's `dashboard.html` (token filled in, title "StickyBot (Rust)") and `static/` unchanged |
| `GET /api/status` | the Python shape; unported parts are `null`/empty (`brain`, `wireless_mic`, `github`, `weather`…), plus `held_back` |
| `GET /api/events` | SSE: catch-up of the last 150, `link` / `state` / `config` / `sounds` events, keep-alive every 15 s |
| `GET` / `PUT /api/config` | the Python `DEFAULT_CONFIG` keys and checks (brain, OpenRouter model / max tokens, the stick's shape); unknown keys dropped; a new `stick` re-targets the link |
| `POST /api/stick/proto` | `{"line":"@state"}` → `{"ok","via":"ble","reply","error"}` over Bluetooth (USB serial fallback not yet) |
| `GET /api/scan` | sticks advertising the service, strongest first |
| `POST /api/pause` | pause / resume the link |
| `GET` / `PUT /api/sounds`, `POST /api/sounds/play` | the stick's sound settings (`{"sound":…}` → `evt: sounds`, 3 s) |
| `GET /api/rust` | this app: version, home, `ported`, `not_ported` |

Guard as in `server.py`: only `Host: localhost:8670` / `127.0.0.1:8670`, and every non-GET needs
`X-Stick-Token` (from `~/.stickybot-rs/token`, 0600; the page has it built in).

**Not ported yet** (they answer a JSON 404, which the page already shows as "not available in this
version"; `GET /api/rust` lists them): `/api/pet`, `/api/firmware/flash` (USB), `/api/onboarding`
(the wizard keeps its progress in the browser meanwhile), `/api/install*`, `/api/logs`,
`/api/voices`, `/api/voice/test`, `/api/events/test|history|kinds`, `/api/claude-event`,
`/api/mcp*` and `/mcp`, `/api/brain*`, `/api/update*`, `/api/components*`,
`/api/restart`, `/api/window`.

How the pages degrade: the **Stick** page works except the Face card's pet line and USB flashing.
Overview, Activity, Quick commands, Sounds (clips) and Settings → Wireless mic work. Events, Logs,
Settings → Brain (the OpenRouter part) show their "not available" states; the setup page's wireless-mic
step (`/api/install`) isn't ported (use Settings). With no stick picked, the setup wizard opens as in the Python app;
its *Pick your stick* step scans and saves the stick to **this** app's config.

## What's ported (Phase 2)

| Feature | Endpoints / notes |
|---|---|
| **Hold B to talk** | `evt: talk` start → ADPCM frames → stop; decoded (`adpcm.rs`), transcribed on this Mac (sherpa-onnx Parakeet, word hints `stt_hints`, default `["Tauri", "StickyBot", "Claude"]`: "Tory" → "Tauri"), then a "new chat" phrase, a quick command, or Claude. The stick's screens as core.py sends them: `thinking` / `working` (throttled to `stream_interval_ms`) / `done` / `error` / `cancelled`, with "Too short - hold B while you speak.", "Speech model still loading…", "Didn't catch that.", "Still working on the last request.", "Something went wrong: …". Stick-side `evt: cancel` stops Claude. `history.jsonl`, `heard` / `reply` / `progress` / `state` events. `GET /api/history`, `POST /api/ask` (typed), `GET`/`PUT /api/commands`, `POST /api/commands/test`, `GET /api/speech`, `POST /api/speech/retry`, `POST /api/new-chat`. |
| **Claude brain** | `brain.py`'s session mode: one `claude -p --input-format stream-json` process, prewarmed when the stick connects, interrupt to cancel, `--resume` after a crash / settings change, idle stop, 240 s timeout. Status in `/api/status` `brain`. Not yet: one-shot mode, the stick tools (MCP), OpenRouter. |
| **Approvals** | `POST /api/permission` (the hook's long-poll), `POST /api/permission/decide`; the prompt on the stick every 3 s, A / B answers, `prompt` / `prompt_done` events, `status.prompts`. The hook still points at the Python agent (:8660); see *Testing*. |
| **Sound clips** | `GET /api/clips`, `POST /api/clips/play`: packs in `~/.stickybot-rs/sounds/<pack>/` (ogg, wav, flac, mp3, aiff), made loud, AUDIO_IN packets (first / last acknowledged, the rest paced writes without response). |
| **Wireless Wispr mic** | Hold A away from USB: the stick streams its mic (`prepare` → ADPCM → `stop`); this app makes the **`<stick>-bt-mic`** device (the Python app's name, `stick_mic_device.py`'s slug rules, so Wispr's ranking finds it; UID **`com.stickkey.rust-wireless-mic`**, never the Python agent's `com.stickkey.wireless-mic`), waits `wispr_switch_ms` (250) after creating it, answers `ready`, plays the frames (in order by seq, a lost one = 20 ms of silence, resampled, ~80 ms jitter buffer) into the carrier, keeps the device `wispr_tail_ms` (800) after `stop` (≤ 4 s more while Wispr still reads it), then removes it. `wmic` / `wready` in the hello. Carrier as `wireless_mic_carrier` (`auto`: the tap once its inaudible -80 dBFS check passed, else BlackHole 2ch; `tap`; `blackhole`). The stick on USB (`evt: usb`): no device, `ready` with `"device": false` (Wispr uses the stick's USB mic). `POST /api/wireless-mic/show` `{secs}`, `POST /api/wireless-mic/check`, `status.wireless_mic`, `wispr_mic` / `stick_mic` events. Cleanup of *our UID only* at start, on quit (Tauri `Exit`, headless Ctrl-C) and on disconnect / USB / off / rename. |
| **Firmware over Bluetooth** | `GET /api/firmware` (bundle + connected stick; `usb` always empty), `POST /api/firmware/ota` `{force}`, `GET /api/firmware/jobs/{id}` (SSE), `POST /api/firmware/jobs/{id}/cancel`. Bundles in `~/.stickybot-rs/firmware/<version>|local/` or `$STICKYBOT_RS_FIRMWARE`. `ota.py`'s checks (support, pending, versions / force, manifest SHA-256), windowed transfer, resume, then waits for the stick to come back with the new version (confirmed / rolled back). |

**Write-without-response pacing** (`corebt.rs`): before each packet on AUDIO_IN / OTA_DATA,
`canSendWriteWithoutResponse` is checked and the packet queued in one block on the Bluetooth
queue; with no room, it waits for `peripheralIsReadyToSendWriteWithoutResponse:` (2 s at most,
then sends anyway, like link.py; the protocols cope with a lost packet).

### Not done in the link yet

- The hello carries `t` / `tz` / `wmic` / `wready` (no home card, weather, `world`).
- The stick renamed on the stick itself (the `@state` name check on connect) doesn't rename the mic
  yet; picking the stick again in the dashboard does.
- Speech output (reading replies aloud), USB serial for `@…` lines, USB flashing.

## Safety: one stick, one connection

The stick takes one Bluetooth central, and the Python agent normally has it. So:

- A fresh `~/.stickybot-rs/` has **no stick picked**: the app never connects (and never touches
  Bluetooth) until you pick one in its dashboard.
- Before **every** connect attempt the link asks the Python agent's `GET /api/status` on :8660
  (HTTP only; no files, no token). While its link is `connected`, `searching` or `starting`, this
  app waits (status: "Waiting: the Python agent … has the stick") and asks again every 3 s.
  `STICKYBOT_RS_ALLOW_SHARED=1` turns that off (don't).
- Scanning is read-only and fine at any time.

## Testing

`cargo test --workspace` never touches audio hardware: the wireless mic's tests run on the fake
Core Audio / outputs (`macos-audio/src/fake.rs`) and the fake stick (`FakeStick::wispr_hold` holds A
like firmware 1.7.1+: `prepare`, frames in real time, the key on `ready` or after 800 ms, the 250 ms
tail, `stop`). `tests/wmic_e2e.rs` (opt-in, `STICKYBOT_RS_AUDIO_E2E=1`) does it for real: `say` →
ADPCM → the fake stick → the app → a real tap + aggregate (Rust UID, the test name
`stickybot-rs-e2e-mic` so Wispr's ranking doesn't switch to it) → the test reads the device's input
and compares it with what was said (10 ms envelopes), then checks the device and tap counts are back
to what they were. It needs System Audio Recording for the terminal; without it, it stops after the
inaudible check. Last run (MacBook Pro, macOS 27.2, Bluetooth earbuds as the default output): check
OK in ~0.4 s, key down 354 ms after `prepare`, 297/297 voiced 10 ms blocks heard, envelope
correlation 0.99, 0 lost, 0 gated, kept 953 ms after the stream, devices 8/8 and taps 0/0
before/after.

## Testing against the real stick

1. Build the bundle (`cargo tauri build --bundles app` in `crates/stickybot-app`) and open
   `target/release/bundle/macos/StickyBot (Rust).app`. (Running the debug binary from a terminal
   works too; then Bluetooth permission goes to the terminal app.)
2. **Pause the Python agent** (not stop it, no launchctl): its dashboard
   <http://localhost:8660> → Overview → **Pause**. Its link goes to `paused` and it disconnects.
3. Rust dashboard: menu bar icon → **Open dashboard** (or <http://localhost:8670>). The setup
   wizard opens → *Pick your stick* → **Use this one**. macOS asks for Bluetooth the first time
   ("StickyBot (Rust)"). Or copy the Python agent's pick:
   `curl -s localhost:8660/api/config | jq .stick` and save it with
   `curl -X PUT localhost:8670/api/config -H "X-Stick-Token: $(cat ~/.stickybot-rs/token)" -H 'Content-Type: application/json' -d '{"stick": {"address": "…", "name": "…"}}'`.
4. Within a few seconds the menu bar line says "<name>: connected" and the stick shows *agent
   ready* (the hello). Check: Stick page loads (`@state`), change Brightness / a gesture toggle /
   Face → the stick changes; *Test combo*; Sounds volume; Refresh; Overview → Pause / Resume;
   `RUST_LOG=debug` shows every message from the stick. Try a long reply under load (e.g.
   *Refresh* repeatedly) to exercise the "Insufficient Resource" retry.
5. **Hand it back**: Rust dashboard → Overview → **Pause** (or Quit the app), then the Python
   dashboard → **Resume**. The Python agent reconnects on its own.

### Phase 2 checks with the real stick (steps 1-3 above first; the speech model downloads on the first start)

- **Talk**: hold B, say "open pull request twelve thirty four" (after setting a repo on the Quick
  commands page) → the stick shows *Opened GitHub PR #1234* and the browser opens it. Hold B, ask
  "what's the weather in Paris?" → *Thinking…*, *working* updates as Claude writes, then the answer.
  Press B briefly without speaking → the screen just closes. Say "Tauri" → Activity shows "Tauri".
  Hold B on a long question and cancel on the stick → nothing more is shown. `RUST_LOG=info` logs
  `heard (… s audio, … s, N lost): …` — check `lost` stays 0 near the Mac.
- **Clips**: copy a sound pack (e.g. Kenney Interface Sounds `.ogg` files) to
  `~/.stickybot-rs/sounds/<pack>/`, Sounds page → play one. It should play whole, without
  stutter; `clip …: X s of audio sent in Y s` in the log.
- **Firmware over Bluetooth** (the stick on battery ≥ 30 % or USB power; ≥ 1.8.0):
  `make -C m5-stick-s3 release` makes a bundle in `~/.stick-agent/firmware/local`; copy that folder
  to `~/.stickybot-rs/firmware/local` (or point `STICKYBOT_RS_FIRMWARE` at it). Stick page →
  Firmware → update over Bluetooth (force for the same version). Watch KB/s and `resent` in the log;
  try walking away mid-transfer (it resumes) and B on the stick (cancels, nothing changes).
- **Approvals**: point a Claude Code hook at this app (`curl -X POST localhost:8670/api/permission
  -H "X-Stick-Token: $(cat ~/.stickybot-rs/token)" -d '{"tool":"Bash","hint":"ls","wait":30}'`
  simulates it) → A on the stick answers `{"decision":"allow"}`, B `deny`.

### Wireless Wispr mic checks with the real stick (steps 1-3 above first)

The Python agent, paused, has no device; its own device (UID `com.stickkey.wireless-mic`) and this
app's (`com.stickkey.rust-wireless-mic`) never clean up each other's. Both are named `<stick>-bt-mic`,
so Wispr's ranking needs no change.

1. **Turn it on**: Rust dashboard → Settings → *Wireless mic* on, *Carried by* Automatic → Save. The
   inaudible tap check runs (-80 dBFS through a `…-bt-mic-check` device). The bundled app asks for
   **System Audio Recording** for "StickyBot (Rust)" the first time (a debug binary run from a
   terminal uses the terminal's permission). If Settings says the tap isn't allowed yet: System
   Settings → Privacy & Security → Screen & System Audio Recording → System Audio Recording Only →
   StickyBot (Rust), then *Check again*. `curl -s localhost:8670/api/status | jq .wireless_mic` →
   `enabled: true`, `carrier: "tap"`, `tap.state: "ok"`, `unavailable: ""`. (BlackHole 2ch works too:
   *Carried by* BlackHole.)
2. **Wispr**: nothing to change if `<stick>-bt-mic` is already ranked first (the Python app's name).
   Otherwise Settings → *Show the mic for 60 s* and drag it to the top in Wispr → Settings → Microphone.
3. **Hold A on battery** (unplugged from USB), start speaking right at the press, release → Wispr types
   the whole sentence, the first word included, and **nothing comes out of the speakers or
   headphones**. With `RUST_LOG=info`: `stick mic: <name> created in … ms`, `ready … ms after
   prepare (… lead 250 ms)`, `Wispr opened it … ms after ready`, `wispr stream: … 0 lost … 0
   underruns`, `Wispr closed it …`, `removed … ms after the stream ended`. Settings → Wireless mic
   shows the same timings. If the first word is cut: raise *Switch time* (`wispr_switch_ms`) to 400.
4. **Again within the tail** (press A within ~1 s of releasing): `ready` at once (no lead), the device
   isn't made twice.
5. **Bluetooth headphones** as the default output (AirPods…): hold A again. The log says the device is
   clocked by `BuiltInSpeakerDevice` (see *Wireless mic and Bluetooth output* below); Wispr types it,
   nothing in the headphones or the speakers.
6. **USB**: plug the stick into the Mac → `the stick is on USB`; hold A → Wispr uses `<stick>-usb-mic`
   and no `<stick>-bt-mic` appears (Audio MIDI Setup). Unplug → the Bluetooth mic again.
7. **Cleanup**: after each hold and after Quit, Audio MIDI Setup lists no `<stick>-bt-mic`, and
   `cargo run -p macos-audio --example tapmic -- --list` shows no `com.stickkey.rust-wireless-mic`
   device or tap. Kill the app hard mid-hold (`kill -9`), start it again → the log says it removed
   the left-over device.
8. Turn *Wireless mic* off before handing the stick back (the Python agent's Wispr mic keeps working
   as before).

#### Wireless mic and Bluetooth output

`stick_mic_device.py` clocks the tap's aggregate by the default output and plays the stick into it.
With Bluetooth headphones as the default output that fails on macOS 27: the aggregate's input never
runs (0 callbacks), and audio played into Bluetooth reaches the tap only after ~1.1 s. So this app
uses the default output unless it's Bluetooth, and then the **built-in output** for both (measured:
the -80 dBFS probe heard after 21 ms, the full sentence in `wmic_e2e`). The tap mutes this app's sound
on every device, and the check runs through exactly that path before anything is played.

The Rust app's identity is `com.stickybot.rs`, so its Bluetooth permission is separate from
StickyBot.app's. The bundle is signed ad hoc (Tauri's default): **every rebuild is a new code
identity, and macOS asks for Bluetooth again**. See *Open decisions*.

## Next

1. Wireless mic leftovers: the setup page's step (`/api/install`), the `@state` rename check, a
   panic hook that removes the device (today: the next start does).
2. Brain: one-shot mode, the stick tools (MCP server), OpenRouter; questions from a session
   (`ask_user`); speech output.
3. Events / GitHub / pet / world / home card / weather in the hello; logs page; onboarding /
   install / updates (release downloads for firmware bundles); USB serial + esptool flashing.
4. Packaging: a stable signing identity, login item, and the switch-over from the Python app.

## Open decisions

- **Signing identity.** TCC (Bluetooth, System Audio Recording, microphone) is tied to the code
  signature; ad-hoc signing means re-prompts after every build (and an ad-hoc build's tap check
  fails until System Audio Recording is granted again: the wireless mic then says so and plays nothing). Options: a Developer ID (best,
  costs the Apple program), or a self-signed certificate in the login keychain used for all local
  builds (`bundle.macOS.signingIdentity`).
- **Bundle id at switch-over**: keep `com.stickybot.rs` or take over `com.stickybot.app` (the
  Python app's), which matters for existing permissions and the login item.
- **Serving `static/`**: Phase 1 embeds the Python app's files (single source of truth while both
  apps live). Later: move them to a shared folder, or into the Rust app for good.
- **STT default**: sherpa-onnx now (spike verdict); FluidAudio (ANE) as the upgrade once the Swift
  build step is in place.
