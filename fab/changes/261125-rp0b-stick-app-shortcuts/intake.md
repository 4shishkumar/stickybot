# Stick app shortcuts

## Intent
Bring the user's favourite stick apps to the front of the Launcher and let their assistant open a named stick tile. Gesture shortcuts to stick and Mac apps already ship in the parent Buttons change.

## Requirements
- Save favourites on each stick from its dashboard Launcher editor. Preserve configured visibility and relative order within each group. Factory favourites are empty. Opening the grid should make favourites readily accessible.
- Expose stick_open(app) for the eight canonical built-in tile IDs. Use the tool broker, Assistant profile and profile-derived local tools. Open the tile on the bound device on its current Bluetooth connection. Report disconnected, unsupported, busy and invalid arguments. No user approval is needed for stick navigation.
- Opening a tile from an agent must not run a Mac command, send a remote control command, change settings, select a face or start an unlock. Air mouse must not emit HID movement simply because an agent opened it. Protect modal prompts, host selection and active recordings.
- Respect earlier peers through capabilities, bound input and protocol replies. Update control and protocol docs and test both boards, Rust, dashboard and legacy Python.
- Coordinate with Claude's agent track. Keep mcp::call() intact. The new eval belongs on top of the agent-evals PR when that PR is merged.

## Affected Memory
- firmware/stick-firmware and apps/mac-apps.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Reuse the existing Launcher tile identities and #7 gesture paths | Owner's approved plan | S:99 R:95 A:99 D:99 |
| 2 | Confident | Partition visible tiles into favourites then others, retaining order | Makes favourites predictable and reversible | S:90 R:95 A:95 D:92 |
| 3 | Certain | Stack this feature on the unmerged Buttons PR | Depends on its gesture dispatch | S:99 R:90 A:98 D:99 |
