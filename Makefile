# One entry point for the whole repo. `make help` lists the targets.
#
# The Mac agent runs in its own virtualenv (~/.stick-agent/venv, see mac-agent/README.md);
# the firmware uses PlatformIO in ~/.platformio/penv. Override with PY=... / PIO=... if yours differ.

PY  ?= $(HOME)/.stick-agent/venv/bin/python
PIO ?= $(HOME)/.platformio/penv/bin/pio
RUFF = $(PY) -m ruff

.PHONY: help test check lint format agent-test agent-test-all firmware-test firmware-build dev-setup

help:
	@echo "make test            agent tests + firmware native tests"
	@echo "make check           lint + all tests (what CI runs)"
	@echo "make lint            ruff on mac-agent"
	@echo "make agent-test      mac-agent pytest (fast; no Bluetooth, network or real Claude)"
	@echo "make agent-test-all  ... including slow tests (real speech model)"
	@echo "make firmware-test   M5StickS3 unit tests on the Mac (PlatformIO native)"
	@echo "make firmware-build  build the M5StickS3 firmware"
	@echo "make dev-setup       install the agent's dev tools and the pre-commit hook"

test: agent-test firmware-test

check: lint test

lint:
	cd mac-agent && $(RUFF) check .

# Formatting is configured but the existing code keeps its hand-aligned style;
# use this on files you're writing, e.g.  make format FILES=mac-agent/stick_agent/new.py
format:
	$(RUFF) format $(FILES)

agent-test:
	cd mac-agent && $(PY) -m pytest

agent-test-all:
	cd mac-agent && $(PY) -m pytest -m "slow or not slow"

firmware-test:
	$(MAKE) -C m5-stick-s3 test PIO=$(PIO)

firmware-build:
	$(MAKE) -C m5-stick-s3 build PIO=$(PIO)

dev-setup:
	$(PY) -m pip install -e "mac-agent[dev]"
	$(PY) -m pre_commit install
