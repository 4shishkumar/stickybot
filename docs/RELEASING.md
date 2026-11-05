# Releasing

stick-key has **one release version**, `vX.Y.Z`: the Mac agent's version
(`mac-agent/stick_agent/__init__.py` and `mac-agent/pyproject.toml`). The stick
firmware keeps its own `FW_VERSION` (`m5-stick-s3/src/config.h`). It changes only when
the firmware does, and each release's `manifest.json` lists both versions.

| Tag | What it is | Who gets it |
|---|---|---|
| `v1.8.0` | a stable release | everyone (update channel **stable**, the default) |
| `v1.8.0-beta.1` | a beta: published as a GitHub **prerelease** | only agents on the **beta** channel |

Ordering follows SemVer: `1.8.0-beta.2 < 1.8.0-beta.10 < 1.8.0-rc.1 < 1.8.0`.

## Cutting a release

1. **Describe the changes** under `## [Unreleased]` in [CHANGELOG.md](../CHANGELOG.md)
   ([Keep a Changelog](https://keepachangelog.com/en/1.1.0/): Added / Changed / Fixed / Removed).
   That section becomes the release notes on GitHub, in the dashboard banner and in
   Settings → Updates.
2. **Firmware changed?** Bump `FW_VERSION` in `m5-stick-s3/src/config.h` and commit it.
   (Leave it alone for an agent-only release. The manifest still carries the firmware, and
   agents don't offer it to sticks that already run it.)
3. **Check:** `make check` on a clean `main`.
4. **Tag, locally:**
   ```sh
   tools/release.sh 1.8.0            # or: make release VERSION=1.8.0
   tools/release.sh 1.8.0-beta.1     # a beta
   tools/release.sh --dry-run 1.8.0  # show what it would do and the notes
   ```
   It sets the version in `mac-agent`, turns `[Unreleased]` into `## [1.8.0] - <today>`
   (and updates the links at the bottom), commits `Release v1.8.0`, and creates the
   annotated tag `v1.8.0`. **It never pushes.** It refuses to run on a dirty tree or when
   the tag already exists.
5. **Publish:** push the commit and the tag. The script prints the exact command:
   ```sh
   git push origin main v1.8.0
   ```
6. The **Release** workflow ([.github/workflows/release.yml](../.github/workflows/release.yml))
   runs on the tag, on one ubuntu-latest runner (about 5 minutes; PlatformIO is cached):
   - checks that the tag matches the agent version and that CHANGELOG.md has its entry
   - runs ruff and the agent tests (without the macOS-only libraries, which the tests don't need)
   - runs the firmware's native unit tests and builds the firmware
   - `m5-stick-s3/tools/make_release.py --out dist`: `bootloader.bin`, `partitions.bin`,
     `boot_app0.bin`, `app.bin` + `manifest.json` (firmware and agent versions, git commit,
     every part's offset / size / sha256, the flash parameters)
   - creates the GitHub release with those 5 assets and the changelog entry as notes,
     as a prerelease when the tag has a `-` suffix

   Only the built-in `GITHUB_TOKEN` is used. If the workflow fails, fix the problem, delete
   the tag (`git tag -d v1.8.0 && git push origin :refs/tags/v1.8.0`) and the GitHub
   release if one was created, then tag again.

### The first release

`main` is already at 1.7.2 with a dated changelog entry, so:
```sh
tools/release.sh 1.7.2        # nothing to bump: tags v1.7.2 (re-dates the entry if today differs)
git push origin v1.7.2
```

## What users see

- **Update check:** the agent checks at start-up and every 24 h (Settings → Updates:
  *Check automatically*, *Channel*, *Check now*; or the menu bar's *Check for Updates…*). It lists releases with `gh` when that's
  installed and logged in (higher API rate limits), and otherwise over plain HTTPS, no login.
- **Firmware download:** when a release has firmware that isn't on this Mac yet, the agent
  downloads it into `~/.stick-agent/firmware/<firmware version>/`. Each part is checked
  against the manifest's sha256 (and GitHub's own asset digest) before it's used. It keeps
  the 3 newest versions. The installer's `firmware` component downloads the latest release
  first too.
- **Notices:** a banner in the dashboard with the release notes and an **Update** button,
  "Update available: vX" in the menu bar, and one toast on the stick per release
  ("Update vX ready — see the dashboard"; the *update* event kind in Events, held back during
  quiet hours).
- **Update button:**
  - *Agent*, for installs made by `install.sh` (a clone in `~/.stick-agent/src`):
    `git fetch --tags` + check out the release tag + `uv pip install -e` + restart, with
    progress in the banner. A failed step puts the old checkout back. A developer checkout
    (the agent runs from a clone somewhere else) is never touched: the dashboard shows the
    commands to run instead.
  - *Stick firmware*: Stick → Firmware → **Update via USB**. Other ways, such as Bluetooth
    OTA, plug in through `updates.register_firmware_updater()` and appear next to it.
- **Re-running install.sh** moves a release install to the newest stable release after it.
  A fresh install starts on the latest stable release (or `main` before the first one).

## Developers: firmware without a release

`make -C m5-stick-s3 release` builds the firmware and writes it to
`~/.stick-agent/firmware/local/`. The agent uses the highest firmware version it has, and a
local build wins a tie with a downloaded release of the same version. Firmware binaries
are not committed any more (`mac-agent/stick_agent/firmware/` is git-ignored).
