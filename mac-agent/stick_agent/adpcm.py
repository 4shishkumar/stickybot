"""Decoder for the stick's audio frames (see PROTOCOL.md, "Mac link").

Frame: [seq u16][predictor i16][step index u8][4-bit IMA-ADPCM samples, low
nibble first]. Each frame carries the decoder state at its start, so frames
decode independently and a lost frame just becomes 20 ms of silence.
"""
import struct

import numpy as np

STEP = [7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88,
        97, 107, 118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658,
        724, 796, 876, 963, 1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660,
        4026, 4428, 4871, 5358, 5894, 6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818,
        18500, 20350, 22385, 24623, 27086, 29794, 32767]
ADJ = [-1, -1, -1, -1, 2, 4, 6, 8] * 2
RATE = 16000


def decode_frame(frame: bytes):
    seq, pred, idx = struct.unpack_from("<HhB", frame)
    out = []
    for b in frame[5:]:
        for code in (b & 15, b >> 4):
            step = STEP[idx]
            d = step >> 3
            if code & 4:
                d += step
            if code & 2:
                d += step >> 1
            if code & 1:
                d += step >> 2
            pred = max(-32768, min(32767, pred - d if code & 8 else pred + d))
            idx = max(0, min(88, idx + ADJ[code]))
            out.append(pred)
    return seq, out


class Recording:
    """Collects one talk's frames and turns them into float32 audio."""

    def __init__(self):
        self.frames = {}

    def add(self, frame: bytes):
        if len(frame) > 5:
            seq, samples = decode_frame(frame)
            self.frames[seq] = samples

    def audio(self) -> np.ndarray:
        if not self.frames:
            return np.zeros(0, dtype=np.float32)
        size = max(len(s) for s in self.frames.values())
        pcm = []
        for seq in range(min(self.frames), max(self.frames) + 1):
            pcm.extend(self.frames.get(seq, [0] * size))  # lost frame: silence
        return np.asarray(pcm, dtype=np.float32) / 32768.0

    @property
    def lost(self):
        if not self.frames:
            return 0
        return (max(self.frames) - min(self.frames) + 1) - len(self.frames)


def encode(pcm, frame_samples=320):
    """int16 samples -> list of frames in the stick's format (each decodes on its own)."""
    frames, pred, idx = [], 0, 0
    for seq, start in enumerate(range(0, len(pcm), frame_samples)):
        chunk = pcm[start:start + frame_samples]
        out = bytearray(struct.pack("<HhB", seq & 0xFFFF, pred, idx))
        byte = 0
        for i, s in enumerate(chunk):
            step = STEP[idx]
            diff = int(s) - pred
            code = 0
            if diff < 0:
                code, diff = 8, -diff
            d = step >> 3
            if diff >= step:
                code |= 4
                diff -= step
                d += step
            step >>= 1
            if diff >= step:
                code |= 2
                diff -= step
                d += step
            step >>= 1
            if diff >= step:
                code |= 1
                d += step
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
