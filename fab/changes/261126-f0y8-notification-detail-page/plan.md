# Notification detail page

## Requirements
Implement item #6 on PR #25's optional actor/url/body. Each device requests an action only from the authenticated Mac that sent its notification, on the same current connection. Open only the exact previously delivered valid HTTP(S) URL. Read the host's stored event text. Ignore forged, stale, failed-delivery, disconnected and replayed actions. Preserve capability compatibility.

## Wire
Mac hello advertises eventActions:1. Stick sends {"src":"agent","evt":"event_action","id":"e5","action":"open","url":"https://example.org/review"} or {"src":"agent","evt":"event_action","id":"e5","action":"read"}. An open carries the exact URL; a read carries no text. The owner Mac independently verifies each request. Use bounded in-memory, expiring successful-delivery records per recipient connection; an action can be consumed once per event and kind.

## Tasks
- [x] T001 [P] Parse optional fields and implement notification detail view, A/B/hold controls, interaction priority, sender lease and tests on firmware.
- [x] T002 [P] Bind Rust host actions to exact delivered events, connection generation and destination, with regressions for rejection paths.
- [x] T003 [P] Implement legacy Python host parity, optional event fields and tests.
- [x] T004 Document firmware wire, controls, and changelog.
- [x] T005 Verify both board builds and firmware native, Rust, Python and dashboard tests.
- [x] T006 Exercise delivery-to-action and compatibility using simulated peers.

## Acceptance
- [x] Bounded optional fields render on both boards; toast, detail, read, open and close controls consume the intended physical gesture.
- [x] Only an exact delivered event can trigger the validated host action on the same connection; old agents remain compatible.
- [x] Native and host suites pass and both board builds fit. State physical hardware coverage.

## Verification

- 756 Rust tests passed. Clippy and Ruff passed. Python passed 883 tests (one skipped, five deselected); 45 dashboard tests passed.
- 303 native firmware tests passed. StickS3 and StickC Plus2 built at 73.3% and 71.2% of the app partition. Physical hardware testing remains open.
- Simulated peers exercised open and read, forgery, replay, disconnect and switching. Firmware-rendered portrait and landscape previews were inspected using complete and older notifications.

## Deletion Candidates

None. The per-connection event routes replace the former acknowledgement-only route table; that table was removed.
