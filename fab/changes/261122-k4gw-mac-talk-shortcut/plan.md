# Mac keyboard push-to-talk

## Requirements

The opt-in Mac shortcut and stick button reach the configured agent. A Mac turn uses the system microphone only during the hold. Releasing the chord stops capture immediately. The branded overlay stays over the current app through transcription, thinking, tool waits and the complete reply. The approved appearance and wording are specified in the intake.

## Tasks

- [x] Add Mac-global settings, a shortcut recorder and conflict-safe registration.
- [x] Implement press/release, bounded audio capture, permission handling and cancellation.
- [x] Route Mac and stick turns through shared speech, brain and tool policy with explicit ownership.
- [x] Build the persistent non-activating panel, including the approved active and completed states.
- [x] Integrate registration, device selection, shutdown and event delivery in the Mac app.
- [x] Exercise shortcut and UI behavior, cross-input isolation, workspace tests, Clippy and dashboard regressions.

## Acceptance

- [x] A configured shortcut starts a Mac turn in another app. Releasing it stops capture. Quitting or losing the mic releases the recording.
- [x] A second input cannot replace, answer for or cancel the active turn. Mac read-only questions work with the stick offline, and actions needing stick confirmation still require it.
- [x] The overlay keeps focus in the front app, shows the correct logo and wording, respects Reduce Motion, and stays readable until dismissal or the next turn.
- [x] Settings report registration and permission failures and preserve the last working shortcut.

## Verification

The native harness exercised Carbon registration and collision rollback, synthetic stereo input, persistence, dimensions, focus and cancellation. A signed app bundle served the opt-in settings with capture disabled. A separate synthetic spoken question passed through real Parakeet, sandboxed Goose and llama.cpp and returned a full answer with the stick disconnected. The draft PR retains manual checks for a physical microphone, macOS permission prompt, a held shortcut over another app, and hold B on hardware.
