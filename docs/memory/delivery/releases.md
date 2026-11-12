---
type: memory
description: "Releases: one version vX.Y.Z for the Mac apps set by tools/release.sh, a separate FW_VERSION for the firmware, both in manifest.json; a tag runs the release workflow (tests, firmware parts, StickyBot.app zip); CHANGELOG.md is the release notes."
---
# Releases

**Domain**: delivery

## Overview

How a version of StickyBot is made and what reaches users. The step-by-step is
`docs/RELEASING.md`; the tooling is `tools/release.sh`, `tools/release_tool.py`,
`m5-stick-s3/tools/make_release.py` and `.github/workflows/release.yml`.

## Requirements

### Requirement: One release version, one firmware version
A release is `vX.Y.Z` (or `vX.Y.Z-beta.N`, published as a prerelease for the beta channel). That
version is the Python app's (`__init__.py`, `pyproject.toml`) and the Rust workspace's
(`desktop/Cargo.toml`, `Cargo.lock`). The firmware keeps `FW_VERSION` in
`m5-stick-s3/src/config.h`, bumped only when the firmware changes. Each release's `manifest.json`
lists both.

### Requirement: The release tool makes the version, people publish it
`tools/release.sh X.Y.Z` sets the versions, turns `[Unreleased]` in `CHANGELOG.md` into the dated
entry, commits `Release vX.Y.Z` and creates the annotated tag. It never pushes, and refuses a
dirty tree or an existing tag. Versions are not edited by hand.

### Requirement: The tag builds and checks everything
The release workflow runs on the tag: it checks the tag against the app versions and the
changelog entry, runs the Python lint and tests and the firmware's native tests, builds the
firmware into four flash parts plus `manifest.json` (versions, offsets, sizes, sha256), builds
StickyBot.app on macOS (tests, clippy, ad-hoc signed, zipped as
`StickyBot-X.Y.Z-macos-arm64.zip`), lists the zip under `apps` in the manifest and creates the
GitHub release with the changelog entry as notes.

#### Scenario: The release workflow fails
- **GIVEN** a pushed tag whose workflow failed
- **WHEN** the problem is fixed
- **THEN** the tag (and any release created) is deleted and the release is tagged again

### Requirement: The changelog is written for users
Every user-visible change adds a line under `[Unreleased]` (Keep a Changelog groups: Added,
Changed, Fixed, Removed), saying what changed for the person using StickyBot. The entry is shown
as the GitHub release notes, in the dashboard banner and in Settings → Updates.

### Requirement: Downloads are verified
Apps download a release's firmware parts and check each against the manifest's sha256 before use.
The speech model and pinned third-party downloads (sherpa-onnx libraries, the BlackHole package)
are checked against pinned hashes.

### Requirement: The Mac app tells, the user installs
The app checks for releases at start and every 24 h. For a newer app it shows "Update available"
and opens the release page; new stick firmware is downloaded and offered on the Stick page, over
Bluetooth or USB.

## Design Decisions

### Firmware ships as separate parts
**Decision**: A release carries `bootloader.bin`, `partitions.bin`, `boot_app0.bin` and `app.bin` with their offsets, not one merged image.
**Why**: Writing the parts at their offsets leaves the settings storage between them untouched, so pairings and settings survive a USB flash.
**Rejected**: A single merged image, whose padding would cover the settings storage and wipe it.
*Introduced by*: predates fab; see `m5-stick-s3/tools/make_release.py`

### Tooling pins live at the top of the workflows
**Decision**: The sherpa-onnx archive (version, name, sha256) and the Tauri CLI version are pinned as workflow environment values, the same in `ci.yml` and `release.yml`.
**Why**: The crate's own build script would download the libraries unchecked, and CI and release builds must not differ.
**Rejected**: Letting each build fetch the latest.
*Introduced by*: predates fab; see `docs/RELEASING.md`

## Sources

`docs/RELEASING.md`, `CHANGELOG.md`, `.github/workflows/release.yml`, `tools/release.sh`.
Related: [CI and review](/delivery/ci-and-review.md), [firmware update](/link/firmware-update.md).
