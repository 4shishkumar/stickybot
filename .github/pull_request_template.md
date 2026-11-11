<!-- Title: what it does, in a few words ("Events: a sound per event"). One topic per PR.
     A face? Use the face template instead: add ?template=face.md to the "Open a pull request" URL. -->

## What and why

<!-- What changes for the person using StickyBot, and why. Link the feedback ID (SB-XXXXXX) or issue. -->

## How it was tested

- [ ] CI is green (firmware, Mac app, dashboard, secrets, changelog)
- [ ] On a real stick — firmware changes: flashed and tried (`fw x.y.z` on the stick)
- [ ] In the real app — Mac app / dashboard changes: built and clicked through

<!-- What you tried on the device, and what you saw. -->

## Screenshots

<!-- Dashboard pages, stick screens (photo or the face_shots renderer), before / after. -->

## Checklist

- [ ] A line under `[Unreleased]` in `CHANGELOG.md` (or the `no-changelog` label: docs, CI, tests only)
- [ ] `PROTOCOL.md` updated if a message between the Mac and the stick changed
- [ ] No keys, tokens, passwords or personal data (names, repos, emails) in code, tests or logs
- [ ] Older firmware / older apps still work, or the change says what they need
