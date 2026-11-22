# Mac keyboard push-to-talk

## Origin

The user wants to talk to StickyBot with a global Mac key combination as well as the stick. They requested a plan and screen preview before implementation.

## What the user will see

In Settings → Speech & replies, enable **Talk from Mac keyboard** and choose a shortcut. The proposed default is **Control + Shift + Space**, with an editable binding and conflict feedback. Ask for microphone access when the user enables the feature. Display the system input device name. Use existing speech and brain choices, including local Goose when selected.

Hold the keys in any app. A small floating panel appears above the Dock on the active display, with the existing StickyBot Mac app icon, a live input meter and “Listening”. Keep the current app focused. Releasing the shortcut stops the microphone immediately. Keep that same panel and logo visible through transcription, thinking, any tool or approval wait, and every partial reply. Show the recognized question when available. Show “Reply complete” only when the agent finishes and leave the full answer available until dismissed or a new turn starts. A reply delivered in one piece can go directly from thinking to complete. Long replies can open the dashboard. Escape cancels a recording. Hold B continues to use the stick microphone.

**Design acceptance.** Verify the actual bundled StickyBot icon stays in the panel header at every stage, including errors. Test a slow turn with a long gap between reply chunks and another app in focus. Verify the panel remains on screen throughout, the mic meter stops on release, later chunks update the same panel, and the finished reply stays readable. A failed turn leaves a visible error in the same panel.

## Implementation plan

Use a 46 pt app icon beside a 13 pt status heading throughout the popup. A gentle teal glow and a 1.8-second pulse indicate an active turn. With Reduce Motion enabled, use a steady glow. Keep the first post-release heading “Listening” and change the detail to “Microphone off · processing your words”. Continue the pulse through Thinking and Answering, then stop the pulse and glow when the reply is complete.

1. Add Mac-only dashboard settings for opt-in and shortcut capture. Register only the configured chord; handle press, release, repeats and conflicts. Preserve the last working binding if a replacement fails. Verify registration and focus on the minimum supported macOS and on the target Mac. Test modifier release order and VoiceOver navigation.
2. Capture the system input only for a deliberate hold, downmix and resample to the existing 16 kHz speech input. Bound each recording, discard on Escape, sleep, screen lock, input loss or shutdown. Fail clearly on denied microphone permission or a missing input. Add the microphone usage description to the signed app and verify the actual TCC prompt.
3. Factor the common transcribe/ask/reply path in stickybot-server/src/talk.rs. Keep stick transport generation checks on the stick route. Add a Mac-origin turn ID and pin any target device when the turn starts. Handle two inputs competing for the same agent with one owner; the second input sees a busy message. Only the owning turn can send, cancel or receive its answer. Keep existing brain and tool policies. Support ordinary Mac questions with no connected stick; where device-scoped local MCP requires it, introduce a Mac-scoped read-only context. A tool requiring stick approval must still receive that exact approval or explain it needs the stick.
4. Render listening, transcribing, thinking, approval-needed, answer and recoverable errors in a non-activating native panel. Show live mic level and reduced-motion feedback. Preserve full Mac reply text; keep existing formatting and routing for stick-origin turns. In v1 replies are visual by default; respect the existing read-aloud preference when enabled and route a Mac-origin spoken reply to the Mac.
5. Verify with fake mic/brain, native shortcut down/up, wrong-device/stale approvals, device switch mid-turn, overlapping inputs, offline stick, denied mic, silence, long hold and cancellation. On a real Mac check focus over another app/full screen, brief answer and long answer, actual local Goose STT→reply, and hold B with the same agent. Run workspace tests, Clippy and affected dashboard tests; ship a draft PR through Fab after approval.

## Affected Memory

- apps/mac-apps — desktop shortcut and overlay

## Scope and open questions

Proposed v1 is hold-to-talk. A hands-free toggle or text insertion into other apps would be a separate decision. Confirm the suggested chord and panel placement during design review. No firmware change is planned.

## Assumptions

| # | Grade | Decision | Rationale | Scores |
|---|-------|----------|-----------|--------|
| 1 | Certain | Preserve stick talk and reuse the selected agent | Explicit user goal | S:99 R:90 A:95 D:98 |
| 2 | Confident | Start with a configurable hold-to-talk chord | Mirrors hold B and the requested experience | S:85 R:95 A:90 D:80 |
| 3 | Confident | Show a compact bottom-center panel that expands for the reply | Keeps the active app usable; preview supplied for approval | S:80 R:95 A:88 D:80 |
