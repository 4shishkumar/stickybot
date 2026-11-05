# One entry point for the whole repo. `make help` lists the targets.
#
# The Mac agent runs in its own virtualenv (~/.stick-agent/venv, see mac-agent/README.md);
# the firmware uses PlatformIO in ~/.platformio/penv. Override with PY=... / PIO=... if yours differ.

PY  ?= $(HOME)/.stick-agent/venv/bin/python
PIO ?= $(HOME)/.platformio/penv/bin/pio
RUFF = $(PY) -m ruff

.PHONY: help test check lint format agent-test agent-test-all firmware-test firmware-build dev-setup shellcheck install-e2e \
        release release-dry-run

help:
	@echo "make test            agent tests + firmware native tests"
	@echo "make check           lint + all tests (what CI runs)"
	@echo "make lint            ruff on mac-agent"
	@echo "make agent-test      mac-agent pytest (fast; no Bluetooth, network or real Claude)"
	@echo "make agent-test-all  ... including slow tests (real speech model)"
	@echo "make firmware-test   M5StickS3 unit tests on the Mac (PlatformIO native)"
	@echo "make firmware-build  build the M5StickS3 firmware"
	@echo "make dev-setup       install the agent's dev tools and the pre-commit hook"
	@echo "make release VERSION=1.8.0   bump + date the changelog + commit + tag, locally (never pushes)"

test: agent-test firmware-test

check: lint test

lint:
	cd mac-agent && $(RUFF) check .
	$(RUFF) check --config mac-agent/pyproject.toml tools/release_tool.py m5-stick-s3/tools/make_release.py
	@if command -v shellcheck >/dev/null; then shellcheck -s sh install.sh tools/release.sh; else echo "(shellcheck not installed — skipping the shell scripts)"; fi

# Formatting is configured but the existing code keeps its hand-aligned style;
# use this on files you're writing, e.g.  make format FILES=mac-agent/stick_agent/new.py
format:
	$(RUFF) format $(FILES)

agent-test:
	cd mac-agent && $(PY) -m pytest

agent-test-all:
	cd mac-agent && $(PY) -m pytest -m "slow or not slow"

shellcheck:  ## lint the shell scripts (brew install shellcheck)
	shellcheck -s sh install.sh tools/release.sh

install-e2e:  ## run install.sh for real into a throwaway HOME (slow, needs the network)
	cd mac-agent && $(PY) -m pytest -m slow tests/test_install_e2e.py

firmware-test:
	$(MAKE) -C m5-stick-s3 test PIO=$(PIO)

firmware-build:
	$(MAKE) -C m5-stick-s3 build PIO=$(PIO)

dev-setup:
	$(PY) -m pip install -e "mac-agent[dev]"
	$(PY) -m pre_commit install

# Releases (docs/RELEASING.md): local commit + tag only; push the tag to publish
release:
	@test -n "$(VERSION)" || { echo "usage: make release VERSION=1.8.0 (or 1.8.0-beta.1)"; exit 2; }
	tools/release.sh $(VERSION)

release-dry-run:
	@test -n "$(VERSION)" || { echo "usage: make release-dry-run VERSION=1.8.0"; exit 2; }
	tools/release.sh --dry-run $(VERSION)
