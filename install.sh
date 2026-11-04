#!/bin/sh
# stick-agent installer — the companion app for the M5StickS3 "stick-key".
#
# One line (once the setup site is up):
#   curl -fsSL https://<site>/install.sh | sh
# Today (private repo), from a clone:
#   ./install.sh
# or without cloning first:
#   gh api repos/ashish-kumar-noon/stick-key/contents/install.sh -H "Accept: application/vnd.github.raw" | sh
#
# What it does: checks this is an Apple Silicon Mac, installs uv (a standalone
# Python manager — no Homebrew or system Python needed) if it's missing, makes
# ~/.stick-agent/venv with Python 3.14, installs stick-agent into it, then hands
# over to `stick-agent setup`, which asks what else to install (speech model,
# Claude Code hooks, GitHub alerts, flashing a stick). Safe to run again: it
# updates the code and only changes what's missing.
#
# Options (passed on to `stick-agent setup`):
#   --yes                 no questions: install the defaults
#   --with a,b            also install these   (core model claude github firmware)
#   --without a,b         skip these
#   --dry-run             say what would happen, change nothing
#   --uninstall [a,b|all] remove parts again   (--purge also deletes ~/.stick-agent)
#   --no-login-item       don't add the login item (testing)
# Own options:
#   --src DIR             use this checkout instead of cloning / the script's folder
#
# Environment: STICK_AGENT_HOME (default ~/.stick-agent), STICK_AGENT_PORT, STICK_AGENT_LABEL.

set -eu

REPO="ashish-kumar-noon/stick-key"
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
install.sh — install (or update) the stick-agent companion app on this Mac

Usage: ./install.sh [options]          (or: curl -fsSL <url>/install.sh | sh -s -- [options])

  --yes                 no questions: install the default components
  --with a,b            also install these components (model, claude, github, firmware)
  --without a,b         skip these components
  --dry-run             show what would happen, change nothing
  --uninstall [all|a,b] remove the app (or just some components); --purge also deletes ~/.stick-agent
  --src PATH            use this checkout instead of cloning the repo
  -h, --help            this help

It installs uv (if missing) into ~/.local/bin, Python + stick-agent into ~/.stick-agent,
then runs `stick-agent setup` to pick the components. Re-running it updates.
USAGE
      exit 0 ;;
  esac
done

say "stick-agent installer — log: $LOG"

# --- our own options; the rest goes to `stick-agent setup` -------------------------
SRC=""
DRY_RUN=0
UNINSTALL=0
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --uninstall) UNINSTALL=1 ;;
  esac
done
set_src_next=0
setup_args=""
for arg in "$@"; do
  if [ "$set_src_next" = 1 ]; then SRC="$arg"; set_src_next=0; continue; fi
  case "$arg" in
    --src) set_src_next=1 ;;
    --src=*) SRC="${arg#--src=}" ;;
    *) setup_args="$setup_args $(printf '%s' "$arg" | sed "s/'/'\\\\''/g; s/^/'/; s/\$/'/")" ;;
  esac
done

# --- 1. this Mac ------------------------------------------------------------------
step "Checking this Mac"
[ "$(uname -s)" = "Darwin" ] || die "this installer is for macOS"
[ "$(uname -m)" = "arm64" ] || die "an Apple Silicon Mac is needed (the speech model runs on its GPU)"
say "macOS $(sw_vers -productVersion 2>/dev/null || echo '?') on Apple Silicon ✓"

if [ "$UNINSTALL" = 1 ]; then
  [ -x "$VENV/bin/stick-agent" ] || die "stick-agent isn't installed in $VENV"
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
step "Getting stick-agent"
here="$(cd "$(dirname "$0")" 2>/dev/null && pwd || true)"
if [ -z "$SRC" ] && [ -n "$here" ] && [ -f "$here/mac-agent/pyproject.toml" ]; then
  SRC="$here"   # run from a clone: use it as is (editable install)
fi
if [ -z "$SRC" ]; then
  SRC="$AGENT_HOME/src"
  if [ -d "$SRC/.git" ]; then
    if [ "$DRY_RUN" = 1 ]; then say "would update $SRC (git pull)"; else
      say "updating $SRC…"
      run git -C "$SRC" pull --ff-only
    fi
  elif [ "$DRY_RUN" = 1 ]; then
    say "would clone $REPO into $SRC"
  elif command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    say "cloning $REPO with gh…"
    run gh repo clone "$REPO" "$SRC"
  else
    say "cloning $REPO with git (needs access to the private repo)…"
    run git clone "git@github.com:$REPO.git" "$SRC"
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
  say "installing stick-agent and its libraries (first time: a minute or two)…"
  run "$UV" pip install --python "$VENV/bin/python" -e "$SRC/mac-agent"
  say "stick-agent $("$VENV/bin/python" -c 'import stick_agent; print(stick_agent.__version__)') ✓"
fi

# --- 5. hand over ------------------------------------------------------------------
step "Setting up"
if [ "$DRY_RUN" = 1 ] && [ ! -x "$VENV/bin/stick-agent" ]; then
  say "would run: stick-agent setup$setup_args"
  say "(dry run — nothing changed)"
  exit 0
fi
eval "set -- $setup_args"
if [ ! -t 0 ] && (: </dev/tty) 2>/dev/null; then
  exec "$VENV/bin/stick-agent" setup "$@" </dev/tty   # piped from curl: questions go to the terminal
fi
exec "$VENV/bin/stick-agent" setup "$@"
