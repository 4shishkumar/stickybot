# Building from scratch

Everything needed to go from a fresh Mac to a working stick + Mac agent.
For day-to-day development (tests, debugging, adding features) see
[DEVELOPMENT.md](DEVELOPMENT.md).

## Hardware

| | |
|---|---|
| **M5Stack M5StickS3** | the voice stick (screen, mic, speaker, IMU, battery) |
| **USB-C data cable** | a charge-only cable won't show a serial port; plug straight into the Mac, not a monitor hub, while flashing |
| Seeed XIAO ESP32-C3 *(optional)* | the simpler one-button macro keyboard in `xiao-esp32-c3/` |

## Software

| What | How | Why |
|---|---|---|
| Homebrew | https://brew.sh | to install the rest |
| Python **3.14** | `brew install python@3.14` | Not 3.15: the pioarduino PlatformIO platform rejects it. The agent works on 3.11+. |
| PlatformIO | `python3.14 -m venv ~/.platformio/penv && ~/.platformio/penv/bin/pip install platformio` | builds and flashes the firmware; the first build downloads the ESP32 toolchain (a few minutes) |
| Claude Code | https://claude.com/claude-code, then `claude` once to log in | the agent's "brain" uses your login |
| GitHub CLI *(optional)* | `brew install gh && gh auth login` | PR / CI alerts on the stick |
| Xcode command-line tools | `xcode-select --install` | `git`, compilers for the native tests |

Optional, so `pio` works without the full path: `export PATH="$HOME/.platformio/penv/bin:$PATH"` in `~/.zshrc`.

Only needed to *regenerate* generated files (they're committed): Pillow (fonts),
numpy + soundfile + soxr (sounds) — all already in the agent's venv.

## 1. Back up the stick's factory firmware (once)

```sh
cd m5-stick-s3 && tools/backup.sh /dev/cu.usbmodemXXXX   # writes backup/sticks3-factory-<date>.bin (git-ignored)
```
Pass the stick's port: without it the script takes the first `/dev/cu.usbmodem*`, which can
be a monitor's USB hub. List ports with `~/.platformio/penv/bin/pio device list` (the stick is
the Espressif one, VID 303A).
`tools/restore.sh <file>` puts it back. Details in [m5-stick-s3/README.md](../m5-stick-s3/README.md).

## 2. Build and flash the stick

```sh
cd m5-stick-s3
pio run -e sticks3 -t upload           # build + flash over USB
```

- The firmware uses TinyUSB, so PlatformIO resets the stick into the bootloader with a
  **1200-baud "touch"** (`board_upload.use_1200bps_touch` in `platformio.ini`). It happens
  automatically; the serial port name changes briefly.
- **"Failed to connect / No serial data received"** — usually something else holds the
  port (a serial monitor, the dev portal's log, a browser tab with the USB settings page).
  Close it and retry; a second attempt normally works.
- **No port at all / the stick is stuck** — **manual download mode**: hold the **side
  power/reset button** until the green LED flashes, then flash. The power chip keeps it in
  download mode afterwards: press the side button once to start normally.
- Then pair it with the Mac: System Settings → Bluetooth → **stick-key** → Connect, and
  accept the code on the stick (button A).

### Over Wi-Fi (OTA)
1. Copy `secrets.example.ini` to `secrets.ini` (git-ignored) and put your **device
   password** in it (`upload_flags = --auth=<password>`; set the password on the settings page).
2. Turn on the stick's Wi-Fi: menu → Web settings, or Dev mode while on USB power.
3. `pio run -e sticks3_ota -t upload` (uploads to `stick-key.local`).

### The XIAO ESP32-C3 (optional)
`cd xiao-esp32-c3 && pio run -e xiao_c3 -t upload`. If the port doesn't appear: unplug, hold
**BOOT**, plug in, release, flash, then press **RESET**. See its [README](../xiao-esp32-c3/README.md).

## 3. Install and start the Mac agent

```sh
./install.sh            # from the repo root
```

This installs uv if needed, creates `~/.stick-agent/venv` (Python 3.14), installs
`mac-agent/` into it **editable** (so code changes apply after `stick-agent restart`),
and runs `stick-agent setup` to pick the components (speech model, Claude Code
hooks, GitHub alerts, flashing a stick). See [mac-agent/README.md](../mac-agent/README.md#install)
for the components and options. Re-run it any time; it only changes what's missing.

Testing the installer without touching your own setup: `make install-e2e` runs it
for real into a throwaway HOME (namespaced login item and port, its own Claude
settings file), checks `stick-agent setup --status`, re-runs it and uninstalls.
These variables move everything it touches: `STICK_AGENT_HOME`, `STICK_AGENT_PORT`,
`STICK_AGENT_LABEL`, `STICK_AGENT_BIN`, `STICK_AGENT_CLAUDE_SETTINGS`, and
`STICK_AGENT_NO_LOGIN_ITEM=1` (or `--no-login-item`).

The manual way, if you need it:

```sh
/opt/homebrew/bin/python3.14 -m venv ~/.stick-agent/venv
~/.stick-agent/venv/bin/pip install -e mac-agent
~/.stick-agent/venv/bin/stick-agent setup
```

## 4. Regenerating fonts and sounds (only when changing them)

```sh
cd m5-stick-s3
~/.stick-agent/venv/bin/python tools/make_fonts.py    # Inter (OFL) -> src/fonts_vlw.{h,cpp}
~/.stick-agent/venv/bin/python tools/make_sounds.py   # Kenney packs (CC0, downloaded) -> src/sounds_clips.{h,cpp}
```
Edit the `FONTS` / `CLIPS` tables at the top of each script. Rebuild and flash afterwards.

## Troubleshooting

| Problem | Fix |
|---|---|
| Port busy / "resource busy" | close serial monitors, the dev portal log, the USB settings page tab; retry |
| Flash "failed to connect" | retry once; then manual download mode (side button until the green LED flashes) |
| Stick not found by the agent | is it paired in Bluetooth settings and awake? Dashboard → Settings → Find sticks nearby; `stick-agent restart` |
| Agent stuck on "Thinking…" | `stick-agent logs` for a traceback; click **B** on the stick to cancel; `stick-agent restart` |
| Speech misheard as another language | the default speech model is English-only (`stt_model` in `~/.stick-agent/config.json`) |
| `stick-agent: command not found` | the `ln -sf` step above, or use `~/.stick-agent/venv/bin/stick-agent` |
| No sound from the stick | menu → Sounds ON and Volume; the menu-bar icon → Stick volume |

## Flashing keeps your settings — don't use a merged image

The stick keeps its settings (Mac pairings, combo, Mac password, Wi-Fi, sounds…)
in the flash's NVS area (0x9000–0xDFFF). Normal flashing — `pio run -t upload`,
`make -C m5-stick-s3 flash`, the dashboard's **Update via USB**, `stick-agent flash`
— writes only the bootloader (0x0), partition table (0x8000), boot_app0 (0xE000)
and the app (0x10000), so settings survive.

A single merged/"factory" image written at 0x0 (e.g. pioarduino's
`firmware.factory.bin`) covers the NVS area with blank padding and **erases the
settings and Bluetooth pairing** — the stick then has to be re-paired with the Mac.
The agent's flasher refuses images that overlap NVS. Restoring a full backup
(`tools/restore.sh`) is the one deliberate exception.

Also: never run a serial logger while flashing — it grabs the stick's bootloader
port too.
