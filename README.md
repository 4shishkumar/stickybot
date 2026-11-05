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

**Quick start — the StickyBot Mac app** (voice agent, approvals, alerts, dashboard):
`curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stickybot/main/install.sh | sh`,
or `./install.sh` from the repo root, then follow the dashboard's setup wizard. Setting
up someone else's stick: [docs/TEAMMATE-SETUP.md](docs/TEAMMATE-SETUP.md). Building
everything from scratch: [docs/BUILDING.md](docs/BUILDING.md).

Releases: one version `vX.Y.Z` for the whole project ([CHANGELOG.md](CHANGELOG.md)).
Each GitHub release carries the stick firmware, and the agent checks for new ones and
updates itself and the stick ([docs/RELEASING.md](docs/RELEASING.md)).

Not in git: `.pio/` (build cache, recreated automatically), `backup/`
(per-device factory images) and firmware binaries (they're release assets; a local
build goes to `~/.stick-agent/firmware/local/`).
