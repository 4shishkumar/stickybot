"""Shared test setup.

The agent keeps its files in STICK_AGENT_HOME (~/.stick-agent by default). It's
read when stick_agent.config is imported, so we point it at a throwaway folder
*before* anything imports the package, and then give every test its own fresh
folder via the `home` fixture. Tests never touch the real ~/.stick-agent.
"""
import os
import tempfile

os.environ["STICK_AGENT_HOME"] = tempfile.mkdtemp(prefix="stick-agent-test-")
os.environ["STICK_AGENT_PORT"] = "18660"

import pytest  # noqa: E402

from stick_agent import clips, config, events, github  # noqa: E402


@pytest.fixture
def home(tmp_path, monkeypatch):
    """A fresh agent home folder (config, commands, token, workspace...)."""
    monkeypatch.setattr(config, "HOME", tmp_path)
    for name, file in [("CONFIG", "config.json"), ("COMMANDS", "commands.json"), ("HISTORY", "history.jsonl"),
                       ("LOG", "agent.log"), ("TOKEN", "token"), ("WORKSPACE", "workspace")]:
        monkeypatch.setattr(config, name, tmp_path / file)
    monkeypatch.setattr(events, "EVENTS_LOG", tmp_path / "events.jsonl")
    monkeypatch.setattr(github, "STATE", tmp_path / "github_state.json")
    monkeypatch.setattr(clips, "SOUNDS", tmp_path / "sounds")
    config.ensure_home()
    # Never reach the real Claude Code from a test (auto-detection would find it)
    cfg = config.load()
    cfg["claude_path"] = "/usr/bin/false"
    config.save(cfg)
    return tmp_path


class GuardBrain:
    """The default brain in tests: fails loudly if a request unexpectedly goes to Claude."""
    busy = False
    cancelled = False
    proc = None

    async def ask(self, text, on_progress):
        raise AssertionError(f"unexpected request to Claude in a test: {text!r}")

    def cancel(self):
        pass

    def forget(self):
        pass


class FakeLink:
    """Stands in for the Bluetooth link: records what the agent sends."""

    def __init__(self, connected=True):
        self.sent = []      # JSON messages
        self.audio = []     # lists of clip packets
        self.connected = connected
        self.mtu = 256
        self.paused = False
        self.target = None

    async def send(self, msg):
        if not self.connected:
            return False
        self.sent.append(msg)
        return True

    async def send_audio(self, pkts):
        if not self.connected:
            return False
        self.audio.append(list(pkts))
        return True

    def set_target(self, target):
        self.target = target

    def pause(self, paused):
        self.paused = paused

    def states(self):
        """The agent's status messages to the stick, in order (state, text)."""
        return [(m["state"], m.get("text", "")) for m in self.sent if m.get("src") == "agent" and "state" in m]


class FakeSTT:
    """Speech to text without the model: returns whatever the test sets."""

    def __init__(self, text=""):
        from concurrent.futures import ThreadPoolExecutor
        self.text = text
        self.thread = ThreadPoolExecutor(max_workers=1)
        self.calls = 0

    def transcribe(self, audio):
        self.calls += 1
        if isinstance(self.text, Exception):
            raise self.text
        return self.text


@pytest.fixture
def agent(home, monkeypatch):
    """An Agent wired to a FakeLink and FakeSTT (no Bluetooth, no model, no `say`)."""
    from stick_agent import core
    monkeypatch.setattr(core.home, "claude_sessions", lambda exclude_pid=None: 0)
    a = core.Agent()
    a.link = FakeLink()
    a.stt = FakeSTT()
    a.brain = GuardBrain()
    a.stt_ready = True
    a.link_state = "connected"
    return a
