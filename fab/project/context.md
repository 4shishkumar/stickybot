# Project Context

StickyBot is an M5StickS3 (ESP32-S3, 135×240 screen, two buttons A and B, mic, speaker, IMU,
IR) that sits on a desk, and the Mac app that drives it. Over one Bluetooth connection the stick
is a keyboard / mouse (HID: a macro combo on A, air mouse, presenter, media keys) and has a
"Mac link" (JSON lines + audio) to the app: hold B to talk to Claude Code, answer Claude Code's
permission prompts with A / B, get alerts, show faces, play games, learn and send IR codes.

Public repo: `github.com/4shishkumar/stickybot` (the local folder name may differ). One repo,
several parts that only talk through what `PROTOCOL.md` describes. Deeper reading:
`docs/ARCHITECTURE.md` (what runs where), `docs/DEVELOPMENT.md` (layout, tests, debugging),
`docs/RELEASING.md`, `docs/CONTRIBUTING.md` (branches, PRs, CI), and `docs/memory/`.

## PROTOCOL.md (the contract)

The wire formats between the boards, the settings pages and the Mac apps: Bluetooth identity and
slots, the combo format, USB serial commands (`@state`, `@keys`, …), the Wi-Fi settings page, and
section 5, the Mac link (Nordic UART service on the HID connection: RX / TX JSON lines, audio
out, audio in, OTA data).

- Change it **first** when a message changes, then both sides, in the same PR.
- Each feature says how it is detected (`@state` keys on the stick, hello flags from the app) and
  what older firmware / older apps do with it. Keep that sentence for every new message.

## m5-stick-s3/ (the firmware)

- **Stack**: C++17, Arduino core 3.x through PlatformIO with the pioarduino platform, M5Unified /
  M5GFX / M5PM1 (pinned), NimBLE-Arduino 2.x, ArduinoJson 7, TinyUSB (serial + USB mic), a vendored
  and modified HID keyboard library in `lib/`.
- **Entry points**: `src/main.cpp` (buttons, modes, serial commands, `render()`), `buddy.cpp` (the
  Mac link), `hid.cpp`, `ui.cpp` + `faces/`, `ota.cpp`, `protocol.cpp` (the `@…` commands),
  `settings.cpp` (NVS). `src/config.h` holds `FW_VERSION`.
- **Pure logic**: `src/logic/` (no Arduino, no hardware), unit-tested natively with Unity from
  `test/test_*/`. Hardware-facing code is a thin layer over it (`ota.cpp` over `logic/ota_rx.*`).
- **Commands** (in `m5-stick-s3/`): `pio test -e native` (tests, a few seconds), `pio run -e sticks3`
  (build; prints `Flash:` use), `pio run -e sticks3 -t upload` (flash over USB: only a person with
  the stick does this). `make test` / `make build` wrap them with the PlatformIO in
  `~/.platformio/penv`.
- **Conventions**: 2-space indent, one `namespace` per module, constants with a comment giving the
  unit and the reason, wire strings named after what goes on the wire. Text on the stick is short
  and limited to ASCII plus `°’‘“”–—…•·×` (what the fonts hold).
- **Gotchas**:
  - Flash budget: the app must stay at or under 85 % of the 3,342,336-byte slot; CI fails above it.
  - gcc (CI, the stick) and clang (a Mac) must give the same test results. Two random draws in one
    call ran in a different order on each and stopped a release: one side effect per statement.
  - Bluetooth buffers are few (NimBLE msys blocks raised to 48): a long `@state` reply once starved
    key presses and audio. Keep replies short and paged (`@faces` pages by 6).
  - Changing the HID report map or services changes GATT handles; macOS caches them per paired
    device. If the Mac link stops after such a change: forget the stick in the Mac's Bluetooth
    settings and pair again. Call this out in the PR and the changelog.
  - The stick is a keyboard, so macOS keeps the Bluetooth link up even when no app is running: the
    stick cannot tell "app gone" from the link. Presence is the hello every ≤ 10 s (offline after
    25 s without one) and the app's `bye` before it pauses or quits.
  - A new firmware image boots "pending" and confirms itself after 25 s with Bluetooth up;
    anything that resets before then rolls back. Don't add work that blocks the loop at start-up.
  - `secrets.ini` (OTA password) is git-ignored; CI copies `secrets.example.ini`. Generated files:
    `src/web_page.h` (ignored), `src/fonts_vlw.cpp` and `src/sounds_clips.cpp` (committed, made by
    `tools/make_fonts.py` / `tools/make_sounds.py`; never edit by hand).
  - Only one program can hold the serial port: close the monitor before flashing.

## desktop/ (StickyBot.app, the Mac app that ships)

- **Stack**: Rust (stable, edition 2024), a Cargo workspace; Tauri 2 for the menu bar app and the
  window; axum for the dashboard server and API; CoreBluetooth through `objc2` (not btleplug: it
  hides failed writes and the write-without-response pacing calls); Core Audio taps for the
  wireless mic; sherpa-onnx (Parakeet) for speech to text. Apple silicon, macOS 13+. Bundle id
  `com.stickybot.rs`, data in `~/.stickybot-rs/`.
- **Crates** (`desktop/crates/`): `stick-link` (the wire format with no I/O in `proto.rs`, the
  link, ADPCM, the windowed transfer for firmware and faces, CoreBluetooth, a fake stick),
  `stickybot-server` (axum: dashboard, API, talk, speech, brain, approvals, firmware, faces, IR,
  wireless mic, setup, Claude Code hooks; bin `stickybot-headless`), `stickybot-app` (the Tauri
  shell: tray, icon, window), `macos-audio` (the aggregate device + tap, jitter buffer, player,
  with an in-memory Core Audio), `stt`.
- **Commands** (in `desktop/`): `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/build-app.sh` (the bundle,
  signed ad hoc). `rustfmt.toml`: width 120. For looking at the dashboard without Bluetooth:
  `cargo run -p stickybot-server --bin stickybot-headless -- --fake-stick`.
- **Conventions**: hardware and the OS sit behind traits (`Connector` / `Connection`, `CoreAudio`,
  `System`) so the link, the API and every test run against fakes; module docs (`//!`) say what
  the module is a port of and why it is shaped that way; one version for the workspace
  (`[workspace.package] version`, set by the release tool).
- **Gotchas**:
  - The dashboard is **not** in `desktop/`: the server serves `dashboard.html` and `static/` from
    `mac-agent/stick_agent/` (read from disk in debug builds, embedded in release builds). A
    dashboard change is a change to both apps, and CI's `mac-app` job runs for it.
  - Ad-hoc signing gives every build a new identity: macOS asks for Bluetooth and System Audio
    Recording again after each update or rebuild. Don't "fix" a permission problem in code before
    checking this. A stable signing identity (`STICKYBOT_SIGNING_IDENTITY`) avoids it locally.
  - The stick takes one Bluetooth central. Next to the Python app the Rust app waits while that app
    has the stick, and uses port 8670; alone (or switched over) it is StickyBot on 8660.
  - `cargo test --workspace` never touches audio hardware or Bluetooth. The real Core Audio test is
    opt-in (`STICKYBOT_RS_AUDIO_E2E=1`) and the real-stick checklist is in `desktop/README.md`.
  - Tests that need something key-shaped build it with `concat!` so secret scanners stay quiet.
  - The sherpa-onnx and Tauri CLI versions are pinned at the top of both workflows; move them
    together.

## mac-agent/ (the legacy Python app, and the shared dashboard)

- **Stack**: Python 3.11+ (`stick_agent` package, `stickybot` command): asyncio, bleak, aiohttp,
  rumps for the menu bar, MLX Parakeet for speech. The dashboard is `stick_agent/dashboard.html`
  plus vanilla JS modules in `stick_agent/static/` (no framework, no build step).
- **Role**: the older app (installed by `install.sh`, data in `~/.stick-agent/`, port 8660). Still
  supported; the Rust app is a port of it module by module (`link.py` → `stick-link`, `core.py` →
  `talk.rs`, `ota.py` → `ota.rs`). Behaviour changes usually belong in both until it is retired.
- **Commands**: `make check` at the repo root (ruff + pytest + the firmware's native tests);
  in `mac-agent/`: `python -m ruff check .`, `python -m pytest` (fast: no Bluetooth, network or real
  Claude; also runs the dashboard's `tests/js/*.test.mjs` with `node --test` when Node is there).
  Slow tests (`-m slow`) and real Core Audio (`-m coreaudio`) are opt-in.
- **Conventions**: ruff, line length 120; the existing code keeps its hand-aligned style, so the
  formatter is only for new files. Tests get a temporary home (`home` fixture), a `FakeLink`, a
  `FakeSTT` and `tests/fake_claude.py`; they never reach GitHub, git, audio devices or the Keychain.
- **Gotchas**: the JS modules run in the app's webview and in a browser, with the API token built
  into the page; every API call resolves (`{error}` instead of throwing), and an endpoint one app
  lacks answers `missing: true`, so pages must cope with both apps.

## faces/ and tools/faces/ (face packages and the catalog)

- A face is **data only** (a manifest + indexed-colour PNGs, both orientations), compiled to one
  `.sfb` file. Format and budgets: `docs/faces/FORMAT.md`; how faces get in:
  `docs/faces/GUIDELINES.md`; the catalog: `docs/faces/CATALOG.md`.
- `faces/examples/<id>/` are ours, `faces/community/<id>/` come by PR, `faces/retired.json` lists
  ids never to reuse. `tools/faces/` (`validate.py`, `facepkg.py`, `build_catalog.py`,
  `sign_catalog.py`, `pr_check.py`) is standard-library Python, linted with the agent's ruff config
  and tested from `mac-agent/tests/test_face_*.py`.
- `.github/workflows/faces.yml` builds the catalog on pushes to `main` that touch `faces/` (and
  nightly), signs `catalog.json` with ed25519 (the private key is an Actions secret; the apps
  carry the public keys) and deploys it to GitHub Pages.
- Previews are rendered by the firmware's own drawing code compiled for the Mac
  (`m5-stick-s3/tools/face_shots`); committed previews must match a re-render.

## tools/ (release and repo tooling)

`tools/release.sh` + `release_tool.py` (bump, date the changelog, commit, tag; never pushes),
`tools/setup_labels.sh`, `tools/devportal/` (a local page to build, flash and watch logs),
`m5-stick-s3/tools/make_release.py` (the four flash parts + `manifest.json`). Shell scripts are
POSIX `sh` and pass shellcheck.

## xiao-esp32-c3/ (the older prototype)

A one-button macro keyboard on a Seeed XIAO ESP32-C3 with the same outside behaviour (PROTOCOL.md
sections 1 to 4, no Mac link). CI still builds it (`pio run -e xiao_c3`). Change it only when the
shared protocol changes or it breaks.

## Working rules that are easy to miss

- Branches: `feat/<topic>`, `fix/<topic>`, `chore/<topic>`, `release/vX.Y.Z`; a fab change uses
  its change folder's name (`YYMMDD-xxxx-slug`). One topic per PR.
- CI runs only the jobs for the parts a PR touches; **CI passed** sums them up.
- A user-visible change needs a `CHANGELOG.md` line under `[Unreleased]`; otherwise the PR gets
  the `no-changelog` label.
- Never run the app against, connect to, or flash someone's stick as part of automated work; use
  the fake stick and temporary homes. Ports 8660 / 8670 and `~/.stickybot-rs`, `~/.stick-agent`
  belong to the user's running app.
