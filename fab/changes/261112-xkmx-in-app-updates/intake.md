# Intake: In-app updates for StickyBot.app

**Change**: 261112-xkmx-in-app-updates
**Created**: 2026-11-12

## Origin

One-shot request (no back-and-forth; the brief stood in for the user, and every point a skill
would have asked about is decided below and listed under Assumptions):

> Build in-app updates for the StickyBot Mac app ("click → downloads in the background → verifies
> → installs → restarts", like every other app). Use Tauri's updater plugin (tauri-plugin-updater,
> matching the Tauri 2 version in Cargo.lock) — or, if it cannot work for this app (justify), an
> equivalent in-app updater with the same signature guarantees. Releases are signed with an update
> key (minisign, Tauri signer); the public key goes in tauri.conf.json `plugins.updater.pubkey`; the
> private key and its password will be the GitHub Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and
> `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (the owner adds them). The release workflow builds the
> updater artifacts when the secrets are present and uploads them plus `latest.json`; a build
> without the secrets must still pass. The app shows "Install update vX and restart…" in the menu
> and a banner / card with release notes, **Install and restart**, a progress bar and clear errors
> in the dashboard, waits while the stick is in use, tells the stick, and says "Updated to vX"
> after the restart. A setting `update_auto` (default false) installs when idle. Extend
> `/api/update`, add `POST /api/update/install`. Tests, docs, a security section in the PR,
> screenshots.

Constraints given with the request: another change (a Calendar connection) is being built on
another branch at the same time and touches `desktop/crates/stickybot-server`, `tauri.conf.json`
(Info.plist keys / entitlements) and a new Connections page, so edits to shared files must be
small and well separated. Never launch the real app, never touch `/Applications/StickyBot.app`,
ports 8660 / 8670, the stick, `~/.stickybot-rs`, `~/.stickybot-signing`, `~/.claude*` or the
Keychain. No private keys in the repo.

## Why

1. **The pain**: today the Rust app only *finds* a newer release (`updates.rs`: "Update available:
   vX — download…" in the menu, a card in Settings → Updates). The person then downloads
   `StickyBot-<ver>-macos-arm64.zip`, unzips it, quits the app and replaces
   `/Applications/StickyBot.app` by hand. Most people don't, so fixes don't reach them
   (`POST /api/update/agent` answers 409 "the Rust app doesn't update itself yet").
2. **If we don't**: releases keep piling up on machines that never get them; every bug report
   starts with "which version?"; and a hand-replaced app is exactly where people grab a build from
   the wrong place. An update that is checked against a key the app carries is safer than a
   download page.
3. **Why this way**: Tauri 2 has an updater plugin that does download → minisign signature check →
   replace the `.app` → relaunch, and the Tauri CLI already builds the signed
   `StickyBot.app.tar.gz` (`createUpdaterArtifacts`). It needs no Apple Developer ID: the minisign
   signature, not Gatekeeper, is what the app trusts. Writing our own would mean owning tar
   extraction, atomic bundle swap and signature code for no gain. The plugin lives in the Tauri
   crate (`stickybot-app`); everything that decides (state machine, busy gate, channel → endpoint,
   downgrade refusal) stays in `stickybot-server` behind a trait, with a fake, as Constitution V
   asks.

## What Changes

### 1. Signing key and Tauri configuration

- `desktop/crates/stickybot-app/Cargo.toml`: add `tauri-plugin-updater = "2"` (the release that
  goes with tauri 2.12.x in `desktop/Cargo.lock`; let cargo resolve it, never hand-edit the lock).
- `desktop/crates/stickybot-app/tauri.conf.json`: add a top-level `plugins` object only (leave
  `app`, `bundle.macOS` etc. alone: the Calendar change edits Info.plist keys / entitlements):

  ```json
  "plugins": {
    "updater": {
      "pubkey": "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDE2RURCNUI3QzlEMDVENTAKUldSUVhkREp0N1h0Rm9vUzUzZkJHbVc1UUlVaXpOTnVqTG1zQXkyRDZlNWZhNzhFOHZjWVRKWU4K",
      "endpoints": ["https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json"]
    }
  }
  ```

  `bundle.createUpdaterArtifacts` stays **absent** in the file (so a build without the private key
  passes); `scripts/build-app.sh` turns it on with `--config` only when
  `TAURI_SIGNING_PRIVATE_KEY` is set and non-empty.
- The public key is pinned in the binary (the config is compiled in by `generate_context!`). The
  private key and its password exist only as the Actions secrets `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`. Local tests make a throwaway key pair (in the test's temp
  folder, or the scratch folder for manual runs). Nobody reads `~/.stickybot-signing`.

### 2. Release workflow, build script, release tool

- `desktop/scripts/build-app.sh`: when `TAURI_SIGNING_PRIVATE_KEY` is non-empty add
  `--config '{"bundle":{"createUpdaterArtifacts":true}}'`, and after the build check that
  `target/release/bundle/macos/StickyBot.app.tar.gz` and `.tar.gz.sig` exist; otherwise say
  "no update key: updater artifacts skipped" and build as today. Must stay shellcheck-clean.
- `.github/workflows/release.yml`, job `mac-app`:
  - the build step gets `TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}` and
    `TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.… }}` in `env` (empty when the secret is not
    set, e.g. a dispatch build from a fork: unsigned test build, artifacts skipped, job green);
  - on a **tag** push the job FAILS when the secrets are missing only if the owner opted in? No:
    decide simply — on a tag without the secrets the release is still published without updater
    files and the job prints a clear `::warning::` ("no update signing key: this release cannot be
    installed from inside the app"); see Assumption 9;
  - a new step checks the tarball: untar into a temp folder, `codesign --verify --deep --strict`
    on the `.app` inside it and `Signature=adhoc` + the `runtime` flag (hardened runtime) in
    `codesign -dv`, so the ad-hoc signature is still valid inside the tarball;
  - the tarball and its `.sig` are uploaded as part of the `mac-app` artifact under release names:
    `StickyBot-<version>-macos-arm64.app.tar.gz` and `…app.tar.gz.sig` (a job output `updater`
    says whether they exist).
- job `publish`: when the updater files exist, run
  `python3 tools/release_tool.py latest-json …` to write `dist/latest.json` and add the tarball,
  the `.sig` and `latest.json` to `gh release create`. The zip (first installs) and
  `manifest.json` `"apps"` stay exactly as they are.
- `tools/release_tool.py`: new sub-command

  ```
  release_tool.py latest-json <out> --version X.Y.Z --tag vX.Y.Z --sig <file.sig> \
      --asset StickyBot-X.Y.Z-macos-arm64.app.tar.gz [--repo 4shishkumar/stickybot] [--pub-date ISO]
  ```

  writing (Tauri's static-JSON updater format):

  ```json
  {
    "version": "2.6.0",
    "notes": "<the version's CHANGELOG section, as `notes` prints it>",
    "pub_date": "2026-11-20T10:00:00Z",
    "platforms": {
      "darwin-aarch64": {
        "signature": "<contents of the .sig file>",
        "url": "https://github.com/4shishkumar/stickybot/releases/download/v2.6.0/StickyBot-2.6.0-macos-arm64.app.tar.gz"
      }
    }
  }
  ```

  `pub_date` is RFC 3339 UTC (default: now). It refuses an empty signature, a version that is not
  SemVer, and a URL that would not be `https://github.com/<repo>/releases/download/<tag>/<asset>`.
  Standard library only; tests in `mac-agent/tests/test_release_tool.py`.
- **Endpoints and channels**:
  - stable: `https://github.com/4shishkumar/stickybot/releases/latest/download/latest.json`
    (GitHub's "latest" never points at a prerelease);
  - beta (`update_channel: "beta"`): the app already lists the releases through the API
    (`releases::latest`), newest SemVer first, prereleases included. The endpoint is then
    **built**, not taken from the API answer:
    `https://github.com/<repo>/releases/download/<tag>/latest.json` for the tag the check picked,
    and handed to the updater at run time. If the newest release on beta is a stable one, that is
    simply its `latest.json`.
  - a release counts as installable in-app only when the check saw a `latest.json` asset on it
    (`updater_asset`); older releases and releases cut without the key keep the download page.
- Releases ≤ 2.5.0 have no updater inside: the first release with it is installed by hand once
  (documented in README, RELEASING.md, the changelog line).

### 3. Server: `selfupdate.rs` (pure logic, new module) + small hooks

New module `desktop/crates/stickybot-server/src/selfupdate.rs` (so `updates.rs`, `api.rs`,
`lib.rs`, `config.rs` get only a few lines each):

- **Trait** the Tauri app implements, with a fake for tests (like `appctl::AppControl`):

  ```rust
  /// What only the menu bar app can do for an app update (the Tauri updater plugin).
  pub trait Installer: Send + Sync {
      /// Why this app can't replace itself here, in words (None = it can): the headless binary,
      /// a translocated or read-only path, not inside a .app bundle.
      fn unavailable(&self) -> Option<String>;
      /// Downloads the update at `endpoint` and checks its signature; `progress(bytes, total)`.
      /// Nothing on disk is replaced yet.
      fn download(&self, endpoint: &str, progress: Progress) -> BoxFuture<'_, Result<Box<dyn Staged>, InstallError>>;
  }
  pub trait Staged: Send {
      fn version(&self) -> String;
      /// Replaces the bundle (blocking). The relaunch is `AppControl::restart`.
      fn install(self: Box<Self>) -> Result<(), InstallError>;
  }
  pub enum InstallError { Signature(String), Network(String), NotWritable(String), NoUpdate(String), Other(String) }
  ```

  (exact shapes are the implementer's; the split "download+verify" / "replace" / "relaunch" and
  the error kinds are the contract). Each error kind maps to plain words:
  signature → "The download didn't match StickyBot's signing key, so it was not installed.";
  network → "Couldn't download the update (no connection?)."; not writable → "StickyBot can't
  replace itself at <path> (move it to Applications, or download it by hand)."; each with the
  download page as the fallback.
- **State machine** (`Phase`): `idle | checking | available | downloading {bytes,total} |
  installing | restarting | error {kind, message}`; `checking` and `available` are derived from
  the existing check state; `downloading → installing → restarting` and `→ error` are the
  install's. A `waiting` reason ("busy — will install when idle: a firmware update is running")
  rides on the state while a verified download waits for the stick to be free. Only one install
  at a time (a second `POST` answers 409 with the current state).
- **Busy gate** (pure function over a small `Busy` snapshot, unit-tested):
  busy while any of — a firmware update over Bluetooth or a USB flash is running
  (`st.jobs.busy()` and the flasher's equivalent), a voice request is in flight (agent state
  `listening | transcribing | thinking | working`, or `brain.busy()`), a permission prompt or a
  stick question is waiting (`approvals.pending()` / `asking()`), the wireless mic is held
  (`wmic.streaming()`), a face install is running. The answer is the reason in words.
  Being **paused** is not busy (see Assumption 6).
- **Install flow** (`install(st, by)`), in order:
  1. refuse with words when: no app update is available; the target is not newer than this app
     (`releases::newer`; a lower or equal version is never installed — nothing in the UI lets the
     user pick one); the release has no `latest.json`; `Installer::unavailable()` says why;
  2. `downloading`: `Installer::download(endpoint, progress)`; progress events at most ~5 a
     second; the plugin verifies the minisign signature when the download completes; the staged
     version must equal the checked version and be newer, else error;
  3. gate: while busy, `waiting` with the reason, re-checked every 5 s, for at most 30 min (then
     error "still busy — try again"); `POST /api/update/cancel` drops a waiting install;
  4. `installing`: tell the stick with the existing notify path (`events::notify`, kind
     `update`, title "Updating StickyBot…", body "back in a moment") — no new message, no
     PROTOCOL.md change; write `pending: {from, to, at}` into `updates.json`; `Staged::install()`
     on a blocking thread;
  5. `restarting`: `bye` to the stick (`link.bye()`, as quit does), then
     `AppControl::restart()`. If install fails: clear `pending`, state `error`, nothing was
     replaced (the plugin swaps the bundle only after extraction succeeded).
- **After the restart**: on start, when `updates.json` has `pending.to` equal to this app's
  version, emit the hub event `updated` `{from, to}` and remember it for the status (`updated:
  {from, to, at}` in `/api/update` until dismissed / 24 h), and once the stick is connected send
  one `events::notify` (kind `update`, "Updated to vX"); then clear `pending` so it happens once.
  If the version is not the pending one (the install did not take), report
  "The update to vX didn't take — still on vY" once as an error and clear it.
- **`update_auto`** (config default `false`, accepted by `PUT /api/config` like `update_check`):
  when on, the check loop installs an available app update by itself once the app has been idle:
  nothing busy (the gate above) **and** no button on the stick for `AUTO_IDLE = 10 min`
  (`st.last_input()`) and no install tried for this version in the last 24 h (a failed automatic
  install is not retried in a loop). Quiet hours do not stop it (see Assumption 5). Needs
  `update_check` on (no check, no update). Never with the fake stick unless a fake installer is
  set (tests).
- **Channel → endpoint** (`endpoint(repo, channel, tag) -> String`, pure): as in section 2. In
  **release** builds the repo is the compiled-in `releases::DEFAULT_REPO` and nothing in the
  config or environment changes the endpoint (when `STICKYBOT_RS_REPO` points somewhere else,
  in-app install is unavailable: "this build checks another repo — download it by hand"). In
  **debug** builds and tests `STICKYBOT_RS_UPDATE_ENDPOINT` may override it, and only with a
  `http://127.0.0.1:<port>/…` or `http://localhost:<port>/…` URL; anything else is ignored with
  a log line.
- **Existing files, minimal edits**:
  - `updates.rs`: `fetch` also records `updater_asset` (the release has `latest.json`);
    `status()` / `full()` gain `state`, `progress {bytes,total}`, `waiting`, `install_error
    {kind,message}`, `updated`, `auto`, and `install.can_update` / `install.why` come from the
    installer (true when installable in-app); `title` unchanged ("Update available: vX").
    `NO_SELF_UPDATE` goes away where the installer is present; the headless binary keeps a "only
    the menu bar app updates itself" reason. All new fields are additions: the legacy Python app's
    `/api/update` has none of them and the shared JS must cope (feature-detect).
  - `api.rs`: routes `POST /api/update/install` (202 + the status; 409 with words when refused or
    already running), `POST /api/update/cancel`; `POST /api/update/agent` stays for the shared
    JS's old path and now does the same as `/install` in the Rust app (it used to answer 409).
    Handlers live in `selfupdate.rs`.
  - `lib.rs`: one field on `AppState` (`installer: Mutex<Arc<dyn selfupdate::Installer>>`, default
    `NoInstaller`), `config.rs`: `"update_auto": false`.
  - progress reaches the dashboard through the existing hub (`st.hub.emit("update", …)` with
    `state`, `progress`, `waiting`, `install_error`), which the dashboard already streams; no new
    SSE endpoint.

### 4. Tauri app: the plugin behind the trait, and the menu

- `stickybot-app/src/updater.rs` (new): `TauriInstaller(AppHandle)` implementing
  `selfupdate::Installer` with `tauri_plugin_updater::UpdaterExt`:
  `app.updater_builder().endpoints(vec![endpoint])…build()?.check().await` → `update.download(on_chunk,
  on_done)` (signature verified by the plugin with the pinned key) → `Staged` holding the bytes →
  `update.install(bytes)`. The plugin's default version comparator (remote > current) stays as a
  second downgrade guard. `unavailable()`: the running executable is not inside a `.app`; the
  path is under `/AppTranslocation/` (Gatekeeper's randomised read-only copy: "StickyBot is running
  from a temporary read-only copy — move it to Applications and open it again"); the bundle's
  parent folder is not writable for this user (no admin prompt: fall back to the download page).
  Errors from the plugin are mapped to `InstallError` kinds (signature / network / not writable).
- `main.rs`: `.plugin(tauri_plugin_updater::Builder::new().build())` and set
  `state.installer` next to `login_item` (two lines).
- `tray.rs`: `update_text` — an app update that can be installed in-app reads
  **"Install update vX and restart…"**; while it runs the item shows the state ("Downloading
  update… 42 %", "Installing update…", "Update waits: <reason>"); when it cannot be installed
  in-app the old "Update available: vX — download…" (release page) stays. A click starts the
  install (`selfupdate::install`), or opens the release page in the fallback case. "Check for
  Updates…" and the check cadence (20 s after start, every 24 h) are unchanged.

### 5. Dashboard (shared JS; both apps)

`mac-agent/stick_agent/static/updates.js` (served by both apps; Rust-only bits may go in
`desktop/crates/stickybot-server/assets/rust.js` if that is where Rust-only UI lives):

- feature-detect: `u.state !== undefined` (the Rust app with this change) switches the banner and
  the Settings → Updates card to the new flow; without it (the Python app, an older Rust app) the
  existing code path runs untouched (`/api/update/agent` + jobs for `install.sh` installs).
- banner / card when an app update is available and `install.can_update`: title, versions line,
  release notes (existing `notesBlock`), a primary **Install and restart** button, "Later";
- while it runs: a progress bar with "Downloading… 12.3 MB of 38.0 MB" (`progress.bytes/total`;
  indeterminate when `total` is unknown), then "Checking the signature and installing…", then
  "Restarting StickyBot…" and the existing `reloadWhenBack` poll;
- `waiting`: "Busy — will install when idle: <reason>" with **Cancel**;
- errors: the words from `install_error.message`, the kind as a short heading ("The update's
  signature didn't match", "No connection", "Can't write to <folder>"), a **Try again** button and
  an **Open the download page** link (`install.download`), which is also what shows when
  `can_update` is false (with `install.why`);
- after the restart: a one-time "Updated to vX" notice (from `updated`), with the notes link;
- Settings → Updates gets a toggle **Install updates automatically** (`update_auto`), shown only
  when the app reports `auto !== undefined`; saved with the other settings.
- pure helpers (state → what to show, byte formatting) live in a plain module with a
  `node --test` suite under `mac-agent/tests/js/`.

### 6. Tests

- Rust unit tests (in `selfupdate.rs` or `selfupdate_tests.rs`): the state machine (every
  transition incl. error and cancel), the busy gate (each reason, paused is not busy), auto
  install (idle minutes, once per version), channel → endpoint (stable, beta, release-vs-debug
  override rules, localhost only), downgrade / same-version refusal, staged-version mismatch, the
  pending → "Updated to vX" once logic, the API (`/api/update` shape, `/install` 202 / 409,
  `/cancel`) with a `FakeInstaller`.
- Tray: `update_text` / click target for installable, running, fallback.
- Integration test in `stickybot-app/tests/` (or a `#[cfg(test)]` module if the plugin needs the
  crate's internals): a throwaway minisign key pair generated in the test (dev-dependency, built
  at run time — no key material in the repo), a local `127.0.0.1` HTTP server serving `latest.json`
  + a signed `StickyBot.app.tar.gz` of a tiny fake bundle, the real plugin (`tauri::test` mock app
  or the plugin's builder) pointed at it with the test public key. It goes as far as the plugin
  allows: preferably installing into a **temporary copy** of a bundle through
  `UpdaterBuilder::executable_path`, and checking the temp bundle's content changed; otherwise up
  to "verified". Also: a tampered tarball and a wrong key are refused with the signature error
  and nothing is replaced. It never touches `/Applications`.
- pytest: `latest-json` (shape, notes from the changelog, URL, refusals).
- JS: the new helpers; the legacy shape (no `state`) still renders the old card.
- Run: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `python -m ruff check .` + `python -m pytest` in `mac-agent/` (venv:
  `<scratch>/py-venv`;
  one hooks test fails locally only because port 8660 is in use — ignore it), shellcheck on the
  scripts, actionlint on the workflows when a binary is at hand (download with a checksum into the
  scratch folder).
- A manual local check (reported in the PR, not a test): build the bundle with a throwaway key,
  confirm the tarball + `.sig` appear, untar and `codesign --verify --deep --strict` the `.app`,
  confirm hardened runtime + ad hoc; and without the key the build passes and skips them.

### 7. Docs and screenshots

- `CHANGELOG.md` `[Unreleased]` → Added, in user words (install from the menu / dashboard,
  automatic installs setting, the one manual install, macOS asking for permissions again).
- `README.md` "Updating"; `docs/RELEASING.md`: the updater files, `latest.json`, the channels, and
  a new section **Update signing key** (what it is, how the owner adds the two secrets — Settings
  → Secrets and variables → Actions —, that the private key is nowhere else, rotation: new pair,
  new public key in `tauri.conf.json`, a release signed with the old key that carries the new
  public key while the old key is still at hand; if the old key is lost, a release people install
  by hand once); `desktop/README.md` where it lists what isn't ported (`selfupdate.py`);
  `THIRD_PARTY_NOTICES.md` for `tauri-plugin-updater` (MIT / Apache-2.0) and what it brings
  (minisign-verify, reqwest …) per the file's convention for Rust crates.
- Screenshots (headless Chromium from `~/Library/Caches/ms-playwright`, a temp profile, the
  dashboard served by `stickybot-headless --fake-stick` on a free port ≠ 8660/8670 with a
  temporary home and a fake installer / a small fake server) into
  `<scratch>/update-ui/`:
  banner (available), progress, waiting, an error, "Updated to vX", the Settings card. Every
  browser and server started is killed afterwards. Screenshots are not committed.

### 8. Security points (go into the PR body)

Public key pinned in the binary; https only (the plugin refuses other schemes; the only exception
is a localhost endpoint in debug / test builds); the signature is verified over the whole download
before anything is replaced; never a lower or equal version; the endpoint cannot be changed by
config in release builds; updates are allowed while paused; the private key exists only in Actions
secrets; a lost key means a new key shipped through one manual install. The updater does not
check Apple code signatures (the app is ad hoc signed): the minisign signature is the trust
anchor. After an update macOS may ask for Bluetooth / System Audio Recording again (ad-hoc
identity changes per build) — say so in the changelog and the PR.

## Affected Memory

- `apps/mac-apps`: (modify) the Rust app updates itself: the updater plugin behind
  `selfupdate::Installer`, the install flow and its states, the busy gate, `update_auto`,
  `/api/update` additions, menu and dashboard wording, the fallback to the download page.
- `delivery/releases`: (modify) signed updater artifacts (`.app.tar.gz` + `.sig`), `latest.json`,
  the stable / beta endpoints, the two Actions secrets, key rotation, builds without the key.

## Impact

- `desktop/crates/stickybot-app/`: `Cargo.toml`, `tauri.conf.json` (new `plugins` key),
  `src/main.rs` (2 lines), `src/tray.rs`, new `src/updater.rs`, new integration test;
  `desktop/Cargo.lock` (by cargo).
- `desktop/crates/stickybot-server/`: new `src/selfupdate.rs` (+ tests), small edits in
  `updates.rs`, `api.rs`, `lib.rs`, `config.rs`; maybe `assets/rust.js`.
- `desktop/scripts/build-app.sh`, `.github/workflows/release.yml`, `tools/release_tool.py`,
  `mac-agent/tests/test_release_tool.py`.
- `mac-agent/stick_agent/static/updates.js` (+ a helper module and its `node --test` suite): a
  shared file, so both apps' behaviour is reviewed.
- Docs: `CHANGELOG.md`, `README.md`, `docs/RELEASING.md`, `desktop/README.md`,
  `THIRD_PARTY_NOTICES.md`; `docs/memory/` at hydrate.
- No change to `PROTOCOL.md`, the firmware, or `FW_VERSION`: the stick only gets ordinary notify
  messages it already knows. Labels for the PR: `mac-app`, `feature`, `needs-hardware` (the
  toasts on the stick and the real replace-and-relaunch in `/Applications` need a person).
- Cannot be checked by the agent: replacing a real `/Applications/StickyBot.app` and relaunching,
  the toasts on a real stick, the release workflow with the real secrets.

## Open Questions

- None blocking. Whether a tag pushed without the signing secrets should fail the release instead
  of warning is the owner's call (Assumption 9).

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Use `tauri-plugin-updater` 2.x inside `stickybot-app`, behind a `selfupdate::Installer` trait defined in `stickybot-server` | The brief names the plugin; the server crate has no Tauri and already uses this trait pattern (`appctl::AppControl`, `login::LoginItem`) | S:90 R:70 A:90 D:90 |
| 2 | Certain | Public key in `tauri.conf.json` `plugins.updater.pubkey`; private key + password only as the Actions secrets `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`; tests use a throwaway pair | Stated in the brief; Constitution IV | S:100 R:80 A:95 D:95 |
| 3 | Confident | Stable endpoint is `releases/latest/download/latest.json`; beta builds `releases/download/<tag>/latest.json` from the tag the existing release check picked and passes it at run time | The brief proposes exactly this mapping; GitHub's "latest" skips prereleases; no extra hosting | S:80 R:75 A:80 D:75 |
| 4 | Confident | `createUpdaterArtifacts` is switched on by `build-app.sh` only when the key is in the environment, not in `tauri.conf.json` | The Tauri CLI fails a build that asks for updater artifacts without a private key; the brief requires key-less builds to pass | S:75 R:85 A:80 D:80 |
| 5 | Confident | `update_auto` installs when nothing is busy and no stick button was pressed for 10 minutes, also during quiet hours; a failed automatic install is not retried for that version within 24 h | Brief: "decide and document". Quiet hours mean "don't disturb me", and that is when nobody is mid-request; the stick toast still follows the event rules | S:60 R:85 A:65 D:60 |
| 6 | Confident | Being paused does not block an update (manual or automatic) | Brief: "decide". Pause releases the stick; replacing the app then disturbs nothing, and the app comes back paused-or-not as it starts today | S:55 R:85 A:65 D:65 |
| 7 | Confident | The busy gate sits after download + verify and before replace + relaunch; a busy install waits (re-check every 5 s, at most 30 min, cancellable) | Downloading in the background disturbs nothing; the brief asks for "busy — will install when idle" with a retry | S:70 R:80 A:70 D:65 |
| 8 | Confident | No admin prompt: when the bundle's folder is not writable or the app is translocated, in-app install is unavailable and the download page is offered | Brief: "say so and fall back to the download page"; a password prompt from a menu bar app is a surprise | S:75 R:80 A:70 D:70 |
| 9 | Tentative | A tag pushed while the signing secrets are missing still publishes (zip + firmware) with a workflow warning, rather than failing | The brief only requires PR / dispatch builds to pass; failing would block firmware releases on a missing key, but a silent non-updatable release is a risk too — the owner may prefer failing | S:45 R:80 A:50 D:45 |
| 10 | Confident | Updater assets are named `StickyBot-<version>-macos-arm64.app.tar.gz` (+ `.sig`) on the release; `latest.json` points at them | Matches the zip's naming; `StickyBot.app.tar.gz` is only the build output's name | S:60 R:85 A:75 D:70 |
| 11 | Confident | In release builds the endpoint ignores config and environment (a different `STICKYBOT_RS_REPO` turns in-app install off); debug / test builds accept only a localhost `STICKYBOT_RS_UPDATE_ENDPOINT` | Security point in the brief | S:80 R:75 A:75 D:70 |
| 12 | Confident | No new message to the stick: "Updating StickyBot…" and "Updated to vX" use the existing notify path with kind `update`; `PROTOCOL.md` is unchanged | Brief: "via the existing event/notify path"; Constitution II needs nothing new | S:85 R:85 A:85 D:85 |
| 13 | Confident | `POST /api/update/agent` in the Rust app becomes an alias of `/api/update/install`; the Python app's endpoints and the shared JS's old path are untouched, the new UI is feature-detected on `state` | The brief asks the legacy page to keep working with the shared JS | S:70 R:85 A:75 D:70 |
| 14 | Confident | A lower or equal version is never installed; there is no UI to pick an older one in this change | Brief: "never install a lower version unless the user picks it" — no picker exists, so never | S:75 R:80 A:80 D:75 |
| 15 | Tentative | The integration test installs into a temporary copy of a fake bundle via the plugin's `executable_path`, if the plugin can run under `tauri::test`; otherwise it stops at "downloaded and verified" | Depends on what the plugin allows without a real app; the brief accepts either | S:65 R:90 A:45 D:50 |

15 assumptions (2 certain, 11 confident, 2 tentative, 0 unresolved).
