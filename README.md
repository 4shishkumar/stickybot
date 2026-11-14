# StickyBot

An M5StickS3 on your desk that pages you, takes voice requests for Claude Code and
works as a dictation key — plus the Mac app that drives it. It grew out of small ESP32
boards that act as a Bluetooth keyboard with one programmable key combo — press for
once, double press for twice, hold to keep it held. Built as a hands-free trigger for apps like Wispr Flow (default combo ⌥ Option + ⌘ Command).

Each board is a **self-contained project** — copy just its folder to get
started. They all follow the same outside behaviour defined in
[PROTOCOL.md](PROTOCOL.md) (serial commands, settings page API, combo format,
Mac slots), so the same settings page works with any of them.

| Folder | Board | Status |
|---|---|---|
| [xiao-esp32-c3/](xiao-esp32-c3/) | Seeed XIAO ESP32-C3 | M1.5: 3 Mac slots, USB + Wi-Fi settings page, OTA |
| [m5-stick-s3/](m5-stick-s3/) | M5Stack M5StickS3 (StickyBot) | screen, menu, settings pages, 3 Mac slots, USB mic, speaker, voice agent link |
| [mac-agent/](mac-agent/) | StickyBot, the Mac app for the M5StickS3 | voice assistant (Claude Code), approvals, alerts, dashboard — **[what it can do](mac-agent/README.md#what-it-can-do)** |

Start with the board's README: install PlatformIO, flash, pair.

**Quick start — the StickyBot Mac app** (voice agent, approvals, alerts, dashboard): download
**StickyBot.app** from the latest release, see [Install the Mac app](#install-the-mac-app) below.
The older Python app is the legacy option for now:
`curl -fsSL https://raw.githubusercontent.com/4shishkumar/stickybot/main/install.sh | sh`,
or `./install.sh` from the repo root, then follow the dashboard's setup wizard. Setting
up someone else's stick: [docs/TEAMMATE-SETUP.md](docs/TEAMMATE-SETUP.md). Building
everything from scratch: [docs/BUILDING.md](docs/BUILDING.md).

## Install the Mac app

StickyBot.app is the Mac app in Rust ([desktop/](desktop/)): menu bar icon, dashboard, the
Bluetooth link to the stick, hold B to talk (speech to text on your Mac), Claude Code approvals on
the stick, the wireless Wispr mic, firmware updates. It needs a Mac with **Apple silicon** and
**macOS 13** or later.

1. Download `StickyBot-<version>-macos-arm64.zip` from the
   [latest release](https://github.com/4shishkumar/stickybot/releases/latest) (under *Assets*).
2. Unzip it (double-click) and move **StickyBot.app** to **/Applications**.
3. **Open it the first time.** The app is not signed with an Apple Developer ID or notarized (it
   is signed ad hoc by the release build on GitHub Actions, from this repo's code), so macOS
   blocks a plain double-click with "Apple could not verify…". Either:
   - in Finder, **right-click** (or Control-click) StickyBot → **Open** → **Open**; or, when macOS
     offers only *Done* / *Move to Trash* (macOS 15 and later): click *Done*, then **System
     Settings → Privacy & Security**, scroll down to "StickyBot was blocked…" → **Open Anyway**
     (your password), then **Open**; or
   - in Terminal: `xattr -dr com.apple.quarantine /Applications/StickyBot.app`, then open it as
     usual.

   macOS remembers this: later opens are a normal double-click.
4. StickyBot lives in the **menu bar** (no Dock icon). Menu → **Open StickyBot** opens the
   dashboard at <http://localhost:8660>. (On a Mac that has the Python app, it starts next to it
   instead, as "StickyBot (Rust)" on <http://localhost:8670>, until you switch over: see below.)
5. **Permissions** macOS asks for, each the first time it's needed:
   - **Bluetooth**: to find and talk to the stick (the first scan or connect);
   - **System Audio Recording**: only for the wireless Wispr mic (Settings → Wireless mic): the
     app taps its own sound to make the stick's mic appear on the Mac;
   - **Login Items**: if you turn on Settings → *Start at login* (System Settings → General →
     Login Items may ask you to allow StickyBot);
   - **Calendars** (Full Access): only if you press Connect on the dashboard's Connections page. StickyBot
     then reads your Mac's calendars for the next meeting on the stick and a reminder before it
     starts. It never changes your calendar, keeps the events only in memory and sends them nowhere
     but to the stick (and to Claude when you ask it about your day);
   - no **Microphone** access: the stick's mic does the listening.

   Because the app is signed ad hoc, macOS sees every new version as a new app: after an update it
   asks for Bluetooth and System Audio Recording again (allow them again; if a switch in
   *Privacy & Security* is already on but nothing works, turn it off and on).
6. **Setup**: the menu's **Finish setup…** opens the setup page: the speech model (a one-time
   487 MB download into `~/.stickybot-rs/models/`, not part of the app), Claude Code and its hooks
   (approvals on the stick), the stick tools for terminal sessions, GitHub, the wireless mic, and
   **Pick your stick** (scans and pairs over Bluetooth). Settings and data live in
   `~/.stickybot-rs/`.
7. **Firmware**: the app downloads each release's stick firmware. The dashboard's **Stick** page →
   *Firmware* updates the stick **over Bluetooth**, or **via USB** (needs esptool:
   `brew install esptool`; a new stick that has never run StickyBot is flashed this way, see
   [m5-stick-s3/](m5-stick-s3/)).

**Updating**: the app checks for releases (Settings → Updates). When there's a newer StickyBot,
the menu shows **Update available: vX — download…** (it opens the release page) and the dashboard
says so too; it can't install itself yet: download the new zip, quit StickyBot and replace
StickyBot.app in /Applications (`~/.stickybot-rs/` stays). New stick firmware is downloaded for
you and offered on the Stick page. **Building it yourself** (or signing it with your own
certificate, so permissions survive updates): [desktop/README.md](desktop/README.md).

**Coming from the Python app** (`install.sh`, `~/.stick-agent/`): StickyBot.app notices it on its
first start (that folder, or the app answering on port 8660) and doesn't take its place. The
Python app stays installed and keeps working; the two run side by side (the Python one on port
8660, StickyBot.app as "StickyBot (Rust)" on 8670, holding off the stick while the Python app has
it). To move over, follow
[desktop/SWITCHOVER.md](desktop/SWITCHOVER.md): *Import from StickyBot* takes your settings, voice
commands, sounds, history, events, GitHub state and the pet (merged with anything the new app
already has; `~/.stick-agent/` is only read), then you stop the Python app, *Switch over* (the new
app takes port 8660 and the Python app's token, so Claude Code's hooks and terminal sessions keep
working) and restart. Going back is described there too.

Releases: one version `vX.Y.Z` for the whole project ([CHANGELOG.md](CHANGELOG.md)).
Each GitHub release carries the stick firmware, and the agent checks for new ones and
updates itself and the stick ([docs/RELEASING.md](docs/RELEASING.md)).

Not in git: `.pio/` (build cache, recreated automatically), `backup/`
(per-device factory images) and firmware binaries (they're release assets; a local
build goes to `~/.stick-agent/firmware/local/`).
