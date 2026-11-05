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

from stick_agent import audio_procs, clips, config, events, github, wispr_mic  # noqa: E402


class FakeStream:
    """A sounddevice.OutputStream that plays nothing: tests pull blocks with play(n)."""

    def __init__(self, **kw):
        self.kw = kw
        self.started = self.closed = False

    def start(self):
        self.started = True

    def stop(self):
        self.started = False

    def close(self):
        self.closed = True

    def play(self, frames):
        import numpy as np
        out = np.ones((frames, self.kw["channels"]), dtype=np.float32)  # ones: catches unwritten samples
        self.kw["callback"](out, frames, None, None)
        return out


class FakeInputStream(FakeStream):
    """A sounddevice.InputStream that records nothing: tests hand it blocks with feed(x)."""

    def feed(self, x):
        import numpy as np
        x = np.asarray(x, dtype=np.float32).reshape(-1, 1)
        self.kw["callback"](x, len(x), None, None)


class FakeSoundDevice:
    """Stands in for the sounddevice module (no PortAudio, no real audio devices).
    `pending` devices only show up after a PortAudio restart (_terminate + _initialize)."""

    def __init__(self, devices=()):
        self.devices = list(devices)
        self.pending = []
        self.streams = []
        self.inputs = []
        self.fail_open = None
        self.fail_input = None
        self.restarts = 0

    def query_devices(self):
        return [dict(d, index=i) for i, d in enumerate(self.devices)]

    def _terminate(self):
        self.restarts += 1

    def _initialize(self):
        self.devices += self.pending
        self.pending = []

    def OutputStream(self, **kw):  # noqa: N802 — sounddevice's name
        if self.fail_open:
            raise self.fail_open
        s = FakeStream(**kw)
        self.streams.append(s)
        return s

    def InputStream(self, **kw):  # noqa: N802
        if self.fail_input:
            raise self.fail_input
        s = FakeInputStream(**kw)
        self.inputs.append(s)
        return s


class FakeCoreAudio:
    """Stands in for audio_procs.CoreAudio: who records from which device, the default input."""

    def __init__(self):
        self.names = {120: "BlackHole 2ch", 115: "MacBook Pro Microphone", 108: "MacBook Pro Speakers"}
        self.users = {}           # device id -> [(pid, name)]
        self.default = "MacBook Pro Microphone"
        self.builtin = ["MacBook Pro Microphone"]

    def device_ids(self):
        return tuple(self.names)

    def devices(self):
        return dict(self.names)

    def default_input_name(self):
        return self.default

    def builtin_input_names(self):
        return list(self.builtin)

    def input_users(self, dev):
        return list(self.users.get(dev, []))


BLACKHOLE = {"name": "BlackHole 2ch", "max_output_channels": 2, "max_input_channels": 2, "default_samplerate": 48000.0}


@pytest.fixture(autouse=True)
def no_real_audio(monkeypatch):
    """Tests never open a real audio device: the wireless mic sees no devices unless a test adds one."""
    fake = FakeSoundDevice()
    monkeypatch.setattr(wispr_mic, "_sounddevice", lambda: fake)
    return fake


MACBOOK_MIC = {"name": "MacBook Pro Microphone", "max_output_channels": 0, "max_input_channels": 1,
               "default_samplerate": 48000.0}


@pytest.fixture(autouse=True)
def no_core_audio(monkeypatch):
    """...and never ask the real CoreAudio who is recording: a fake with nobody recording."""
    fake = FakeCoreAudio()
    monkeypatch.setattr(audio_procs, "CoreAudio", lambda: fake)
    return fake


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

    async def ask(self, text, on_progress, on_text=None):
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
