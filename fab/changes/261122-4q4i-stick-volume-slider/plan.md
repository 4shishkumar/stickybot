# Plan: Stick volume slider

**Change**: 261122-4q4i-stick-volume-slider  
**Intake**: [Intake](intake.md)

> Fab adopted the initial implementation. During review, the apply stage fixed gesture reconciliation and listener cleanup.

## Requirements

The menu bar and desktop dashboard let the user choose any whole volume percentage. The speaker icon shows the audible or muted state, and its accessible label names the action. Mute retains the exact device level and unmute restores it. Reports from the stick update the controls. Writes during a drag are bounded and simultaneous requests must receive their own acknowledgements. A device change must keep a pending operation from reaching the wrong stick.

## Tasks

- [x] Implement the controls, per-device request handling and regression coverage.
- [x] Reconcile hardware reports at gesture end and prevent obsolete Sounds renders from subscribing after a device switch. Test stationary drags, failed writes and overlapping renders.
- [x] Replace text mute controls in both native Mac menus and both dashboard volume controls with speaker icons and state-specific tooltips and accessibility labels.

## Acceptance

- [x] Verify volume and mute using native AppKit controls and the simulated stick, including concurrent dashboard and menu writes.
- [x] Preserve compatibility with the existing sound protocol. A physical slider test remains a manual check on the draft.
