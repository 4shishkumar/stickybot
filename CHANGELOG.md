# Changelog

All notable changes to stick-key — the Mac companion app (`stick-agent`) and the
M5StickS3 firmware — are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
[Semantic Versioning](https://semver.org/): `vX.Y.Z` is a stable release,
`vX.Y.Z-beta.N` a beta (a GitHub prerelease, offered on the beta channel).

One version number covers the release (the agent's `__version__`); the stick
firmware keeps its own `FW_VERSION`, and each release's `manifest.json` lists both.
How to cut a release: [docs/RELEASING.md](docs/RELEASING.md).

## [Unreleased]

## [1.8.3] - 2026-11-05

### Changed
- **Mic names follow the stick's name**, so two sticks' mics differ: the USB mic is
  `<stick>-mic-usb` (firmware 1.8.1) and the Bluetooth one `<stick>-mic-bluetooth`, e.g.
  `stick-key-mic-usb` / `arbaaz-sticks3-mic-bluetooth`. Wispr sees new devices: rank them
  once more (hold A away from USB for the Bluetooth one; plug the stick in for the USB one).
  A custom *Mic name* in Settings still wins. Renaming the stick renames its mics.
- The home screen shows the stick's name (not "stick-key") until the Mac sends the time.

## [1.8.2] - 2026-11-05

### Added
- **Check for Updates…** in the menu bar (besides the daily check and Settings → Updates →
  Check now); the item says what it found.

### Fixed
- `stick-agent <command> --help` only prints the help: `stick-agent flash --help` used to
  start flashing. `stick-agent flash` refuses options it doesn't know.
- The dashboard checks for a newer copy of its page on every load, so controls added by an
  update (like *Update over Bluetooth*) show up without a hard reload.

## [1.8.1] - 2026-11-05

The first published release (the v1.8.0 build stopped on a test). Firmware 1.8.0.

### Fixed
- A component change from the dashboard always reports when it's done, even if applying
  the new setting fails.

## [1.8.0] - 2026-11-05

The first tagged release: everything on `main` so far. Firmware 1.8.0.

### Added
- **Installer:** `install.sh` (uv, its own Python 3.14 in `~/.stick-agent`, no Homebrew
  needed) and `stick-agent setup` with installable components (core, speech model,
  Claude Code hooks, GitHub alerts, firmware, wireless mic, stick tools), also shown in
  the dashboard's Components card. `--yes`, `--with`, `--without`, `--dry-run`,
  `--status`, `--uninstall [--purge]`.
- **Fn / Globe key** for button A (firmware 1.6.1): the stick re-pairs with a new
  keyboard identity when Fn is added or removed.
- **Wireless Wispr mic:** holding A away from USB streams the stick's mic over
  Bluetooth; the agent plays it into BlackHole 2ch behind a `stick-mic-bluetooth`
  microphone that exists only while A is held. Over USB the stick is the
  `stick-mic-usb` microphone (firmware 1.7.0 / 1.7.1).
- **Persistent Claude Code session:** one long-lived `claude` process (stream-json in
  and out) so a request doesn't pay the start-up cost; one-shot `claude -p` stays as the
  fallback. Replies stream to the stick while Claude writes them and are spoken sentence
  by sentence.
- **Stick tools (MCP):** an MCP server in the agent (`http://127.0.0.1:8660/mcp`) so any
  Claude Code session can notify you, celebrate, ask A/B questions and show progress on
  the stick; `stick-agent mcp install` for terminal sessions, a dashboard card to turn it
  on or off.
- **Firmware 1.7.2:** questions from Claude sessions get their own screen with the two
  answers on A and B.
- **Releases:** one version for the project, this changelog, `tools/release.sh` (local
  commit + tag) and a release workflow that tests, builds the firmware and publishes it as
  release assets with a sha256 manifest.
- **Updates:** the agent checks for new releases daily (stable or beta channel) and shows
  them in the dashboard (banner with the notes and an **Update** button), the menu bar and,
  once, on the stick. Installs made by `install.sh` update themselves in one click. Release
  firmware is downloaded and sha256-checked into `~/.stick-agent/firmware/`.
- **Firmware updates over Bluetooth (firmware 1.8.0):** no cable needed from now on —
  Stick → Firmware → *Update over Bluetooth*, or `stick-agent flash --ble`. The stick
  checks the image's sha256, and rolls back by itself if the new firmware doesn't start
  properly. (Sticks on older firmware need one USB flash to 1.8.0 first.)
- MIT licence and third-party notices; CI on pull requests (macOS runners).

### Changed
- Firmware binaries are no longer committed: they come from releases, or from
  `make -C m5-stick-s3 release` (now written to `~/.stick-agent/firmware/local/`).
- `install.sh` checks out the latest stable release (main before the first one) and
  clones over HTTPS when `gh` isn't logged in. The repo is public: install with
  `curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stick-key/main/install.sh | sh`.
- The PR / Devin quick commands ship with `<org>/<repo>` placeholders (the setup wizard
  fills in your repo) and GitHub alerts have no default repo.

[Unreleased]: https://github.com/ashish-kumar-noon/stick-key/compare/v1.8.3...HEAD
[1.8.3]: https://github.com/ashish-kumar-noon/stick-key/releases/tag/v1.8.3
[1.8.2]: https://github.com/ashish-kumar-noon/stick-key/releases/tag/v1.8.2
[1.8.1]: https://github.com/ashish-kumar-noon/stick-key/releases/tag/v1.8.1
[1.8.0]: https://github.com/ashish-kumar-noon/stick-key/releases/tag/v1.8.0
