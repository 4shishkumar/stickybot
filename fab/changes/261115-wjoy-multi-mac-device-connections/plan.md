# Plan: Multiple Macs and devices with recognizable host names

**Change**: 261115-wjoy-multi-mac-device-connections
**Intake**: `intake.md`

## Requirements

### Link: identities, routing and compatibility

#### R1: Authenticated host protocol
The firmware SHALL expose stable `deviceId`, `board`, `multiHost:1` and `maxHosts:3` in state. New app hellos SHALL carry `host:{id,name,label,session}`. Shared-mode writes SHALL include `hs` matching the authenticated connection's hello session. Identity is bound to the bonded peer, never the supplied label. Per-peer RX and targeted TX/audio/HID SHALL prevent cross-host leakage, including characteristic reads. Host state SHALL be queried through `@hosts` and emitted as `evt:hosts`.
- GIVEN two authenticated hosts WHEN their JSON fragments interleave THEN independent messages and responses reach only their originating connection.

#### R2: Explicit host focus and exclusive resources
One selected host SHALL receive input. Host selection on the stick SHALL release existing HID state before switching and SHALL refuse busy switches until operations finish. Remote focus requests SHALL require stick confirmation. Disconnected focus SHALL not silently select another host. Recording, speaker and bulk transfers SHALL have a single authenticated owner; delayed/other-host packets SHALL not mutate that operation. Prompts/events SHALL retain their originating peer and label.
- GIVEN Mac A focused and Mac B reconnected WHEN a key is pressed THEN only A receives it.
- GIVEN B's approval displayed WHEN approved THEN only B's pending request resolves.

#### R3: Pairing and legacy migration
Shared mode SHALL be opt-in via `@hosts shared=1`, retain existing slot bonds, and use the active slot's stable identity. Legacy mode SHALL preserve existing single-slot behavior. Old applications SHALL continue working in legacy mode, and unsupported shared mode SHALL explain upgrade requirements. Per-host passwords SHALL be write-only; an old global password SHALL not be used on a new host without explicit assignment. Shared mode SHALL prefer Bluetooth audio unless USB host identity is established.
- GIVEN legacy pairings WHEN firmware is upgraded THEN no bonds are erased and default behavior remains legacy.

### Apps: devices and host names

#### R4: Recognizable host names
The Mac app SHALL persist installation ID and detect ComputerName through its injectable System seam. `host_name` is the detected full name (max 80 characters); `host_label` is an optional custom short label (max 12 printable display characters; empty resets auto label). The dashboard and stick SHALL show effective label, full name where space permits, active/online state and stable disambiguation for duplicates. Renaming SHALL propagate on hello without re-pairing.
- GIVEN a host named Test Laptop WHEN user sets label Desk THEN dashboard and stick show Desk while stable identity and bonds remain unchanged.

#### R5: Independent multiple-device runtime
Rust SHALL maintain independent per-device links, capabilities, recordings/conversations, replies and operations. Shared Mac services SHOULD remain shared. Bluetooth scans SHALL be coordinated. Old `stick` configuration SHALL migrate without losing settings; discovered devices SHALL require explicit selection. Device failures SHALL not stop other sessions. One dictation stream per Mac SHALL be enforced explicitly.
- GIVEN two configured sticks WHEN one disconnects THEN the other remains usable and receives only its own replies.

#### R6: Device-scoped API, UI and tools
`GET /api/devices` SHALL list configured devices and their status. `POST /api/devices` SHALL add a selected discovery target. `DELETE /api/devices/{id}` SHALL remove it. `POST /api/devices/{id}/default` SHALL set the default. `POST /api/devices/{id}/pause` SHALL control its connection. Requests under `/api/devices/{id}/api/...` SHALL execute the existing API against that device, retaining device identity for the entire request. Events SHALL identify device. Unscoped legacy routes and hooks SHALL use the persisted default, not the browser's selected view. MCP tools MAY specify device_id. Dashboard device cards and selector SHALL expose model, battery, online state and hosts, with per-device actions.
- GIVEN a firmware job started for stick A WHEN browser selection changes to B THEN the job remains bound to A.

#### R7: Approval and notification routing
Mac requests SHALL target default or explicit devices, approval decisions SHALL resolve once and clear other displays, and source host/device/session SHALL prevent stale cross-routing. Notification destinations SHALL be explicit; default remains default device.
- GIVEN the same prompt sent to two devices WHEN both answer THEN the first valid decision wins once.

#### R8: Board-safe firmware
Firmware updates SHALL require compatible canonical board identity and chip/layout, reject unknown/mismatched artifacts before writes, and return a clear unavailable message when no compatible bundle exists. Existing aliases SHALL normalize. Capabilities SHALL hide unsupported hardware features.
- GIVEN a Plus2 and only S3 firmware WHEN update requested THEN no bytes are written and the dashboard explains no compatible image.

#### R9: Tests, evidence and honest compatibility
Native firmware logic tests, Rust fake tests, shared dashboard JS/Python checks and firmware builds SHALL validate touched paths. No test SHALL touch real app/config/hardware. Two Macs/two mixed boards hardware checks SHALL be documented as pending, not fabricated. Firmware SHALL stay within flash budgets. Changelog/protocol/docs SHALL describe actual behavior.
- GIVEN no physical hardware validation WHEN draft PR is opened THEN it carries an explicit hardware checklist and needs-hardware status.

### Non-Goals
- Adding a Mac-link service to XIAO C3; existing HID support remains intact.
- Automatic merge, release, or flashing the user's device.

## Tasks

### Phase 1: Contract
- [x] T001 Define protocol host handshake, focus/state, resource ownership, compatibility and API contract in PROTOCOL.md. <!-- R1 -->

### Phase 2: Core implementation
- [x] T002 [P] Implement bounded host/session/focus logic and native tests in m5-stick-s3/src/logic/hosts.* and test/test_hosts/. <!-- R1 -->
- [x] T003 [P] Integrate per-peer HID, buddy RX/TX/audio/transfer ownership in m5-stick-s3/src/{hid,buddy,ota,mic}.cpp and lib/HijelHID_BLEKeyboard/. <!-- R2 -->
- [x] T004 [P] Integrate shared-mode persistence, peer passwords, host picker and names in firmware settings/protocol/menu/main/ui. <!-- R3 -->
- [x] T005 [P] Add Rust host identity/name validation and hello/session exchange in stickybot-server/hosts.rs, lib.rs, home.rs, config.rs and stick-link. <!-- R4 -->
- [x] T006 [P] Build device registry/session lifecycle and independent runtime in stickybot-server/devices.rs, lib.rs and stick-link/corebt.rs. <!-- R5 -->
- [x] T007 [P] Add scoped API, default routing, device events and MCP selection in stickybot-server/{api,devices,mcp,hub}.rs. <!-- R6 -->
- [x] T008 [P] Add dashboard device cards/selector and host labels/controls in static/pages/devices.js, static/lib.js and static/app.js. <!-- R6 -->

### Phase 3: Integration and edge cases
- [x] T009 Implement source-bound approvals, notification destinations and shared speech contention in stickybot-server/{approvals,talk,wmic,events}.rs. <!-- R7 -->
- [x] T010 Add board-aware bundle selection and pre-write validation in stickybot-server/{firmware,flasher,releases}.rs and release tooling. <!-- R8 -->
- [x] T011 Add migration and mixed-peer/device isolation tests across Rust, firmware and shared JS; retain legacy Python fallback. <!-- R3 -->
- [x] T012 Run required checks/builds; update CHANGELOG.md and hardware/UI checklists; record exact validation evidence. <!-- R9 -->

## Execution Order
T001 establishes the interface for firmware (T002–T004), Rust (T005–T007,T009) and dashboard (T008). These groups own disjoint files. T010 follows the Rust interface, and T011–T012 integrate all groups.

## Acceptance

### Functional Completeness
- [ ] A-001 R1: Peer/session binding and targeted data paths prevent interleaved message and read leakage.
- [ ] A-002 R2: Focus switching releases input, rejects busy switches and never silently steals another host's focus.
- [ ] A-003 R3: Legacy slots/bonds work unchanged by default; shared-mode enablement and password assignment are explicit.
- [ ] A-004 R4: Full computer name and editable short label appear on dashboard and stick and update without re-pairing.
- [ ] A-005 R5: Two configured device sessions connect independently and isolate state, replies, jobs and reconnects.
- [ ] A-006 R6: Scoped API, dashboard selectors, default hooks and MCP targeting preserve operation identity.
- [ ] A-007 R7: Approvals resolve once with origin ownership; notification policy is explicit; competing mic requests fail busy.
- [ ] A-008 R8: Wrong-board and unknown images are rejected before writing, including USB and BLE paths.
- [ ] A-009 R9: Appropriate automated checks pass, with exact hardware evidence limits documented.

### Edge Cases and Security
- [ ] A-010 R1: Invalid, unencrypted, stale-session and oversized host payloads are rejected without corrupting other peers.
- [ ] A-011 R2: Active-peer disconnect, busy transfer, held keys, and delayed approval responses retain safe ownership.
- [ ] A-012 R3: A cable to another Mac cannot receive the active host's dictation; secrets are never returned/logged.
- [ ] A-013 R4: Empty, duplicate, unicode, oversized and control-character labels have bounded deterministic handling.

### Code Quality
- [ ] A-014: Pure logic uses hardware-free tests; style matches neighboring code and utilities are reused.
- [ ] A-015: Protocol changes are capability gated on both sides with documented old/new behavior.
- [ ] A-016: Fixed firmware buffers, bounded queues and flash <=85% are documented from builds.
- [ ] A-017: No test touches live hardware, network accounts, Keychain or personal configuration.
- [ ] A-018: No unnecessary duplication, new unlicensed assets, hand-edited generated files or manual version bumps.

### Compatibility
- [ ] A-019: Legacy Python/shared dashboard and older firmware remain usable in single-device/legacy mode.
- [ ] A-020: S3 and Plus2 build; XIAO retains its existing supported HID scope.

### Real Stick Check
- [ ] A-021: Two-Mac/two-device hardware checklist records pairing, labels, keyboard, voice, approvals, reconnect, sleep/wake and OTA; unperformed checks stay pending and draft carries needs-hardware.

## Notes
Apply marks tasks; independent review marks acceptance. Actual physical validation is not asserted by fake tests.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Host record capacity is three, initial hardware acceptance is two hosts | Existing three slots and bounded NimBLE defaults; hardware evidence remains pending | S:90 R:85 A:85 D:90 |
| 2 | Certain | Device-specific existing APIs use /api/devices/{id}/api/... | Reuses established handlers with explicit request-scoped state | S:80 R:90 A:90 D:85 |
| 3 | Certain | Empty custom label derives a bounded ASCII display label from full ComputerName | Existing firmware font constraints and explicit user naming request | S:95 R:95 A:90 D:90 |

3 assumptions (3 certain, 0 confident, 0 tentative).

### Apply validation evidence

- Shared Python/dashboard suite, from `mac-agent/`: `../.venv/bin/python -m pytest -o addopts="-m 'not slow and not coreaudio'" -q --tb=short` — **869 passed, 1 collection skip, 5 deselected**, 96.92 s. The skip is the unchanged legacy menu-bar module because optional `rumps` is absent; deselected tests require a real speech model/CoreAudio/hardware. Fake localhost servers ran with sandbox socket restrictions lifted; no real app or hardware was started.
- `../.venv/bin/ruff check .` from `mac-agent/` — passed. `ruff check --config mac-agent/pyproject.toml mac-agent/tests/test_make_release_boards.py m5-stick-s3/tools/make_release.py` — passed.
- `node --test mac-agent/tests/js/*.test.mjs` — **34 passed**, including source-bound firmware/face callbacks and late status response isolation. Syntax checks passed for all changed dashboard modules.
- Release packaging tests cover both supported board layouts and reject wrong app/bootloader silicon before modifying an output bundle.
- Physical tests remain **pending**. `docs/MULTI_DEVICE.md` records the two-Mac/two-board checklist, USB cross-host audio check, update/resume/rollback checks and exact dashboard/stick screenshots to capture. A draft must retain `needs-hardware`; no hardware acceptance is claimed.

- Firmware native: `~/.platformio/penv/bin/pio test -e native` — **283/283 passed**, including **9/9 host routing/authentication/resume tests**, 27.645 s (`/tmp/wjoy-native-verified.log`).
- Firmware builds: `pio run -e sticks3 -e stickc_plus2` passed; a final warning-only cleanup was followed by another successful `pio run -e sticks3` (67.622 s). **Final source**: S3 flash **2,464,767 / 3,342,336 bytes (73.7%)**, RAM **142,828 / 327,680**; Plus2 flash **2,377,571 / 3,342,336 (71.1%)**, RAM **116,004 / 327,680**. Both remain below the 85% flash budget. Logs: `/tmp/wjoy-firmware-verified.log`, `/tmp/wjoy-s3-warning-clean.log`.

- Final Rust workspace: `cargo test --workspace --manifest-path desktop/Cargo.toml` — **660 passed**, zero failures (macos-audio 61, macos-calendar 4, stick-link 65, app 30, server 497, integrations 3; doctests passed). This run includes source-generation guards, root-authoritative config/name/migration writes, default tray routing, and scoped activity/event-history regression tests. Log: `/tmp/wjoy-workspace-final.log`.
- Final Rust lint: `cargo clippy --workspace --all-targets --manifest-path desktop/Cargo.toml -- -D warnings` — passed (`/tmp/wjoy-clippy-final.log`). Stable Rust was added to PATH and the existing sherpa-onnx static library directory supplied via `SHERPA_ONNX_LIB_DIR`; no app/hardware process ran.
- Final `git diff --check` — passed. T001–T012 are complete; acceptance items remain unchecked for the independent review. Review must include authored vendored HID changes because connection targeting and read isolation depend on them. No remaining known implementation blocker; physical verification remains pending as described above.
