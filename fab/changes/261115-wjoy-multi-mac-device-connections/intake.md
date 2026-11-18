# Intake: Multiple Macs and devices with recognizable host names

**Change**: 261115-wjoy-multi-mac-device-connections
**Created**: 2026-11-15

## Origin

Conversational design approved by the user: "implementationplan approved start implementing. one ting i want ot add is is possible to show in dashbaord as well as on stick what is mac1 and mac2 etc. their real names or abbriavtionuser can set ,a short name".

The approved plan is simultaneous many-to-many Bluetooth connectivity: one stick remains connected to multiple Macs, sending keyboard/mouse/voice to one selected host; one Rust Mac app manages multiple sticks concurrently. The requested endpoint is an OPEN DRAFT PR through fab-new + fab-ff (implementation, independent review, hydrate, CI evidence), not merge/release. The primary session manages stage transitions, commits and pushes. Real hardware remains untouched until explicitly authorized; mark hardware validation pending in the draft. This worktree starts from origin/main e567cea (release v2.6.0).

## Why

Current firmware has three saved slots with distinct Bluetooth identities and reboots when switching slots. hid.cpp, the vendored HID library, and buddy.cpp assume a single peer; buddy RX has one inbox, notification sends are untargeted, prompts/audio are global and getPeerInfo(0) selects a peer arbitrarily. Simply raising the Bluetooth connection count would route input and private responses incorrectly. The Mac app has one configured stick and one AppState.link, despite CoreBluetooth already keeping per-peripheral routes.

People need several Macs and boards without re-pairing for routine selection, and need to recognize hosts by their real name or chosen short label. Device sessions, authenticated peer routing, and explicit resource ownership provide that behavior without a cloud relay or Wi-Fi setup.

## What Changes

### Identity, host names and protocol

Update PROTOCOL.md before implementing message changes. Add stable physical device identity, board identity, capability/limit advertisement, stable persisted Mac installation identity, host metadata and focus-selection messages. Bind claims to the authenticated Bluetooth peer; supplied IDs and names are display/routing metadata, not credentials. Include a session generation so delayed responses cannot act on a new session. Keep bounded per-peer input assembly and targeted notifications (including HID and audio), and prevent shared GATT reads exposing another host's payload.

Detect the macOS ComputerName through an injectable System interface, retaining a safe fallback if unavailable; do not run OS commands in fakes. Persist a user-editable short label (proposed 1–12 displayable characters) separately from the detected full name (bounded to 80 characters). Empty custom label restores an automatically derived short name. Dashboard lists full name and short label, pairing/online/active state; the stick displays the short label in its host picker, active-host indicator and prompt origin. Changes propagate on hello without reboot/re-pairing; retain offline host labels. Use stable IDs for identity, never names. Duplicate labels are disambiguated in selection UI; unicode names are safely displayed/transliterated for firmware fonts; validation rejects control characters and bounds bytes. Use synthetic host names in tests/docs, never actual machine names or passwords.

### Rust device manager

Separate shared Mac services (credentials, speech model/worker, calendar/GitHub polling, app updates) from per-device runtime (Link, capabilities, state, recording/conversation/reply, jobs, host focus, reconnect). Each session has its own lifecycle and cancellation; one unavailable device must not break others. Centralize scanning to avoid stopScan races between simultaneous connections. Add explicit device-scoped API routing, SSE events and MCP/tool targeting; keep existing unscoped calls routed to a persisted default device. A UI-selected device is a view only, not a global mutable target for in-flight operations. Migrate the old single `stick` config into the device list without losing settings; no unexpected auto-connect to discovered but unselected devices. Python coexistence checks apply to the particular target.

Voice requests reply only to the originating stick with conversation isolation. Share the speech model but arbitrate contention explicitly. First release permits one dictation stream per Mac; another requester receives a clear busy response, not mixed audio. Existing hooks use the default device. Notification destinations are configurable; approvals shown on several devices are resolved exactly once and cleared elsewhere. Per-Mac request ownership prevents approval response or cancellation crossing hosts.

### Firmware multiple hosts and resource arbitration

Use bounded authenticated peer records instead of single active connection globals. One selected input host receives keyboard, mouse, media, unlock and voice; other connected hosts can deliver bounded notifications and host-labelled prompts. A prompt response goes to its origin, independent of the input target. Unrelated hosts cannot cancel, replace or complete another host's operation. Only one owner uses the physical microphone, speaker, display prompt or bulk channel at a time; requests fail visibly busy or enter bounded queues, with expiry/disconnect cleanup. An OTA/face-transfer owner is tied to peer/session.

The stick host picker changes focus without disconnecting all hosts or rebooting. Before focus changes, release keys/mouse buttons on the old host; finish/cancel ongoing recording or wait for exclusive transfer. A Mac can request takeover but needs confirmation on the stick; reconnect never steals focus. If active host disappears, stop input and show offline, never silently redirect to a different computer. App presence is heartbeat per host; HID may remain available when its app is absent. Maintain per-peer HID auth, subscription and connection-parameter state in the vendored library, not just buddy routing.

### Compatibility and pairing migration

Preserve existing three slot identities/bonds in legacy mode. Simultaneous mode is opt-in and uses one persistent identity (reuse a selected existing slot identity where possible); other previously slotted Macs may require guided pairing with that identity. No automatic bond wipe. Existing firmware + new Rust app and new firmware in legacy mode + old app must continue working. Old apps do not participate in simultaneous mode without the new routing handshake; make that requirement visible and retain legacy fallback. Python remains single-device, shared UI must capability-gate new controls gracefully. XIAO C3 remains supported as existing HID-only hardware; adding its missing Mac-link service is outside this change.

The current global unlock password must not be reused for every Mac. Store secrets per authenticated host in simultaneous mode; require explicit assignment before migrating a legacy password. Values remain write-only and never logged/returned. USB microphone currently knows only mounted/unmounted, not which Mac: in simultaneous mode use wireless audio unless USB host association is verified. Never send speech to the cabled Mac when another Mac is selected.

### Dashboard and mixed hardware

Device cards show model, name, battery, capabilities, connected hosts and active Mac. Per-device views retain settings/faces/sounds/IR/firmware behavior without acting on other devices. Provide add/remove/connect/pause/default actions and host short-name editing. Host labels appear on both dashboard and stick; these are actual controls, not just documented protocol fields.

S3 and Plus2 share the architecture; features follow reported capabilities (USB mic, IR receiver, speaker/buzzer, etc.), not assumed board parity. Target an initial acceptance matrix of two Macs and two sticks, including mixed hardware. Connection limits are bounded and reported; don't claim larger limits without evidence. Select firmware by canonical board/chip/layout, including existing spelling aliases; refuse unknown/mismatched firmware before writes. Release bundle tooling must safely identify images per board; when no compatible bundle exists, explain that rather than offering S3 firmware for Plus2.

### Validation and delivery

Use native firmware tests, Rust fake transports/OS/audio, Python/shared-JS compatibility tests; build S3, Plus2 and unchanged XIAO as appropriate. Cover mixed RX chunks, simultaneous host/device IDs, spoofed/unpaired host, targeted HID/audio/read isolation, stale sessions/replies, dropped/slow connections, held-key switching, duplicate approvals, per-host passwords, USB wrong-host routing, resource contention, wrong-board updates and old/new combinations. Check flash <=85% and bounded memory. Run workspace tests/clippy, pytest/ruff/JS and firmware builds for touched areas. Real two-Mac/two-device connection/audio/sleep/wake/OTA tests remain documented pending, with needs-hardware label. Include UI preview evidence or a screenshot checklist. No live app/serial/Bluetooth/Keychain/user-home effects in automated work.

## Affected Memory

- `link/mac-link`: (modify) simultaneous authenticated peers and device sessions.
- `link/protocol-compatibility`: (modify) handshake/capability gates and legacy mode.
- `link/firmware-update`: (modify) owner/device and board-specific update selection.
- `link/wireless-mic`: (modify) input target, contention and USB association.
- `apps/mac-apps`: (modify) device manager, scoped APIs, host labels, legacy compatibility.
- `firmware/stick-firmware`: (modify) peer state, host picker, budgets, pairing migration.

## Impact

PROTOCOL.md; m5-stick-s3/src/{hid,buddy,settings,main,menu,ui,mic,ota} and pure logic/tests; vendored HID library; desktop/crates/stick-link/{transport,corebt,link,fake}; stickybot-server/{lib,api,config,talk,approvals,mcp,firmware,flasher,releases,wmic,home,setup}; shared dashboard/static modules; applicable Python compatibility paths; release helpers; tests, memory, CHANGELOG.md. Keep real hardware and release working tree untouched.

## Open Questions

No product decisions block implementation: simultaneous connections and draft-only delivery were approved. Engineering limits must be measured and represented honestly; hardware behavior cannot be certified by fake tests. Record practical choices in plan.md and retain the full approved scope rather than silently reducing it to a scaffold.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Simultaneous host connections, one selected input host | User approved the presented architecture | S:100 R:80 A:90 D:100 |
| 2 | Certain | Multiple Rust device sessions, Python legacy compatibility | Approved plan explicitly names this scope | S:100 R:85 A:90 D:95 |
| 3 | Certain | Detect full Mac name and allow custom short label on both surfaces | Explicit new user requirement | S:100 R:95 A:95 D:100 |
| 4 | Certain | Short labels up to 12 display characters, full name up to 80 | Small screen and bounded protocol; adjustable without identity change | S:75 R:90 A:85 D:65 |
| 5 | Certain | Initial acceptance two Macs and two S3/Plus2 devices; XIAO stays HID-only | Approved hardware scope | S:95 R:85 A:90 D:95 |
| 6 | Certain | Stop after reviewed/hydrated open draft PR; hardware checks pending | User requested draft; constitution requires separate hardware evidence | S:100 R:95 A:95 D:100 |
| 7 | Certain | Preserve legacy pairing and require explicit shared-mode migration | Approved pairing migration and compatibility requirement | S:95 R:80 A:90 D:95 |

7 assumptions (7 certain, 0 confident, 0 tentative, 0 unresolved).
