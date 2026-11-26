# Buttons & gestures

## Requirements
Read the four global bindings from the selected stick, update and persist the table atomically, and execute a mapping only in the appropriate physical-input context. Keep older peers working. Mac actions may launch only a named application or an existing saved quick command.

## Wire contract
Use an optional object named `buttons` in the state reply, with string keys `b2`, `b3`, `ab`, `bh`. The command is `@buttons b2=...&b3=...&ab=...&bh=...`, using the existing URL query escaping. Its reply includes the acknowledged buttons object. Discrete choices are `unlock`, `launcher`, `reply`, `talk`, `none`, `tile:<id>`, `app:<name>`, `quick:<name>`. Hold choices are `talk` and `dictation`. Names are trimmed, bounded to 32 UTF-8 bytes, and must not contain controls, slash or backslash. Factory bindings in order are unlock, launcher, reply, talk. Send a physical input event for a Mac mapping only, with `src:"agent", evt:"button_action"`, fields `gesture` and `action` (the mapped value). Route to the connected Mac using the normal firmware event channel. No MCP or host command may synthesize such an event. Retain existing ownership and generation checks. If protocol limits require changing this contract, update all peers and tests together.

## Tasks
- [x] T001 [P] Firmware model, parser, native fixtures, NVS storage and protocol reporting.
- [x] T002 Firmware physical dispatch including talk and dictation release, contextual hints, wake and modal precedence.
- [x] T003 [P] Rust physical event consumption and fake-stick configuration, with isolated tests.
- [x] T004 [P] Dashboard capability-based Buttons & gestures page with device-bound editing, validation and tests.
- [x] T005 Document wire messages, controls, capability and Unreleased entry.
- [x] T006 Exercise dashboard, Rust and firmware gates including binary sizes; document hardware coverage.

## Acceptance
- [x] Defaults, a double/triple swap, all tiles and named actions round-trip; invalid changes leave the previous table intact.
- [x] Hints and dispatch follow the mapping, while modal controls, sleep and button release use the original input ownership.
- [x] The selected device owns its editor and events, unsupported devices hide the page, and Mac named actions use existing safe resolution.
- [x] Both board targets build and local tests exercise firmware, fake device, host events and dashboard.

## Notes
On the final firmware builds, StickS3 used 2,456,563 bytes (73.5%) and StickC Plus2 used 2,387,571 bytes (71.4%) of each 3,342,336-byte app slot. Both are below the 85% budget. Native firmware tests passed (305). Desktop workspace tests passed (755), as did Clippy, 49 dashboard tests and 877 Python tests. On macOS the legacy launcher environment test ran last to isolate its subprocess environment.

A real Chromium dashboard on the Rust fake-stick server confirmed factory values, the double/triple swap, the save acknowledgement and reload. Physical timing, visible hints and power-cycle persistence await a real-stick check before release. The PR carries the needs-hardware label.

## Deletion Candidates

No remaining candidates. Review removed the test-only global mapping argument from the firmware’s screen-action lookup.
