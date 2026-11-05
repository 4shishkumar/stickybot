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

## [1.7.2] - 2026-11-05

The first tagged release: everything on `main` so far. Firmware 1.7.2.

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
- MIT licence and third-party notices; CI on pull requests (macOS runners).

[Unreleased]: https://github.com/ashish-kumar-noon/stick-key/compare/v1.7.2...HEAD
[1.7.2]: https://github.com/ashish-kumar-noon/stick-key/releases/tag/v1.7.2
