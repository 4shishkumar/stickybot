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
# StickyBot.app (app.py): never ~/Applications. Tests that build one set their own path.
os.environ["STICK_AGENT_APP"] = os.path.join(tempfile.mkdtemp(prefix="stickybot-app-test-"), "StickyBot.app")

import pytest  # noqa: E402

from stick_agent import clips, config, events, github, keychain, releases, stick_mic_device, wispr_mic  # noqa: E402


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

    def query_devices(self, kind=None):
        devs = [dict(d, index=i) for i, d in enumerate(self.devices)]
        if kind == "output":  # PortAudio's default output: the first one marked "default", else the first output
            outs = [d for d in devs if d.get("max_output_channels", 0) > 0]
            pick = next((d for d in outs if d.get("default")), outs[0] if outs else None)
            if pick is None:
                raise ValueError("no default output device")
            return pick
        return devs

    def OutputStream(self, **kw):  # noqa: N802 — sounddevice's name
        if self.fail_open:
            raise self.fail_open
        s = FakeStream(**kw)
        self.streams.append(s)
        return s


BLACKHOLE = {"name": "BlackHole 2ch", "max_output_channels": 2, "max_input_channels": 2, "default_samplerate": 48000.0}
SPEAKERS = {"name": "MacBook Pro Speakers", "max_output_channels": 2, "max_input_channels": 0,
            "default_samplerate": 48000.0, "default": True}


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
        # process taps (macOS 14.2+)
        self.tap_list = {}        # tap id -> (uuid, name, process)
        self.tap_calls = []       # ("create", name, process) / ("destroy", id)
        self.aggregates = {}      # device id -> the description it was made with
        self.fail_tap = 0         # OSStatus for create_tap
        self.tap_error = None     # an exception create_tap raises (e.g. Unavailable: unknown selector)
        self.output_uid = "BuiltInSpeaker"
        self.transports = {120: "virt", 115: "bltn", 108: "bltn"}  # device id -> kAudioDeviceTransportType
        self.outputs = {120, 108}  # devices with output streams
        self.process = 77        # the agent's process object
        self.tap_hears = True     # measure_input: the tone comes through (the permission is granted)
        self.measured = []        # (dev, secs, amp)

    def devices(self):
        return [(d, name, uid) for d, (name, uid) in self.list.items()]

    def create_aggregate(self, name, uid, sub_uid, tap_uid=None):
        self.created.append((name, uid, sub_uid) if tap_uid is None else (name, uid, sub_uid, tap_uid))
        if self.fail_create:
            return self.fail_create, 0
        self._next += 1
        self.list[self._next] = (name, uid)
        self.aggregates[self._next] = {"name": name, "uid": uid, "private": 0, "master": sub_uid,
                                       "subdevices": [{"uid": sub_uid}],
                                       **({"taps": [{"uid": tap_uid, "drift": 1}], "tapautostart": 1}
                                          if tap_uid else {})}
        return 0, self._next

    def process_object(self, pid):
        return self.process

    def default_output_uid(self):
        return self.output_uid

    def transport_type(self, dev):
        return self.transports.get(dev, "grup" if dev in self.aggregates else "")

    def has_output(self, dev):
        return dev in self.outputs or dev in self.aggregates

    def create_tap(self, name, process):
        self.tap_calls.append(("create", name, process))
        if self.tap_error:
            raise self.tap_error
        if self.fail_tap:
            return self.fail_tap, 0, ""
        self._next += 1
        uuid = f"TAP-UUID-{self._next}"
        self.tap_list[self._next] = (uuid, name, process)
        return 0, self._next, uuid

    def destroy_tap(self, tap):
        self.tap_calls.append(("destroy", tap))
        self.tap_list.pop(tap, None)
        return 0

    def taps(self):
        return [(t, uuid, name) for t, (uuid, name, _p) in self.tap_list.items()]

    def measure_input(self, dev, secs, amp, freq=440.0):
        self.measured.append((dev, secs, amp))
        if self.tap_hears:
            return {"callbacks": 50, "rms": amp / 2 ** 0.5, "first_ms": 70.0, "ms": 130.0}
        return {"callbacks": 140, "rms": 0.0, "first_ms": None, "ms": secs * 1000}

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
def no_tap_by_default(monkeypatch):
    """Tests run as on a Mac without process taps (the BlackHole carrier) unless they turn them on
    (the `tap_mac` fixture in test_tap_mic.py), whatever Mac runs them."""
    monkeypatch.setattr(stick_mic_device, "macos_supports_tap", lambda: False)


@pytest.fixture(autouse=True)
def no_real_github(monkeypatch):
    """Tests never reach GitHub (gh or HTTPS): tests/test_releases.py fakes both."""
    def refuse(*a, **kw):
        raise AssertionError(f"a test reached for GitHub: {a[:1]}")
    monkeypatch.setattr(releases, "run", refuse)
    monkeypatch.setattr(releases, "urlopen", refuse)
    monkeypatch.setattr(releases, "gh_path", lambda: None)


@pytest.fixture(autouse=True)
def no_real_self_update(monkeypatch):
    """...never run git / uv for a self-update, and never restart the test process."""
    from stick_agent import core, selfupdate

    def refuse(*a, **kw):
        raise AssertionError(f"a test ran a real self-update command: {a}")
    monkeypatch.setattr(selfupdate, "git_out", refuse)
    monkeypatch.setattr(selfupdate, "run_cmd", refuse)
    monkeypatch.setattr(core.Agent, "restart_now", refuse)


@pytest.fixture(autouse=True)
def no_real_keychain(monkeypatch):
    """Tests never read or write the real Keychain: keys live in memory (keychain.MemoryBackend)."""
    def refuse(*a, **kw):
        raise AssertionError("a test reached for the real Keychain")
    monkeypatch.setattr(keychain.MacKeychain, "_load", refuse)
    monkeypatch.delenv("STICK_AGENT_OPENROUTER_KEY", raising=False)
    fake = keychain.MemoryBackend()
    keychain.set_backend(fake)
    yield fake
    keychain.set_backend(None)


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

    async def bye(self):
        return await self.send({"src": "agent", "bye": True})

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
