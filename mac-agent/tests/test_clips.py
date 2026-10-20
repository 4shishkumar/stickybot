"""Sound clips for the stick's speaker: loudness processing and packets."""
import struct

import numpy as np
import pytest

from stick_agent import adpcm, clips


def test_loud_stays_in_range_and_gets_louder():
    t = np.arange(8000) / 16000
    quiet = (0.1 * np.sin(2 * np.pi * 800 * t)).astype(np.float32)
    out = clips.loud(quiet)
    assert np.abs(out).max() <= 1.0
    rms = lambda x: np.sqrt(np.mean(x ** 2))
    assert rms(out) > 3 * rms(quiet)


def test_loud_removes_bass():
    # equal parts 60 Hz hum and a 2 kHz tone: after loud(), the hum is much weaker than the tone
    t = np.arange(16000) / 16000
    mix = (0.2 * np.sin(2 * np.pi * 60 * t) + 0.2 * np.sin(2 * np.pi * 2000 * t)).astype(np.float32)
    spec = np.abs(np.fft.rfft(clips.loud(mix)))
    hum, tone = spec[60], spec[2000]          # 1 Hz bins over 1 s
    assert hum < tone / 3


def test_loud_handles_silence_and_empty():
    assert clips.loud(np.zeros(0, dtype=np.float32)).size == 0
    assert np.all(clips.loud(np.zeros(100, dtype=np.float32)) == 0)


def test_packets_start_data_end():
    pcm = (np.sin(np.arange(5000) / 5) * 9000).astype(np.int16)
    pk = clips.packets(pcm)
    assert pk[0][0] == 0 and struct.unpack("<IH", pk[0][1:]) == (5000, 16000)
    assert pk[-1] == b"\x02"
    data = pk[1:-1]
    assert all(p[0] == 1 for p in data)
    assert max(len(p) for p in data) <= 253          # fits one notification at MTU 256
    seqs = [adpcm.decode_frame(p[1:])[0] for p in data]
    assert seqs == list(range(len(data)))


def test_library_and_find(home):
    pack = clips.SOUNDS / "kenney-test"
    pack.mkdir(parents=True)
    (pack / "ding_001.ogg").write_bytes(b"x")
    (pack / "notes.txt").write_text("not a sound")
    assert clips.library() == [{"pack": "kenney-test", "name": "ding_001"}]
    assert clips.find("kenney-test", "ding_001").name == "ding_001.ogg"
    assert clips.find("kenney-test", "nope") is None
    assert clips.find("..", "etc/passwd") is None   # no path tricks


@pytest.mark.slow
def test_load_real_clip():
    lib = clips.library()
    if not lib:
        pytest.skip("no sound packs installed")
    pcm = clips.load(clips.find(lib[0]["pack"], lib[0]["name"]))
    assert pcm.dtype == np.int16 and len(pcm) > 0
