"""The stick's audio frames: IMA-ADPCM, self-contained 20 ms frames."""
import struct

import numpy as np

from stick_agent import adpcm


def tone(seconds=0.5, hz=440, amp=12000):
    t = np.arange(int(16000 * seconds)) / 16000
    return (amp * np.sin(2 * np.pi * hz * t)).astype(np.int16)


def snr_db(ref, out):
    ref = ref.astype(np.float64)
    noise = ref - out.astype(np.float64)
    return 10 * np.log10(np.sum(ref ** 2) / max(np.sum(noise ** 2), 1e-9))


def decode_all(frames):
    pcm = []
    for f in frames:
        pcm.extend(adpcm.decode_frame(f)[1])
    return np.array(pcm, dtype=np.int16)


def test_frame_format():
    frames = adpcm.encode(tone(0.05).tolist())
    assert len(frames) == 3                                  # 800 samples -> 320 + 320 + 160
    seq, pred, idx = struct.unpack_from("<HhB", frames[1])
    assert seq == 1 and 0 <= idx <= 88
    assert len(frames[0]) == 5 + 320 // 2                    # header + 4 bits per sample
    assert len(frames[2]) == 5 + 160 // 2


def test_round_trip_quality():
    pcm = tone(1.0)
    out = decode_all(adpcm.encode(pcm.tolist()))[: len(pcm)]
    assert len(out) == len(pcm)
    assert snr_db(pcm, out) > 20  # IMA-ADPCM on a tone: comfortably above 20 dB


def test_odd_sample_count():
    frames = adpcm.encode([i * 37 for i in range(101)])      # 101 samples
    assert sum(len(adpcm.decode_frame(f)[1]) for f in frames) == 102  # padded to whole bytes


def test_each_frame_decodes_on_its_own():
    frames = adpcm.encode(tone(0.2).tolist())
    whole = decode_all(frames)
    alone = np.array(adpcm.decode_frame(frames[3])[1], dtype=np.int16)
    assert np.array_equal(alone, whole[3 * 320: 4 * 320])


def test_recording_orders_frames_and_fills_lost_ones_with_silence():
    frames = adpcm.encode(tone(0.2).tolist())                # 10 frames
    rec = adpcm.Recording()
    for f in reversed(frames):
        if struct.unpack_from("<H", f)[0] != 4:              # lose frame 4
            rec.add(f)
    audio = rec.audio()
    assert rec.lost == 1
    assert len(audio) == 10 * 320
    assert np.all(audio[4 * 320: 5 * 320] == 0)
    assert np.abs(audio[5 * 320:]).max() > 0.1
    assert audio.dtype == np.float32 and np.abs(audio).max() <= 1.0


def test_recording_ignores_runt_frames_and_empty_is_empty():
    rec = adpcm.Recording()
    rec.add(b"\x00\x00\x00\x00\x00")                         # header only
    assert rec.audio().size == 0 and rec.lost == 0
