# Contributing: branches, pull requests, CI and releases

`main` is protected: every change reaches it through a pull request with green CI. Nobody pushes
to `main` directly (the owner can bypass in an emergency, and says so in the PR or commit).

## The flow

1. **Branch from `main`**, one topic per branch:
   `feat/<topic>` (new behaviour), `fix/<topic>` (a bug), `chore/<topic>` (CI, tooling, docs),
   `release/vX.Y.Z` (a release PR). Agents work in their own git worktrees on such branches.
2. **Commit** as you go. The PR is squash-merged, so the branch's commits can be small and messy;
   the PR title becomes the commit on `main` — make it say what changed.
3. **Open a pull request** into `main` (`gh pr create`). Fill in the template: what and why, how it
   was tested (CI, and on a real stick / in the real app when it touches them), screenshots.
4. **CI** (`.github/workflows/ci.yml`) runs only the parts the PR touches:

   | Job | Checks |
   |---|---|
   | firmware | native tests with gcc, the stick build, flash use under 85 % |
   | agent | the Python app's ruff + pytest, the dashboard's JS tests |
   | mac-app | the Rust app's tests + clippy; a **test build** of `StickyBot.app` (Actions → the run → Artifacts) |
   | secrets | gitleaks: nothing that looks like a key or token |
   | changelog | a line under `[Unreleased]` in `CHANGELOG.md`, unless labelled `no-changelog` |
   | workflows | actionlint, when a workflow changed |

   **CI passed** sums them up; it's the check `main` requires.
5. **Review and merge**: at least **one approval** from someone other than the PR's author is
   required (GitHub asks the people in `.github/CODEOWNERS` automatically). New commits after an
   approval need a fresh one, and every review comment has to be resolved. Then **Squash and
   merge**; the branch is deleted after the merge.

   An author can't approve their own PR: a PR opened from one account is approved from another
   (the owner's, or a teammate's).

### Labels

`stick` · `mac-app` · `dashboard` · `firmware` · `release` · `bug` · `feature` · `no-changelog` ·
`needs-hardware` (wait for a test on a real stick before merging). `tools/setup_labels.sh` creates
them.

### Test fakes, not secrets

Tests that need something key-shaped build it from pieces (`concat!("sk-or-v1-", …)` in Rust,
`"sk-or-v1-" + …` in Python), so secret scanners don't flag them. Real keys live in the Keychain,
`m5-stick-s3/secrets.ini` (git-ignored) or GitHub Actions secrets — never in the repo.

## Releases

Today: on an up-to-date `main`, `tools/release.sh X.Y.Z` makes the release commit and tag locally,
and the owner pushes both (`git push origin main vX.Y.Z`); the tag runs `release.yml` (tests,
firmware, StickyBot.app, the GitHub release). With `main` protected, the release commit goes
through a PR instead (planned next: a release PR whose merge tags automatically). See
`docs/RELEASING.md`.

## Setting up the protection (owner, once)

GitHub → the repo → **Settings → Rules → Rulesets → New ruleset → New branch ruleset**:

- **Name:** `main`; **Enforcement:** Active
- **Bypass list:** add **Repository admin** (yourself, for emergencies) — optional
- **Target branches:** *Add target → Include default branch*
- Rules:
  - **Restrict deletions** and **Block force pushes**
  - **Require a pull request before merging** — *Required approvals*: **1**; tick *Dismiss stale
    pull request approvals when new commits are pushed*, *Require approval of the most recent
    reviewable push* and *Require conversation resolution before merging*; *Allowed merge
    methods*: **Squash** only
  - **Require status checks to pass** → *Add checks* → **CI passed** (it appears after the first
    CI run); tick *Require branches to be up to date before merging*
- **Create**

And under **Settings → General → Pull Requests**: allow only *squash merging*, default commit
message *Pull request title*, and tick **Automatically delete head branches**.
