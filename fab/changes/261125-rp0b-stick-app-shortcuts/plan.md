# Stick app shortcuts

## Requirements
Offer favourites on capable sticks and expose approval-free, device-scoped navigation through the existing tool broker. Share tile IDs and launch logic with physical shortcuts. Keep previous firmware and app combinations usable.

## Wire contract
On capable firmware, state includes `launcherFav` (favourite visible tile letters, empty by default) and `stickOpen:1`. Keep `launcher` as the configured visible order, before favourite grouping, for existing editors. Use `@launcher tiles=<ids>&favorites=<comma-separated full IDs or ->` to save the complete layout with favourites. With tiles only, preserve existing favourites that remain visible. Reset clears favourites. Use `@stick-open app=<full tile id>` for navigation, with `{"ok":true,"app":"air"}` or a bounded error. Dashboard saves pin a current BLE device identity for favourites; unsupported firmware can still edit its original layout.

## Tasks
- [x] T001 [P] Implement favourites, persistence, ordering and protocol on both firmware targets with native tests.
- [x] T002 Implement capability-gated stick navigation, modal ownership and safe remote tile entry.
- [x] T003 [P] Register and dispatch the bounded tool in Rust and fake stick, expose through Assistant and the derived local list, exercise errors and routing. Pin favourite saves to the live BLE peer.
- [x] T004 [P] Add favourite controls in the selected stick's existing Launcher editor with dashboard tests.
- [x] T005 Add Python MCP parity, identity-aware favourite saves and protocol, controls and changelog documentation.
- [x] T006 Run desktop, dashboard and native gates, both board builds and a simulated-device integration check.

## Acceptance
- [x] Visible favourites lead the physical grid in stable order after save/reopen, and empty/default layouts retain prior behavior.
- [x] A tool call for each valid tile navigates the correct stick without a Mac or IR action and preserves protected interactions.
- [x] Older peers, bad input and a target switch cannot silently open or configure the wrong device.
- [x] Tests pass, both board binaries fit the flash budget, hardware coverage is stated.

## Verification

- Rust workspace 761 passed. Clippy with warnings denied and Ruff passed.
- Python 897 passed, 1 skipped, 5 deselected. Dashboard 57 passed. Native firmware 314 passed.
- StickS3 and StickC Plus2 built under the 85% flash budget. No physical stick was flashed.
- Chromium on the simulated Rust device saved and reloaded Air mouse as the first favorite. HTTP MCP opened Air mouse and refused an unknown app.

## Deletion Candidates

None. The extracted Launcher editor replaced the previous inline implementation.
