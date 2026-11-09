# StickyBot desktop (Rust + Tauri 2)

The rewrite of StickyBot's Mac app (`mac-agent/`, Python) in Rust. **Phase 1** runs *next to* the
Python agent and replaces nothing: its own port (**8670**, Python: 8660), its own folder
(**`~/.stickybot-rs/`**, never `~/.stick-agent/`), its own bundle id (**`com.stickybot.rs`**).

What it does today: the Bluetooth link to the stick, the dashboard (the Python agent's own page)
served from Rust with the API slice the **Stick** page needs, and a menu bar app.

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
(see *Safety*), `RUST_LOG=debug` (every message from the stick is logged at debug).

## Layout

| Crate | What |
|---|---|
| `crates/stick-link` | The Bluetooth link (port of `link.py`). `proto.rs`: the wire format, **no I/O** (compact ASCII JSON lines, 180-byte writes, line decoder, hello / `proto` / sound messages, the "Insufficient Resource" retry rule). `link.rs`: reconnect loop, hello every 8 s, `@…` requests answered by id (5 s), message + audio streams, pause, re-target, a connect gate. `corebt.rs`: CoreBluetooth through `objc2-core-bluetooth`. `fake.rs`: a stick in memory (answers `@state`, `@keys`, settings, sounds; test knobs for busy / silent / absent / drops). `examples/scan.rs`. |
| `crates/stickybot-server` | axum: the dashboard page + `static/` from `mac-agent/stick_agent/` (rust-embed: read from disk in debug builds, embedded in release), the API slice, config + token in `~/.stickybot-rs/`, the SSE event feed, the Python-agent check. Bin `stickybot-headless`. |
| `crates/stickybot-app` | Tauri 2 shell: tray icon (status line, Open dashboard, Open in browser, Quit), webview window on `http://localhost:8670/`, no Dock icon (`ActivationPolicy::Accessory` + `LSUIElement`). |
| `crates/macos-audio` | Phase-0 spike: the wireless-mic Core Audio tap + aggregate device (`spikes/TAP.md`). |
| `crates/stt` | Phase-0 spike: speech to text, sherpa-onnx Parakeet TDT 0.6b v3 by default (`spikes/STT.md`; whisper behind `--features whisper`, FluidAudio behind `fluid`). |

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
version"; `GET /api/rust` lists them): `/api/pet`, `/api/firmware*` (USB flash, Bluetooth OTA,
jobs), `/api/onboarding` (the wizard keeps its progress in the browser meanwhile), `/api/install*`,
`/api/history`, `/api/logs`, `/api/ask`, `/api/commands*`, `/api/permission*`, `/api/clips*`,
`/api/voices`, `/api/voice/test`, `/api/events/test|history|kinds`, `/api/claude-event`,
`/api/wireless-mic/*`, `/api/mcp*` and `/mcp`, `/api/brain*`, `/api/update*`, `/api/components*`,
`/api/new-chat`, `/api/restart`, `/api/window`.

How the pages degrade: the **Stick** page works fully except the Face card's pet line and the
Firmware card ("Firmware updates aren't available yet"). Overview shows the link and "Speech model
loading…" (no STT yet). Activity, Quick commands, Events, Logs, Settings → Brain / wireless mic show
their "not available" states. With no stick picked, the setup wizard opens as in the Python app;
its *Pick your stick* step scans and saves the stick to **this** app's config.

### Not done in the link yet

- The hello carries only `t` / `tz` (no home card, weather, `world`, `wmic`, `wready`), so the stick
  never streams Wispr audio to this app.
- Talk (`evt: talk`) and Wispr (`evt: wispr`) messages and audio frames arrive on the streams but
  nothing uses them yet: holding B on the stick while it's connected to this app ends in its 20 s
  "Thinking…" timeout.
- AUDIO_IN (clips) and OTA_DATA writes (need `canSendWriteWithoutResponse` pacing).

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

The Rust app's identity is `com.stickybot.rs`, so its Bluetooth permission is separate from
StickyBot.app's. The bundle is signed ad hoc (Tauri's default): **every rebuild is a new code
identity, and macOS asks for Bluetooth again**. See *Open decisions*.

## Next: Phase 2

1. **Talk pipeline**: ADPCM decode (port `adpcm.py`) + `Recording`, the `stt` crate (sherpa-onnx,
   Parakeet TDT 0.6b v3, model download with progress), `talk start/stop`, quick commands
   (`commands.py`), screen states to the stick.
2. **Wireless Wispr mic**: `macos-audio` into the link: `wmic` / `wready` in the hello, `prepare` →
   device → `ready`, the output stream + jitter buffer (`wispr_mic.py`), USB-wins, cleanup.
3. **Brain**: the long-lived `claude -p --input-format stream-json` session (`brain.py`), streaming
   to the stick; OpenRouter later.
4. **Write-without-response pacing** in `corebt.rs`, then sound clips (AUDIO_IN) and the Bluetooth
   firmware update (`ota.py`).
5. Approvals (hook long-poll), the MCP server, events / GitHub / pet / world, history + logs pages,
   onboarding / install / updates; USB serial for `@…` lines.
6. Packaging: a stable signing identity, login item, and the switch-over from the Python app.

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
