"""The smart Wispr mic: who Wispr hears through BlackHole (the stick, a real mic, nothing),
the fallback mic choice, permission silence, the CoreAudio monitor and the agent / API side.
Fake sounddevice + fake CoreAudio only: no real audio device or CoreAudio call, ever."""
import logging

import numpy as np
import pytest
from conftest import BLACKHOLE, MACBOOK_MIC

from stick_agent import adpcm, audio_procs, config, mic_router, server
from stick_agent.audio_procs import RecordingMonitor
from stick_agent.mic_router import PERMISSION, STICK_FIRST_S, STICK_GRACE_S, SmartMic, choose
from stick_agent.wispr_mic import WisprMic

WISPR = (4242, "Wispr Flow Helper")
AIRPODS = {"name": "AirPods Pro", "max_output_channels": 0, "max_input_channels": 1, "default_samplerate": 24000.0}
HEADSET = {"name": "USB Headset", "max_output_channels": 2, "max_input_channels": 1, "default_samplerate": 48000.0}


class Clock:
    def __init__(self):
        self.t = 1000.0

    def __call__(self):
        return self.t


def frames(seconds):
    n = int(16000 * seconds)
    return adpcm.encode((8000 * np.sin(2 * np.pi * 440 * np.arange(n) / 16000)).astype(np.int16).tolist())


@pytest.fixture
def rig(no_real_audio, no_core_audio):
    """A WisprMic + SmartMic on the fakes, run synchronously (no threads)."""
    no_real_audio.devices = [BLACKHOLE, MACBOOK_MIC, HEADSET]

    class Rig:
        sd, ca, clock = no_real_audio, no_core_audio, Clock()
        stick_ready = False

        def make(self, choice="default", enabled=True):
            self.wispr = WisprMic()
            self.wispr.set_enabled(enabled)
            self.smart = SmartMic(self.wispr, clock=self.clock, stick_ready=lambda: self.stick_ready)
            self.smart.configure(enabled, choice)
            self.smart.start(threaded=False)
            return self.smart

        def record(self, on=True):
            self.ca.users = {120: [WISPR]} if on else {}
            self.smart.monitor.poll()

        def later(self, s):
            self.clock.t += s
            self.smart.reconcile()

        @property
        def out(self):
            return self.sd.streams[0]

        @property
        def mic(self):
            return self.sd.inputs[-1]

        def hears(self):
            return self.wispr.status()["hears"]

    return Rig()


def speak(stream, value=0.25, n=480):
    stream.feed(np.full(n, value, dtype=np.float32))


# --- routing ------------------------------------------------------------------------------------

def test_keyboard_dictation_forwards_the_default_mic(rig):
    rig.make()
    assert rig.out.started and rig.sd.inputs == [] and rig.hears() is None  # output ready, no mic yet
    rig.record()
    assert len(rig.sd.inputs) == 1 and rig.mic.started
    assert rig.mic.kw["device"] == 1 and rig.mic.kw["channels"] == 1  # MacBook Pro Microphone
    assert rig.hears() == "MacBook Pro Microphone" and rig.wispr.status()["recorder"] == "Wispr Flow Helper"
    for _ in range(10):
        speak(rig.mic)
    out = np.concatenate([rig.out.play(480) for _ in range(10)])
    assert np.isclose(out, 0.25).sum() > 2000 and np.array_equal(out[:, 0], out[:, 1])
    rig.clock.t += 2.5
    rig.record(False)
    assert rig.mic.closed and rig.hears() is None and rig.smart.mic is None
    last = rig.wispr.status()["session"]
    assert last["who"] == "Wispr Flow Helper" and last["mic"] == "MacBook Pro Microphone"
    assert last["sources"] == {"MacBook Pro Microphone": 2.5} and last["open_ms"] is not None


def test_the_stick_wins_and_a_stick_dictation_never_opens_the_mic(rig):
    rig.stick_ready = True
    rig.make()
    rig.record()                                  # Wispr starts (the stick's key)...
    assert rig.sd.inputs == [] and rig.hears() == ""
    rig.wispr.start(16000)                        # ...and its stream shows up within STICK_FIRST_S
    assert rig.hears() == "stick"
    for f in frames(0.3):
        rig.wispr.add(f)
    rig.later(0.3)
    rig.wispr.stop()
    rig.later(0.2)                                # Wispr still recording for a moment after the key-up
    rig.record(False)
    assert rig.sd.inputs == []                    # the real mic was never touched
    assert rig.wispr.status()["session"]["sources"].get("stick", 0) > 0


def test_stick_mid_dictation_takes_over_then_hands_back(rig):
    rig.make()
    rig.record()
    mic = rig.mic
    for _ in range(10):
        speak(mic, 0.25)
    rig.wispr.start(16000)                        # A pressed during a keyboard dictation
    assert rig.hears() == "stick" and not mic.closed  # the mic stays open (Wispr still records), muted
    for _ in range(5):
        speak(mic, 0.25)
    for f in frames(0.2):
        rig.wispr.add(f)
    out = np.concatenate([rig.out.play(480) for _ in range(20)])[:, 0]
    assert not np.isclose(out, 0.25).any()        # nothing from the mic while the stick plays
    assert np.abs(out).max() > 0.1
    rig.wispr.stop()
    rig.later(STICK_GRACE_S / 2)
    assert rig.hears() == "stick"                 # the grace: Wispr may be about to stop
    rig.later(STICK_GRACE_S)
    assert rig.hears() == "MacBook Pro Microphone" and len(rig.sd.inputs) == 1
    for _ in range(20):
        rig.out.play(480)                         # the stick's tail plays out
    for _ in range(10):
        speak(mic, 0.5)
    out = np.concatenate([rig.out.play(480) for _ in range(5)])[:, 0]
    assert np.isclose(out, 0.5).any()


def test_stick_stream_ending_while_wispr_records_falls_back(rig):
    rig.stick_ready = True
    rig.make()
    rig.wispr.start(16000)
    rig.record()
    rig.wispr.stop()
    rig.later(0.1)
    assert rig.sd.inputs == []
    rig.later(STICK_GRACE_S)                      # Wispr is still recording: the real mic
    assert len(rig.sd.inputs) == 1 and rig.hears() == "MacBook Pro Microphone"


def test_keyboard_dictation_waits_briefly_for_the_stick_when_it_could_stream(rig):
    rig.stick_ready = True
    rig.make()
    rig.record()
    assert rig.sd.inputs == []
    rig.later(STICK_FIRST_S + 0.01)               # no stick stream: it's the keyboard
    assert len(rig.sd.inputs) == 1


def test_stopping_closes_the_mic_promptly(rig):
    rig.make()
    rig.record()
    assert not rig.mic.closed
    rig.record(False)
    assert rig.mic.closed and rig.wispr.fallback is None
    rig.later(5)
    assert len(rig.sd.inputs) == 1                # not reopened


def test_off_means_only_the_stick(rig):
    rig.make("off")
    assert rig.smart.monitor is None and not rig.smart.on
    assert rig.wispr.status()["smart"] == "off"
    rig.smart.reconcile()
    assert rig.sd.inputs == [] and rig.sd.streams == []  # no output either until the stick connects


def test_wireless_mic_off_never_touches_audio(rig):
    rig.make(enabled=False)
    rig.ca.users = {120: [WISPR]}
    rig.smart.reconcile()
    assert rig.sd.inputs == [] and rig.sd.streams == [] and rig.smart.monitor is None


def test_unsupported_macos_degrades(rig, monkeypatch):
    def old_macos():
        raise audio_procs.Unsupported("needs macOS 14.2 or later (per-process audio objects)")
    monkeypatch.setattr(audio_procs, "CoreAudio", old_macos)
    rig.make()
    st = rig.wispr.status()
    assert st["smart"] == "unsupported" and "14.2" in st["smart_error"] and not rig.smart.on
    assert rig.sd.inputs == []


def test_our_own_process_doesnt_count(rig):
    import os
    rig.make()
    rig.ca.users = {120: [(os.getpid(), "python3.12")]}
    rig.smart.monitor.poll()
    assert not rig.smart.recording and rig.sd.inputs == []


# --- the fallback choice ---------------------------------------------------------------------------

def test_blackhole_is_never_a_fallback(rig):
    inputs = mic_router.input_devices(rig.sd)
    assert [d["name"] for d in inputs] == ["MacBook Pro Microphone", "USB Headset"]
    dev, _ = choose("default", inputs, "BlackHole 2ch", ["MacBook Pro Microphone"])
    assert dev["name"] == "MacBook Pro Microphone"  # default is BlackHole: the built-in mic
    dev, _ = choose("BlackHole 2ch", inputs, "BlackHole 2ch", [])
    assert dev["name"] == "MacBook Pro Microphone"  # by name too (guessed without CoreAudio's help)


def test_default_follows_system_settings_at_each_dictation(rig):
    rig.make()
    rig.ca.default = "USB Headset"
    rig.record()
    assert rig.mic.kw["device"] == 2 and rig.hears() == "USB Headset"
    rig.record(False)
    rig.ca.default = "MacBook Pro Microphone"
    rig.record()
    assert rig.mic.kw["device"] == 1


def test_a_mic_by_name_and_when_its_gone(rig):
    rig.make("USB Headset")
    rig.record()
    assert rig.hears() == "USB Headset"
    rig.record(False)
    rig.sd.devices = [BLACKHOLE, MACBOOK_MIC]  # unplugged
    rig.record()
    assert rig.hears() == "MacBook Pro Microphone"


def test_a_new_device_needs_a_portaudio_restart(rig):
    rig.make()
    rig.sd.pending = [AIRPODS]                # connected after the agent started
    rig.ca.names[200] = "AirPods Pro"
    rig.ca.default = "AirPods Pro"
    rig.record()
    assert rig.sd.restarts == 1 and rig.hears() == "AirPods Pro"
    assert rig.mic.kw["samplerate"] == 24000   # opened at its own rate...
    for _ in range(10):
        speak(rig.mic, 0.25, n=240)
    out = np.concatenate([rig.out.play(480) for _ in range(8)])[:, 0]
    assert np.isclose(out, 0.25, atol=0.02).sum() > 1000  # ...resampled to BlackHole's 48 kHz


def test_devices_changing_reloads_portaudio_while_idle(rig):
    rig.make()
    rig.smart.monitor.poll()                  # the devices as they were at the start
    rig.ca.names[200] = "AirPods Pro"
    rig.smart.monitor._ids_at = -1e9
    rig.smart.monitor.poll()
    assert rig.sd.restarts == 1 and rig.out is not rig.sd.streams[-1] and rig.sd.streams[-1].started


# --- permission ---------------------------------------------------------------------------------------

def test_silence_from_the_mic_means_no_permission(rig, caplog):
    caplog.set_level(logging.WARNING, logger="wispr")
    rig.make()
    rig.record()
    speak(rig.mic, 0.0)
    rig.later(0.5)
    assert rig.smart.mic_error == ""
    speak(rig.mic, 0.0)
    rig.later(0.6)
    err = rig.wispr.status()["mic_error"]
    assert PERMISSION in err and "Privacy & Security → Microphone" in err
    rig.record(False)
    rig.record()                                   # again: still no permission
    rig.later(1.1)
    assert sum(PERMISSION in r.message for r in caplog.records) == 1  # logged once
    speak(rig.mic, 0.1)                            # allowed now
    rig.later(0.1)
    assert rig.smart.mic_error == ""


def test_portaudio_error_says_how_to_allow_the_mic(rig):
    rig.make()
    rig.sd.fail_input = OSError("Internal PortAudio error [PaErrorCode -9986]")
    rig.record()
    st = rig.wispr.status()
    assert rig.smart.mic is None and "PaErrorCode" in st["mic_error"] and PERMISSION in st["mic_error"]
    assert st["hears"] == ""                       # Wispr records, nothing to give it
    rig.sd.fail_input = None
    rig.later(0.5)
    assert rig.smart.mic is None                   # not hammering it
    rig.later(mic_router.RETRY_S)
    assert rig.smart.mic is not None


# --- the monitor --------------------------------------------------------------------------------------

def test_monitor_reports_starts_stops_and_who(no_core_audio):
    seen, devs = [], []
    m = RecordingMonitor(on_change=lambda r, w: seen.append((r, w)), on_devices=lambda: devs.append(1),
                         backend=no_core_audio, own_pid=1)
    m.poll()
    assert seen == [] and m.dev == 120
    no_core_audio.users = {120: [WISPR], 115: [(77, "Zoom")]}  # Zoom on the real mic doesn't count
    m.poll()
    m.poll()
    no_core_audio.users = {}
    m.poll()
    assert seen == [(True, "Wispr Flow Helper"), (False, "")]
    no_core_audio.names[300] = "AirPods Pro"
    m._ids_at = -1e9
    m.poll()
    assert devs == [1]


def test_monitor_without_blackhole_reports_nothing(no_core_audio):
    del no_core_audio.names[120]
    m = RecordingMonitor(backend=no_core_audio)
    no_core_audio.users = {115: [WISPR]}
    m.poll()
    assert m.dev is None and not m.recording


# --- the agent + API ----------------------------------------------------------------------------------

def test_config_default_follows_the_mac(home):
    assert config.load()["wispr_fallback_mic"] == "default"


async def test_agent_status_and_disconnect_keeps_the_output(agent, no_real_audio, no_core_audio):
    no_real_audio.devices = [BLACKHOLE, MACBOOK_MIC]
    agent.update_config({"wireless_mic": True})
    agent.smart.start(threaded=False)
    st = agent.status()["wireless_mic"]
    assert st["smart"] == "on" and st["fallback"] == "default" and st["hears"] is None
    agent.link.connected = False
    agent._on_link("searching")
    assert not no_real_audio.streams[0].closed     # keyboard dictation still works without the stick
    no_core_audio.users = {120: [WISPR]}
    agent.smart.monitor.poll()
    assert agent.status()["wireless_mic"]["hears"] == "MacBook Pro Microphone"
    agent.update_config({"wispr_fallback_mic": "off"})
    assert no_real_audio.inputs[0].closed and agent.status()["wireless_mic"]["smart"] == "off"


async def test_api_lists_inputs_and_saves_the_choice(agent, aiohttp_client, no_real_audio):
    no_real_audio.devices = [BLACKHOLE, MACBOOK_MIC, HEADSET]
    c = await aiohttp_client(server.make_app(agent))
    host = {"Host": f"localhost:{config.PORT}"}
    r = await c.get("/api/audio/inputs", headers=host)
    assert await r.json() == {"devices": ["MacBook Pro Microphone", "USB Headset"], "default": "MacBook Pro Microphone"}
    r = await c.put("/api/config", headers={**host, "X-Stick-Token": config.token()},
                    json={"wispr_fallback_mic": "USB Headset"})
    assert (await r.json())["wispr_fallback_mic"] == "USB Headset" and agent.smart.choice == "USB Headset"


def test_threads_route_a_dictation(no_real_audio, no_core_audio):
    """The real wiring: monitor thread + router thread (fakes underneath)."""
    import time
    no_real_audio.devices = [BLACKHOLE, MACBOOK_MIC]
    w = WisprMic()
    w.set_enabled(True)
    smart = SmartMic(w)
    smart.configure(True, "default")
    smart.start()

    def wait(cond):
        end = time.monotonic() + 3
        while not cond() and time.monotonic() < end:
            time.sleep(0.01)
        return cond()
    try:
        assert wait(lambda: no_real_audio.streams and no_real_audio.streams[0].started)
        no_core_audio.users = {120: [WISPR]}
        assert wait(lambda: smart.hears == "MacBook Pro Microphone")
        no_core_audio.users = {}
        assert wait(lambda: no_real_audio.inputs[0].closed and smart.hears is None)
    finally:
        smart.stop()
    assert not smart.monitor._thread


def test_menu_bar_line():
    from stick_agent.menubar import wispr_line
    assert wispr_line({"enabled": False, "hears": "stick"}) is None
    assert wispr_line({"enabled": True, "hears": None}) is None
    assert wispr_line({"enabled": True, "streaming": True}) == "Wispr hears: the stick"
    assert wispr_line({"enabled": True, "hears": "AirPods Pro"}) == "Wispr hears: AirPods Pro"
    assert wispr_line({"enabled": True, "hears": "", "wispr_recording": True}) == "Wispr hears: nothing"
    assert "Microphone access" in wispr_line({"enabled": True, "mic_error": f"No sound. {PERMISSION}"})
