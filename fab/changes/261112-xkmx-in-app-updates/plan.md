# Plan: In-app updates for StickyBot.app

**Change**: 261112-xkmx-in-app-updates
**Intake**: `intake.md`

## Requirements

### Delivery: signed updater artifacts

#### R1: Update key and plugin configuration
`desktop/crates/stickybot-app/tauri.conf.json` MUST carry `plugins.updater.pubkey` (the public key
given in the intake) and the stable endpoint. No private key, password or key-shaped test value
MAY appear in the repo. `bundle.createUpdaterArtifacts` MUST NOT be set in the file.

- **GIVEN** a checkout without `TAURI_SIGNING_PRIVATE_KEY`
- **WHEN** `desktop/scripts/build-app.sh` runs
- **THEN** StickyBot.app is built and signed ad hoc as before, and the script says the updater artifacts were skipped
- **AND** `git grep` finds no private key material

#### R2: Build script makes the updater artifacts when the key is present
`build-app.sh` MUST add `createUpdaterArtifacts` through `--config` only when
`TAURI_SIGNING_PRIVATE_KEY` is non-empty, and then MUST fail if `StickyBot.app.tar.gz` or its
`.sig` is missing.

- **GIVEN** a throwaway key pair in the environment
- **WHEN** the script runs
- **THEN** `target/release/bundle/macos/StickyBot.app.tar.gz` and `.tar.gz.sig` exist
- **AND** the `.app` inside the tarball passes `codesign --verify --deep --strict`, is ad hoc signed and has the hardened runtime flag

#### R3: Release workflow publishes the updater files and `latest.json`
`release.yml` `mac-app` MUST pass the two secrets to the build, verify the tarball's code
signature, and upload the tarball and `.sig` under release names; `publish` MUST add them and a
generated `latest.json` to the release when they exist. The zip and `manifest.json` `"apps"` MUST
stay as they are. A run without the secrets (dispatch, fork) MUST pass with the artifacts skipped;
a tag without them publishes with a `::warning::`.

- **GIVEN** a tag push with both secrets set
- **WHEN** the workflow finishes
- **THEN** the release has the zip, the firmware parts, `manifest.json`, `StickyBot-<ver>-macos-arm64.app.tar.gz`, its `.sig` and `latest.json`
- **GIVEN** a `workflow_dispatch` run without secrets
- **THEN** `mac-app` is green and no updater file is produced

#### R4: `release_tool.py latest-json`
The tool MUST write Tauri's static update JSON (`version`, `notes` from the changelog section,
`pub_date` RFC 3339 UTC, `platforms.darwin-aarch64.{signature,url}`) and MUST refuse an empty
signature, a non-SemVer version, and an asset name or repo that would not give
`https://github.com/<repo>/releases/download/<tag>/<asset>`.

- **GIVEN** version 2.6.0, tag v2.6.0, a `.sig` file and the asset name
- **WHEN** `latest-json` runs
- **THEN** the file parses as JSON with exactly those fields and the notes of `## [2.6.0]`
- **AND** an empty `.sig` exits non-zero with words

### Apps: the update logic in `stickybot-server`

#### R5: The installer trait and its error words
`selfupdate.rs` MUST define the `Installer` / `Staged` traits (download + verify, replace) with a
`NoInstaller` default and a fake for tests, and `InstallError` kinds `signature`, `network`,
`not-writable`, `no-update`, `other`, each shown to the person as a sentence plus the download
page as the fallback. No Tauri type MAY appear in `stickybot-server`.

- **GIVEN** the headless binary (no installer set)
- **WHEN** `/api/update` is read while a newer app release exists
- **THEN** `install.can_update` is false, `install.why` says only the menu bar app updates itself, and `install.download` is the release page

#### R6: State machine
`/api/update` and the status's `update` MUST report `state`: `idle | checking | available |
downloading | installing | restarting | error`, with `progress {bytes,total}` while downloading,
`waiting` (the reason) while a verified download waits, and `install_error {kind,message}` on
error. Only one install MAY run at a time. State changes MUST be emitted on the hub as `update`
events (progress at most about 5 a second).

- **GIVEN** an available update and a fake installer
- **WHEN** an install runs to the end
- **THEN** the states seen are `downloading → installing → restarting`, the fake's `install` ran once and `AppControl::restart` was called once
- **GIVEN** an install already running
- **WHEN** `POST /api/update/install` is sent again
- **THEN** it answers 409 with the current state

#### R7: Refusals: nothing to install, downgrade, version mismatch, no `latest.json`
An install MUST be refused in words when there is no app update, when the release's version is
not newer than this app's (never a lower or equal version), when the release has no `latest.json`
asset, when the installer is unavailable, and when the downloaded update's version is not the
checked one.

- **GIVEN** this app is 2.6.0 and the latest release is 2.5.0 or 2.6.0
- **WHEN** an install is asked for
- **THEN** it is refused, nothing is downloaded
- **GIVEN** the staged update says 2.5.9 while the check said 2.7.0
- **THEN** state `error`, nothing is replaced

#### R8: Busy gate
A verified download MUST NOT be installed while a firmware update (Bluetooth or USB), a voice
request, a permission prompt or stick question, a wireless-mic hold or a face install is in
progress; the state carries `waiting: "busy — will install when idle: <reason>"`, re-checked
every 5 s for at most 30 min, and `POST /api/update/cancel` drops it. Paused is not busy.

- **GIVEN** a firmware update job is running
- **WHEN** an install has downloaded and verified
- **THEN** `waiting` names the firmware update and nothing is replaced
- **AND** when the job ends the install continues by itself
- **GIVEN** the link is paused and nothing else runs
- **THEN** the gate is open

#### R9: Around the restart
Before replacing, the app MUST notify the stick through `events::notify` (kind `update`,
"Updating StickyBot…") and record `pending {from,to,at}` in `updates.json`; before relaunching it
MUST send `bye`. After the restart, when this app's version is the pending one, it MUST emit
`updated {from,to}` on the hub, show it in `/api/update` (`updated`), and notify the stick
"Updated to vX" once; a pending version that did not take is reported once as an error. No new
message type is sent to the stick.

- **GIVEN** `updates.json` with `pending.to` = this version
- **WHEN** the app starts and the stick connects
- **THEN** one `update` notify "Updated to vX" is sent and `pending` is cleared
- **AND** a second start sends nothing

#### R10: `update_auto`
Config `update_auto` (default `false`, settable through the config API) MUST make the check loop
install an available app update when the gate is open, no stick button was pressed for 10 minutes,
`update_check` is on, and no install of that version was tried in the last 24 h. Quiet hours do
not block it.

- **GIVEN** `update_auto: true`, an available update, last input 11 minutes ago
- **WHEN** the loop ticks
- **THEN** an install starts
- **GIVEN** last input 2 minutes ago, or `update_auto: false`
- **THEN** nothing starts

#### R11: Channel → endpoint, and no override in release builds
`endpoint(repo, channel, tag)` MUST give
`https://github.com/<repo>/releases/latest/download/latest.json` for stable and
`https://github.com/<repo>/releases/download/<tag>/latest.json` for beta. In release builds the
repo MUST be the compiled-in default and no config or environment value MAY change the endpoint;
in debug / test builds `STICKYBOT_RS_UPDATE_ENDPOINT` MAY override it only with an
`http://127.0.0.1` / `http://localhost` URL.

- **GIVEN** channel beta and the picked tag `v2.7.0-beta.1`
- **THEN** the endpoint is `…/releases/download/v2.7.0-beta.1/latest.json`
- **GIVEN** an override `https://evil.example/latest.json`
- **THEN** it is ignored

#### R12: API
`GET /api/update` MUST add the fields of R6 / R9 plus `auto` and `updater_asset` on `latest`
without removing or renaming an existing field. `POST /api/update/install` MUST start an install
(202 + status) or refuse (409 + words); `POST /api/update/cancel` MUST drop a waiting install;
`POST /api/update/agent` MUST behave as `/install`. All need the API token like the other POSTs.

- **GIVEN** a valid token and an installable update
- **WHEN** `POST /api/update/install`
- **THEN** 202 and `state` is `downloading` (or later)

### Apps: the Tauri shell

#### R13: The plugin behind the trait
`stickybot-app` MUST implement `Installer` with `tauri-plugin-updater` (signature checked with
the pinned key before anything is replaced; https only except the debug localhost override),
map the plugin's errors to the kinds of R5, and report `unavailable()` in words when the app is
not in a `.app`, runs translocated, or its folder is not writable (no admin prompt).

- **GIVEN** an executable path under `/private/var/folders/…/AppTranslocation/…`
- **THEN** `unavailable()` says to move StickyBot to Applications
- **GIVEN** a tampered tarball
- **WHEN** the plugin downloads it
- **THEN** the error kind is `signature` and the bundle is unchanged

#### R14: Menu
The update item MUST read "Install update vX and restart…" when the app update can be installed
in-app (a click starts it), show the running state while it runs, and keep "Update available: vX —
download…" (release page) otherwise. "Check for Updates…" and the cadence are unchanged.

- **GIVEN** `agent_update` and `install.can_update`
- **THEN** the item reads `Install update v2.6.0 and restart…`

### Dashboard (shared JS)

#### R15: Banner and Settings card
`updates.js` MUST, when `/api/update` has `state`, show the banner / card with notes, **Install
and restart**, a progress bar (downloaded of total), `waiting` with Cancel, errors with Try again
and the download page, the fallback when `can_update` is false, the one-time "Updated to vX"
notice and the `update_auto` toggle. Without `state` (the Python app, older Rust apps) it MUST
behave exactly as before.

- **GIVEN** `/api/update` without `state`
- **THEN** the old card and `/api/update/agent` path render as today
- **GIVEN** `state: "downloading", progress: {bytes: 12900000, total: 39800000}`
- **THEN** the bar is at 32 % and the text reads "Downloading… 12.3 MB of 38.0 MB"

### Docs

#### R16: Docs, changelog, notices
`CHANGELOG.md` `[Unreleased]` (user words), `README.md` "Updating", `docs/RELEASING.md` (updater
files, `latest.json`, channels, a section "Update signing key" with adding the two secrets and
rotation / loss), `desktop/README.md`, and `THIRD_PARTY_NOTICES.md` MUST be updated; the one
manual install for ≤ 2.5.0 and macOS asking for permissions again MUST be said.

- **GIVEN** the owner reads RELEASING.md "Update signing key"
- **THEN** they can add both secrets and know what to do if the key is lost

### Non-Goals

- Apple Developer ID signing / notarisation — the minisign key is the trust anchor.
- Updating the legacy Python app differently — its `selfupdate.py` path is untouched.
- A version picker (installing an older release on purpose).
- Any change to `PROTOCOL.md`, the firmware or `FW_VERSION`.
- An admin-password prompt to write into a folder the user cannot write.

### Design Decisions

#### Tauri's updater plugin behind a server-side trait
**Decision**: `tauri-plugin-updater` does download, signature check and bundle swap inside `stickybot-app`; `stickybot-server/src/selfupdate.rs` owns the state machine, gate, endpoint and API behind `Installer`.
**Why**: the server crate has no Tauri (headless binary, tests with fakes); the plugin already does the dangerous parts and is what the Tauri CLI's artifacts are made for.
**Rejected**: an own updater in the server (own tar extraction, swap and signature code to maintain); calling the plugin from the dashboard's JS (the page is served over HTTP to browsers too, and has no Tauri IPC).
*Introduced by*: 261112-xkmx-in-app-updates

#### Beta channel through the release list, not a second feed
**Decision**: stable uses GitHub's `releases/latest/download/latest.json`; beta builds the per-tag `latest.json` URL from the release the existing check picked and passes it to the updater at run time.
**Why**: GitHub's "latest" never is a prerelease; the app already lists releases; the URL is built from the compiled-in repo and a validated tag, never copied from an API answer.
**Rejected**: a `latest-beta.json` on a moving tag or on Pages (another thing to publish and keep in step).
*Introduced by*: 261112-xkmx-in-app-updates

#### Updater artifacts only when the key is in the environment
**Decision**: `createUpdaterArtifacts` is passed by `build-app.sh` when `TAURI_SIGNING_PRIVATE_KEY` is set, never set in `tauri.conf.json`.
**Why**: the Tauri CLI fails a build that asks for them without a key; PR, dispatch and local builds have none.
**Rejected**: a second config file for releases (two places to keep in step).
*Introduced by*: 261112-xkmx-in-app-updates

#### The gate sits between "verified" and "replace"
**Decision**: download and verify at once; wait for the stick to be free only before replacing and relaunching.
**Why**: a download disturbs nothing; the restart is what would cut a talk, a prompt or a firmware update.
**Rejected**: refusing the click while busy (the person would have to come back and try again).
*Introduced by*: 261112-xkmx-in-app-updates

## Tasks

### Phase 1: Setup

- [x] T001 Add `tauri-plugin-updater = "2"` to `desktop/crates/stickybot-app/Cargo.toml` (let cargo update `desktop/Cargo.lock`), add the `plugins.updater` block (pubkey + stable endpoint) to `desktop/crates/stickybot-app/tauri.conf.json` touching no other key, register the plugin in `src/main.rs`; add the plugin and what it brings to `THIRD_PARTY_NOTICES.md` following that file's convention <!-- R1 -->
- [x] T002 [P] `"update_auto": false` in `desktop/crates/stickybot-server/src/config.rs` defaults and wherever the config API lists allowed keys (follow `update_check`) <!-- R10 -->

### Phase 2: Core Implementation

- [x] T003 New `desktop/crates/stickybot-server/src/selfupdate.rs`: `Installer` / `Staged` traits, `NoInstaller`, `FakeInstaller` (tests), `InstallError` kinds and their sentences, `Phase` state machine, `endpoint()` with the release / debug override rules; unit tests for transitions, error words, endpoint <!-- R5 R6 R11 -->
- [x] T004 Busy gate in `selfupdate.rs`: a `Busy` snapshot read off `AppState` (firmware job, USB flash, talk / brain, approvals / asks, wireless mic, face install) and a pure `busy_reason()`; unit tests for each reason and paused-is-not-busy <!-- R8 -->
- [x] T005 Install flow in `selfupdate.rs` (`install`, `cancel`): refusals, download with throttled progress events, version check of the staged update, wait-while-busy (5 s / 30 min, cancellable), notify the stick, `pending` in `updates.json`, `Staged::install` on a blocking thread, `bye`, `AppControl::restart`; tests with `FakeInstaller` + `FakeApp` incl. downgrade, mismatch, signature error leaves nothing replaced <!-- R6 R7 R8 R9 --> <!-- rework: cancel / switch to Installing under one lock; re-check the busy gate after the stick notify, before Staged::install -->
- [x] T006 `desktop/crates/stickybot-server/src/updates.rs`: record `updater_asset` in `fetch`, add `state` / `progress` / `waiting` / `install_error` / `updated` / `auto` and installer-driven `install.can_update` / `why` to `status()` / `full()` and to `emit`, the pending → "Updated to vX" once logic at start-up and on connect, the `update_auto` branch in the check loop (idle 10 min, once per version per 24 h); `lib.rs`: the `installer` field; keep every existing field and test passing, add tests <!-- R6 R9 R10 R12 --> <!-- rework: update_auto idle must count from max(last stick input, app start): no automatic install in the first 10 min after launch (R10); stale comment at updates.rs:138 -->
- [x] T007 `desktop/crates/stickybot-server/src/api.rs`: routes `POST /api/update/install`, `POST /api/update/cancel`, `/api/update/agent` as an alias (handlers in `selfupdate.rs`), the route list at the top; API tests (202, 409, cancel, token needed, shape keeps old fields) <!-- R12 -->
- [x] T008 New `desktop/crates/stickybot-app/src/updater.rs`: `TauriInstaller` over `tauri_plugin_updater::UpdaterExt` (run-time endpoint, download with progress, staged bytes, install), error mapping, `unavailable()` (not in a `.app`, `/AppTranslocation/`, folder not writable) as pure functions over a path with unit tests; set it on `state.installer` in `main.rs` <!-- R13 -->
- [x] T009 `desktop/crates/stickybot-app/src/tray.rs`: "Install update vX and restart…", the running-state texts, click → `selfupdate::install` or the release page; update the module doc's menu sketch; unit tests <!-- R14 -->
- [x] T010 [P] `tools/release_tool.py` `latest-json` + tests in `mac-agent/tests/test_release_tool.py` (shape, notes, URL, refusals) <!-- R4 -->
- [x] T011 [P] `desktop/scripts/build-app.sh`: updater artifacts when the key is set, check they exist, the skip message; shellcheck clean <!-- R2 -->
- [x] T012 `.github/workflows/release.yml`: secrets into the build env, tarball codesign check (ad hoc + hardened runtime), upload under release names, job output, `publish` builds `latest.json` and attaches the three files when present, warning on a tag without the key, header comment updated; actionlint if a binary can be had (download with checksum into the scratch folder) <!-- R3 --> <!-- rework: pass the signing key only on tag pushes (github.event_name == 'push'), so a workflow_dispatch on a branch never signs -->
- [x] T013 Dashboard: `mac-agent/stick_agent/static/updates.js` new flow behind `u.state !== undefined` (banner, card, progress, waiting + Cancel, errors + Try again + download page, "Updated to vX", `update_auto` toggle wired into the settings save), pure helpers in a small module with `mac-agent/tests/js/updates.test.mjs`; the old path untouched <!-- R15 --> <!-- rework: new banner flow: skip the rebuild when nothing shown changed -->

### Phase 3: Integration & Edge Cases

- [x] T014 Integration test in `desktop/crates/stickybot-app/tests/` (or a test module): throwaway minisign key made at run time, local 127.0.0.1 server with `latest.json` + signed tarball of a tiny fake bundle, the real plugin; install into a temp copy via `executable_path` if the plugin allows, else up to verified; tampered tarball and wrong key refused. Never `/Applications` <!-- R13 -->
- [x] T015 Local bundle check (manual, results noted in `## Notes` below for the PR): `build-app.sh` with a throwaway key from the scratch folder → tarball + `.sig`, untar, `codesign --verify --deep --strict`, hardened runtime + ad hoc; and without the key → passes, skipped. If the full bundle build is not possible here, say exactly what was and wasn't run <!-- R2 -->
- [x] T016 Run and fix: `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` in `desktop/`; `python -m ruff check .` and `python -m pytest` in `mac-agent/` with the scratch venv (the one port-8660 hooks failure is known); shellcheck; record the outputs' summaries in `## Notes` <!-- R6 -->

### Phase 4: Polish

- [x] T017 Docs: `CHANGELOG.md` `[Unreleased]` → Added; `README.md` "Updating"; `docs/RELEASING.md` (updater files, `latest.json`, channels, "Update signing key": adding the two secrets, rotation, a lost key, first updater release by hand); `desktop/README.md` (selfupdate now ported, the new endpoints) <!-- R16 --> <!-- rework: RELEASING.md Update signing key: there is no pull_request trigger; rotation order (the old key signs the release that carries the new public key, then swap the secrets); desktop/SWITCHOVER.md:157 self-update row is stale -->
- [x] T018 Screenshots into `<scratch>/update-ui/` (headless Chromium from `~/Library/Caches/ms-playwright`, temp profile, `stickybot-headless --fake-stick` on a free port with a temp home, or a small fake API server): banner, progress, waiting, error, "Updated to vX", Settings card; kill every browser and server started; list the files in `## Notes` <!-- R15 -->

## Execution Order

- T003 → T004 → T005 → T006 → T007 (one module growing; T006 needs the types)
- T008, T009 need T003–T006; T014 needs T008
- T010, T011 are independent; T012 needs both
- T013 needs the field names of T006; T018 needs T013

## Acceptance

### Functional Completeness

- [x] A-001 R1: `tauri.conf.json` has `plugins.updater` with the given public key and the stable endpoint; no other key of the file changed; no `createUpdaterArtifacts` in it
- [x] A-002 R2: `build-app.sh` makes and checks the updater artifacts only when the key is set, and is shellcheck clean
- [x] A-003 R3: `release.yml` passes the secrets, verifies the tarball's signature, uploads tarball + `.sig`, and `publish` attaches them with `latest.json` only when present; zip and manifest steps unchanged
- [x] A-004 R4: `release_tool.py latest-json` writes the documented shape and refuses bad input
- [x] A-005 R5: `selfupdate.rs` defines the traits, `NoInstaller`, a fake, and error kinds with sentences; no Tauri in the server crate
- [x] A-006 R6: `/api/update` reports `state`, `progress`, `waiting`, `install_error`; hub `update` events carry them
- [x] A-007 R7: every refusal in R7 is implemented and worded
- [x] A-008 R8: the gate covers firmware (Bluetooth + USB), talk, approvals / asks, wireless mic, face install; waits and can be cancelled
- [x] A-009 R9: stick notify before replace, `pending` recorded, `bye` before relaunch, "Updated to vX" once after
- [x] A-010 R10: `update_auto` default false, settable, installs only when idle 10 min and once per version per 24 h
- [x] A-011 R11: `endpoint()` maps both channels; release builds ignore overrides; debug accepts only localhost
- [x] A-012 R12: the three routes exist, need the token, and the old fields of `/api/update` are intact
- [x] A-013 R13: `TauriInstaller` uses the plugin, maps errors, and reports translocated / not writable / not a bundle in words
- [x] A-014 R14: the menu item wording and click behaviour match R14
- [x] A-015 R15: the dashboard shows banner, notes, Install and restart, progress, waiting, errors, fallback, "Updated to vX", the auto toggle
- [x] A-016 R16: changelog, README, RELEASING.md ("Update signing key"), desktop/README.md, THIRD_PARTY_NOTICES.md updated

### Behavioral Correctness

- [x] A-017 R12: `POST /api/update/agent` no longer answers a flat 409 in the Rust app when an install is possible; with no installer it still answers 409 with where to download
- [x] A-018 R14: "Update available: vX — download…" still appears, and opens the release page, when in-app install is unavailable

### Scenario Coverage

- [x] A-019 R6: a test drives `downloading → installing → restarting` with the fake installer and checks one restart
- [x] A-020 R8: a test holds an install behind a running firmware job and sees it continue after
- [x] A-021 R9: a test shows "Updated to vX" is sent once across two starts
- [x] A-022 R13: the integration test serves a signed fake update from 127.0.0.1 and reaches at least "verified" (state how far in Notes)
- [x] A-023 R15: a JS test covers the legacy shape and the progress text

### Edge Cases & Error Handling

- [x] A-024 R7: lower and equal versions are refused; a staged version mismatch is an error with nothing replaced
- [x] A-025 R13: a tampered tarball and a wrong key give the `signature` kind
- [x] A-026 R6: a second install while one runs answers 409
- [x] A-027 R9: a pending update that did not take is reported once and cleared
- [x] A-028 R3: a build without the secrets passes (script path shown to work locally or by reading the workflow logic)

### Code Quality

- [x] A-029 Pattern consistency: new code follows the style of `updates.rs` / `appctl.rs` / `tray.rs` (module docs that say why, traits + fakes, width 120)
- [x] A-030 No unnecessary duplication: `releases::newer`, `events::notify`, `AppControl::restart`, `link.bye`, `reloadWhenBack`, `notesBlock` are reused, not re-made
- [x] A-031 Logic kept away from hardware / the OS: gate, state machine, endpoint, path checks are pure functions with tests
- [x] A-032 Errors reach the person as words, with the download page as the way out
- [x] A-033 Constants have a name, a unit and the reason (5 s, 30 min, 10 min, 24 h, progress rate)
- [x] A-034 Tests touch no real home, Bluetooth, Keychain, GitHub or `/Applications`; key material is made at run time
- [x] A-035 No hand-bumped version; `Cargo.lock` changed only by cargo
- [x] A-036 The changelog line is written for the person using StickyBot

### Security

- [x] A-037 R1: the public key is compiled in; the private key is referenced only as Actions secrets
- [x] A-038 R11: https only; no config or environment value changes the endpoint in a release build
- [x] A-039 R13: the signature is verified before anything is replaced
- [x] A-040 R7: downgrade protection holds in the server and in the plugin's comparator

### compatibility

- [x] A-041: the legacy Python app's update page still works with the shared JS (no `state` → old path); older firmware sees only ordinary notify messages; `PROTOCOL.md` unchanged
- [x] A-042: an older Rust app (≤ 2.5.0) reading a release with the new assets still finds its zip and firmware (`manifest.json` unchanged)

### real_stick_check

- [ ] A-043: the PR lists what a person must try: the "Updating StickyBot…" and "Updated to vX" toasts on a real stick, a real install from `/Applications` (replace + relaunch, permissions asked again), the release workflow with the real secrets — recorded as open, `needs-hardware` <!-- review: open, needs a person with a real stick and the real app (needs-hardware); not a failure -->

## Notes

- Check items as you review: `- [x]`
- All acceptance items must pass before `/fab-continue` (hydrate)
- If an item is not applicable, mark checked and prefix with **N/A**: `- [x] A-NNN **N/A**: {reason}`

### Apply notes (T014 to T018)

**T016 — test runs** (all in this worktree, scratch venv):
- `cargo test --workspace` in `desktop/`: all green. stickybot-server 419 passed (384 before; 35 new in
  `selfupdate_tests.rs`), stickybot-app 30 passed (the 3 tray tests changed, 14 new in `updater_tests.rs`, incl. the real-plugin ones), stick-link 61, macos-audio and the others unchanged.
- `cargo clippy --workspace --all-targets -- -D warnings`: clean.
- `python -m ruff check .` in `mac-agent/`: clean. `python -m pytest` in `mac-agent/`: everything passes (incl. the new
  `test_release_tool.py` tests and the Node suite `tests/js/updates.test.mjs`, 9 tests) except the one known
  `tests/test_hooks.py::test_claude_hooks_agent_not_running_is_fine` (local only: port 8660 is in use here; its stderr also
  has a local Python "platform dependent libraries" notice).
- `shellcheck desktop/scripts/build-app.sh`: clean. `actionlint` 1.7.12 (darwin_arm64 tarball downloaded from the release,
  checked with `shasum -a 256 -c` against that release's `actionlint_1.7.12_checksums.txt`: OK) on all of
  `.github/workflows/*.yml`: no findings.

**T014 — integration test, how far it goes** (`desktop/crates/stickybot-app/src/updater_tests.rs`, a `#[cfg(test)]` module
because `stickybot-app` is a bin-only crate that `tests/` cannot import): all the way. A minisign key pair is made in the test
(dev-dependency `minisign`), a tiny fake `StickyBot.app` is tarred and signed, a local `127.0.0.1` server serves `latest.json`
and the tarball, and the real `tauri-plugin-updater` (mock app from `tauri::test`, configured like `tauri.conf.json`,
including `requireSignedVersion`) downloads it through `TauriInstaller::download`, verifies the signature and then
`Staged::install` swaps it into a **temporary copy of a bundle** (via `UpdaterBuilder::executable_path`): the bundle's
executable changes from `old` to `new` and stays executable. Also tested and refused with kind `signature` and the bundle
untouched: a tampered tarball, a signature from another key, garbage / empty signature, a genuine older signature announced
as a newer version (`requireSignedVersion`). Plugin comparator: an announced version that is not newer gives `no-update`. A
missing `latest.json` or nothing listening gives `network`. It never touches `/Applications`.

**T015 — local bundle check** (`desktop/scripts/build-app.sh`, throwaway key made with `cargo tauri signer generate` in the
scratch folder, never in the repo):
- with the key (and also with `STICKYBOT_VERSION` set, i.e. two `--config` flags): the build passes, `StickyBot.app.tar.gz`
  (17 MB) and `StickyBot.app.tar.gz.sig` appear; the CLI warns that the throwaway key does not match the pinned public key
  (expected, and shows the pinned `plugins.updater.pubkey` is read). The tarball holds only `StickyBot.app`; untarred,
  `codesign --verify --deep --strict` passes and `codesign -dv` says `Signature=adhoc` and
  `flags=0x10002(adhoc,runtime)` (hardened runtime): the workflow's check regexes match this output. The `.sig` decodes to a
  minisign signature whose trusted comment has `version:2.5.0` (so `requireSignedVersion` works with Tauri CLI 2.12.4).
  `release_tool.py latest-json` accepts the real `.sig` format (checked with `cargo tauri signer sign` output).
- without the key (stale artifacts deleted first): exit 0, prints `no update key: updater artifacts skipped…`, builds and
  ad-hoc signs the `.app`, no tarball and no `.sig`.
- Not run: the release workflow itself (needs the real secrets and GitHub); a real replace-and-relaunch in `/Applications`.

**T018 — screenshots** (headless Chromium from `~/Library/Caches/ms-playwright`, temp profile removed; a fake Rust-app API
server on port 8793 for the real dashboard files; both killed afterwards), in
`<scratch>/update-ui/`
(not committed): `1-banner-available.png`, `2-banner-downloading.png`, `3-banner-waiting.png`, `4-banner-installing.png`,
`5-banner-error.png`, `6-banner-fallback.png`, `7-banner-updated.png`, `8-settings-card-downloading.png`,
`9-settings-card-available.png`, `10-banner-legacy-python.png` (the old shape: old banner, no auto toggle). The driver also
checked that a live `update` event moves the bar to 75 % and that Install and restart starts the flow; no page errors.
`stickybot-headless --fake-stick` has no fake installer (`NoInstaller`), which is why a fake API server stood in for it.

**Decisions and deviations**
- The installer lives on `Updates` (`st.updates.run`) rather than as a new `AppState` field: `status()` needs it and this
  leaves `lib.rs` with only the `pub mod selfupdate;` line.
- `update_auto` default is added with `m.insert(...)` in `config.rs` (the `json!` literal hit the macro recursion limit);
  `PUT /api/config` rejects a non-boolean `update_auto`.
- `tauri.conf.json` also has `"requireSignedVersion": true` (the plugin then refuses a signature made for another version
  than the one `latest.json` announces: a downgrade guard on top of the version comparator). Verified the CLI records the version.
- `tools/release_tool.py` uses `timezone.utc`, not `datetime.UTC`: `tools/release.sh` runs it with the system python3 (3.9).
- `THIRD_PARTY_NOTICES.md` had no Rust section; one was added for the updater and what it brings.
- The owner has added the two Actions secrets; the workflow still passes without them (PR / dispatch / fork) and warns on a
  tag without them, as planned.
- Dashboard "Updated to vX" is dismissed per browser (localStorage `updatedSeen`), no extra endpoint.

## Deletion Candidates

- None — this change adds new functionality without making existing code redundant (re-review after rework cycle 1: the stale `desktop/SWITCHOVER.md` self-update row and the `updates.rs` "(it can't install itself)" comment are fixed; `NO_SELF_UPDATE`, `api.rs` `update_agent` and `tray.rs` `update_link` were already removed by the change, and no reference to them is left).

### Rework cycle 1

- R10: idle for `update_auto` is now measured from the later of the last stick button and this app's start (`auto_tick_at`
  takes `now` so tests can move it): no automatic install in the first 10 minutes after start-up. The old "no button since
  start-up is idle" assertion became the opposite; new tests cover the first 10 minutes and a button after the start. Stale
  comment in `updates.rs` fixed.
- T005: `Run::enter_installing` decides "cancelled?" and the switch to Installing under one lock, so `cancel` can't answer
  200 while the install goes on (test added). The busy gate is looked at again after the "Updating StickyBot…" notify and
  before `Staged::install`; if busy again it goes back to waiting (this re-check is code-reviewed only: no deterministic way
  to start a talk inside the notify's await without a hook, so no test).
- T012: the signing key and password go to the build only on a tag push
  (`github.event_name == 'push' && secrets.… || ''`); a run by hand is always unsigned. Workflow comments and RELEASING.md
  say so; actionlint is clean.
- T013: the banner's new flow rebuilds only when the view, the "Updated" notice or the title changed (progress changes the
  view, so the bar still moves); legacy path untouched; screenshot driver re-run, all checks pass.
- T017: RELEASING.md: no pull request trigger, and the rotation order (the release with the new public key is signed with the
  old key, then swap the secrets); `desktop/SWITCHOVER.md` self-update row updated.
- Re-run: `cargo test --workspace` (server 421), clippy `-D warnings`, ruff, pytest (only the known hooks test), node tests
  (20), shellcheck: all as before.


## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Confident | Handlers for the new routes live in `selfupdate.rs`, `api.rs` only gains the routes | Keeps the diff in a file another change is editing to a few lines | S:70 R:90 A:80 D:75 |
| 2 | Confident | `waiting` is a field next to `state` (which stays `downloading`'s successor `available`-like "verified, waiting"), not an eighth state name | The brief lists seven states; additive field keeps that list | S:60 R:85 A:70 D:60 |
| 3 | Confident | Progress and state reach the dashboard on the existing hub `update` event, no new SSE endpoint | The dashboard already streams the hub; brief allows either | S:75 R:85 A:80 D:75 |
| 4 | Tentative | Waiting for idle is capped at 30 min with a 5 s re-check | Long enough for a firmware update over Bluetooth (minutes), short enough not to restart the app hours later by surprise | S:45 R:90 A:55 D:50 |
| 5 | Confident | Release asset names `StickyBot-<ver>-macos-arm64.app.tar.gz` / `.sig`; the publish job renames the build output | Mirrors the zip's name | S:60 R:85 A:75 D:70 |
| 6 | Confident | The installer is stored on `Updates` (`st.updates.run`), not as an `AppState` field | `updates::status()` needs `install.can_update`; keeps `lib.rs` to one line | S:75 R:90 A:80 D:70 |
| 7 | Confident | `"requireSignedVersion": true` is added to `plugins.updater` in `tauri.conf.json` | The Tauri CLI 2.12.4 writes `version:` into the signature, so it works; it stops an endpoint pairing a newer version number with an older signed tarball | S:70 R:80 A:80 D:70 |
| 8 | Confident | While a verified download waits for the stick, `state` is `available` with `waiting` set (internal phase `Waiting`) | Keeps the seven state names; the pages and menu check `waiting` first | S:70 R:85 A:75 D:70 |
| 9 | Confident | An install start (any way) counts as "tried" for the 24 h rule of `update_auto`; `pending` older than 24 h is dropped silently at start-up | A cancelled or failing install is not retried in a loop; an install that died long ago must not report "didn't take" later | S:65 R:85 A:70 D:65 |
| 10 | Confident | The integration test is a test module in `stickybot-app` (bin-only crate), going through download, signature check and a swap into a temporary bundle | `tests/` cannot import a binary's modules; the plugin does run under `tauri::test` with `executable_path` | S:80 R:90 A:80 D:75 |

10 assumptions (0 certain, 9 confident, 1 tentative).
