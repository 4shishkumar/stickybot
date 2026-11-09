# StickyBot desktop (Rust + Tauri 2)

The rewrite of StickyBot's Mac app (`mac-agent/`, Python) in Rust. **Phase 1** runs *next to* the
Python agent and replaces nothing: its own port (**8670**, Python: 8660), its own folder
(**`~/.stickybot-rs/`**, never `~/.stick-agent/`), its own bundle id (**`com.stickybot.rs`**).

What it does today: the Bluetooth link to the stick, the dashboard (the Python agent's own page)
served from Rust, a menu bar app, and (Phase 2) **hold B to talk** (on-device speech to text, quick
commands, Claude Code streamed to the stick), Claude Code **approvals** on the stick, **sound clips**
and the **firmware update over Bluetooth**.

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
```

Environment: `STICKYBOT_RS_PORT` (8670), `STICKYBOT_RS_HOME` (`~/.stickybot-rs`),
`STICKYBOT_RS_FAKE_STICK=1`, `STICKYBOT_RS_PYTHON_PORT` (8660), `STICKYBOT_RS_ALLOW_SHARED=1`
(see *Safety*), `RUST_LOG=debug` (every message from the stick is logged at debug),
`STICKYBOT_RS_MODELS` (`<home>/models`, the speech model), `STICKYBOT_RS_NO_STT=1` (don't download /
load it), `STICKYBOT_RS_FIRMWARE` (a firmware bundle folder), `STICKYBOT_RS_SOUNDS`
(`<home>/sounds`, sound packs).

The first start downloads the speech model (Parakeet TDT 0.6b v3 int8 ONNX, 487 MB, from
sherpa-onnx's GitHub release; SHA-256 checked, resumable) into `~/.stickybot-rs/models/` and
loads it in ~0.7 s; `GET /api/speech` and `speech` events show the progress.

## Layout

| Crate | What |
|---|---|
| `crates/stick-link` | The Bluetooth link (port of `link.py`). `proto.rs`: the wire format, **no I/O** (compact ASCII JSON lines, 180-byte writes, line decoder, hello / `proto` / sound messages, the "Insufficient Resource" retry rule). `link.rs`: reconnect loop, hello every 8 s, `@…` requests answered by id (5 s), message + audio streams (and both in order: `incoming()`), clips (`send_audio`), OTA packets, pause, re-target, a connect gate. `adpcm.rs`: the stick's IMA-ADPCM frames. `ota.rs`: the windowed firmware transfer (`ota.py`). `corebt.rs`: CoreBluetooth through `objc2-core-bluetooth`, with write-without-response pacing. `fake.rs`: a stick in memory (answers `@state`, `@keys`, settings, sounds; holds B with any audio; plays clips; takes firmware updates; test knobs for busy / silent / absent / drops / lost packets / rollback). `examples/scan.rs`. |
| `crates/stickybot-server` | axum: the dashboard page + `static/` from `mac-agent/stick_agent/` (rust-embed: read from disk in debug builds, embedded in release), the API slice, config + token in `~/.stickybot-rs/`, the SSE event feed, the Python-agent check. `talk.rs` (hold B: core.py's flow), `speech.rs` (model download + STT worker), `commands.rs` (quick commands), `brain.rs` (Claude Code session), `approvals.rs`, `clips.rs`, `firmware.rs`. Bin `stickybot-headless`. Test `tests/talk_e2e.rs`. |
| `crates/stickybot-app` | Tauri 2 shell: tray icon (status line, Open dashboard, Open in browser, Quit), webview window on `http://localhost:8670/`, no Dock icon (`ActivationPolicy::Accessory` + `LSUIElement`). |
| `crates/macos-audio` | Phase-0 spike: the wireless-mic Core Audio tap + aggregate device (`spikes/TAP.md`). |
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
`/api/wireless-mic/*`, `/api/mcp*` and `/mcp`, `/api/brain*`, `/api/update*`, `/api/components*`,
`/api/restart`, `/api/window`.

How the pages degrade: the **Stick** page works except the Face card's pet line and USB flashing.
Overview, Activity, Quick commands and Sounds (clips) work. Events, Logs, Settings → Brain (the
OpenRouter part) / wireless mic show their "not available" states. With no stick picked, the setup wizard opens as in the Python app;
its *Pick your stick* step scans and saves the stick to **this** app's config.

## What's ported (Phase 2)

| Feature | Endpoints / notes |
|---|---|
| **Hold B to talk** | `evt: talk` start → ADPCM frames → stop; decoded (`adpcm.rs`), transcribed on this Mac (sherpa-onnx Parakeet, word hints `stt_hints`, default `["Tauri", "StickyBot", "Claude"]`: "Tory" → "Tauri"), then a "new chat" phrase, a quick command, or Claude. The stick's screens as core.py sends them: `thinking` / `working` (throttled to `stream_interval_ms`) / `done` / `error` / `cancelled`, with "Too short - hold B while you speak.", "Speech model still loading…", "Didn't catch that.", "Still working on the last request.", "Something went wrong: …". Stick-side `evt: cancel` stops Claude. `history.jsonl`, `heard` / `reply` / `progress` / `state` events. `GET /api/history`, `POST /api/ask` (typed), `GET`/`PUT /api/commands`, `POST /api/commands/test`, `GET /api/speech`, `POST /api/speech/retry`, `POST /api/new-chat`. |
| **Claude brain** | `brain.py`'s session mode: one `claude -p --input-format stream-json` process, prewarmed when the stick connects, interrupt to cancel, `--resume` after a crash / settings change, idle stop, 240 s timeout. Status in `/api/status` `brain`. Not yet: one-shot mode, the stick tools (MCP), OpenRouter. |
| **Approvals** | `POST /api/permission` (the hook's long-poll), `POST /api/permission/decide`; the prompt on the stick every 3 s, A / B answers, `prompt` / `prompt_done` events, `status.prompts`. The hook still points at the Python agent (:8660); see *Testing*. |
| **Sound clips** | `GET /api/clips`, `POST /api/clips/play`: packs in `~/.stickybot-rs/sounds/<pack>/` (ogg, wav, flac, mp3, aiff), made loud, AUDIO_IN packets (first / last acknowledged, the rest paced writes without response). |
| **Firmware over Bluetooth** | `GET /api/firmware` (bundle + connected stick; `usb` always empty), `POST /api/firmware/ota` `{force}`, `GET /api/firmware/jobs/{id}` (SSE), `POST /api/firmware/jobs/{id}/cancel`. Bundles in `~/.stickybot-rs/firmware/<version>|local/` or `$STICKYBOT_RS_FIRMWARE`. `ota.py`'s checks (support, pending, versions / force, manifest SHA-256), windowed transfer, resume, then waits for the stick to come back with the new version (confirmed / rolled back). |

**Write-without-response pacing** (`corebt.rs`): before each packet on AUDIO_IN / OTA_DATA,
`canSendWriteWithoutResponse` is checked and the packet queued in one block on the Bluetooth
queue; with no room, it waits for `peripheralIsReadyToSendWriteWithoutResponse:` (2 s at most,
then sends anyway, like link.py; the protocols cope with a lost packet).

### Not done in the link yet

- The hello carries only `t` / `tz` (no home card, weather, `world`, `wmic`, `wready`), so the stick
  never streams Wispr audio to this app (`evt: wispr` is ignored).
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

The Rust app's identity is `com.stickybot.rs`, so its Bluetooth permission is separate from
StickyBot.app's. The bundle is signed ad hoc (Tauri's default): **every rebuild is a new code
identity, and macOS asks for Bluetooth again**. See *Open decisions*.

## Next

1. **Wireless Wispr mic**: `macos-audio` into the link: `wmic` / `wready` in the hello, `prepare` →
   device → `ready`, the output stream + jitter buffer (`wispr_mic.py`), USB-wins, cleanup (waits
   on a user test).
2. Brain: one-shot mode, the stick tools (MCP server), OpenRouter; questions from a session
   (`ask_user`); speech output.
3. Events / GitHub / pet / world / home card / weather in the hello; logs page; onboarding /
   install / updates (release downloads for firmware bundles); USB serial + esptool flashing.
4. Packaging: a stable signing identity, login item, and the switch-over from the Python app.

## Open decisions

- **Signing identity.** TCC (Bluetooth, System Audio Recording, microphone) is tied to the code
  signature; ad-hoc signing means re-prompts after every build. Options: a Developer ID (best,
  costs the Apple program), or a self-signed certificate in the login keychain used for all local
  builds (`bundle.macOS.signingIdentity`).
- **Bundle id at switch-over**: keep `com.stickybot.rs` or take over `com.stickybot.app` (the
  Python app's), which matters for existing permissions and the login item.
- **Serving `static/`**: Phase 1 embeds the Python app's files (single source of truth while both
  apps live). Later: move them to a shared folder, or into the Rust app for good.
- **STT default**: sherpa-onnx now (spike verdict); FluidAudio (ANE) as the upgrade once the Swift
  build step is in place.
