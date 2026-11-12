# Development

How the repo is laid out, how to test and debug each part, and how to add the
common things. First-time setup is in [BUILDING.md](BUILDING.md).

## Layout

| Path | What |
|---|---|
| `m5-stick-s3/` | firmware for the M5StickS3 (PlatformIO, Arduino core 3.x): `src/` modules, `test/` unit tests, `tools/` scripts (fonts, sounds, backup) |
| `xiao-esp32-c3/` | firmware for the XIAO ESP32-C3 macro button |
| `mac-agent/` | the StickyBot Mac app (`stickybot` command): `stick_agent/` package, `tests/` |
| `PROTOCOL.md` | **the contract** between boards, the settings pages and the Mac agent: serial commands, HTTP API, the Bluetooth "Mac link" (JSON messages, audio frames, sound clips, events) |
| `tools/devportal/` | small web UI to build, flash and watch logs for both boards |
| `docs/` | this file, BUILDING.md, RELEASING.md (versions, tags, the release workflow) |
| `tools/release.sh` | cut a release locally: bump, date the CHANGELOG, commit, tag (never pushes) |

The pieces only talk through what PROTOCOL.md describes — change it there first
when a message changes, then both sides.

## Tests and lint

```sh
make check            # lint + agent tests + firmware native tests (what CI runs)
make agent-test       # mac-agent: pytest, ~2 s, no Bluetooth / network / real Claude
make agent-test-all   # + slow tests (the real speech model; needs it downloaded)
make firmware-test    # m5-stick-s3: pure-logic unit tests compiled for the Mac (pio test -e native)
make firmware-build   # build the stick firmware
make lint             # ruff on mac-agent
```

### Mac agent tests
- Every test gets its own temporary agent home (`home` fixture in `tests/conftest.py`), so
  nothing touches the real `~/.stick-agent`, and `claude_path` points at `/usr/bin/false`.
- **The fake stick:** `FakeLink` records every JSON message and audio packet the agent
  sends; `FakeSTT` returns whatever text a test sets; the `agent` fixture wires both in.
  `tests/test_core.py` drives a whole talk (talk start → ADPCM frames → talk stop) through it.
- `tests/fake_claude.py` stands in for `claude -p --output-format stream-json` (modes:
  answer, denied, error, crash, slow).
- GitHub change detection runs on `tests/fixtures/github_base.json` (GraphQL-shaped).
- The hook scripts run as real subprocesses against a test server on the agent's port.
- Mark anything needing the model or hardware `@pytest.mark.slow`.
- **Releases and updates:** tests never reach GitHub or run git / uv for real. `conftest.py`
  refuses `releases.run` / `releases.urlopen` and `selfupdate.git_out` / `run_cmd` (and the
  restart); `tests/test_releases.py` fakes gh and HTTPS, `tests/test_selfupdate.py` fakes git.
  `tests/test_release_tool.py` runs `tools/release.sh` in a throwaway repo.
- **Audio devices:** tests never open a real audio device (`FakeSoundDevice`) and never create
  a CoreAudio device (`FakeCoreAudio` replaces `stick_mic_device.CoreAudioDevices`, autouse).
  The one real end-to-end check creates and removes a test-named aggregate:
  `cd mac-agent && STICK_MIC_E2E=1 ~/.stick-agent/venv/bin/python -m pytest -m coreaudio`
  (deselected by default). Check nothing is left: `system_profiler SPAudioDataType | grep stick-mic`.

### Firmware tests
Pure logic (no Arduino, no hardware) lives in headers/files the `native` PlatformIO env can
compile on the Mac; see `m5-stick-s3/test/` and `m5-stick-s3/Makefile`.

## Debugging

| Where | How |
|---|---|
| **Stick serial log** | `pio device monitor` in `m5-stick-s3/` (115200), or the dev portal (`tools/devportal/start.sh`) → Logs. Only one program can hold the port: close it before flashing. |
| **Stick log over Wi-Fi** | with Wi-Fi on (dev mode on USB power): `nc stickybot.local 23` — same commands as serial |
| **Agent log** | `stickybot logs` (`~/.stick-agent/agent.log`), dashboard → **Logs**, or Activity with "show log lines" |
| **Agent crash at start** | `~/.stick-agent/launchd.out` |
| **Bluetooth messages** | the stick logs `buddy>` (sent) / `agent<` (received) lines |

Useful serial commands on the stick (type `help` for all):

| Command | |
|---|---|
| `show` | name, combo, Bluetooth, battery, slots |
| `screens` | steps through every screen with sample data, portrait then landscape (~3 min) |
| `screens landscape` | only the landscape layouts (button A on the right, then on the left) |
| `sound play <event or tune>` · `sound test` · `sound vol <0-255>` · `sound on/off` | try sounds |
| `event celebrate\|nice\|headsup <title>\|<sub>\|<icon>` | show an event screen |
| `@state` | the full state as JSON (machine command) |
| `imulog` / `imurec <s>` / `chglog` | IMU and charger diagnostics |

## Adding things

**A quick command** (no code): dashboard → Quick commands, or `~/.stick-agent/commands.json`.
Phrases use `{n}` (a number), `{app}` / `{q}` (words); actions `open_url` (+ `browser`),
`open_app`, `shell`. New defaults for everyone: `mac-agent/stick_agent/defaults/commands.json`.
Add a case to `tests/test_commands.py`.

**A sound:** a beep → add it to `TUNES` in `m5-stick-s3/src/sound.cpp`; a real clip → add it
to `CLIPS` in `tools/make_sounds.py` and run it (CC0 / public-domain sources only, or one made
by `tools/synth_sounds.py` as `"synth:<name>"`; list its source in `m5-stick-s3/SOUNDS.md`;
`test_builtin_clips` checks every clip decodes). A new *moment* → a `Cue` in `sound.h`, an
`EVENTS` row in `sound.cpp` (name + default), call `sound::play(...)`, and add the row to
`SOUND_EVENTS` in the dashboard (`dashboard.html`).

**An event kind** (Mac agent): add it to `KINDS` in `stick_agent/events.py` (default level,
icon, sample title; its own sound, if any, in `DEFAULT_SOUNDS`) and call
`agent.notifier.notify("<kind>", title, sub)` where it happens. The Rust app has the same table
in `desktop/crates/stickybot-server/src/events.rs`.
The dashboard's Events tab picks it up from `/api/events/kinds`. Test it in `tests/test_events.py`.

**A screen** (stick): a `draw…` function in `src/ui.cpp` (declared in `ui.h`), shown from
`render()` in `main.cpp`; if it animates, add it to the frame-rate block in `loop()`. Add it to
the `screens` demo. Text must stick to ASCII + `°’‘“”–—…•·×` (what the fonts contain).

**A message between stick and Mac:** describe it in PROTOCOL.md, then `buddy.cpp` (stick)
and `core.py` / `link.py` (agent), with a test on the agent side.

## Conventions

- Python: ruff (`make lint`), line length 120. The existing code keeps its hand-aligned
  style, so the formatter isn't run over it; `make format FILES=...` for new files.
- C++: match the surrounding code: 2-space indent, `namespace` per module, short comments
  saying *why*.
- User-facing text on the stick is short and plain; the agent's replies go through
  `stick_text()`.
- Secrets never go in git: the OTA password lives in `secrets.ini` (git-ignored); the agent's
  token in `~/.stick-agent/token`; the Mac password only on the stick (write-only).

## Releases

`tools/release.sh X.Y.Z` (or `make release VERSION=X.Y.Z`), then push the tag. See
[RELEASING.md](RELEASING.md). Add your changes under `## [Unreleased]` in CHANGELOG.md as
you go.

## Commits

One topic per commit; a short imperative subject ("Add the ticker on/off switch"), a body
saying what and why when it isn't obvious. Run `make check` first; `make dev-setup`
installs the pre-commit hook (ruff, whitespace, no `secrets.ini`, no large files).
