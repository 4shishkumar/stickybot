# StickyBot Constitution

## Core Principles

### I. Every Change Reaches `main` Through a Reviewed Pull Request
`main` is protected. Every change MUST reach it through a pull request whose **CI passed** check is green and that has one approval from someone other than its author; an approval given before the last push does not count, and every review comment MUST be resolved. Pull requests MUST be squash-merged, so the PR title SHALL say what changed. Nobody pushes to `main` directly; an owner who bypasses this in an emergency MUST say so in the PR or commit. Agents MUST NOT merge their own work, approve it, or push without being asked.

*Why*: the stick sits on people's desks and types into their Macs. A second pair of eyes and a green build are the cheapest way to keep a bad change from getting there. (docs/CONTRIBUTING.md)

### II. Old and New Work Together
A newer Mac app MUST work with older stick firmware, and newer firmware with an older app. Either side MUST ignore messages and fields it does not know, and MUST only use a feature the other side has said it has (the stick's `@state` keys such as `ota`, `pkgFaces`, `ir`; the app's hello flags such as `wmic`, `wready`, `speak`). A change that needs a minimum version on the other side MUST say which, and MUST fall back or explain itself in plain words when it is missing. Any change to a message between the Mac and the stick MUST update `PROTOCOL.md` in the same pull request, before or with the code on both sides.

*Why*: the app and the firmware update separately, and a stick is not always in reach of a cable. Nobody should have to update two things at once to keep a working setup. (PROTOCOL.md)

### III. The Real Stick Is the Truth
Firmware changes and changes to what the stick shows, plays or receives MUST be tried on a real stick before they are released; the pull request MUST say what was tried and what was seen, and carries `needs-hardware` until then. A passing build or a fake stick is not that check. Updates MUST keep the stick's settings and pairings: only the app image is written over Bluetooth, USB flashing writes the release's separate parts at their offsets, and a merged or full-flash image (which would overwrite the settings storage) MUST NOT be flashed as an update. A factory backup is only ever written back when its owner asks for exactly that. A firmware update MUST be resumable after a dropped link and cancellable from the Mac and from the stick, MUST be checked against its sha256 before it boots, and a new image that does not confirm itself MUST roll back.

*Why*: Bluetooth, the screen, audio and power only show their real behaviour on the device, and a stick that forgot its Mac or will not boot is the worst thing an update can do. (docs/ARCHITECTURE.md, PROTOCOL.md "Firmware update over Bluetooth", m5-stick-s3/tools/make_release.py)

### IV. No Secrets, No Personal Data
Keys, tokens, passwords and personal or employer data (names, private repos, emails) MUST NOT appear in the repo, in tests, in logs or in anything a user sends as feedback. Real keys live in the macOS Keychain, the git-ignored `secrets.ini`, or GitHub Actions secrets. Tests that need something key-shaped MUST build it from pieces so secret scanners do not flag it. Diagnostics MUST be opt-in and redacted before they leave the Mac; the project collects no telemetry. Values the user typed as secrets (the Mac password on the stick, the OpenRouter key) MUST be write-only: never read back, logged or returned by an API.

*Why*: the repo is public, and the app runs on work machines next to private code. (docs/CONTRIBUTING.md "Test fakes, not secrets", docs/ARCHITECTURE.md "Security model")

### V. Pure Logic Is Kept Apart and Tested Without Hardware
Logic that does not need the hardware MUST live apart from the code that touches it (firmware: `m5-stick-s3/src/logic/`; Rust: modules behind traits, with fakes; dashboard: plain JS modules) and MUST have unit tests that run without a stick, Bluetooth, audio devices, the network or a real Claude. New logic SHOULD arrive with its tests. The firmware's native tests MUST give the same result with gcc (CI on Linux, the stick's compiler family) and clang (a Mac): code MUST NOT depend on unspecified behaviour such as the order in which arguments are evaluated.

*Why*: most of what can go wrong is logic, and logic can be checked in seconds on any machine. The two compilers have disagreed before and stopped a release. (m5-stick-s3/README.md "Tests", CHANGELOG.md 2.3.2)

### VI. The Stick's Resources Are Budgets
The firmware MUST stay at or below 85 % of the app partition (3,342,336 bytes); CI fails above that. Changes SHOULD account for what they cost in Bluetooth buffers, memory and battery, and MUST NOT make replies such as `@state` grow without limit. Third-party code, fonts, sounds and images MAY be added only under a licence compatible with the project's MIT licence, and MUST be recorded in `THIRD_PARTY_NOTICES.md` in the same pull request.

*Why*: the stick has a fixed amount of flash and a small battery, and an over-full Bluetooth buffer has dropped key presses before. A licence found out later is much harder to fix. (.github/workflows/ci.yml, m5-stick-s3/platformio.ini, THIRD_PARTY_NOTICES.md)

### VII. One Version per Release, and a Changelog People Can Read
A release has one version, set by `tools/release.sh` for the Mac apps together; the firmware keeps `FW_VERSION` (bumped only when the firmware changes), and each release's manifest lists both. Versions MUST NOT be bumped by hand outside that flow. Every change a user can notice MUST add a line under `[Unreleased]` in `CHANGELOG.md`, written for the person using StickyBot: what changed for them, in plain words. Changes with nothing to notice (docs, CI, tests, tooling) carry the `no-changelog` label instead.

*Why*: the changelog becomes the release notes, the dashboard banner and the update screen, so it is read by people who never see the code. (docs/RELEASING.md, CHANGELOG.md)

## Additional Constraints

### Test Integrity
Tests MUST conform to the implementation spec — never the other way around. When tests fail, the fix SHALL either (a) update the tests to match the spec, or (b) update the implementation to match the spec. Modifying implementation code solely to accommodate test fixtures or test infrastructure is prohibited. Specs are the source of truth; tests verify conformance to specs.

### Do Not Touch What Is Running
Automated work MUST NOT connect to, flash or reconfigure a user's stick, start or stop their running StickyBot, or change files under their home directory (`~/.stickybot-rs`, `~/.stick-agent`, `~/.claude`) unless the user asked for exactly that. The stick takes one Bluetooth connection, and tests use fakes and temporary homes.

## Governance

**Version**: 1.0.0 | **Ratified**: 2026-11-12 | **Last Amended**: 2026-11-12
