# Intake: Stick volume slider

**Change**: 261122-4q4i-stick-volume-slider  
**Created**: 2026-11-23

## Origin

Adopted from the existing stick-volume-slider branch. The user asked for a proper volume slider and a mute button in the menu bar and desktop app. After seeing the behavior and test results, the user approved pushing it as a draft PR and requested the Fab pipeline.

## Why

The menu presets force a choice between five positions. Users need intermediate levels and a way to silence the stick temporarily without losing the level they chose.

## What Changes

The macOS menu and the dashboard's Sounds and Stick pages accept whole percentages and expose a mute button. The stick's on/off flag preserves the exact hardware volume across a mute cycle. Both apps serialize sound requests because the device replies without request IDs. Rapid slider changes converge on the last selected value. The Rust menu addresses the default stick, and the dashboard uses the selected device route.

## Affected Memory

- `apps/mac-apps` (modify) — menu, dashboard and volume behavior.

## Impact

Affects the Tauri and legacy Python menus, shared dashboard and sound request handling. Uses the existing 0–255 device volume and on/off protocol.

## Open Questions

None.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Offer 0–100% and a separate mute button | User requested both surfaces and approved the described behavior | S:95 R:90 A:95 D:95 |
| 2 | Certain | Preserve the device's exact prior volume during mute | Matches the demonstrated and approved control behavior | S:95 R:95 A:95 D:95 |

2 assumptions (2 certain, 0 confident, 0 tentative, 0 unresolved).
