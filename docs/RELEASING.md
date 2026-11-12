# Releasing

StickyBot has **one release version**, `vX.Y.Z`: the Mac agent's version
(`mac-agent/stick_agent/__init__.py` and `mac-agent/pyproject.toml`), which is also the Rust
app's (`desktop/Cargo.toml` `[workspace.package] version`, used by `stickybot-server` and
`stickybot-app`, and their `desktop/Cargo.lock` entries: `CARGO_PKG_VERSION`, StickyBot.app's
bundle version, `/api/rust`, and what its update check compares releases with). The stick
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
   It sets the version in `mac-agent` and `desktop/` (`Cargo.toml` + `Cargo.lock`), turns `[Unreleased]` into `## [1.8.0] - <today>`
   (and updates the links at the bottom), commits `Release v1.8.0`, and creates the
   annotated tag `v1.8.0`. **It never pushes.** It refuses to run on a dirty tree or when
   the tag already exists.
5. **Publish:** push the commit and the tag. The script prints the exact command:
   ```sh
   git push origin main v1.8.0
   ```
6. The **Release** workflow ([.github/workflows/release.yml](../.github/workflows/release.yml))
   runs on the tag, in three jobs:
   - **tests** (ubuntu-latest, about 5 minutes; PlatformIO is cached):
     - checks that the tag matches the agent and Rust app versions and that CHANGELOG.md has its entry
     - runs ruff and the agent tests (without the macOS-only libraries, which the tests don't need)
     - runs the firmware's native unit tests and builds the firmware
     - `m5-stick-s3/tools/make_release.py --out dist`: `bootloader.bin`, `partitions.bin`,
       `boot_app0.bin`, `app.bin` + `manifest.json` (firmware and agent versions, git commit,
       every part's offset / size / sha256, the flash parameters)
   - **mac-app** (macos-15, Apple silicon; after *tests*): the Rust app in `desktop/`:
     `cargo test --workspace`, `cargo clippy -D warnings`, then `desktop/scripts/build-app.sh`
     with the release's version, **signed ad hoc** (no Apple certificate in CI: users open it with
     right-click → Open, see the README's *Install the Mac app*), zipped with `ditto` as
     `StickyBot-X.Y.Z-macos-arm64.zip` (~16 MB). With the update key (below) it also makes the
     signed update for the in-app updater: `StickyBot.app.tar.gz` and its `.sig`, checked after the
     build (the `.app` inside must still pass `codesign --verify --deep --strict`, be signed ad hoc
     and have the hardened runtime flag) and uploaded as
     `StickyBot-X.Y.Z-macos-arm64.app.tar.gz` and `….app.tar.gz.sig`. Cached: cargo's registry and `target/`
     (Swatinem/rust-cache), the Tauri CLI (`TAURI_CLI_VERSION`, installed with
     `cargo install --locked` on a cache miss) and sherpa-onnx's prebuilt libraries (a pinned
     URL, checked against `SHERPA_ONNX_SHA256`; the crate's own build script would download them
     unchecked). The speech model is never bundled: the app downloads it on first start. About
     15–25 minutes cold, 5–10 with the caches.
   - **publish** (ubuntu-latest): `tools/release_tool.py add-app` lists the zip in
     `manifest.json` (`"apps": [{name, platform, version, file, size, sha256}]`; agents read only
     the firmware keys, so older ones are unaffected), writes `latest.json` for the in-app updater
     when the signed update exists (`tools/release_tool.py latest-json`: the version, the
     changelog entry as `notes`, the update's signature and its download URL), then creates the
     GitHub release "StickyBot vX.Y.Z" with the 5 firmware assets, the zip, the update files
     (`.app.tar.gz`, `.sig`, `latest.json`) and the changelog entry as notes, as a prerelease when
     the tag has a `-` suffix

   **Trying the Mac build before tagging:** Actions → Release → *Run workflow* on a branch runs
   only *mac-app* (no tag check, nothing published); the zip
   (`StickyBot-<version>-dev.<commit>-macos-arm64.zip`) is the run's `mac-app` artifact, with the
   update files next to it when the secrets are set (without them the build still passes and says
   it skipped them).

   Moving to a newer sherpa-onnx (`desktop/crates/stt/Cargo.toml`) or Tauri: update
   `SHERPA_ONNX_VERSION` / `SHERPA_ONNX_ARCHIVE` / `SHERPA_ONNX_SHA256` (`shasum -a 256` of the
   `osx-arm64-static-lib` archive from k2-fsa's release) or `TAURI_CLI_VERSION` at the top of the
   workflow.

   Besides the built-in `GITHUB_TOKEN`, the only secrets are the two update-key ones
   (`TAURI_SIGNING_PRIVATE_KEY` and its password, see *Update signing key*). **A tag pushed without
   them still publishes the release**, with a warning in the run: it just has no update file, so
   people install it by hand (a release that can't be installed from inside the app is better than
   firmware that can't be released at all; if you'd rather fail the tag, add `exit 1` to the two
   "no update" branches of the workflow). If the workflow fails, fix the problem, delete
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
    Before fetching it points an install clone's `origin` that still names
    an old name (`ashish-kumar-noon/stick-key` or `ashish-kumar-noon/stickybot`) at `4shishkumar/stickybot` (install.sh does the same).
    The new code then builds StickyBot.app if Python changed or it isn't built yet.
  - *Stick firmware*: Stick → Firmware → **Update via USB**. Other ways, such as Bluetooth
    OTA, plug in through `updates.register_firmware_updater()` and appear next to it.
- **Re-running install.sh** moves a release install to the newest stable release after it.
  A fresh install starts on the latest stable release (or `main` before the first one).
- **StickyBot.app (the Rust app) installs its own updates** (*Install update vX and restart…* in
  the menu, *Install and restart* in the dashboard): see the next section.

## Updates inside StickyBot.app

A release that has `latest.json` (and the signed `StickyBot-X.Y.Z-macos-arm64.app.tar.gz` it
points at) can be installed from inside the Mac app. The app (`stickybot-app`, Tauri's updater
plugin) reads `latest.json`, downloads the tarball, checks its minisign signature **over the whole
download** against the public key built into the app (`plugins.updater.pubkey` in
`desktop/crates/stickybot-app/tauri.conf.json`, with `requireSignedVersion` so a signature also
binds the version it was made for), and only then swaps the `.app` for the new one and opens it
again. It never installs a version that isn't newer than the running one, and it does nothing
without that signature.

| Channel (Settings → Updates) | `latest.json` it reads |
|---|---|
| stable (default) | `https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json` (GitHub's "latest" is never a prerelease) |
| beta | `https://github.com/4shishkumar/stickybot/releases/download/<tag>/latest.json` for the release the update check picked (newest by SemVer, prereleases included; for a stable one that is its own file) |

The address is built from the repo compiled into the app and a checked tag; no setting or
environment variable changes it in a release build. A release without `latest.json` (anything up to
2.5.0, or one published without the key) keeps the old behaviour: *Update available: vX —
download…* and the release page.

The Mac app can't replace itself where it can't write (a translocated, read-only copy; an
Applications folder this user can't change) or when it isn't a `.app` (a developer build): there it
falls back to the release page, and it never asks for an administrator password.

**Versions up to 2.5.0 have no updater inside.** The first release with it is installed by hand,
once, like any first install (README: *Install the Mac app*); the changelog line and the release
notes say so.

### Update signing key

The update key is a minisign key pair made by the Tauri CLI. **The public key is in the repo**
(`tauri.conf.json`); **the private key lives only in this repository's Actions secrets** (plus the
owner's own backup of it), nowhere else: never in the repo, a test or a log.

Add the two secrets (the owner does this; once):

1. Settings → Secrets and variables → Actions → *New repository secret*.
2. `TAURI_SIGNING_PRIVATE_KEY`: the private key (the content of the key file, the long base64
   text, or the path where the Tauri CLI wrote it when run locally).
3. `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: its password (empty if the key has none).

Make a key pair (only when there isn't one, or to rotate): `cargo tauri signer generate -w <file>`
asks for a password and writes the private key to `<file>` and the public key to `<file>.pub`;
`TAURI_SIGNING_PRIVATE_KEY` / `_PASSWORD` in the environment (or the same secrets) are what
`desktop/scripts/build-app.sh` and the workflow sign with. Keep a copy of the private key and its
password somewhere safe, apart from the repo.

Without the secrets a build still works: `build-app.sh` says *no update key: updater artifacts
skipped*, the workflow's `mac-app` job passes on a pull request or a run by hand, and a tag still
publishes the release (see above), without update files.

**Rotating the key** (it may have leaked, or the password is forgotten but the key file is not):
make a new pair, put the new public key in `tauri.conf.json` and the new private key in the
secrets, and cut a release **signed with the old key** that carries the new public key — people
update to it (their app trusts the old key), and from then on releases are signed with the new
one. So: first release with the new `pubkey` is still signed with the old private key; only the
release after that is signed with the new key. The old private key must still be at hand for that
one release.

**If the private key is lost** (or it leaked and nobody can use it to ship the rotation release):
no installed app can verify an update signed with a new key, so nothing can be updated in-app any
more. Make a new pair, put its public key in `tauri.conf.json`, release, and tell people to
install that release **by hand once** (as for the first updater release); in-app updates work
again from then on.

### Checking a release's update file

`latest.json` is plain JSON (Tauri's static update format: `version`, `notes`, `pub_date`,
`platforms.darwin-aarch64.{signature,url}`); `signature` is the `.sig` file's content. Before
tagging, `tools/release_tool.py latest-json out.json --version X.Y.Z --tag vX.Y.Z --sig <file>.sig
--asset StickyBot-X.Y.Z-macos-arm64.app.tar.gz` shows what the workflow will write; it refuses an
empty signature, a version that isn't SemVer and an asset or repo name that wouldn't give
`https://github.com/<repo>/releases/download/<tag>/<asset>`.

## The BlackHole pin (setup page)

The setup page's one-click BlackHole 2ch install uses one tested package, pinned in
`installer.BLACKHOLE_PIN` (version, url, sha256), and checks its signature (Existential
Audio's Developer ID, notarized). To move to a newer BlackHole: test it with Wispr first
(wireless mic, a few dictations), then copy `version`, `url` and `sha256` from
https://formulae.brew.sh/api/cask/blackhole-2ch.json into the pin and note it in the
changelog. An installed BlackHole is never reinstalled or downgraded.

## Developers: firmware without a release

`make -C m5-stick-s3 release` builds the firmware and writes it to
`~/.stick-agent/firmware/local/`. The agent uses the highest firmware version it has, and a
local build wins a tie with a downloaded release of the same version. Firmware binaries
are not committed any more (`mac-agent/stick_agent/firmware/` is git-ignored).
