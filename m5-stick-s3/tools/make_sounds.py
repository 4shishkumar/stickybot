#!/usr/bin/env python3
"""Builds the sound clips the stick carries in flash (src/sounds_clips.{h,cpp}).

Sources: Kenney's "Interface Sounds", "UI Audio" and "Music Jingles" packs (CC0, kenney.nl),
unpacked into tools/sounds/<pack>/ (the script downloads them if missing), and the celebration
sounds (applause, cheer, ta-da...) synthesized from scratch by synth_sounds.py ("synth:<name>").
Where each clip comes from: m5-stick-s3/SOUNDS.md.
Each clip: mono, 16 kHz, bass below 400 Hz removed (the speaker can't play it),
soft-compressed to be loud, then IMA-ADPCM in the same frame format as the mic.

    ~/.stick-agent/venv/bin/python tools/make_sounds.py     # needs numpy, soundfile, soxr

Edit CLIPS to change which sounds are built in (a "+" joins clips).
"""
import io
import struct
import urllib.request
import zipfile
from pathlib import Path

import numpy as np
import soundfile as sf
import soxr
import synth_sounds

HERE = Path(__file__).parent
SRC = HERE / "sounds"
OUT = HERE.parent / "src"
RATE = 16000
PACKS = {
    "kenney-interface": "https://kenney.nl/media/pages/assets/interface-sounds/fa43c1dd4d-1677589452/kenney_interface-sounds.zip",
    "kenney-ui": "https://kenney.nl/media/pages/assets/ui-audio/490d233f68-1677590494/kenney_ui-audio.zip",
    "kenney-jingles": "https://kenney.nl/media/pages/assets/music-jingles/f37e530b9e-1677590399/kenney_music-jingles.zip",
}

# name on the stick -> clip(s) from the packs
CLIPS = {
    "drop_002": "drop_002",
    "select_003": "select_003",
    "confirmation_001": "confirmation_001",
    "confirmation_002": "confirmation_002",
    "back_002": "back_002",
    "error_004": "error_004",
    "error_008": "error_008",
    "open_002": "open_002",
    "close_003": "close_003",
    "pluck_001": "pluck_001",
    "question_001": "question_001",
    "glass_002": "glass_002",
    "unlock_click": "toggle_002+glass_001",
    "jingle_win": "jingles_NES00",   # celebrations (8-bit victory, 1.8 s)
    "jingle_sax": "jingles_SAX07",   # alternative celebration (1.7 s)
    # celebrations for events (firmware 2.7.0), made by synth_sounds.py: no recordings involved
    "applause": "synth:applause",
    "clap": "synth:clap",
    "cheer": "synth:cheer",
    "party_horn": "synth:party_horn",
    "tada": "synth:tada",
    "drumroll": "synth:drumroll",
    "level_up": "synth:level_up",
    "coins": "synth:coins",
}
# softer compression for the noisy ones, so the claps and hits keep their snap
DRIVE = {"applause": 2.0, "clap": 2.0, "cheer": 2.5, "drumroll": 2.5, "coins": 2.5}

STEP = [7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88,
        97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
        724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660,
        4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818,
        18500, 20350, 22385, 24623, 27086, 29794, 32767]
ADJ = [-1, -1, -1, -1, 2, 4, 6, 8] * 2


def fetch_packs():
    for pack, url in PACKS.items():
        d = SRC / pack
        if d.exists():
            continue
        print("downloading", pack)
        z = zipfile.ZipFile(io.BytesIO(urllib.request.urlopen(url).read()))
        d.mkdir(parents=True)
        for n in z.namelist():
            if n.lower().endswith((".ogg", "license.txt")):
                (d / Path(n).name).write_bytes(z.read(n))


def find(name):
    for f in SRC.glob(f"*/{name}.ogg"):
        return f
    raise SystemExit(f"no clip {name!r} in {SRC}")


def load(name):
    if name.startswith("synth:"):
        return synth_sounds.make(name[6:])
    data, sr = sf.read(str(find(name)), dtype="float32", always_2d=True)
    return soxr.resample(data.mean(axis=1), sr, RATE)


def loud(x, cutoff_hz=400, drive=4.0):
    a = np.exp(-2 * np.pi * cutoff_hz / RATE)
    y = np.empty_like(x)
    px = py = 0.0
    for i, v in enumerate(x):
        py = a * (py + v - px)
        px = v
        y[i] = py
    y = np.tanh(drive * y / (np.max(np.abs(y)) or 1)) / np.tanh(drive)
    return (y * 0.97 * 32767).astype(np.int16)


def encode(pcm, frame=320):
    frames, pred, idx = [], 0, 0
    for seq, start in enumerate(range(0, len(pcm), frame)):
        out, byte = bytearray(struct.pack("<HhB", seq & 0xFFFF, pred, idx)), 0
        chunk = pcm[start:start + frame]
        for i, s in enumerate(chunk):
            step, diff, code = STEP[idx], int(s) - pred, 0
            if diff < 0:
                code, diff = 8, -diff
            d = step >> 3
            if diff >= step: code |= 4; diff -= step; d += step
            step >>= 1
            if diff >= step: code |= 2; diff -= step; d += step
            step >>= 1
            if diff >= step: code |= 1; d += step
            pred = max(-32768, min(32767, pred - d if code & 8 else pred + d))
            idx = max(0, min(88, idx + ADJ[code]))
            if i & 1:
                out.append(byte | (code << 4))
            else:
                byte = code
        if len(chunk) & 1:
            out.append(byte)
        frames.append(bytes(out))
    return frames


def main():
    fetch_packs()
    h = ["// Generated by tools/make_sounds.py from Kenney's sound packs (CC0, kenney.nl) and",
         "// synth_sounds.py (made from scratch). Sources per clip: m5-stick-s3/SOUNDS.md.",
         "#pragma once", "#include <stdint.h>", "#include <stddef.h>", "",
         "struct BuiltinClip {", "  const char* name;", "  const uint8_t* data;  // [len u8][ADPCM frame] ...",
         "  size_t bytes;", "  uint32_t samples;  // at 16 kHz", "};", "",
         f"extern const BuiltinClip BUILTIN_CLIPS[];", f"constexpr size_t BUILTIN_CLIP_COUNT = {len(CLIPS)};"]
    c = ['#include "sounds_clips.h"', ""]
    total = 0
    for i, (name, parts) in enumerate(CLIPS.items()):
        pcm = loud(np.concatenate([load(p) for p in parts.split("+")]), drive=DRIVE.get(name, 4.0))
        blob = b"".join(bytes([len(f)]) + f for f in encode(pcm))
        total += len(blob)
        rows = [", ".join(f"0x{b:02x}" for b in blob[j:j + 24]) for j in range(0, len(blob), 24)]
        c.append(f"static const uint8_t CLIP_{i}[] = {{\n  " + ",\n  ".join(rows) + "\n};")
        print(f"{name:18} {len(pcm) / RATE:.2f}s  {len(blob) / 1024:.1f} KB")
    c.append("\nconst BuiltinClip BUILTIN_CLIPS[] = {")
    for i, (name, parts) in enumerate(CLIPS.items()):
        n = sum(len(load(p)) for p in parts.split("+"))
        c.append(f'  {{"{name}", CLIP_{i}, sizeof(CLIP_{i}), {n}}},')
    c.append("};")
    (OUT / "sounds_clips.h").write_text("\n".join(h) + "\n")
    (OUT / "sounds_clips.cpp").write_text("\n".join(c) + "\n")
    print(f"total {total / 1024:.1f} KB -> src/sounds_clips.cpp")


if __name__ == "__main__":
    main()
