"""Sound clips for the stick's speaker: sound packs in ~/.stick-agent/sounds/<pack>/
(Kenney's Interface Sounds and UI Audio, CC0), and later spoken replies.

A clip is mixed to mono, resampled to 16 kHz, normalised (the speaker is tiny)
and sent as IMA-ADPCM packets on the stick's "audio in" characteristic:
  [0][samples u32][rate u16]  start
  [1][frame]                  data (same frame format as the mic's)
  [2]                         end -> the stick plays it
"""
import struct

import numpy as np

from . import adpcm, config

RATE = 16000
SOUNDS = config.HOME / "sounds"
EXTS = (".ogg", ".wav", ".flac", ".aiff", ".aif", ".mp3")


def library():
    """[{"pack", "name"}] for every clip in the sound packs."""
    out = []
    for pack in sorted(p for p in SOUNDS.glob("*") if p.is_dir()):
        for f in sorted(pack.iterdir()):
            if f.suffix.lower() in EXTS:
                out.append({"pack": pack.name, "name": f.stem})
    return out


def find(pack, name):
    d = SOUNDS / pack
    for ext in EXTS:
        f = d / (name + ext)
        if f.exists() and f.resolve().parent == d.resolve():  # no path tricks
            return f
    return None


def load(path, max_seconds=30, speech=False):
    import soundfile as sf
    import soxr
    data, sr = sf.read(str(path), dtype="float32", always_2d=True)
    mono = data.mean(axis=1)
    if sr != RATE:
        mono = soxr.resample(mono, sr, RATE)
    mono = mono[: RATE * max_seconds]
    # speech: a bit gentler than UI sounds (drive 3 vs 4) so words stay clear
    return (loud(mono, cutoff_hz=300, drive=3.0 if speech else 4.0) * 32767).astype(np.int16)


def loud(x, cutoff_hz=400, drive=4.0):
    """Makes a clip as loud as the tiny speaker allows: drops the bass it can't
    play (1st-order high-pass), then compresses with a soft clipper so the
    average level is near full scale, like a beep."""
    if not len(x):
        return x
    a = np.exp(-2 * np.pi * cutoff_hz / RATE)
    y = np.empty_like(x)
    prev_x = prev_y = 0.0
    for i, v in enumerate(x):  # y[n] = a * (y[n-1] + x[n] - x[n-1])
        prev_y = a * (prev_y + v - prev_x)
        prev_x = v
        y[i] = prev_y
    peak = float(np.max(np.abs(y))) or 1.0
    y = np.tanh(drive * y / peak) / np.tanh(drive)
    return (y * 0.97).astype(np.float32)


def packets(pcm):
    pkts = [b"\x00" + struct.pack("<IH", len(pcm), RATE)]
    pkts += [b"\x01" + f for f in adpcm.encode(pcm.tolist())]
    pkts.append(b"\x02")
    return pkts
