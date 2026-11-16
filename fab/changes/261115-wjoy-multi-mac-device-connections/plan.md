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
- [x] T003 [P] Integrate per-peer HID, buddy RX/TX/audio/transfer ownership in m5-stick-s3/src/{hid,buddy,ota,mic}.cpp and lib/HijelHID_BLEKeyboard/. <!-- R2 --> <!-- rework: Clear held mouse state on disconnect before selecting another host. -->
- [x] T004 [P] Integrate shared-mode persistence, peer passwords, host picker and names in firmware settings/protocol/menu/main/ui. <!-- R3 -->
- [x] T005 [P] Add Rust host identity/name validation and hello/session exchange in stickybot-server/hosts.rs, lib.rs, home.rs, config.rs and stick-link. <!-- R4 -->
- [x] T006 [P] Build device registry/session lifecycle and independent runtime in stickybot-server/devices.rs, lib.rs and stick-link/corebt.rs. <!-- R5 --> <!-- rework: Keep home display and rotation state per device while sharing polling services. -->
- [x] T007 [P] Add scoped API, default routing, device events and MCP selection in stickybot-server/{api,devices,mcp,hub}.rs. <!-- R6 --> <!-- rework: Preserve explicit primary MCP notification targeting. -->
- [x] T008 [P] Add dashboard device cards/selector and host labels/controls in static/pages/devices.js, static/lib.js and static/app.js. <!-- R6 -->

### Phase 3: Integration and edge cases
- [x] T009 Implement source-bound approvals, notification destinations and shared speech contention in stickybot-server/{approvals,talk,wmic,events}.rs. <!-- R7 --> <!-- rework: Scope approval queries and bind question resend and cleanup to originating generation. -->
- [x] T010 Add board-aware bundle selection and pre-write validation in stickybot-server/{firmware,flasher,releases}.rs and release tooling. <!-- R8 -->
- [x] T011 Add migration and mixed-peer/device isolation tests across Rust, firmware and shared JS; retain legacy Python fallback. <!-- R3 --> <!-- rework: Add permanent reproductions for cross-device cards, explicit routing, stale questions and held mouse disconnect. -->
- [x] T012 Run required checks/builds; update CHANGELOG.md and hardware/UI checklists; record exact validation evidence. <!-- R9 --> <!-- rework: Rerun checks for review fixes and update evidence. -->

### Phase 4: Diagnostic privacy boundary
- [x] T013 Audit payload-bearing firmware diagnostics and prevent host/private values reaching USB or TCP diagnostic sinks; retain intentional protocol responses and add production-path sentinel regression coverage. <!-- R1; rework: shared-host prompt/event data reaches TeeLog beyond buddy suppression -->

### Phase 5: Shared-service delivery independence
- [x] T014 Make Calendar reminder eligibility and delivery accounting use configured notification destinations rather than the primary link; preserve single shared polling and prevent missed or duplicate reminders with fake multi-device regression tests. <!-- R5/R7; hydrate cross-check found primary-link gating -->

## Execution Order
T001 establishes the interface for firmware (T002–T004), Rust (T005–T007,T009) and dashboard (T008). These groups own disjoint files. T010 follows the Rust interface, and T011–T012 integrate all groups. T013 closes the diagnostic privacy boundary; repeat the affected T012 checks after it. T014 corrects shared-service destination eligibility; repeat the affected Rust T012 checks after it.

## Acceptance

### Functional Completeness
- [x] A-001 R1: Peer/session binding and targeted data paths prevent interleaved message and read leakage. <!-- Review cycle 3: authenticated peer/session routing and notify-only private characteristics verified; fixed diagnostics remove the remaining USB/TCP payload path. Scoped native host/HID/netlog tests passed 19/19. -->
- [x] A-002 R2: Focus switching releases input, rejects busy switches and never silently steals another host's focus. <!-- Review cycle 2: HeldReports clears keyboard/Fn, consumer and mouse holds on disconnect/end and before selection; native regressions passed. -->
- [x] A-003 R3: Legacy slots/bonds work unchanged by default; shared-mode enablement and password assignment are explicit.
- [x] A-004 R4: Full computer name and editable short label appear on dashboard and stick and update without re-pairing.
- [x] A-005 R5: Two configured device sessions connect independently and isolate state, replies, jobs and reconnects. <!-- Review cycle 2: Device-owned Home rotation/send caches and owner/destination-scoped approval reads verified; device and approval regressions passed. -->
- [x] A-006 R6: Scoped API, dashboard selectors, default hooks and MCP targeting preserve operation identity. <!-- Review cycle 2: Explicit API/MCP targeting overrides default notification policy, including spawned RPC calls; regression passed. -->
- [x] A-007 R7: Approvals resolve once with origin ownership; notification policy is explicit; competing mic requests fail busy. <!-- Review cycle 2: Question resend and cleanup retain captured generation; cancellation and first-valid fanout decision regressions passed. -->
- [x] A-008 R8: Wrong-board and unknown images are rejected before writing, including USB and BLE paths.
- [x] A-009 R9: Appropriate automated checks pass, with exact hardware evidence limits documented.

### Edge Cases and Security
- [x] A-010 R1: Invalid, unencrypted, stale-session and oversized host payloads are rejected without corrupting other peers.
- [x] A-011 R2: Active-peer disconnect, busy transfer, held keys, and delayed approval responses retain safe ownership. <!-- Review cycle 2: Held-input disconnect state clears and questions cancel across reconnect generations; native/Rust regressions passed. -->
- [x] A-012 R3: A cable to another Mac cannot receive the active host's dictation; secrets are never returned/logged.
- [x] A-013 R4: Empty, duplicate, unicode, oversized and control-character labels have bounded deterministic handling.

### Code Quality
- [x] A-014: Pure logic uses hardware-free tests; style matches neighboring code and utilities are reused.
- [x] A-015: Protocol changes are capability gated on both sides with documented old/new behavior.
- [x] A-016: Fixed firmware buffers, bounded queues and flash <=85% are documented from builds.
- [x] A-017: No test touches live hardware, network accounts, Keychain or personal configuration.
- [x] A-018: No unnecessary duplication, new unlicensed assets, hand-edited generated files or manual version bumps.

### Compatibility
- [x] A-019: Legacy Python/shared dashboard and older firmware remain usable in single-device/legacy mode.
- [x] A-020: S3 and Plus2 build; XIAO retains its existing supported HID scope.

### Diagnostic Privacy
- [x] A-022 R1: Private host labels, prompts/options/hints, events and user-supplied payload fields do not reach unsolicited USB/TCP diagnostics; hardware-free sentinel tests exercise the production logging path. <!-- Review cycle 3: actual TeeLog fixed-event/numeric paths and fail-closed Print writes audited across authored firmware including vendored HID; 7 production-path regressions passed. Independent /tmp/wjoy-review3-netlog.cpp probes passed all 256 byte values, interleaved partial client commands, and recycled-client inbox isolation. USB/TCP machine framing and human replies remain requester-only. -->

### Shared-Service Delivery
- [x] A-023 R5/R7: A connected configured notification destination receives due Calendar reminders when primary is offline; an offline destination does not cause a reminder to be marked delivered, and reconnect within the alert window delivers once. Disabled routing and fanout retain intentional behavior. <!-- Independent Calendar review: 29/29 Calendar tests and explicit-primary API/MCP regression passed; external fake probe verified stale-generation rejection and successful current-generation retry. Root/background policy, configured/default secondary delivery, reconnect, concurrent ticks, failed writes and partial-success fanout audited. Empty destinations consume immediately; off rules consume when a destination is online, retaining legacy pending behavior while all destinations are offline. Prior accepted source and validation remain applicable; hardware checks remain pending. -->

### Real Stick Check
- [x] A-021: Two-Mac/two-device hardware checklist records pairing, labels, keyboard, voice, approvals, reconnect, sleep/wake and OTA; unperformed checks stay pending and draft carries needs-hardware. <!-- Checklist and draft label verified only; all actual physical checks remain pending. -->

## Notes
Apply marks tasks; independent review marks acceptance. Actual physical validation is not asserted by fake tests.

## Deletion Candidates

- `desktop/crates/stickybot-server/src/python_agent.rs::check` — production connections now call `check_target`; only legacy-wrapper tests call `check`. Consider moving those tests to the production path and removing the wrapper.
- `desktop/crates/stickybot-server/src/mcp.rs::brain_config` — production voice sessions now use `brain_config_device`; only the legacy wrapper's test calls this function. Consider moving that test to the production path and removing the wrapper.

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
- Initial apply `git diff --check` — passed. Independent review subsequently reopened T003, T006, T007, T009, T011 and T012 for the four findings recorded above. Rework validation below supersedes the initial completion claim. Review includes authored vendored HID changes because connection targeting and read isolation depend on them; physical verification remains pending.


### Rework cycle 1

The independent review reproduced four defects: shared home rotation/cards and unscoped approval reads; explicit primary notifications being rerouted through default policy; questions resent/cleared across connection generations; and held mouse state surviving peer disconnect. Apply uses the FIX CODE path, with permanent regressions and fresh Rust/firmware checks. Acceptance is reserved for the independent reviewer and is not changed by apply.

- Finding 4: the vendored driver now uses pure `HeldReports` state for production keyboard/Fn, consumer and mouse holds and mouse packet construction. Disconnect/end clear it unconditionally; reselection releases/clears the previous state even after disconnect. Permanent tests in `test/test_hid_reports/` are `test_disconnected_host_cannot_carry_buttons_to_new_host_movement`, `test_connected_switch_releases_all_held_reports_before_new_input`, and `test_idle_release_avoids_spurious_consumer_and_mouse_notifications`.
- Rework firmware checks: `pio test -e native` — **286/286 passed**, 35.625 s. Final `pio run -e sticks3` — passed, **2,464,811/3,342,336 bytes (73.7%)**, RAM **142,828/327,680**. Final Plus2 build — passed, **2,377,615/3,342,336 (71.1%)**, RAM **116,004/327,680**. Logs: `/tmp/wjoy-rework1-native-final.log`, `/tmp/wjoy-rework1-s3-final.log`, and final Plus2 result in `/tmp/wjoy-rework1-firmware.log`. The combined log retains an initial stale-accessor compile error, fixed before both final-source builds. Physical verification remains pending.

- Findings 1–3: each device now owns Home rotation/pins/progress/reply/world-send caches while weather/feed/GitHub/pet providers remain shared. Approval visibility and busy checks use owner/destination scope. Explicit tool/API targets override default/background delivery policy, including primary RPC calls without an argument-level `device_id`. Question disconnect/generation changes return cancellation; resend and asynchronous cleanup retain the captured generation.
- Permanent Rust regressions: `devices::tests::home_display_slots_reply_cache_and_world_sends_are_per_device`; `devices::tests::explicit_primary_notifications_override_default_and_policy_without_changing_background_delivery` (also scoped primary RPC); `approvals::multi_device_tests::questions_and_home_approval_cards_do_not_leak_or_block_sibling_devices`; `approvals::multi_device_tests::disconnected_question_retires_and_late_cleanup_cannot_clear_new_session`. Existing fanout regression also verifies outsider/destination Home visibility and busy checks.
- Final rework Rust: `cargo test --workspace --manifest-path desktop/Cargo.toml` — **664 passed**, zero failures (audio 61, calendar 4, link 65, app 30, server 501, integrations 3; doctests passed). `cargo clippy --workspace --all-targets --manifest-path desktop/Cargo.toml -- -D warnings` — passed. Logs: `/tmp/wjoy-rework-workspace.log`, `/tmp/wjoy-rework-clippy.log`.
- The original unmodified reviewer harness was rerun (`/tmp/wjoy-review-probe`, output `/tmp/wjoy-rework-probe.log`): B no longer displays A's pin/reply/question; clearing B preserves A's pin; explicit primary notification reaches primary and leaves B history empty; B is not falsely busy; old question is not resent after generation 1→3 and its stale answer is rejected. Permanent regressions supply assertions for these outcomes.
- Final rework `git diff --check` — passed. All four must-fix findings are implemented and T001–T012 are complete. Sources are frozen for fresh independent review. Acceptance marks/comments were not edited by apply. Hardware checks remain pending; no real stick was connected or flashed.

### Rework cycle 2 plan revision

Independent review resolved the four initial findings but identified an uncovered diagnostic output boundary. T013 and A-022 cover all payload-bearing firmware logs (main, buddy, protocol, IR, face install and OTA), with intentional protocol responses kept distinct from unsolicited diagnostics. This extends privacy coverage without changing the approved product behavior.


### Rework cycle 2 implementation

- All authored firmware diagnostics now use the fixed `diagnostics::Event` catalogue or typed numeric mechanical counters. The audit covers prompt/options/hints/origins/events in main, buddy RX/TX, protocol settings/name/SSID, web uploads, launcher configuration, IR labels/codes, face IDs/versions/errors, OTA supplied versions, bonded addresses and vendored HID key/pairing debug paths. No raw `Log.print/println/printf/write` callsite remains in authored firmware. Vendored HID logging invokes a payload-free callback and never writes directly to Serial.
- Production `TeeLog` rejects all raw Print bytes with a fixed omitted-payload marker, independently of legacy/shared mode. Fixed diagnostics retain `ready device`, start/failure/completion status and useful byte/frame/drop/retry/storage/timing counters. Detailed payload-bearing debug prose is intentionally unavailable on broadcast sinks; explicit command responses remain separate.
- USB/TCP protocol handlers receive a requester-owned `Print&`. TCP commands use the originating client; USB defaults to Serial. Production `machineReply` preserves exact `@` + JSON + CRLF framing. Remaining direct Serial responses in the main loop belong to synchronous USB `event`/`sound` commands. `PROTOCOL.md`, the firmware README, `docs/MULTI_DEVICE.md` and CHANGELOG describe the boundary and pending USB/TCP hardware checks.
- `test/test_netlog/` compiles actual `netlog.cpp` with fake Arduino/Wi-Fi sinks and the production machine response formatter. Seven regressions cover private sentinels via Print/println/printf/fragmented/raw writes across USB and three TCP sinks; fixed events and startup readiness; numeric counters; USB machine response isolation; TCP machine response isolation; human TCP requester isolation while diagnostics still fan out; and generic human success JSON acknowledgement on only its USB/TCP requester. Only command execution and physical sink I/O are substituted.

- Final cycle-2 native check: `~/.platformio/penv/bin/pio test -e native` from `m5-stick-s3/` — **293/293 passed**, 31.617 s, including **7/7 diagnostic/response regressions** (`/tmp/wjoy-rework2-native-final.log`). Regression names: `test_real_teelog_rejects_private_print_printf_and_raw_fragments_on_all_sinks`, `test_fixed_events_reach_usb_and_tcp_without_payload_or_protocol_prefix`, `test_typed_mechanical_counters_preserve_values_without_private_text`, `test_usb_machine_response_keeps_legacy_frame_and_never_reaches_tcp`, `test_human_success_reply_is_explicit_and_only_goes_to_requester`, `test_tcp_machine_response_reaches_only_its_requesting_client`, and `test_tcp_human_response_is_requester_only_while_events_still_fan_out`.

- Final cycle-2 board builds: `~/.platformio/penv/bin/pio run -e sticks3 -e stickc_plus2`, followed by `~/.platformio/penv/bin/pio run -e sticks3` after restoring the face error discriminator used by the intentional response — **both final-source builds passed**. S3 flash **2,455,779/3,342,336 bytes (73.5%)**, RAM **142,820/327,680**; Plus2 flash **2,368,679/3,342,336 (70.9%)**, RAM **115,996/327,680**. Both remain below the 85% flash budget. Logs: `/tmp/wjoy-rework2-s3-final.log` and final Plus2 result in `/tmp/wjoy-rework2-firmware.log`; the combined log retains the initial corrected S3 compile failure. Existing framework/NimBLE configuration/deprecation and volatile-access compiler warnings remain; no new build failure is outstanding.
- No Rust, Python or dashboard source changed in cycle 2; their prior full-suite/lint evidence above remains applicable. Final cycle-2 `git diff --check` passed. T001–T013 are complete and source is frozen for independent review. Acceptance marks/comments were not edited by apply. Real USB/TCP, Bluetooth, display, sleep/wake and update checks remain pending in the hardware checklist; no physical hardware was accessed.

### Post-review shared-service correction

The hydrate source cross-check found Calendar tick still gating due reminders on the primary link. Apply is reopened for T014/A-023, followed by fresh independent review and a new hydrate dispatch. Existing reviewed implementation and memory edits are retained; the stale Calendar memory clause must be reconciled after correction.

- Calendar uses the shared event destination resolver and consumes an occurrence after at least one successful link send, or intentional suppression by an empty destination list. An off event rule consumes the occurrence when at least one destination is reachable; with all destinations offline, it stays pending as in legacy behavior. Offline destinations and failed writes leave the occurrence pending within the existing alert window; ticks with no reachable recipient do not add event-history attempts. Fanout remains one occurrence rather than replaying to already served devices. A shared tick mutex prevents overlapping loop/API checks from duplicating delivery; device-scoped Calendar requests retain the background notification policy and root service context.
- Added six fake Calendar / fake stick regressions covering selected and default secondary recipients with primary offline; secondary offline while primary is online followed by reconnect inside the window; scoped API fanout; empty destinations and off event rules; write failure/retry; partial fanout success without replay. Tests also assert no duplicate EventKit reads or notification history from idle ticks. The nearby Home path still reads the shared Calendar through each device's existing hello/observer and computes its device-local card; no additional poller or unrelated Home change was introduced.
- `cargo test -p stickybot-server connections::calendar::tests` — **29/29 passed**, including all six new regressions (`/tmp/wjoy-calendar-focused.log`).
- `cargo test --workspace` — **670 passed**, zero failures (audio 61, calendar 4, link 65, app 30, server 507, integrations 3; doctests passed), including the existing explicit API/MCP notification routing regressions (`/tmp/wjoy-calendar-workspace-tests.log`). The first sandboxed run hit `Operation not permitted` when existing updater tests bound temporary localhost sockets; the approved rerun with local socket access passed. All tests used fake services and temporary homes, with no real app, hardware, Keychain or network account accessed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed (`/tmp/wjoy-calendar-clippy.log`). Targeted `rustfmt --edition 2024 --check` for the edited Rust modules and `git diff --check` passed. Stable Rust was added to PATH and the existing sherpa-onnx static library directory supplied via `SHERPA_ONNX_LIB_DIR` as in the previous validation.
- Added an Unreleased changelog entry for notification-destination reminder delivery. No firmware, Python or dashboard source changed in this correction, so their existing evidence remains applicable; hardware/UI checklists remain pending. T001–T014 are complete and source is frozen for fresh independent review. Apply did not edit Acceptance or any of the paused hydration memory files, and ran no commit, push, PR edit or stage transition.

### PR 12 review corrections

All eleven inline code findings were addressed. Permissions follow their captured devices across reconnects while stale connection work remains rejected; questions retain their existing session lifetime. Empty or invalid destinations return immediately, device removal prunes routing lists, and invalid saved entries cannot abort startup. Capability negotiation retries after timeouts. Legacy mode bypasses shared-host enrollment; shared mode provides confirmed menu recovery and `@hosts forget` without erasing all settings. Pending HID selections no longer repeat teardown before encryption.

USB discovery passively enumerates generic WCH bridges without opening them. Each explicit flash probes firmware identity at most once. Recovery requires an explicitly selected port/model, verifies the ROM chip before writing, and retains bundle/image/hash checks. S3 reset accepts a differing ROM serial only for one newly appeared bootloader with the app gone and no competing bootloader. The dashboard exposes recovery only when the backend supports it. Public `created_by` metadata is blank. Protocol, usage and memory documents describe the corrected behavior.

- Final Rust workspace: **678 passed**, zero failures (audio 61, Calendar 4, link 66, app 30, server 514, integrations 3); doctests passed. Regression coverage includes reconnect delivery/stale decisions, empty and removed destinations, invalid saved devices, negotiation retry, passive WCH discovery, one flash identity probe, ROM recovery/chip rejection and differing/ambiguous bootloader descriptors. Log: `/tmp/wjoy-pr12-final-tests.log`.
- Firmware native: **294/294 passed**. Both production board builds passed: S3 **2,457,547 / 3,342,336 bytes (73.5%)**, Plus2 **2,370,623 / 3,342,336 (70.9%)**. Logs: `/tmp/wjoy-pr12-native.log`, `/tmp/wjoy-pr12-firmware.log`.
- Python: **869 passed, 1 skipped, 5 deselected**; Ruff passed. JavaScript: **35 passed**, including explicit recovery port/model selection, originating-device job routing and legacy payload compatibility. Logs: `/tmp/wjoy-pr12-python.log`, `/tmp/wjoy-pr12-js.log`.
- Final Clippy with `-D warnings`, targeted Rust formatting, `git diff --check` and the documentation index guard passed. The existing Mac-apps memory size advisory is nonblocking. Rust/firmware/Python tests used fake services and temporary homes; no live app, stick, Keychain or personal configuration was accessed. Clippy log: `/tmp/wjoy-pr12-final-clippy.log`.
- Physical S3 application/ROM descriptor capture and real recovery remain **pending**, as do the existing two-Mac/two-board checks. Fake descriptors establish fallback behavior but do not certify real hardware. PR stays draft with `needs-hardware`; the descriptor-verification review thread stays open until that evidence exists.
- CI's generic credential rule flagged ordinary prose in the Mac-link memory. The sentence was reworded and `.gitleaksignore` records only that exact historical fingerprint, because CI scans all PR commits. No credential or broader detection rule is excluded.
