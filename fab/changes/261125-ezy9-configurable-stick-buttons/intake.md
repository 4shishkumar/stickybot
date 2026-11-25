# Configurable stick buttons

## Intent
Let a user assign the four global stick gestures from the dashboard for the selected stick, including swapping double and triple B. Preserve factory mappings and modal controls.

## Requirements
- Save all four mappings together on the device. Advertise the supported mappings in device state, hide editing on older firmware, validate input on both sides and show the acknowledged device values.
- Defaults are double B unlock, triple B Launcher, A+B last reply, and hold B talk. Offer every Launcher tile, unlock, Launcher, last reply, Mac app by name, a saved quick command, Talk and Nothing for discrete gestures. Hold B offers Talk or Dictation. Keep press/release lifetimes and wake behavior intact.
- Bind Mac actions to an actual physical press on the connected device. Resolve configured names using existing Mac services. Preserve assistant approval and device isolation. Derive contextual hints from the effective actions.
- Update protocol, controls and changelog, exercise Rust, dashboard and native tests, build both firmware targets, and report flash size and hardware verification status.

## Affected Memory
- Firmware controls and Mac dashboard device settings.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Store device-wide bindings in NVS | Explicit owner requirement | S:99 R:95 A:99 D:99 |
| 2 | Certain | Keep default shortcuts intact | Owner wants the swap on their own stick only | S:99 R:95 A:99 D:99 |
| 3 | Confident | Preserve modal buttons and offer last reply as a selectable action | Existing behavior and reversible settings | S:90 R:95 A:90 D:95 |
