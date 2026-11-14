# StickyBot desktop (Rust + Tauri 2)

**Just want the app?** Download `StickyBot-<version>-macos-arm64.zip` from the
[latest release](https://github.com/4shishkumar/stickybot/releases/latest) and follow
[Install the Mac app](../README.md#install-the-mac-app) (it's unsigned: the first open needs
right-click → Open). Coming from the Python app: [SWITCHOVER.md](SWITCHOVER.md). The rest of this
page is for building and working on it.

The rewrite of StickyBot's Mac app (`mac-agent/`, Python) in Rust. **Phase 1** runs *next to* the
Python agent and replaces nothing: its own port (**8670**, Python: 8660), its own folder
(**`~/.stickybot-rs/`**, never `~/.stick-agent/`), its own bundle id (**`com.stickybot.rs`**).

What it does today: the Bluetooth link to the stick, the dashboard (the Python agent's own page)
served from Rust, a menu bar app, and (Phase 2) **hold B to talk** (on-device speech to text, quick
commands, Claude Code streamed to the stick), Claude Code **approvals** on the stick, **sound clips**,
the **firmware update over Bluetooth** and the **wireless Wispr mic** (hold A: the stick's mic reaches
Wispr Flow over Bluetooth, through a Core Audio tap), **face packages** (the faces catalog, installed over
Bluetooth), the stick 2.5.0 / 2.5.1 features (**IR remote**, **games**, **air mouse / presenter**, the
**Launcher** card), and the
**setup page** (first-run steps, components, Claude Code hooks), `/api/restart` and `/api/window`.
Every endpoint the Python app serves (incl. feature/faces-2.4 and the 2.5.0 branches) is here; see
*Endpoint diff* below.

## Run it

Needs Rust (stable, edition 2024) and, for the app bundle, the Tauri CLI
(`cargo install tauri-cli --version 2.12.4 --locked`, the one the release workflow uses). The first
build downloads sherpa-onnx's prebuilt static libraries (~20 MB, from k2-fsa's GitHub release;
`SHERPA_ONNX_ARCHIVE_DIR=<folder with the .tar.bz2>` uses a copy you have). The speech model is not
part of the build: the app downloads it on its first start.

```sh
cd desktop

# the dashboard + link without the menu bar app, with a stick in memory (no Bluetooth at all)
cargo run -p stickybot-server --bin stickybot-headless -- --fake-stick
open http://localhost:8670/#stick

# the menu bar app (tray icon → Open StickyBot (Rust)); add STICKYBOT_RS_FAKE_STICK=1 for the fake stick
cd crates/stickybot-app && cargo tauri dev

# the bundle: target/release/bundle/macos/StickyBot.app, signed ad hoc (what the release ships)
scripts/build-app.sh
# ... signed with a stable identity (permissions survive rebuilds): see SWITCHOVER.md
STICKYBOT_SIGNING_IDENTITY=<sha1> scripts/build-app.sh
# ... with the release's version in the bundle (default: tauri.conf.json's)
STICKYBOT_VERSION=2.4.0 scripts/build-app.sh

cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings
cargo run -p stick-link --example scan        # read-only scan for sticks (never connects)

# the wireless mic end to end with real Core Audio (a temporary tap + aggregate, inaudible; see Testing)
STICKYBOT_RS_AUDIO_E2E=1 cargo test -p stickybot-server --test wmic_e2e -- --nocapture
```

Environment: `STICKYBOT_RS_PORT` (8670 next to the Python app, 8660 once switched over or on a Mac without it), `STICKYBOT_RS_HOME` (`~/.stickybot-rs`),
`STICKYBOT_RS_FAKE_STICK=1`, `STICKYBOT_RS_PYTHON_PORT` (8660), `STICKYBOT_RS_ALLOW_SHARED=1`
(see *Safety*), `RUST_LOG=debug` (every message from the stick is logged at debug),
`STICKYBOT_RS_MODELS` (`<home>/models`, the speech model), `STICKYBOT_RS_NO_STT=1` (don't download /
load it), `STICKYBOT_RS_FIRMWARE` (a firmware bundle folder), `STICKYBOT_RS_REPO` (`4shishkumar/stickybot`, where releases come from), `STICKYBOT_RS_ESPTOOL` (the esptool to flash with; else `esptool` on PATH / Homebrew, else `python3 -m esptool`), `STICKYBOT_RS_SOUNDS`
(`<home>/sounds`, sound packs), `STICKYBOT_RS_IMPORT_FROM` (`~/.stick-agent`, read-only: what
*Import from StickyBot* copies), `STICKYBOT_RS_AUDIO_E2E=1` (runs the wireless-mic test against real
Core Audio).

The first start downloads the speech model (Parakeet TDT 0.6b v3 int8 ONNX, 487 MB, from
sherpa-onnx's GitHub release; SHA-256 checked, resumable) into `~/.stickybot-rs/models/` and
loads it in ~0.7 s; `GET /api/speech` and `speech` events show the progress.

## Layout

| Crate | What |
|---|---|
| `crates/stick-link` | The Bluetooth link (port of `link.py`). `proto.rs`: the wire format, **no I/O** (compact ASCII JSON lines, 180-byte writes, line decoder, hello / `proto` / sound messages, the "Insufficient Resource" retry rule). `link.rs`: reconnect loop, hello every 8 s, `@…` requests answered by id (5 s), message + audio streams (and both in order: `incoming()`), clips (`send_audio`), OTA packets, pause, re-target, a connect gate. `adpcm.rs`: the stick's IMA-ADPCM frames. `ota.rs`: the windowed transfer (`ota.py`'s `Transfer`): firmware, and face packages with the `faces` `Family`. `corebt.rs`: CoreBluetooth through `objc2-core-bluetooth`, with write-without-response pacing. `fake.rs`: a stick in memory (answers `@state`, `@keys`, settings, sounds; holds B with any audio; plays clips; takes firmware updates and face packages (`@faces`, `put` / `rm`, one transfer at a time); `fake_ir.rs`: `@ir`, `@games`, `@air`, `@mode`; test knobs for busy / silent / absent / drops / lost packets / rollback). `examples/scan.rs`. |
| `crates/stickybot-server` | axum: the dashboard page + `static/` from `mac-agent/stick_agent/` (rust-embed: read from disk in debug builds, embedded in release), the API slice, config + token in `~/.stickybot-rs/`, the SSE event feed, the Python-agent check. `talk.rs` (hold B: core.py's flow), `speech.rs` (model download + STT worker), `commands.rs` (quick commands), `brain.rs` (Claude Code session), `approvals.rs`, `clips.rs`, `firmware.rs`, `wmic.rs` (the wireless Wispr mic: core.py's wispr parts + `Holds`), `faces.rs` (face packages), `ir.rs` (the IR remote), `setup.rs` (setup page, components, onboarding, restart / window; tests in `setup_tests.rs`), `hooks.rs` (Claude Code's hooks: `--hook permission|prompt|stop`), `sysrun.rs` (programs the setup page runs, behind `System`), `appctl.rs` (restart / window, set by the menu bar app). Bin `stickybot-headless`. Tests `tests/talk_e2e.rs`, `tests/wmic_e2e.rs`. |
| `crates/stickybot-app` | Tauri 2 shell: the menu bar menu of the Python app (`tray.rs`: status line, update / *Finish setup…* items while needed, Open StickyBot, Open in browser, Check for Updates…, Stick volume, Sounds on the stick, Pause / Resume, Restart, Quit; refreshed every 0.5 s, through the server's functions) with the state icon (`icon.rs`: the brand mark as a template image, the Python states + the approval badge), webview window on `http://localhost:8670/` (`window.rs`: other links open in the browser, reopening the app shows it), no Dock icon (`ActivationPolicy::Accessory` + `LSUIElement`). `appctl.rs`: `/api/restart` (Tauri restart) and `/api/window` (show / focus / create the window at `#page`). `--hook …` runs Claude Code's hook and exits. |
| `crates/macos-audio` | The wireless mic's audio side (`spikes/TAP.md`). Plain Rust, tested anywhere: `carrier.rs` (`pick_carrier`: tap / BlackHole), `device.rs` (`StickMicDevice`: the aggregate + tap, `recording()`, the -80 dBFS check, cleanup by *our* UID only), `jitter.rs` (sequencer, jitter buffer, resampler), `player.rs` (`WisprPlayer`: the stick's audio into the carrier, gated on the tap), `fake.rs` (Core Audio and outputs in memory), `naming.rs`. macOS: `system.rs` (the HAL + cpal backends), `hal.rs`, `tap.rs`, `mic.rs` (the live-object registry). `examples/tapmic.rs` (the spike's self-check). |
| `crates/macos-calendar` | The Mac's Calendar, read-only: the `CalendarSource` trait (access, request, calendars, events, change callback; **no write method exists**), `fake.rs` (`FakeCalendar`, also the `--fake-stick` demo day) and, on macOS, `eventkit.rs` (a thin EventKit converter; made on first use, never by tests). `examples/calendar_dump.rs`: `--status` (never asks), `--list`, `--today` (asks), `--help`. The server's `connections/` module holds the rules (`agenda.rs`, pure) and the Calendar connection (`calendar.rs`). |
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
| `POST /api/stick/proto` | `{"line":"@state"}` → `{"ok","via":"ble","reply","error"}` over Bluetooth; when Bluetooth isn't there or doesn't answer, over the stick's USB serial (`via: "usb"`, `usb.rs`: Espressif ports (VID 0x303A) whose USB product name contains "StickyBot", or a chip in download mode, never another ESP32 board (flasher.py `espressif_ports()` / `is_stick`); 115200 baud, the `@{…}` reply line). The Stick page's cards (incl. the **Launcher** card, firmware 2.5.1: `"launcher"` in @state, `@launcher tiles=…\|reset=1`) all go through here |
| `GET /api/scan` | sticks advertising the service, strongest first |
| `POST /api/pause` | pause / resume the link |
| `GET` / `PUT /api/sounds`, `POST /api/sounds/play` | the stick's sound settings (`{"sound":…}` → `evt: sounds`, 3 s) |
| `GET /api/rust` | this app: version, home, `ported`, `not_ported` |

Guard as in `server.py`: only `Host: localhost:8670` / `127.0.0.1:8670`, and every non-GET needs
`X-Stick-Token` (from `~/.stickybot-rs/token`, 0600; the page has it built in).

Everything else is ported too (tables below). Unknown paths answer a JSON 404 (`"missing": true`),
which the page shows as "not available in this version". `POST /api/update/agent` answers 409: the
app doesn't update itself (build it from the repo).

Every page works (Stick incl. USB flashing, More faces, Mouse & presenter, Games, Remote, Overview,
Activity, Events, Logs, Quick commands, Sounds, Settings incl. Brain (Claude Code / OpenRouter),
Components, the stick tools and voices, the setup page). With no stick picked, the setup wizard opens
as in the Python app; its *Pick your stick* step scans and saves the stick to **this** app's config.

**Ollama**: not ported, because the Python app has no Ollama brain (only a comment in
`openai_brain.py` saying it could reuse the OpenAI-style code). Brains are Claude Code and OpenRouter.

## What's ported (Phase 2)

| Feature | Endpoints / notes |
|---|---|
| **Hold B to talk** | `evt: talk` start → ADPCM frames → stop; decoded (`adpcm.rs`), transcribed on this Mac (sherpa-onnx Parakeet, word hints `stt_hints`, default `["Tauri", "StickyBot", "Claude"]`: "Tory" → "Tauri"), then a "new chat" phrase, a quick command, or Claude. The stick's screens as core.py sends them: `thinking` / `working` (throttled to `stream_interval_ms`) / `done` / `error` / `cancelled`, with "Too short - hold B while you speak.", "Speech model still loading…", "Didn't catch that.", "Still working on the last request.", "Something went wrong: …". Stick-side `evt: cancel` stops Claude. `history.jsonl`, `heard` / `reply` / `progress` / `state` events. `GET /api/history`, `POST /api/ask` (typed), `GET`/`PUT /api/commands`, `POST /api/commands/test`, `GET /api/speech`, `POST /api/speech/retry`, `POST /api/new-chat`. |
| **Claude brain** | `brain.py`'s session mode: one `claude -p --input-format stream-json` process, prewarmed when the stick connects, interrupt to cancel, `--resume` after a crash / settings change, idle stop, 240 s timeout. **One-shot mode** (`brain_mode: "oneshot"`): `claude -p "<text>"` per request, `--resume` within `followup_minutes`, cancel kills it, no prewarm. Both get the stick tools (`--mcp-config <home>/mcp-voice.json`, 0600, the feedback tools pre-allowed) while `mcp_server` is on. Status in `/api/status` `brain` (`mode`, averages per mode). |
| **Stick tools (MCP)** | `mcp.rs` (mcp_server.py): `POST /mcp` (Streamable HTTP, JSON-RPC; legacy `initialize` clients and modern 2026-07-28 `_meta` + header clients, `server/discover`; bearer token or `X-Stick-Token` on every method, `Origin` check, 405 for GET / DELETE, 503 when off; `ask` as SSE with keep-alives when accepted, the client closing it clears the question; `notifications/cancelled`). The ten tools with mcp_server.py's schemas and checks: `notify` / `celebrate` (events.rs, rate limit 1/s + 20/min), `ask` (A / B / shake, `busy: …`), `progress` / `pin_home` (the home card, pushed ≤ every 0.5 s), `stick_status` (`{"cmd":"status"}`, the last press, at the desk), `speak`, `open_url` / `open_app` / `quick_command` (left to Claude Code's prompt; `call_approved` asks on the stick for brains without it). `GET /api/mcp`, `POST /api/mcp/terminal/install|uninstall` (`{"confirm":true}`: `claude mcp add --scope user stick http://127.0.0.1:<port>/mcp …`, behind a runner in tests), `POST /api/mcp/answer`. |
| **Questions from a session** | `approvals::ask_user` (core.py): the prompt message with `kind: "ask"` and the labels, re-sent every 3 s, an approval goes first, cleared on the stick however it ends; `status.asks`, `ask` / `ask_done` events, the home card shows "<from> asks". |
| **Reading replies aloud** | `speak.rs` (speech.py): `speak_replies` (+ `speak_stream`: sentence by sentence while Claude writes, `speak_max_words` cap), `say` → a clip on the stick's speaker (several sentences joined, waits for it to play) or `afplay` on the Mac (`speak_output`, the Mac when the stick is away); quick-command confirmations with `speak_quick`; holding B or cancel stops it. `GET /api/voices`, `POST /api/voice/test`. |
| **OpenRouter brain** | `openrouter.rs` (openai_brain.py): any OpenRouter model with your own key, streamed chat completions (SSE) over HTTPS (ureq on a blocking thread, 10 s connect, 60 s without a byte), the conversation in memory (24 messages, whole turns dropped first; idle `brain_idle_min` = a new one), the stick tools as functions (`mcp::openai_tools`, run through `mcp::call_approved`: open_url / open_app / quick_command ask on the stick first), at most 5 tool rounds, progress lines, cancel closes the stream, cost / first token / rounds in `status.brain.last`, a model that can't use tools is asked again without them (and remembered). `brains.rs` (brains.py) picks Claude Code or OpenRouter per request from config `brain`; switching forgets the old one (Claude's session stops), Claude is pre-warmed only while picked. The key (`keys.rs`, keychain.py): the Keychain item the Python app uses (service `com.stickkey.agent`, account `openrouter`; macOS asks once whether this app may read it), through the Security framework (never the `security` command: a key on a command line shows in the process list); `STICK_AGENT_OPENROUTER_KEY` overrides it; write-only from the outside. With the fake stick the keys live in memory (tests never reach the Keychain). `GET /api/brain`, `PUT` / `DELETE /api/brain/openrouter/key`, `GET /api/brain/openrouter/models` (`?refresh=1`; cached 10 min, a stale list when offline), `POST /api/brain/test`. HTTP sits behind `openrouter::Transport` (tests: a scripted fake). |
| **Approvals** | `POST /api/permission` (the hook's long-poll), `POST /api/permission/decide`; the prompt on the stick every 3 s, A / B answers, `prompt` / `prompt_done` events, `status.prompts`. The Python app's hooks post to :8660 (this app after the switch-over); this app's own hooks (`--hook permission`, setup page → Claude Code hooks, only after the switch-over) do the same. |
| **Face packages** (firmware 2.4.0+) | `faces.rs` (faces.py + faces_api.py): the catalog (config `faces_catalog_url`, default GitHub Pages; https or http://localhost only) cached in `~/.stickybot-rs/faces/` (ETag / If-Modified-Since, at most every 6 h, the last copy offline), its ed25519 signature (`catalog.json.sig`) checked against faces.py's `CATALOG_KEYS` (required for the default URL), every `.sfb` checked (container, budgets, palette indices, id / version / size / sha256 against the catalog), minFirmware 2.4.0 and the face format against `@state`'s `pkgFaces`. Install = `{"faces":{"put":…}}` + the bytes over OTA_DATA with the firmware update's windowed transfer (`ota::Sender` with the `faces` `Family`: acks, gaps, resume after a drop, cancel), `end` → `done`; list `@faces` (all pages), remove `{"faces":{"rm":id}}`, use `@face id=`. One transfer at a time (a firmware update answers 409 while a face installs, and the other way round). `GET /api/faces/catalog` (`?refresh=1`), `GET /api/faces/preview/{id}/{side}`, `POST /api/faces/install` (`{id}` or `{file: base64, name}` → `{job}`), `GET /api/faces/jobs/{id}` (SSE), `POST /api/faces/jobs/{id}/cancel`, `POST /api/faces/remove`, `POST /api/faces/use`. Network behind `faces::Fetch` (the fake stick: offline). |
| **IR remote** (firmware 2.5.0+) | `ir.rs` (ir.py + ir_api.py), the Remote page: `GET /api/ir` (remotes and buttons from `@ir op=list`, the built-in code sets (Samsung, LG, Sony, Philips TVs), the mapped brands, the last learn), `POST /api/ir/send` (`{dev,btn,repeat}`, `{brand,btn}` or `{code}`; a missing learned button falls back to the remote's mapped set), `learn` / `learn/cancel` (the stick's `{"evt":"ir"}` steps → `ir` events), `rename`, `remove`, `fav`, `brand` (config `ir_brands`; `install: true` puts the set on the stick), `GET export` (`stickybot-remotes.json`), `POST import`, `GET diag`. Older firmware: "needs 2.5.0". The TV voice phrases are in the default commands.json and added once to an existing one (`ir_voice_added`); a quick command with `"ir": {dev, btn, repeat}` is sent by the stick (done / error on its screen, `via: quick`). Stick tools `ir_list` / `ir_send` (no approval; also OpenRouter functions). |
| **Games** (firmware 2.5.0+) | `{"evt":"game"}`: the pet gets +1 XP (a game with points) or +3 (a new best), at most 10 XP a day from games (`pet.rs` `game()`); `/api/pet`'s `pet.games` (`xpToday`, `cap`, `best`, `plays`); a `game` event; the world is pushed when XP was earned. The Games card reads `@games` through `/api/stick/proto`. |
| **Air mouse / presenter** (firmware 2.5.0+) | No server code: the Stick page's Mouse & presenter card uses `/api/stick/proto` with `@mode` / `@air`. |
| **Setup page, components, onboarding** | `setup.rs`: `/api/install*` walks the first-run steps (Mac check; the speech model = the sherpa download; Claude Code + its installer; Claude Code hooks; stick tools (MCP); GitHub login with `gh auth login --web` and its one-time code shown on the page; the wireless mic via the Apple audio tap (BlackHole only if already installed: `brew install --cask blackhole-2ch`); your stick), skip / unskip / run, jobs as SSE, `POST /api/install/github/repo` (rewrites the PR links in `commands.json`, sets `github`). `/api/components*` = Settings → Components (model, hooks, GitHub, wireless mic, MCP; the app and firmware can't be changed there). `/api/onboarding` keeps the stick wizard's marks in config `onboarding`. No Python venv, `stickybot` command, LaunchAgent or BlackHole download (start at login is Settings → *Start at login*). Claude Code's hooks run this app's binary with `--hook permission|prompt|stop` (`hooks.rs`: hook.py / claude_hooks.py), posting to its port and token; before the switch-over the Python app's hooks are left alone, after it adding the hooks replaces them (a dated backup first; an invalid settings file is never overwritten). Programs (`gh`, `claude`, `curl`, `sw_vers`, `df`) run behind `sysrun::System`: with the fake stick and in tests a pretend Mac. `POST /api/restart` restarts the menu bar app (the mic device is removed on the way out; headless: 409), `POST /api/window` `{page}` shows / focuses the dashboard window at `#page` (headless: `ok: false`). |
| **Sound clips** | `GET /api/clips`, `POST /api/clips/play`: packs in `~/.stickybot-rs/sounds/<pack>/` (ogg, wav, flac, mp3, aiff), made loud, AUDIO_IN packets (first / last acknowledged, the rest paced writes without response). |
| **Wireless Wispr mic** | Hold A away from USB: the stick streams its mic (`prepare` → ADPCM → `stop`); this app makes the **`<stick>-bt-mic`** device (the Python app's name, `stick_mic_device.py`'s slug rules, so Wispr's ranking finds it; UID **`com.stickkey.rust-wireless-mic`**, never the Python agent's `com.stickkey.wireless-mic`), waits `wispr_switch_ms` (250) after creating it, answers `ready`, plays the frames (in order by seq, a lost one = 20 ms of silence, resampled, ~80 ms jitter buffer) into the carrier, keeps the device `wispr_tail_ms` (800) after `stop` (≤ 4 s more while Wispr still reads it), then removes it. `wmic` / `wready` in the hello. Carrier as `wireless_mic_carrier` (`auto`: the tap once its inaudible -80 dBFS check passed, else BlackHole 2ch; `tap`; `blackhole`). The stick on USB (`evt: usb`): no device, `ready` with `"device": false` (Wispr uses the stick's USB mic). `POST /api/wireless-mic/show` `{secs}`, `POST /api/wireless-mic/check`, `status.wireless_mic`, `wispr_mic` / `stick_mic` events. Cleanup of *our UID only* at start (left-overs of a crash / `kill -9`), on quit (Tauri `Exit`, headless Ctrl-C, SIGTERM / SIGHUP), after a panic on the main thread (a panic hook; panics in tasks don't end the app and are left alone) and on disconnect / USB / off / rename. **Renamed on the stick**: on connect the app asks `@state`; any `@state` reply with another name updates the picked stick (saved, the link isn't re-targeted) and the mic is made again as `<new-name>-bt-mic` (core.py `_note_name`). |
| **Home screen, faces, events** | `home.rs` (home.py + core.py's home / world parts), `weather.rs`, `feed.rs`, `events.rs`, `github.rs`, `pet.rs`, `net.rs`. The hello carries `home` (the card: approvals waiting pinned, a session's `put` line, else every 16 s your last answer / Claude Code sessions open (`ps`, not counting this app's own child) / the feed / a greeting), `world` (PR counts, CI, Claude sessions from `claude_sessions.json`, approvals, review requests, the pet) and `wx` (Open-Meteo weather; `auto` = ipinfo.io / geojs.io, or a city), byte-compatible with core.py; `push_home` / `push_world` send them at once. Weather + feed refresh every minute when due (30 min), the GitHub watcher (`gh api graphql`, your login, every `github.interval_s`, first look records only, backoff to 30 min) turns PR changes into events and pet progress. Events: `{"src":"agent","event":{…}}` with levels from `event_rules`, quiet hours, `events.jsonl`, `event` feed entries, `event_shown` acks. `GET /api/pet`, `POST /api/events/test`, `GET /api/events/history`, `GET /api/events/kinds`, `POST /api/claude-event` (the Stop hook: the pet + "Claude finished in …" past `claude_done_min_s`); `status.weather`, `status.github`. State in `<home>/pet.json`, `github_state.json`, `events.jsonl` (the Python formats). Network and `gh` sit behind `net::Http` / `github::Gh`: tests (and the fake stick) use `FakeHttp` / `FakeGh`. API for the stick tools: `home::put` / `clear` / `push_home`, `events::notify`, `events::quiet_now`. Note: the Claude Code hooks still write `~/.stick-agent/claude_sessions.json` and post to :8660; this app reads `<home>/claude_sessions.json` (until the switch-over points the hooks here). |
| **Logs** | `GET /api/logs?n=` (300): the last lines of `~/.stickybot-rs/app.log` (`logs.rs`: every log line goes to stderr as before (`RUST_LOG`, default `info`), to `app.log` (rotated at 2 MB, `app.log.1`/`.2` kept, like the Python `RotatingFileHandler`) and to the live feed as `log` events, the Logs page's *Follow live*). |
| **Updates (firmware)** | `releases.rs` (releases.py): SemVer precedence, the latest release per channel (`update_channel` stable / beta), listed with `gh` when installed, else api.github.com over HTTPS; the release's firmware downloaded into `~/.stickybot-rs/firmware/<version>/` through a temporary folder, every part checked against the manifest's sha256 and GitHub's asset digest, 3 versions kept. `updates.rs` (updates.py): a check 20 s after start-up and every 24 h (`update_check`), state in `updates.json`, the stick's version from `@state` → `firmware_update` / the banner (`status.update`, `update` events). `GET /api/update`, `POST /api/update/check`, `GET /api/update/jobs/{id}`. The "Update … ready" toast on the stick once per release (`notified` in `updates.json`). A newer **app**: the latest release is newer than this app (`CARGO_PKG_VERSION`, the release version: `tools/release_tool.py bump` sets `[workspace.package] version`) and has `StickyBot-<version>-macos-arm64.zip` → `agent_update`, "Update available: vX", `install.download` = the release page; the menu item opens it. Not ported: the app installing it itself (`POST /api/update/agent` answers 409 with where to download it). |
| **Start at login, switch-over** | `PUT /api/login-item` `{on}` / `GET`: the menu bar app registers itself with `SMAppService.mainAppService` (macOS 13+; no LaunchAgent, no `launchctl`; System Settings → Login Items lists it; a terminal binary says "unavailable"). `POST /api/switch-over` `{confirm}` / `{undo}`, `GET`: see [SWITCHOVER.md](SWITCHOVER.md); on a Mac without the Python app the first start takes port 8660 by itself (`switch::adopt_if_fresh`, `"fresh": true`, no Switch over card). Both are Settings cards from `/static-rs/rust.js`. |
| **Firmware over USB** | `POST /api/firmware/flash` `{port, backup}` → `{job}` (the same job SSE). `flasher.rs` (flasher.py): refuses a port that isn't an Espressif USB device (a port picked by hand: any Espressif device, as `check_port`; the lists and the bootloader wait: sticks only) and any part whose erase range touches NVS (0x9000-0xDFFF: pairings, name, settings stay); the 1200-baud touch, waits for the bootloader's port (12 s), an optional backup of the whole flash (`read-flash` into `~/.stickybot-rs/backups/<serial>-<time>.bin`), then `write-flash` of every part at its offset with the manifest's mode / freq / size, `--before usb-reset --after hard-reset`; esptool's output as `line` / `progress` events. No port is opened for anything else meanwhile. esptool runs as a subprocess behind the `Esptool` trait: tests use a fake and never open a real port or run esptool. |
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
  only; a clip already here is kept), `history.jsonl` (same format; merged with this app's in time
  order, one line per request: the same time `t` is the same request, whichever app wrote the
  line; unknown lines skipped), `workspace/CLAUDE.md` (if you changed it), `events.jsonl` (merged
  like the history, by event `id`), `github_state.json` (first start: unless this app has its own;
  explicit: it wins, `*.before-import` kept).
- `pet.json`: **merged** with this app's pet when both made progress (`migrate::merge_pet`): XP,
  merged PRs, green builds, Claude tasks, releases, today's merges and game plays add up; the towers
  of both; the later streak (joined with the other when they touch), the best of the bests and of
  the game scores; the Python pet's city (`seed`). The Python pet's counters at that import are kept
  in this app's `pet.json` (`python_import`), so importing again later adds only what happened in
  the Python app since, and an import never counts anything twice (a pet copied whole by an older
  version of the import: each counter is the larger one). This app's pet before the merge is kept
  as `pet.json.before-import`.

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
3. Rust dashboard: menu bar icon → **Open StickyBot (Rust)** (or <http://localhost:8670>). The setup
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

## Real-stick checklist

**Nothing below has been verified on the hardware yet.** One list for everything since the last
real-stick round: do steps 1-3 of *Testing against the real stick* first (the speech model downloads
on the first start), tick each item off, and hand the stick back (step 5) at the end.

### Talk, clips, firmware over Bluetooth, approvals

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

### Clip speed, stick tools, speech, home, USB, login item, switch-over

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

### Wireless Wispr mic

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
8. **Renamed on the stick**: rename the stick on the stick itself (or `@name` from the Stick page),
   let it reconnect → the log says `the stick is now called …`, Settings shows the new name, and the
   next hold makes `<new-name>-bt-mic` (the old one is gone from Audio MIDI Setup).
9. **Crash / SIGTERM**: during a hold, `kill <pid>` (SIGTERM, not -9) → the log says `SIGTERM:
   removing the stick mic` and no `<stick>-bt-mic` is left.
10. **Setup page's wireless-mic step** (Setup → Wireless mic → Run) in tap mode: the System Audio
    Recording prompt appears (first time), then the check passes and the step is done.
11. Turn *Wireless mic* off before handing the stick back (the Python agent's Wispr mic keeps working
    as before).

### Face packages (firmware 2.4.0+)

1. Stick page → More faces: the catalog loads and says signed, previews appear, the installed faces
   are listed with free space and max 12.
2. Install catnap from the catalog: the stick's toast shows the percentage, then "Installed Cat Nap";
   *Use* makes it the Home face; *Remove* removes it. The log has `faces: N KB in X s = Y KB/s`.
3. Upload a `.sfb` made with `tools/faces/validate.py --compile DIR` and install it.
4. Walk out of range mid-install → it resumes. B on the stick during the toast → cancelled, nothing
   changes on the stick.
5. While a face installs, a firmware update over Bluetooth is refused (409), and the other way round.
6. A stick on 2.3.0 (no `pkgFaces`): every face says "update the stick to 2.4.0".
7. Offline (Wi-Fi off): the catalog still browses from the cache and says it shows the last copy.

### Stick 2.5.0: IR remote, games, air mouse

1. **Remote page** loads the stick's remotes; learn a TV button from the dashboard → the stick shows
   learning, press the remote twice → steps 1 / 2 then "saved" live; send it → the TV reacts.
2. Rename, star, remove a button; map a brand (e.g. Samsung) and send *power* without learning;
   install a brand set onto the stick; export, then import. Diag says `pins: ok`. A remote with 13+
   buttons lists fully (12 a page).
3. **Voice**: hold B, "turn off the TV" → the stick shows done; "mute the tv" with no such button →
   an error (not Claude). Claude / OpenRouter: "turn on the TV" uses `ir_list` / `ir_send` without
   an approval prompt.
4. **Games**: finish a game with points → Activity shows a `game` event, `/api/pet`'s
   `games.xpToday` grows and stops at 10 a day; the Games card shows the bests from `@games`.
5. **Mouse & presenter** card: start Air mouse, Presenter, Media, then Stop; pointer speed, invert,
   scroll, talk length and slide keys keep their values.
6. **Bluetooth after 2.5.0's HID** (keyboard + consumer + mouse changed the GATT handles): the link
   connects and stays up with no `the stick has no Mac link service` / characteristic errors; the
   app only discovers the NUS service (`corebt.rs`), so macOS's Battery / HID services are left
   alone. If it doesn't connect after the update: forget the stick in the Mac's Bluetooth settings
   and pair again.

### Stick 2.5.1: Launcher, USB filter

1. **Launcher card** (Stick page, shown when @state has `launcher`): it lists the eight tiles in the
   stick's order. Hide Games and move Faces to the top → Save → "Launcher saved"; triple-press B on
   the stick → the grid starts with Faces and has no Games. Quick settings / Settings can't be
   unticked. Reset → the default order (`grapmfqs`) on the card and the stick. Save stays disabled
   until something changes.
2. The same over USB: pause Bluetooth, the Stick page loads `via: "usb"`, Save the Launcher again.
3. **Another ESP32 board on USB** (the MicroPython ESP32-S3, 303a:4001 "Espressif Device"): with
   the stick unplugged and Bluetooth paused, the Stick page says "no stick connected (Bluetooth or
   USB)" and the board gets no `@` lines (its REPL stays quiet); Firmware → USB lists no sticks.
   With both plugged in: only the stick is listed and talked to (no "several sticks on USB").
4. A stick in download mode (hold the side button while plugging in) is listed as "download mode"
   and can be flashed.

### Setup page, components, restart, window

1. Setup page in the bundled app: each step's state is right (Claude installed and logged in; `gh`
   installed or not; the speech model done).
2. GitHub login from the setup page (you do it): the one-time code appears and the
   github.com/login/device flow finishes; *Repo* saves the PR links.
3. After the switch-over: Setup → Claude Code hooks → Run replaces the Python hooks (a backup of
   `settings.json` is kept); a terminal Claude Code request needing approval is answered with A / B on
   the stick; a task longer than `claude_done_min_s` ends with "Claude finished".
4. Settings → Components: install and remove each one (model, hooks, GitHub, wireless mic, MCP).
5. Settings → Restart (`POST /api/restart`): the app comes back, no mic device left behind.
6. `POST /api/window {"page":"install"}` opens / focuses the window at `#install`, also after the
   window was closed.
7. The *Your stick* step's onboarding marks (firmware, named) with the real stick.

### Connections: Calendar (never done by the tests)

Needs a real signed bundle (`scripts/build-app.sh`, which checks the calendars entitlement is in the
signature: the hardened runtime refuses EventKit without it).

1. `cargo run -p macos-calendar --example calendar_dump -- --status` (no prompt), `--list`, then
   `--today` (the prompt names the terminal app; allow it) show the calendars and today's events.
2. Dashboard → Connections → Connect: macOS asks for Calendars; allow Full Access. The pill turns
   Connected, the calendars and today's meetings are listed (the first Connect on a Mac that was
   never asked lists events at once, with no restart or second Connect: the app swaps its event
   store for a fresh one after the grant). Deny it once: the card says how to
   turn it on in System Settings → Privacy & Security → Calendars, and *Connect again* works after.
3. Next meeting on the stick's home card, pinned 15 min before the start until 5 min after; a
   `meeting_soon` heads-up (orange, until a button) `Remind me` minutes before, once per meeting;
   nothing for declined or all-day events. Quiet hours: shown, silent. (Needs firmware that draws
   the `meeting` card kind: every released one does.)
4. Voice: "what's my next meeting", "what's my day like", "join my meeting" (a meeting with a Meet /
   Zoom / Teams / Webex link opens it). The phrases are in Quick commands after the first connect.
5. Add, move and delete an event in Calendar.app: the card and the list follow within seconds.
6. Disconnect: the card goes. Diagnostics (Feedback) show `[meeting]` instead of titles, and
   `app.log` has no meeting text.

## Wireless mic and Bluetooth output

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

1. The real-stick checklist above (plus OpenRouter with a real key: macOS asks once to let this app
   read the Keychain item the Python app saved).
2. The shared `install.js` still says the speech model is "about 2.5 GB, from Hugging Face"; the Rust
   app's is 490 MB from GitHub (the static files are the Python app's: change them there or give the
   Rust app its own copy).
3. The fake stick reports firmware 2.3.0, so with `--fake-stick` More faces says "update to 2.4.0"
   (tests flip `pkgFaces` themselves).

## Endpoint diff

Python routes (`web.get/post/put/delete` in `mac-agent/stick_agent/*.py` on feature/faces-2.4,
feature/air-mouse, feature/games, feature/ir-remote and feature/stick-2.5 up to the launcher, firmware
2.5.1: it added no route) against the Rust router: 84 Python routes, **none missing**. (`/api/install/{step}/run` and `/api/install/github/repo` are
served by `POST /api/install/{step}/{action}`.) Rust-only: `/api/import`, `/api/login-item`,
`/api/rust`, `/api/speech`, `/api/speech/retry`, `/api/switch-over`, `/static-rs/*`.

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
