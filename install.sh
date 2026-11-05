#!/bin/sh
# StickyBot installer — the Mac app for the M5StickS3 StickyBot.
#
# One line:
#   curl -fsSL https://raw.githubusercontent.com/ashish-kumar-noon/stickybot/main/install.sh | sh
# or from a clone:
#   ./install.sh
#
# What it does: checks this is an Apple Silicon Mac, installs uv (a standalone
# Python manager — no Homebrew or system Python needed) if it's missing, makes
# ~/.stick-agent/venv with Python 3.14, installs StickyBot into it, builds
# ~/Applications/StickyBot.app around that Python (so macOS shows "StickyBot", not
# "Python"), starts it (the login item), then opens the setup page in the StickyBot
# window (http://localhost:8660/#install), which walks through the rest one step at a time:
# speech model, Claude Code, approvals, GitHub alerts, the wireless mic, your stick.
# Reopen that page any time: `stickybot install-page`, or the menu bar → Finish setup….
#
# Without a browser — an SSH session, nobody logged in at the screen, `open`
# failing, --no-browser, or any of the setup options below — it hands over to
# `stickybot setup` in the terminal instead, as before. Scripts and AI agents
# use --yes / --with / --without: those never open a browser.
#
# Safe to run again: it updates the code, restarts the agent so the new code
# runs, and opens the setup page at the first step that isn't done.
#
# Code: a fresh install clones the repo into ~/.stick-agent/src and checks out the
# latest stable release tag (main until the first release). Re-running moves a
# release install to the newest stable release after it, and a branch checkout
# (installs from before releases) does `git pull --ff-only`. A clone whose origin
# still has the repo's old name (ashish-kumar-noon/stick-key) gets the new one.
# The dashboard can do the same update in one click (Settings → Updates).
#
# Options (passed on to `stickybot setup`; any of them means: terminal, no browser):
#   --yes                 no questions: install the defaults
#   --with a,b            also install these   (core model claude github firmware wireless_mic mcp)
#   --without a,b         skip these
#   --dry-run             say what would happen, change nothing
#   --uninstall [a,b|all] remove parts again   (--purge also deletes ~/.stick-agent)
#   --no-login-item       don't add the login item (testing)
# Own options:
#   --src DIR             use this checkout instead of cloning / the script's folder
#   --no-browser          set up in the terminal (`stickybot setup`), not on the setup page
#
# Environment: STICK_AGENT_HOME (default ~/.stick-agent), STICK_AGENT_PORT, STICK_AGENT_LABEL;
# for tests: STICK_AGENT_GUI=1|0 (is someone at the screen? default: detected),
# STICK_AGENT_WAIT_S (how long to wait for the agent to answer, default 30).

set -eu

REPO="ashish-kumar-noon/stickybot"
AGENT_HOME="${STICK_AGENT_HOME:-$HOME/.stick-agent}"
VENV="$AGENT_HOME/venv"
LOG="$AGENT_HOME/install.log"
PYTHON_VERSION="3.14"

say() { printf '%s\n' "$*"; printf '%s %s\n' "$(date '+%H:%M:%S')" "$*" >>"$LOG"; }
die() { say "error: $*"; exit 1; }
step() { printf '\n\033[1m%s\033[0m\n' "$*"; printf '%s == %s\n' "$(date '+%H:%M:%S')" "$*" >>"$LOG"; }
run() { "$@" >>"$LOG" 2>&1 || die "failed: $* (details in $LOG)"; }

mkdir -p "$AGENT_HOME"
: >>"$LOG"
# --help: explain and stop before doing anything
for arg in "$@"; do
  case "$arg" in
    -h|--help)
      cat <<'USAGE'
install.sh — install (or update) StickyBot, the stick's Mac app, on this Mac

Usage: ./install.sh [options]          (or: curl -fsSL <raw install.sh url> | sh -s -- [options])

  --yes                 no questions: install the default components
  --with a,b            also install these components (model, claude, github, firmware)
  --without a,b         skip these components
  --dry-run             show what would happen, change nothing
  --uninstall [all|a,b] remove the app (or just some components); --purge also deletes ~/.stick-agent
  --src PATH            use this checkout instead of cloning the repo
  --no-browser          set up in the terminal instead of on the setup page
  -h, --help            this help

It installs uv (if missing) into ~/.local/bin, Python + StickyBot into ~/.stick-agent,
builds ~/Applications/StickyBot.app, starts it, then opens the setup page
(http://localhost:8660/#install) that walks through the rest. Reopen it with `stickybot install-page`. With any of the options above
(or over SSH, or with --no-browser) it runs `stickybot setup` in the terminal instead.
Re-running it updates.
USAGE
      exit 0 ;;
  esac
done

say "StickyBot installer — log: $LOG"

# --- our own options; the rest goes to `stickybot setup` -------------------------
SRC=""
DRY_RUN=0
UNINSTALL=0
MODE=browser   # the setup page; "terminal" = `stickybot setup` here, as before
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1; MODE=terminal ;;
    --uninstall) UNINSTALL=1; MODE=terminal ;;
    --no-browser) MODE=terminal ;;
    # scripts and AI agents: exactly today's non-interactive behaviour, never a browser
    -y|--yes|--with|--with=*|--without|--without=*|--only|--only=*|--status|--json|--purge) MODE=terminal ;;
    --no-login-item) MODE=terminal ;;   # no background agent to show a page
  esac
done
set_src_next=0
setup_args=""
for arg in "$@"; do
  if [ "$set_src_next" = 1 ]; then SRC="$arg"; set_src_next=0; continue; fi
  case "$arg" in
    --src) set_src_next=1 ;;
    --src=*) SRC="${arg#--src=}" ;;
    --no-browser) ;;
    *) setup_args="$setup_args $(printf '%s' "$arg" | sed "s/'/'\\\\''/g; s/^/'/; s/\$/'/")" ;;
  esac
done

# --- 1. this Mac ------------------------------------------------------------------
step "Checking this Mac"
[ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS"
[ "$(uname -m)" = "arm64" ] || die "an Apple Silicon Mac is needed (the speech model runs on its GPU)"
say "macOS $(sw_vers -productVersion 2>/dev/null || echo '?') on Apple Silicon ✓"

if [ "$UNINSTALL" = 1 ]; then
  [ -x "$VENV/bin/stick-agent" ] || die "StickyBot isn't installed in $VENV"
  eval "set -- $setup_args"
  exec "$VENV/bin/stick-agent" setup "$@"
fi

# --- 2. uv ------------------------------------------------------------------------
step "Python manager (uv)"
UV="$(command -v uv 2>/dev/null || true)"
if [ -z "$UV" ] && [ -x "$HOME/.local/bin/uv" ]; then UV="$HOME/.local/bin/uv"; fi
if [ -z "$UV" ]; then
  if [ "$DRY_RUN" = 1 ]; then
    say "would install uv into ~/.local/bin (astral.sh installer)"
  else
    say "installing uv into ~/.local/bin…"
    curl -LsSf https://astral.sh/uv/install.sh | env UV_NO_MODIFY_PATH=1 sh >>"$LOG" 2>&1 \
      || die "couldn't install uv (details in $LOG)"
    UV="$HOME/.local/bin/uv"
  fi
fi
[ -z "$UV" ] || say "uv: $UV ($("$UV" --version 2>/dev/null || echo '?'))"

# --- 3. the code ------------------------------------------------------------------
step "Getting StickyBot"
here="$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)"
if [ -z "$SRC" ] && [ -n "$here" ] && [ -f "$here/mac-agent/pyproject.toml" ]; then
  SRC="$here"   # run from a clone: use it as is (editable install)
fi
# The newest stable release tag (vX.Y.Z, no "-beta") in $SRC; with "contains", only one that
# includes the current commit (so a beta install isn't moved back to an older stable release).
latest_release() {
  if [ "${1:-}" = contains ]; then
    git -C "$SRC" tag --list 'v*' --contains HEAD --sort=-v:refname | grep -v -- - | head -n 1
  else
    git -C "$SRC" tag --list 'v*' --sort=-v:refname | grep -v -- - | head -n 1
  fi
}
if [ -z "$SRC" ]; then
  SRC="$AGENT_HOME/src"
  fresh=0
  if [ -d "$SRC/.git" ]; then
    if [ "$DRY_RUN" = 1 ]; then say "would update $SRC (git fetch, then the latest release or git pull)"; else
      say "updating $SRC…"
      origin="$(git -C "$SRC" remote get-url origin 2>/dev/null || true)"
      case "$origin" in   # the repo was renamed: follow it (GitHub redirects the old name, not forever)
        *github.com[:/]ashish-kumar-noon/stick-key|*github.com[:/]ashish-kumar-noon/stick-key.git|*github.com[:/]ashish-kumar-noon/stick-key/)
          new_origin="$(printf '%s' "$origin" | sed 's#ashish-kumar-noon/stick-key#ashish-kumar-noon/stickybot#')"
          run git -C "$SRC" remote set-url origin "$new_origin"
          say "origin is now $new_origin (the repo was renamed)" ;;
      esac
      run git -C "$SRC" fetch --tags --force --quiet origin
      if git -C "$SRC" symbolic-ref -q HEAD >/dev/null; then
        run git -C "$SRC" pull --ff-only   # on a branch (installs from before releases): follow it
      else
        tag="$(latest_release contains)"   # on a release tag: the newest stable release after it
        [ -z "$tag" ] || run git -C "$SRC" -c advice.detachedHead=false checkout --quiet "$tag"
      fi
    fi
  elif [ "$DRY_RUN" = 1 ]; then
    say "would clone $REPO into $SRC (the latest release, or main before the first one)"
  elif command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    say "cloning $REPO with gh…"
    run gh repo clone "$REPO" "$SRC"
    fresh=1
  else
    say "cloning $REPO over HTTPS…"
    run git clone "https://github.com/$REPO.git" "$SRC"
    fresh=1
  fi
  if [ "$fresh" = 1 ]; then
    tag="$(latest_release)"
    if [ -n "$tag" ]; then
      run git -C "$SRC" -c advice.detachedHead=false checkout --quiet "$tag"
      say "release $tag"
    else
      say "no release yet — using main"
    fi
  fi
fi
[ "$DRY_RUN" = 1 ] || [ -f "$SRC/mac-agent/pyproject.toml" ] || die "no mac-agent/ in $SRC"
say "code: $SRC"

# --- 4. Python env + install -------------------------------------------------------
step "Installing into $VENV"
if [ "$DRY_RUN" = 1 ]; then
  say "would create $VENV (Python $PYTHON_VERSION) and install $SRC/mac-agent into it"
else
  if [ ! -x "$VENV/bin/python" ]; then
    say "creating the Python $PYTHON_VERSION environment…"
    run "$UV" venv --python "$PYTHON_VERSION" "$VENV"
  fi
  say "installing StickyBot and its libraries (first time: a minute or two)…"
  run "$UV" pip install --python "$VENV/bin/python" -e "$SRC/mac-agent"
  say "StickyBot $("$VENV/bin/python" -c 'import stick_agent; print(stick_agent.__version__)') ✓"
fi

# --- 5. hand over: the setup page (or the terminal) -------------------------------
# Someone at this Mac's screen? Not over SSH, `open` exists, and this user owns the console.
gui_session() {
  case "${STICK_AGENT_GUI:-}" in 1) return 0 ;; 0) return 1 ;; esac
  [ -z "${SSH_CONNECTION:-}" ] && [ -z "${SSH_TTY:-}" ] || return 1
  command -v open >/dev/null 2>&1 || return 1
  [ "$(stat -f %Su /dev/console 2>/dev/null)" = "$(id -un)" ]
}
if [ "$MODE" = browser ] && ! gui_session; then
  say "no browser here (SSH, or nobody at the screen) — setting up in the terminal"
  MODE=terminal
fi

if [ "$MODE" = browser ]; then
  PORT="${STICK_AGENT_PORT:-8660}"
  URL="http://localhost:$PORT/#install"
  step "Starting StickyBot"
  : >"$AGENT_HOME/installer-opens-page"   # the agent's first start then doesn't open a second tab
  # core = the `stickybot` command, StickyBot.app + the login item; (re)starting it makes updated code run
  if "$VENV/bin/stick-agent" setup --yes --only core >>"$LOG" 2>&1; then
    say "waiting for it to answer…"
    waited=0
    up=0
    while [ "$waited" -lt "${STICK_AGENT_WAIT_S:-30}" ]; do
      if curl -fsS -o /dev/null --max-time 2 "http://127.0.0.1:$PORT/api/status" 2>/dev/null; then up=1; break; fi
      sleep 1
      waited=$((waited + 1))
    done
    # the StickyBot window if the app shows one, else the browser (install-page does both)
    if [ "$up" = 1 ] && "$VENV/bin/stick-agent" install-page >>"$LOG" 2>&1; then
      printf '\n\033[1mContinue on the setup page → %s\033[0m\n' "$URL"
      say "(reopen it any time: stickybot install-page, or the menu bar icon → Finish setup…)"
      exit 0
    fi
    if [ "$up" = 1 ]; then say "couldn't open the setup page — setting up in the terminal instead"
    else say "the agent didn't answer on port $PORT (details: $AGENT_HOME/launchd.out) — setting up in the terminal instead"; fi
  else
    say "couldn't start the agent (details in $LOG) — setting up in the terminal instead"
  fi
  rm -f "$AGENT_HOME/installer-opens-page"
fi

step "Setting up"
if [ "$DRY_RUN" = 1 ] && [ ! -x "$VENV/bin/stick-agent" ]; then
  say "would run: stickybot setup$setup_args"
  say "(dry run — nothing changed)"
  exit 0
fi
eval "set -- $setup_args"
if [ ! -t 0 ] && (: </dev/tty) 2>/dev/null; then
  exec "$VENV/bin/stick-agent" setup "$@" </dev/tty   # piped from curl: questions go to the terminal
fi
exec "$VENV/bin/stick-agent" setup "$@"
