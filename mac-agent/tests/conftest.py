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

from stick_agent import clips, config, events, github, releases, stick_mic_device, wispr_mic  # noqa: E402


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


class FakeSoundDevice:
    """Stands in for the sounddevice module (no PortAudio, no real audio devices)."""

    def __init__(self, devices=()):
        self.devices = list(devices)
        self.streams = []
        self.fail_open = None

    def query_devices(self):
        return [dict(d, index=i) for i, d in enumerate(self.devices)]

    def OutputStream(self, **kw):  # noqa: N802 — sounddevice's name
        if self.fail_open:
            raise self.fail_open
        s = FakeStream(**kw)
        self.streams.append(s)
        return s


BLACKHOLE = {"name": "BlackHole 2ch", "max_output_channels": 2, "max_input_channels": 2, "default_samplerate": 48000.0}


@pytest.fixture(autouse=True)
def no_real_audio(monkeypatch):
    """Tests never open a real audio device: the wireless mic sees no devices unless a test adds one."""
    fake = FakeSoundDevice()
    monkeypatch.setattr(wispr_mic, "_sounddevice", lambda: fake)
    return fake


REAL_CORE_AUDIO = stick_mic_device.CoreAudioDevices  # for the opt-in end-to-end test only


class FakeCoreAudio:
    """Stands in for stick_mic_device.CoreAudioDevices: a device list, aggregates made and
    destroyed in memory, and who reads which device. Never touches the real CoreAudio."""

    def __init__(self):
        self.list = {120: ("BlackHole 2ch", "BlackHole2ch_UID"), 115: ("MacBook Pro Microphone", "BuiltInMic"),
                     108: ("MacBook Pro Speakers", "BuiltInSpeaker")}
        self.readers = {}         # device id -> [pid]
        self.created = []         # (name, uid, sub uid)
        self.destroyed = []
        self.fail_create = 0      # OSStatus to return
        self.per_process = True   # macOS 14.2+
        self._next = 300

    def devices(self):
        return [(d, name, uid) for d, (name, uid) in self.list.items()]

    def create_aggregate(self, name, uid, sub_uid):
        self.created.append((name, uid, sub_uid))
        if self.fail_create:
            return self.fail_create, 0
        self._next += 1
        self.list[self._next] = (name, uid)
        return 0, self._next

    def destroy_aggregate(self, dev):
        self.destroyed.append(dev)
        self.list.pop(dev, None)
        return 0

    def input_users(self, dev):
        return list(self.readers.get(dev, [])) if self.per_process else None

    def ours(self, uid="com.stickkey.wireless-mic"):
        return [d for d, (_n, u) in self.list.items() if u == uid]


@pytest.fixture(autouse=True)
def no_core_audio(monkeypatch):
    """...and never create a real CoreAudio device (or remove the live agent's)."""
    fake = FakeCoreAudio()
    monkeypatch.setattr(stick_mic_device, "CoreAudioDevices", lambda: fake)
    return fake


@pytest.fixture(autouse=True)
def no_real_github(monkeypatch):
    """Tests never reach GitHub (gh or HTTPS): tests/test_releases.py fakes both."""
    def refuse(*a, **kw):
        raise AssertionError(f"a test reached for GitHub: {a[:1]}")
    monkeypatch.setattr(releases, "run", refuse)
    monkeypatch.setattr(releases, "urlopen", refuse)
    monkeypatch.setattr(releases, "gh_path", lambda: None)


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
