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
# ... signed with a stable identity (permissions survive rebuilds): see SWITCHOVER.md
STICKYBOT_SIGNING_IDENTITY=<sha1> scripts/build-app.sh

cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stick-link --example scan        # read-only scan for sticks (never connects)

# the wireless mic end to end with real Core Audio (a temporary tap + aggregate, inaudible; see Testing)
STICKYBOT_RS_AUDIO_E2E=1 cargo test -p stickybot-server --test wmic_e2e -- --nocapture
```

Environment: `STICKYBOT_RS_PORT` (8670), `STICKYBOT_RS_HOME` (`~/.stickybot-rs`),
`STICKYBOT_RS_FAKE_STICK=1`, `STICKYBOT_RS_PYTHON_PORT` (8660), `STICKYBOT_RS_ALLOW_SHARED=1`
(see *Safety*), `RUST_LOG=debug` (every message from the stick is logged at debug),
`STICKYBOT_RS_MODELS` (`<home>/models`, the speech model), `STICKYBOT_RS_NO_STT=1` (don't download /
load it), `STICKYBOT_RS_FIRMWARE` (a firmware bundle folder), `STICKYBOT_RS_REPO` (`ashish-kumar-noon/stickybot`, where releases come from), `STICKYBOT_RS_ESPTOOL` (the esptool to flash with; else `esptool` on PATH / Homebrew, else `python3 -m esptool`), `STICKYBOT_RS_SOUNDS`
(`<home>/sounds`, sound packs), `STICKYBOT_RS_IMPORT_FROM` (`~/.stick-agent`, read-only: what
*Import from StickyBot* copies), `STICKYBOT_RS_AUDIO_E2E=1` (runs the wireless-mic test against real
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
| `GET /api/status` | the Python shape (`weather`, `github` as below); unported parts are `null`/empty, plus `held_back` |
| `GET /api/events` | SSE: catch-up of the last 150, `link` / `state` / `config` / `sounds` events, keep-alive every 15 s |
| `GET` / `PUT /api/config` | the Python `DEFAULT_CONFIG` keys and checks (brain, OpenRouter model / max tokens, the stick's shape); unknown keys dropped; a new `stick` re-targets the link |
| `POST /api/stick/proto` | `{"line":"@state"}` → `{"ok","via":"ble","reply","error"}` over Bluetooth; when Bluetooth isn't there or doesn't answer, over the stick's USB serial (`via: "usb"`, `usb.rs`: Espressif ports, VID 0x303A, 115200 baud, the `@{…}` reply line) |
| `GET /api/scan` | sticks advertising the service, strongest first |
| `POST /api/pause` | pause / resume the link |
| `GET` / `PUT /api/sounds`, `POST /api/sounds/play` | the stick's sound settings (`{"sound":…}` → `evt: sounds`, 3 s) |
| `GET /api/rust` | this app: version, home, `ported`, `not_ported` |

Guard as in `server.py`: only `Host: localhost:8670` / `127.0.0.1:8670`, and every non-GET needs
`X-Stick-Token` (from `~/.stickybot-rs/token`, 0600; the page has it built in).

**Not ported yet** (they answer a JSON 404, which the page already shows as "not available in this
version"; `GET /api/rust` lists them): `/api/onboarding`
(the wizard keeps its progress in the browser meanwhile), `/api/install*`, `/api/brain*` (until
the OpenRouter brain lands), `/api/components*`, `/api/restart`, `/api/window`, `POST /api/update/agent`
(answers 409: no self-update).

How the pages degrade: every page works (Stick incl. USB flashing, Overview, Activity, Events, Logs,
Quick commands, Sounds, Settings incl. the stick tools and voices) except Settings → Brain's
OpenRouter part, which shows its "not available" state; the setup page's wireless-mic
step (`/api/install`) isn't ported (use Settings). With no stick picked, the setup wizard opens as in the Python app;
its *Pick your stick* step scans and saves the stick to **this** app's config.

## What's ported (Phase 2)

| Feature | Endpoints / notes |
|---|---|
| **Hold B to talk** | `evt: talk` start → ADPCM frames → stop; decoded (`adpcm.rs`), transcribed on this Mac (sherpa-onnx Parakeet, word hints `stt_hints`, default `["Tauri", "StickyBot", "Claude"]`: "Tory" → "Tauri"), then a "new chat" phrase, a quick command, or Claude. The stick's screens as core.py sends them: `thinking` / `working` (throttled to `stream_interval_ms`) / `done` / `error` / `cancelled`, with "Too short - hold B while you speak.", "Speech model still loading…", "Didn't catch that.", "Still working on the last request.", "Something went wrong: …". Stick-side `evt: cancel` stops Claude. `history.jsonl`, `heard` / `reply` / `progress` / `state` events. `GET /api/history`, `POST /api/ask` (typed), `GET`/`PUT /api/commands`, `POST /api/commands/test`, `GET /api/speech`, `POST /api/speech/retry`, `POST /api/new-chat`. |
| **Claude brain** | `brain.py`'s session mode: one `claude -p --input-format stream-json` process, prewarmed when the stick connects, interrupt to cancel, `--resume` after a crash / settings change, idle stop, 240 s timeout. **One-shot mode** (`brain_mode: "oneshot"`): `claude -p "<text>"` per request, `--resume` within `followup_minutes`, cancel kills it, no prewarm. Both get the stick tools (`--mcp-config <home>/mcp-voice.json`, 0600, the feedback tools pre-allowed) while `mcp_server` is on. Status in `/api/status` `brain` (`mode`, averages per mode). |
| **Stick tools (MCP)** | `mcp.rs` (mcp_server.py): `POST /mcp` (Streamable HTTP, JSON-RPC; legacy `initialize` clients and modern 2026-07-28 `_meta` + header clients, `server/discover`; bearer token or `X-Stick-Token` on every method, `Origin` check, 405 for GET / DELETE, 503 when off; `ask` as SSE with keep-alives when accepted, the client closing it clears the question; `notifications/cancelled`). The ten tools with mcp_server.py's schemas and checks: `notify` / `celebrate` (events.rs, rate limit 1/s + 20/min), `ask` (A / B / shake, `busy: …`), `progress` / `pin_home` (the home card, pushed ≤ every 0.5 s), `stick_status` (`{"cmd":"status"}`, the last press, at the desk), `speak`, `open_url` / `open_app` / `quick_command` (left to Claude Code's prompt; `call_approved` asks on the stick for brains without it). `GET /api/mcp`, `POST /api/mcp/terminal/install|uninstall` (`{"confirm":true}`: `claude mcp add --scope user stick http://127.0.0.1:<port>/mcp …`, behind a runner in tests), `POST /api/mcp/answer`. |
| **Questions from a session** | `approvals::ask_user` (core.py): the prompt message with `kind: "ask"` and the labels, re-sent every 3 s, an approval goes first, cleared on the stick however it ends; `status.asks`, `ask` / `ask_done` events, the home card shows "<from> asks". |
| **Reading replies aloud** | `speak.rs` (speech.py): `speak_replies` (+ `speak_stream`: sentence by sentence while Claude writes, `speak_max_words` cap), `say` → a clip on the stick's speaker (several sentences joined, waits for it to play) or `afplay` on the Mac (`speak_output`, the Mac when the stick is away); quick-command confirmations with `speak_quick`; holding B or cancel stops it. `GET /api/voices`, `POST /api/voice/test`. |
| **Approvals** | `POST /api/permission` (the hook's long-poll), `POST /api/permission/decide`; the prompt on the stick every 3 s, A / B answers, `prompt` / `prompt_done` events, `status.prompts`. The hook still points at the Python agent (:8660); see *Testing*. |
| **Sound clips** | `GET /api/clips`, `POST /api/clips/play`: packs in `~/.stickybot-rs/sounds/<pack>/` (ogg, wav, flac, mp3, aiff), made loud, AUDIO_IN packets (first / last acknowledged, the rest paced writes without response). |
| **Wireless Wispr mic** | Hold A away from USB: the stick streams its mic (`prepare` → ADPCM → `stop`); this app makes the **`<stick>-bt-mic`** device (the Python app's name, `stick_mic_device.py`'s slug rules, so Wispr's ranking finds it; UID **`com.stickkey.rust-wireless-mic`**, never the Python agent's `com.stickkey.wireless-mic`), waits `wispr_switch_ms` (250) after creating it, answers `ready`, plays the frames (in order by seq, a lost one = 20 ms of silence, resampled, ~80 ms jitter buffer) into the carrier, keeps the device `wispr_tail_ms` (800) after `stop` (≤ 4 s more while Wispr still reads it), then removes it. `wmic` / `wready` in the hello. Carrier as `wireless_mic_carrier` (`auto`: the tap once its inaudible -80 dBFS check passed, else BlackHole 2ch; `tap`; `blackhole`). The stick on USB (`evt: usb`): no device, `ready` with `"device": false` (Wispr uses the stick's USB mic). `POST /api/wireless-mic/show` `{secs}`, `POST /api/wireless-mic/check`, `status.wireless_mic`, `wispr_mic` / `stick_mic` events. Cleanup of *our UID only* at start, on quit (Tauri `Exit`, headless Ctrl-C) and on disconnect / USB / off / rename. |
| **Home screen, faces, events** | `home.rs` (home.py + core.py's home / world parts), `weather.rs`, `feed.rs`, `events.rs`, `github.rs`, `pet.rs`, `net.rs`. The hello carries `home` (the card: approvals waiting pinned, a session's `put` line, else every 16 s your last answer / Claude Code sessions open (`ps`, not counting this app's own child) / the feed / a greeting), `world` (PR counts, CI, Claude sessions from `claude_sessions.json`, approvals, review requests, the pet) and `wx` (Open-Meteo weather; `auto` = ipinfo.io / geojs.io, or a city), byte-compatible with core.py; `push_home` / `push_world` send them at once. Weather + feed refresh every minute when due (30 min), the GitHub watcher (`gh api graphql`, your login, every `github.interval_s`, first look records only, backoff to 30 min) turns PR changes into events and pet progress. Events: `{"src":"agent","event":{…}}` with levels from `event_rules`, quiet hours, `events.jsonl`, `event` feed entries, `event_shown` acks. `GET /api/pet`, `POST /api/events/test`, `GET /api/events/history`, `GET /api/events/kinds`, `POST /api/claude-event` (the Stop hook: the pet + "Claude finished in …" past `claude_done_min_s`); `status.weather`, `status.github`. State in `<home>/pet.json`, `github_state.json`, `events.jsonl` (the Python formats). Network and `gh` sit behind `net::Http` / `github::Gh`: tests (and the fake stick) use `FakeHttp` / `FakeGh`. API for the stick tools: `home::put` / `clear` / `push_home`, `events::notify`, `events::quiet_now`. Note: the Claude Code hooks still write `~/.stick-agent/claude_sessions.json` and post to :8660; this app reads `<home>/claude_sessions.json` (until the switch-over points the hooks here). |
| **Logs** | `GET /api/logs?n=` (300): the last lines of `~/.stickybot-rs/app.log` (`logs.rs`: every log line goes to stderr as before (`RUST_LOG`, default `info`), to `app.log` (rotated at 2 MB, `app.log.1`/`.2` kept, like the Python `RotatingFileHandler`) and to the live feed as `log` events, the Logs page's *Follow live*). |
| **Updates (firmware)** | `releases.rs` (releases.py): SemVer precedence, the latest release per channel (`update_channel` stable / beta), listed with `gh` when installed, else api.github.com over HTTPS; the release's firmware downloaded into `~/.stickybot-rs/firmware/<version>/` through a temporary folder, every part checked against the manifest's sha256 and GitHub's asset digest, 3 versions kept. `updates.rs` (updates.py): a check 20 s after start-up and every 24 h (`update_check`), state in `updates.json`, the stick's version from `@state` → `firmware_update` / the banner (`status.update`, `update` events). `GET /api/update`, `POST /api/update/check`, `GET /api/update/jobs/{id}`. The "Update … ready" toast on the stick once per release (`notified` in `updates.json`). Not ported: the app updating itself (`POST /api/update/agent` answers 409 with why; build it from the repo). |
| **Start at login, switch-over** | `PUT /api/login-item` `{on}` / `GET`: the menu bar app registers itself with `SMAppService.mainAppService` (macOS 13+; no LaunchAgent, no `launchctl`; System Settings → Login Items lists it; a terminal binary says "unavailable"). `POST /api/switch-over` `{confirm}` / `{undo}`, `GET`: see [SWITCHOVER.md](SWITCHOVER.md). Both are Settings cards from `/static-rs/rust.js`. |
| **Firmware over USB** | `POST /api/firmware/flash` `{port, backup}` → `{job}` (the same job SSE). `flasher.rs` (flasher.py): refuses a port that isn't an Espressif USB device and any part whose erase range touches NVS (0x9000-0xDFFF: pairings, name, settings stay); the 1200-baud touch, waits for the bootloader's port (12 s), an optional backup of the whole flash (`read-flash` into `~/.stickybot-rs/backups/<serial>-<time>.bin`), then `write-flash` of every part at its offset with the manifest's mode / freq / size, `--before usb-reset --after hard-reset`; esptool's output as `line` / `progress` events. No port is opened for anything else meanwhile. esptool runs as a subprocess behind the `Esptool` trait: tests use a fake and never open a real port or run esptool. |
| **Firmware over Bluetooth** | `GET /api/firmware` (bundle + connected stick; `usb`: sticks on USB with their version from `@state`, cached 10 s; a port in the bootloader is listed as "download mode" and never opened), `POST /api/firmware/ota` `{force}`, `GET /api/firmware/jobs/{id}` (SSE), `POST /api/firmware/jobs/{id}/cancel`. Bundles in `~/.stickybot-rs/firmware/<version>|local/` or `$STICKYBOT_RS_FIRMWARE`. `ota.py`'s checks (support, pending, versions / force, manifest SHA-256), windowed transfer, resume, then waits for the stick to come back with the new version (confirmed / rolled back). |

### Importing the Python app's settings (`migrate.rs`)

At switch-over this app takes over `~/.stick-agent/` (**read-only**: files are read and copied,
never written, moved or deleted there; a test checks every byte and mtime):

- `config.json`: every key both apps know (the stick, `github` incl. `primary_repo`, the wireless
  mic and Wispr timings, `brain` / `model` / OpenRouter model, sounds, approvals, events, quiet hours,
  updates…), each checked like `PUT /api/config` (an invalid or wrong-type value keeps the default).
  Not taken: `stt_model` (a different speech model here), removed keys (`wispr_fallback_mic`), the
  token (each app has its own).
- `commands.json` (your voice commands, with their real repo URLs), `sounds/<pack>/*` (sound files
  only; a clip already here is kept), `history.jsonl` (same format; merged in time order, no
  duplicates; unknown lines skipped), `workspace/CLAUDE.md` (if you changed it), `pet.json` and `github_state.json` (first start: unless
this app's pet already made progress; explicit: they win, `*.before-import` kept), `events.jsonl`
(merged like the history).

When: on the **first start** (no `config.json` and no `imported.json` in `~/.stickybot-rs/`; not with
the fake stick) it fills the empty app: commands only if still the defaults. And on demand:
**Settings → Import from StickyBot** (a Rust-only card the page gets from `/static-rs/rust.js`) or
`POST /api/import`: the Python app's settings and commands win, this app's are kept as
`config.json.before-import` / `commands.json.before-import`. `GET /api/import` says whether there is
anything to import and when the last import was (`imported.json`). The source is
`$STICKYBOT_RS_IMPORT_FROM` or `~/.stick-agent`; tests use a fake home (`Options::import_from`).
The settings are applied to the running app (a new stick re-targets the link; the Python-agent
check still holds it back while the Python app has the stick).

**Write-without-response pacing** (`pacing.rs`, used by `corebt.rs`): before each packet on
AUDIO_IN / OTA_DATA, `canSendWriteWithoutResponse` is checked and the packet queued in one block on
the Bluetooth queue; with no room, it waits for `peripheralIsReadyToSendWriteWithoutResponse:`
**or 2 ms**, whichever comes first, and checks again (2 s at most, then sends anyway, like link.py;
the protocols cope with a lost packet). Waiting for the callback alone made clips slower than real
time on the real stick (glass_001: 0.29 s of audio in 0.44 s; link.py, which polls the flag every
5 ms: 0.19 s): the callback comes later than the flag (or not at all). `pacing::Radio` models
CoreBluetooth's queue and the radio for the fake stick (`FakeStick::set_radio`);
`a_clip_goes_out_faster_than_real_time` sends 0.29 s / 1 s / 3 s clips through it. The log line
`clip …: X s of audio sent in Y s (N packets; lock …, start ack …, frames …, end ack …)` shows
where the time goes on the real stick.

### Not done in the link yet

- The stick renamed on the stick itself (the `@state` name check on connect) doesn't rename the mic
  yet; picking the stick again in the dashboard does.

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

### Parity round 2 checks with the real stick (steps 1-3 above first)

- **Clip speed**: Sounds → play glass_001, bong_001, click_001: the log's `clip …: X s of audio sent
  in Y s` must have Y < X (Python: 0.28 s in 0.19 s). If not, the same line says where it went
  (`lock`, `start ack`, `frames` / `slowest`, `end ack`).
- **Stick tools**: ask Claude (hold B) "notify me on the stick that tests passed" → a toast;
  "ask me on the stick whether to deploy" → A / B answers; in a terminal after Settings → *Stick
  tools* → install: `claude` → "use the stick to show progress 3 of 10" → the home card.
  `stick_status` reports the battery.
- **Speech**: Settings → *Read replies aloud* on → hold B, ask a short question → the stick speaks
  it sentence by sentence while the screen fills; hold B again mid-sentence → it stops. Voice →
  Test.
- **One-shot mode**: Settings → Claude Code runs *once per request* → ask twice within 10 min →
  the second `claude -p` has `--resume` (log).
- **Home screen**: the card rotates (your answer, sessions, the feed), weather on the stick, an
  event from Events → Test, a GitHub PR change becomes an event, the pet's face data (`/api/pet`).
- **USB**: plug the stick in, pause Bluetooth → Stick page loads over USB (`via: "usb"`); Firmware →
  USB flashing with a backup (the stick on the desk, not mid-task).
- **Login item**: Settings → *Start at login* on → System Settings → Login Items lists it; log out
  and in.
- **Switch-over**: follow [SWITCHOVER.md](SWITCHOVER.md) once you're ready to retire the Python app.

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

1. OpenRouter brain (`openai_brain.py`, `/api/brain*`, the key in the Keychain).
2. Real-stick checks of everything since the last round (see *Testing against the real stick*: clip
   timing, MCP tools, speech, home card / world / weather / events, USB serial + flashing, updates,
   login item, the switch-over).
3. Wireless mic leftovers: the setup page's step (`/api/install`), the `@state` rename check, a
   panic hook that removes the device (today: the next start does).
4. Onboarding / install (`/api/onboarding`, `/api/install*`, `/api/components*`), `/api/restart`,
   `/api/window`.

## Open decisions

Decided (see [SWITCHOVER.md](SWITCHOVER.md)):

- **Signing identity**: a stable code-signing certificate from the login keychain for every build,
  passed as `STICKYBOT_SIGNING_IDENTITY` to `desktop/scripts/build-app.sh` (Tauri's
  `APPLE_SIGNING_IDENTITY`; nothing personal in `tauri.conf.json`, which stays ad hoc by default).
  TCC (Bluetooth, System Audio Recording) then survives rebuilds. A Developer ID later only for
  distributing to others.
- **Bundle id**: keep **`com.stickybot.rs`** after the switch-over. Taking over `com.stickybot.app`
  would not carry the Python app's permissions anyway (TCC checks the signature too, and the Python
  app's is a different one), would make the two apps indistinguishable to macOS while both exist,
  and the login item is now `SMAppService` (no LaunchAgent label to inherit). The things that
  matter for continuity are the port (8660) and the token, which the switch-over takes.

Open:

- **Serving `static/`**: Phase 1 embeds the Python app's files (single source of truth while both
  apps live). Later: move them to a shared folder, or into the Rust app for good.
- **STT default**: sherpa-onnx now (spike verdict); FluidAudio (ANE) as the upgrade once the Swift
  build step is in place.
