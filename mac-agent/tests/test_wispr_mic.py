"""The wireless Wispr mic: frames in order, lost frames as silence, the jitter
buffer, resampling, the BlackHole device (a fake sounddevice), and the agent's
side of the stick messages. No real audio device is ever opened."""
import numpy as np
import pytest
from conftest import BLACKHOLE

from stick_agent import adpcm, config, wispr_mic
from stick_agent.wispr_mic import JitterBuffer, Resampler, Sequencer, WisprMic


def tone(seconds, rate=16000, amp=8000):
    n = int(rate * seconds)
    return (amp * np.sin(2 * np.pi * 440 * np.arange(n) / rate)).astype(np.int16)


def frames(seconds):
    return adpcm.encode(tone(seconds).tolist())


def chunk(seq, n=320, value=1):
    return seq, np.full(n, value, dtype=np.int16)


# --- Sequencer: frames by seq ----------------------------------------------------------

def test_in_order_frames_come_straight_out():
    s = Sequencer()
    assert [len(x) for x in s.add(*chunk(0))] == [320]
    assert [len(x) for x in s.add(*chunk(1))] == [320]
    assert s.lost == 0 and s.late == 0


def test_a_frame_a_little_late_is_put_back_in_place():
    s = Sequencer(reorder=2)
    out = s.add(*chunk(0, value=0))
    assert s.add(*chunk(2, value=2)) == []  # waiting for 1
    out += s.add(*chunk(1, value=1))
    assert [int(x[0]) for x in out] == [0, 1, 2] and s.lost == 0


def test_a_lost_frame_becomes_silence():
    s = Sequencer(reorder=2)
    out = s.add(*chunk(0, value=5))
    out += s.add(*chunk(2, value=5))
    out += s.add(*chunk(3, value=5))   # still within the reorder window
    assert len(out) == 1
    out += s.add(*chunk(4, value=5))   # 3 frames past the gap: 1 is lost
    assert [int(x[0]) for x in out] == [5, 0, 5, 5, 5]
    assert len(out[1]) == 320 and s.lost == 1
    assert s.add(*chunk(1)) == [] and s.late == 1  # too late now: dropped


def test_flush_fills_the_gaps_at_the_end():
    s = Sequencer(reorder=5)
    out = s.add(*chunk(0)) + s.add(*chunk(3))
    out += s.flush()
    assert [int(x[0]) for x in out] == [1, 0, 0, 1] and s.lost == 2


def test_seq_wraps_around():
    s = Sequencer()
    out = s.add(*chunk(65535)) + s.add(*chunk(0)) + s.add(*chunk(1))
    assert len(out) == 3 and s.lost == 0 and s.late == 0


def test_duplicates_are_dropped():
    s = Sequencer(reorder=3)
    s.add(*chunk(0))
    s.add(*chunk(2))
    assert s.add(*chunk(2)) == [] and s.late == 1


# --- jitter buffer ----------------------------------------------------------------------

def test_jitter_buffer_waits_for_the_target_then_plays():
    jb = JitterBuffer(1000, target_ms=50, max_ms=200)  # 1 kHz: 50 samples target
    jb.push(np.ones(30, dtype=np.float32))
    assert not jb.pull(10).any()          # not enough yet: silence, nothing taken
    assert jb.queued == 30
    jb.push(np.ones(30, dtype=np.float32))
    assert jb.pull(10).all() and jb.queued == 50


def test_jitter_buffer_underrun_pads_with_silence_and_waits_again():
    jb = JitterBuffer(1000, target_ms=20, max_ms=200)
    jb.push(np.ones(25, dtype=np.float32))
    out = jb.pull(40)
    assert out[:25].all() and not out[25:].any()
    assert jb.underruns == 1 and not jb.playing
    jb.push(np.ones(10, dtype=np.float32))
    assert not jb.pull(5).any()  # below the target again: waits


def test_jitter_buffer_caps_the_latency():
    jb = JitterBuffer(1000, target_ms=50, max_ms=100)
    jb.push(np.arange(150, dtype=np.float32))  # a burst (e.g. catching up after a hiccup)
    assert jb.queued == 50 and jb.dropped == 100
    assert jb.pull(1)[0] == 100  # the oldest went


def test_jitter_buffer_drains_the_tail_at_the_end():
    jb = JitterBuffer(1000, target_ms=50)
    jb.push(np.ones(10, dtype=np.float32))  # less than the target
    jb.drain()
    out = jb.pull(20)
    assert out[:10].all() and not out[10:].any()
    assert jb.underruns == 0 and not jb.draining


# --- resampling --------------------------------------------------------------------------

def test_resampler_passes_through_at_the_same_rate():
    x = np.ones(100, dtype=np.float32)
    assert np.array_equal(Resampler(16000, 16000)(x), x)


def test_resampler_16k_to_48k_keeps_the_length_and_pitch():
    r = Resampler(16000, 48000)
    x = tone(0.5).astype(np.float32) / 32768
    out = np.concatenate([r(x[i:i + 320]) for i in range(0, len(x), 320)] + [r(np.zeros(0), last=True)])
    assert abs(len(out) - 3 * len(x)) < 50
    spectrum = np.abs(np.fft.rfft(out[2000:2000 + 48000 // 4]))
    peak_hz = np.argmax(spectrum) * 48000 / (48000 // 4)
    assert abs(peak_hz - 440) < 10


# --- the device + streams (fake sounddevice) ---------------------------------------------

def test_finds_blackhole_by_name(no_real_audio):
    no_real_audio.devices = [{"name": "MacBook Pro Speakers", "max_output_channels": 2, "default_samplerate": 48000},
                             {"name": "BlackHole 2ch", "max_output_channels": 2, "default_samplerate": 44100}]
    d = wispr_mic.find_device(no_real_audio)
    assert d == {"index": 1, "name": "BlackHole 2ch", "rate": 44100, "channels": 2}
    no_real_audio.devices = [{"name": "BlackHole 2ch", "max_output_channels": 0, "default_samplerate": 48000}]
    assert wispr_mic.find_device(no_real_audio) is None  # an input-only match isn't enough


def test_missing_device_is_reported_once_and_doesnt_crash(no_real_audio):
    seen = []
    m = WisprMic(emit=lambda kind, **d: seen.append(d))
    m.set_enabled(True)
    assert not m.ready
    assert not m.start()
    m.add(frames(0.1)[0])  # ignored
    m.stop()
    assert m.status()["device"] is None and "brew install --cask blackhole-2ch" in m.status()["error"]
    errors = [d["error"] for d in seen if d.get("error")]
    assert len(errors) == 1
    m.device(force=True)  # still missing: not reported again
    assert len([d for d in seen if d.get("error")]) == 1


def test_device_appears_later(no_real_audio):
    m = WisprMic()
    m.set_enabled(True)
    assert not m.ready
    no_real_audio.devices = [BLACKHOLE]
    assert not m.ready                   # not looked at again straight away
    assert m.device(force=True) and m.ready and m.error == ""


def test_off_never_touches_audio(no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    m = WisprMic()
    m.set_enabled(False)
    assert not m.ready and not m.start()
    m.warm()
    assert no_real_audio.streams == []


def test_a_stream_plays_into_blackhole(no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    m = WisprMic()
    m.set_enabled(True)
    m.warm()
    stream = no_real_audio.streams[0]
    assert stream.started and stream.kw["samplerate"] == 48000 and stream.kw["channels"] == 2
    assert not stream.play(480).any()  # idle: silence
    assert m.start(16000) and m.status()["streaming"]
    for f in frames(0.5):
        m.add(f)
    out = np.concatenate([stream.play(480) for _ in range(50)])  # 0.5 s at 48 kHz
    assert out.shape == (24000, 2) and np.array_equal(out[:, 0], out[:, 1])
    assert np.abs(out).max() > 0.1
    m.stop()
    tail = np.concatenate([stream.play(480) for _ in range(20)])
    assert not tail[-480:].any()  # played out, then silence
    last = m.status()["last"]
    assert last["frames"] == 25 and last["lost"] == 0 and not m.status()["streaming"]
    m.set_enabled(False)
    assert stream.closed and not m.status()["open"]


def test_lost_frames_dont_shift_the_audio(no_real_audio):
    no_real_audio.devices = [{**BLACKHOLE, "default_samplerate": 16000.0}]  # same rate: no resampling
    m = WisprMic()
    m.set_enabled(True)
    m.start(16000)
    fs = frames(0.2)                     # 10 frames
    for f in fs[:4] + fs[5:]:            # frame 4 lost
        m.add(f)
    m.stop()
    stream = no_real_audio.streams[0]
    out = np.concatenate([stream.play(320) for _ in range(12)])[:, 0]
    start = np.flatnonzero(out)[0]
    assert not out[start + 4 * 320 + 10:start + 5 * 320 - 10].any()  # the lost 20 ms is silence
    assert out[start + 5 * 320 + 10:start + 6 * 320].any()           # and frame 5 is where it belongs
    assert m.last["lost"] == 1


def test_open_failure_is_reported(no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    no_real_audio.fail_open = OSError("device busy")
    m = WisprMic()
    m.set_enabled(True)
    assert not m.start()
    assert "device busy" in m.error and m.dev is None


# --- tap mode: which output (stick_mic_device.choose_tap_output picks it) ----------------------

BUDS = {"name": "OnePlus Buds 3", "max_output_channels": 2, "default_samplerate": 44100.0, "default": True}
SPEAKERS = {"name": "MacBook Pro Speakers", "max_output_channels": 2, "default_samplerate": 48000.0}
TO_SPEAKERS = {"uid": "BuiltInSpeaker", "name": "MacBook Pro Speakers", "redirected": True,
               "why": "default output is Bluetooth (OnePlus Buds 3): playing into MacBook Pro Speakers"}


def tap_mic(output):
    m = WisprMic()
    m.set_carrier("tap", gate=lambda: True, output=output)
    m.set_enabled(True)
    return m


def test_tap_with_a_bluetooth_default_plays_into_the_built_in_speakers(no_real_audio, caplog):
    no_real_audio.devices = [BUDS, SPEAKERS]
    m = tap_mic(lambda: TO_SPEAKERS)
    with caplog.at_level("INFO", logger="wispr"):
        assert m.start(16000)
    assert no_real_audio.streams[0].kw["device"] == 1 and m.dev["name"] == "MacBook Pro Speakers"
    assert "wireless mic: default output is Bluetooth (OnePlus Buds 3): playing into MacBook Pro Speakers" \
        in caplog.text


def test_tap_with_a_built_in_default_is_unchanged(no_real_audio):
    no_real_audio.devices = [BUDS | {"default": False}, SPEAKERS | {"default": True}]
    pick = {"uid": "BuiltInSpeaker", "name": "MacBook Pro Speakers", "redirected": False, "why": ""}
    m = tap_mic(lambda: pick)
    assert m.start(16000) and no_real_audio.streams[0].kw["device"] == 1
    no_real_audio.devices = [BUDS, SPEAKERS]  # no pick at all (no mic device): PortAudio's default, as before
    m2 = tap_mic(None)
    assert m2.start(16000) and no_real_audio.streams[-1].kw["device"] == 0


def test_tap_with_no_other_output_keeps_the_default_and_says_why(no_real_audio, caplog):
    no_real_audio.devices = [BUDS]
    keep = {"uid": "Buds-UID", "name": "OnePlus Buds 3", "redirected": False,
            "why": "default output is Bluetooth (OnePlus Buds 3) and there's no other wired output: playing into it "
                   "anyway (it may lag or fail)"}
    m = tap_mic(lambda: keep)
    with caplog.at_level("WARNING", logger="wispr"):
        assert m.start(16000)
    assert m.dev["name"] == "OnePlus Buds 3" and "no other wired output" in caplog.text


def test_tap_output_portaudio_doesnt_list_falls_back_to_the_default(no_real_audio, caplog):
    no_real_audio.devices = [BUDS]
    m = tap_mic(lambda: TO_SPEAKERS)
    with caplog.at_level("WARNING", logger="wispr"):
        assert m.start(16000)
    assert m.dev["name"] == "OnePlus Buds 3" and "doesn't list MacBook Pro Speakers" in caplog.text
    m.stop()
    m.release()
    assert m.device() is m.dev  # not looked up again every time while nothing changed


def test_tap_output_follows_the_default_between_holds(no_real_audio):
    no_real_audio.devices = [BUDS | {"default": False}, SPEAKERS | {"default": True}]
    pick = {"v": {"uid": "BuiltInSpeaker", "name": "MacBook Pro Speakers", "redirected": False, "why": ""}}
    m = tap_mic(lambda: pick["v"])
    assert m.start(16000) and m.dev["name"] == "MacBook Pro Speakers"
    m.stop()
    m.release()
    pick["v"] = TO_SPEAKERS  # the Buds became the default: the speakers, now on purpose
    no_real_audio.devices = [BUDS, SPEAKERS]
    assert m.start(16000) and m.dev["name"] == "MacBook Pro Speakers" and m._wanted == "MacBook Pro Speakers"


def test_find_device_prefers_the_exact_name(no_real_audio):
    no_real_audio.devices = [{"name": "MacBook Pro Speakers (2)", "max_output_channels": 2},
                             {"name": "MacBook Pro Speakers", "max_output_channels": 2}]
    assert wispr_mic.find_device(no_real_audio, "MacBook Pro Speakers")["index"] == 1


# --- the agent ------------------------------------------------------------------------------

@pytest.fixture
async def wired(agent, no_real_audio):
    no_real_audio.devices = [BLACKHOLE]
    agent.update_config({"wireless_mic": True})
    return agent


def test_config_default_is_off(home):
    assert config.load()["wireless_mic"] is False


async def test_hello_says_whether_the_stick_should_stream(agent, no_real_audio):
    assert agent._hello_extra()["wmic"] is False
    no_real_audio.devices = [BLACKHOLE]
    agent.update_config({"wireless_mic": True})
    assert agent._hello_extra()["wmic"] is True
    assert agent.status()["wireless_mic"]["device"] == "BlackHole 2ch"


async def test_turning_it_on_tells_the_stick_right_away(wired):
    import asyncio
    await asyncio.sleep(0)
    assert {"src": "agent", "wmic": True} in wired.link.sent
    wired.update_config({"wireless_mic": False})
    await asyncio.sleep(0)
    assert wired.link.sent[-1] == {"src": "agent", "wmic": False}


async def test_on_without_blackhole_tells_the_stick_not_to_stream(agent):
    agent.update_config({"wireless_mic": True})
    assert agent._hello_extra()["wmic"] is False
    st = agent.status()["wireless_mic"]
    assert st["enabled"] and st["device"] is None and "blackhole" in st["error"].lower()


async def test_a_wispr_stream_is_not_a_talk(wired, no_real_audio):
    wired._on_message({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000, "codec": "ima-adpcm"})
    assert wired.wispr.active and wired.rec is None and wired.state != "listening"
    for f in frames(0.3):
        wired._on_audio(f)
    wired._on_message({"src": "agent", "evt": "wispr", "state": "stop"})
    assert not wired.wispr.active
    assert wired.stt.calls == 0 and wired.link.states() == []  # no transcription, nothing on the stick
    assert wired.wispr.last["frames"] == 15
    kinds = [e["kind"] for e in wired.events]
    assert "wispr_mic" in kinds and "heard" not in kinds


async def test_talk_frames_still_go_to_the_talk(wired):
    wired._on_message({"src": "agent", "evt": "talk", "state": "start"})
    for f in frames(0.2):
        wired._on_audio(f)
    assert len(wired.rec.frames) == 10 and wired.wispr.seqr is None


async def test_disconnect_stops_and_closes(wired, no_real_audio):
    wired._on_link("connected")
    wired._on_message({"src": "agent", "evt": "wispr", "state": "start", "rate": 16000})
    wired._on_link("searching")
    assert not wired.wispr.active and no_real_audio.streams[0].closed
