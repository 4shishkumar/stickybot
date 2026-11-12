---
type: memory
description: "Getting a change into main: a branch and a pull request, the CI jobs that run per touched part (firmware, agent, mac-app, secrets, changelog, workflows) summed up as CI passed, one approval from someone else, squash merge, labels."
---
# CI and Review

**Domain**: delivery

## Overview

`main` is protected by a ruleset. The flow, the ruleset's settings and the labels are in
`docs/CONTRIBUTING.md`; the jobs are `.github/workflows/ci.yml`; what a reviewer is asked is
`.github/pull_request_template.md`.

## Requirements

### Requirement: Every change goes through a pull request
A change reaches `main` only through a pull request with the **CI passed** check green, one
approval from someone other than its author given after the last push, and every review comment
resolved. The merge is a squash, the PR title is the commit on `main`, and the branch is deleted
after the merge. Force pushes to `main` and deleting it are blocked.

### Requirement: CI runs the jobs for the parts a PR touches
A `changes` job decides which jobs run; **CI passed** collects them and is the one required check.

| Job | Runs when the PR touches | Checks |
|---|---|---|
| firmware | `m5-stick-s3/`, `xiao-esp32-c3/`, `PROTOCOL.md` | native tests with gcc, the stick build, flash use at most 85 %, the XIAO build |
| agent | `mac-agent/`, `tools/` | ruff, pytest, the dashboard's JS tests under Node |
| mac-app | `desktop/`, `mac-agent/stick_agent/static/` | `cargo test --workspace`, clippy with `-D warnings`, a test build of StickyBot.app as an artifact |
| secrets | always | gitleaks over the PR's commits |
| changelog | always | `CHANGELOG.md` changed, unless the PR has the `no-changelog` label |
| workflows | `.github/workflows/` | actionlint |

A change to `ci.yml` itself runs every job.

#### Scenario: A tooling-only pull request
- **GIVEN** a PR that changes only docs, CI or tooling
- **WHEN** it carries the `no-changelog` label
- **THEN** the changelog job passes without a `CHANGELOG.md` line

### Requirement: Hardware checks are a person's job
CI builds the firmware and never flashes it. A PR that changes firmware or what the stick shows,
plays or receives carries `needs-hardware` until someone has tried it on a real stick and written
what they saw under "How it was tested".

### Requirement: Nothing secret gets in
The secrets job scans every commit of the PR. Tests that need something key-shaped build it from
pieces. Real keys live in the Keychain, the git-ignored `m5-stick-s3/secrets.ini` or Actions
secrets; CI copies `secrets.example.ini` to build.

### Requirement: Branches and labels follow the listed names
Branches are `feat/<topic>`, `fix/<topic>`, `chore/<topic>` or `release/vX.Y.Z`, one topic each.
Labels: `stick`, `mac-app`, `dashboard`, `firmware`, `release`, `bug`, `feature`, `no-changelog`,
`needs-hardware`. Reviews are requested from `.github/CODEOWNERS`.

## Design Decisions

### One summary check instead of many required checks
**Decision**: The ruleset requires only **CI passed**, a job that depends on all the others and treats a skipped job as fine.
**Why**: Jobs are skipped when their part is untouched, and a required check that never runs would block the merge forever.
**Rejected**: Requiring each job by name.
*Introduced by*: predates fab; see `.github/workflows/ci.yml`

### The firmware's tests run with gcc in CI
**Decision**: The native firmware tests run on Linux with gcc.
**Why**: gcc is the stick's compiler family, and behaviour that clang on a Mac hides shows up there before a release build does.
**Rejected**: Running them only on macOS runners.
*Introduced by*: predates fab; see `.github/workflows/ci.yml`

## Sources

`docs/CONTRIBUTING.md`, `.github/workflows/ci.yml`, `.github/pull_request_template.md`,
`.github/CODEOWNERS`. Related: [releases](/delivery/releases.md), [stick firmware](/firmware/stick-firmware.md).
