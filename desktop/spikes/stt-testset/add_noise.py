"""Mixes seeded pink-ish noise into a 16-bit mono WAV, in place, at a given SNR (dB). Stdlib only.

usage: add_noise.py file.wav snr_db seed
"""
import array
import math
import random
import sys
import wave

path, snr_db, seed = sys.argv[1], float(sys.argv[2]), sys.argv[3]
with wave.open(path) as w:
    params = w.getparams()
    x = array.array("h", w.readframes(w.getnframes()))
rnd = random.Random(seed)
# pink-ish: a few leaky integrators of white noise plus a little white (good enough for a spike)
b = [0.0, 0.0, 0.0]
noise = []
for _ in x:
    wn = rnd.gauss(0, 1)
    b[0] = 0.997 * b[0] + 0.029591 * wn
    b[1] = 0.985 * b[1] + 0.032534 * wn
    b[2] = 0.950 * b[2] + 0.048056 * wn
    noise.append(b[0] + b[1] + b[2] + 0.05 * wn)
# SNR against the speech's power (the say output has little silence, so whole-file power is fine)
sig_p = sum(s * s for s in x) / len(x)
noise_p = sum(n * n for n in noise) / len(noise)
k = math.sqrt(sig_p / (noise_p * 10 ** (snr_db / 10)))
y = array.array("h", (max(-32768, min(32767, int(s + k * n))) for s, n in zip(x, noise)))
with wave.open(path, "wb") as w:
    w.setparams(params)
    w.writeframes(y.tobytes())
