# Code Review

What the reviewing agent checks before a change goes to a pull request. It mirrors what a human
reviewer is asked in `.github/pull_request_template.md`, so the PR arrives ready for the one
approval `main` requires. The author's side is in `code-quality.md`.

## Severity Definitions

- **Must-fix**: a requirement or acceptance item not met; failing tests, clippy or ruff; a
  constitution rule broken (a message changed without `PROTOCOL.md`; older firmware or an older app
  would stop working; a secret, name, email or private repo name in code, tests or logs; firmware
  over the flash limit; settings or pairings at risk in an update; behaviour that differs between
  gcc and clang; a hand-bumped version); a user-visible change with no `CHANGELOG.md` line.
- **Should-fix**: logic left inside hardware-facing code without a test; a missing fallback or
  unclear words when the other side is too old; unbounded buffers or replies on the stick; error
  codes shown instead of words; comments that don't say why; a new dependency or asset not in
  `THIRD_PARTY_NOTICES.md`; a dashboard change checked in only one of the two apps.
- **Nice-to-have**: naming and wording, smaller functions, extra test cases, tidier docs.

## Review Scope

- The files the change touched, plus their neighbours when a pattern is in question.
- For a protocol change: both ends (firmware `buddy.cpp` / `protocol.cpp`, `desktop/crates/stick-link`
  and `stickybot-server`, `mac-agent/stick_agent`) and `PROTOCOL.md`, even if only one was edited.
- For a dashboard change (`mac-agent/stick_agent/static/`, `dashboard.html`): how it behaves in
  both the Rust app and the Python app.
- Skip generated and vendored files: `m5-stick-s3/src/fonts_vlw.*`, `sounds_clips.*`,
  `web_page.h`, `m5-stick-s3/lib/`, `desktop/Cargo.lock`, `desktop/target/`, `.pio/`, face
  previews and other images, `fab/` and the deployed skills.

## False Positive Policy

- Inline `<!-- review-ignore: {reason} -->` in markdown files
- Inline `// review-ignore: {reason}` or `# review-ignore: {reason}` in code files
- Suppressed findings are noted in the review report but not counted as failures
- Key-shaped strings in tests that are built from pieces (`concat!("sk-or-v1-", …)`,
  `"sk-or-v1-" + …`) are the intended pattern, not a leak.

## Rework Budget

- Max cycles: 3
- After 2 consecutive "fix code" attempts on the same issue, escalate to "revise plan" or "revise requirements"

## Parsimony Pass

- Enabled: true

## Project-Specific Review Rules

The pull request template's questions, as checks:

1. **What and why**: the change does one thing, and the summary says what changes for the person
   using StickyBot.
2. **Tests were run for every part touched** (the table in `code-quality.md`), and the output is
   quoted or summarised. Tests that could not be run are named, not passed over.
3. **Real stick**: a change to firmware, or to what the stick shows, plays or receives, lists what
   to try on a real stick (and the `fw x.y.z` it applies to). The agent cannot do that check: the
   review records it as open, and the PR carries `needs-hardware` until a person has done it.
4. **Real app and screenshots**: a change to a dashboard page, the menu or a stick screen includes
   screenshots or says which to take (dashboard pages; stick screens by photo or the `face_shots`
   renderer; before / after). A UI change with neither is a should-fix.
5. **Changelog**: a line under `[Unreleased]` in the right group (Added / Changed / Fixed /
   Removed), written for the user, or a stated reason for `no-changelog` (docs, CI, tests,
   tooling only).
6. **PROTOCOL.md**: updated for any message, field, command or characteristic that changed,
   including how it is detected and what older firmware / apps do.
7. **Compatibility**: older firmware with this app, and this firmware with older apps, still
   work; unknown messages are ignored; features are gated on `@state` / hello flags.
8. **No secrets or personal data** in code, tests, fixtures, logs, docs or screenshots.
9. **Budgets** (firmware): flash use from the build's `Flash:` line is quoted and ≤ 85 %; nothing
   unbounded was added to a reply or a buffer; frame rate and wake-ups are no worse for battery.
10. **Versions untouched**, except `FW_VERSION` when the firmware's behaviour changed.
11. **Licences**: anything third-party added is compatible with MIT and recorded.

The review never approves or merges the pull request: that is a person's job, and not the
author's.
