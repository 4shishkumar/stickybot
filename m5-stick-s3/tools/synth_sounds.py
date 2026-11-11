"""Celebration sounds made from scratch (numpy only): no recordings, so no licence to track.

Used by make_sounds.py for the CLIPS entries written "synth:<name>". Every clip is drawn from a
fixed random seed, so a rebuild gives the same bytes. Each returns float32 mono at RATE in [-1, 1];
make_sounds.py then removes the bass and makes it loud like the Kenney clips.

    applause    a room clapping: ~25 people, swells in, fades out          1.8 s
    clap        three hand claps (808-style: a few quick bursts + a tail)   0.7 s
    cheer       "wooo!" voices (formant-filtered) + whistles over applause  1.8 s
    party_horn  a party blower: a short toot, then a long buzzy one         1.2 s
    tada        brass "ta-daaa": a G chord stab, then a big C chord         1.4 s
    drumroll    a snare roll getting louder, then a crash cymbal            1.9 s
    level_up    an 8-bit rising arpeggio with a shimmering top note         1.0 s
    coins       a coin "bling", then a shower of small metal pings          1.5 s
"""
import numpy as np

RATE = 16000


def _t(seconds):
    return np.arange(int(seconds * RATE)) / RATE


def _biquad(x, kind, f, q=0.707):
    """RBJ cookbook biquad: 'lp', 'hp' or 'bp' (constant peak gain)."""
    w = 2 * np.pi * min(f, RATE * 0.45) / RATE
    cw, alpha = np.cos(w), np.sin(w) / (2 * q)
    if kind == "lp":
        b = [(1 - cw) / 2, 1 - cw, (1 - cw) / 2]
    elif kind == "hp":
        b = [(1 + cw) / 2, -(1 + cw), (1 + cw) / 2]
    else:
        b = [alpha, 0.0, -alpha]
    a = [1 + alpha, -2 * cw, 1 - alpha]
    b0, b1, b2, a1, a2 = b[0] / a[0], b[1] / a[0], b[2] / a[0], a[1] / a[0], a[2] / a[0]
    y = np.zeros_like(x)
    x1 = x2 = y1 = y2 = 0.0
    for i, v in enumerate(x):
        o = b0 * v + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2
        x2, x1, y2, y1 = x1, v, y1, o
        y[i] = o
    return y


def _add(out, sig, at):
    i = int(at * RATE)
    if i >= len(out):
        return
    n = min(len(sig), len(out) - i)
    out[i:i + n] += sig[:n]


def _norm(x, peak=0.9):
    return (x / (np.max(np.abs(x)) or 1) * peak).astype(np.float32)


def _clap(rng, center=1400.0, decay=0.05, bursts=3):
    """One hand clap: a few 8-10 ms noise bursts (the hands meeting unevenly), then a short tail."""
    n = int((bursts * 0.009 + decay * 5) * RATE)
    env = np.zeros(n)
    for b in range(bursts):
        i = int(b * 0.009 * RATE)
        k = np.arange(n - i) / RATE
        env[i:] += np.exp(-k / (0.004 if b < bursts - 1 else decay)) * (0.8 if b < bursts - 1 else 1.0)
    return _biquad(rng.standard_normal(n), "bp", center, q=0.9) * env


def _crowd_claps(rng, dur, people, rate_hz=4.5):
    out = np.zeros(int(dur * RATE))
    for _ in range(people):
        center, gain = rng.uniform(800, 2800), rng.uniform(0.3, 1.0)
        t = rng.uniform(0, 1 / rate_hz)
        while t < dur:
            _add(out, gain * _clap(rng, center, rng.uniform(0.02, 0.035), bursts=int(rng.integers(1, 3))), t)
            t += rng.normal(1 / rate_hz, 0.025)
    return out


def _fade(x, fade_in, fade_out):
    env = np.ones(len(x))
    a, b = int(fade_in * RATE), int(fade_out * RATE)
    if a:
        env[:a] = np.linspace(0, 1, a) ** 0.7
    if b:
        env[-b:] *= np.linspace(1, 0, b) ** 1.5
    return x * env


def applause():
    rng = np.random.default_rng(11)
    return _norm(_fade(_crowd_claps(rng, 1.8, 25), 0.18, 0.6))


def clap():
    rng = np.random.default_rng(12)
    out = np.zeros(int(0.7 * RATE))
    for i, t in enumerate((0.0, 0.22, 0.44)):
        _add(out, (0.85 if i < 2 else 1.0) * _clap(rng, 1500, 0.05), t)
    return _norm(out)


def _formant_voice(rng, dur, f0, glide, vowel):
    """A cheering voice: a sawtooth with a rising-then-falling pitch through two vowel formants."""
    t = _t(dur)
    shape = np.sin(np.pi * np.clip(t / dur, 0, 1)) ** 0.5  # up, then down
    f = f0 * (1 + glide * shape) * (1 + 0.02 * np.sin(2 * np.pi * rng.uniform(5, 7) * t))
    phase = np.cumsum(f) / RATE
    saw = 2 * (phase % 1) - 1 + 0.15 * rng.standard_normal(len(t))
    f1, f2 = vowel
    v = _biquad(saw, "bp", f1, q=3) + 0.7 * _biquad(saw, "bp", f2, q=4) + 0.3 * _biquad(saw, "bp", 2800, q=4)
    env = np.minimum(1, t / 0.06) * np.minimum(1, (dur - t) / 0.25).clip(0)
    return v * env


def cheer():
    rng = np.random.default_rng(13)
    dur = 1.8
    out = 0.45 * _norm(_crowd_claps(rng, dur, 18))
    vowels = [(700, 1220), (650, 1080), (300, 870), (400, 1700)]  # "ah", "aw", "oo", "eh"
    for _ in range(9):
        start = rng.uniform(0, 0.35)
        vd = rng.uniform(0.8, dur - start)
        voice = _formant_voice(rng, vd, rng.uniform(190, 420), rng.uniform(0.25, 0.6), vowels[rng.integers(4)])
        _add(out, rng.uniform(0.5, 1.0) * _norm(voice), start)
    for _ in range(2):  # whistles: "fweeet"
        start, wd = rng.uniform(0.15, 0.8), rng.uniform(0.35, 0.55)
        t = _t(wd)
        f = 2200 + 900 * np.minimum(1, t / 0.12) + 40 * np.sin(2 * np.pi * 9 * t)
        w = np.sin(2 * np.pi * np.cumsum(f) / RATE) * np.minimum(1, t / 0.03) * np.minimum(1, (wd - t) / 0.08)
        _add(out, 0.35 * w, start)
    return _norm(_fade(out, 0.08, 0.5))


def _buzz(rng, dur, f_from, f_to, rise):
    """A party blower's reed: a pulse wave with a quick pitch rise, flutter, and a little breath."""
    t = _t(dur)
    f = f_to + (f_from - f_to) * np.exp(-t / rise)
    f = f * (1 + 0.012 * np.sin(2 * np.pi * 23 * t))  # paper flutter
    phase = np.cumsum(f) / RATE % 1
    pulse = np.where(phase < 0.3, 1.0, -0.45) + 0.1 * rng.standard_normal(len(t))
    body = _biquad(_biquad(pulse, "lp", 3200), "bp", 1100, q=0.8) + 0.4 * pulse
    env = np.minimum(1, t / 0.015) * np.minimum(1, (dur - t) / 0.05).clip(0)
    return body * env


def party_horn():
    rng = np.random.default_rng(14)
    out = np.zeros(int(1.2 * RATE))
    _add(out, 0.8 * _buzz(rng, 0.22, 330, 420, 0.03), 0.0)
    long = _buzz(rng, 0.85, 340, 440, 0.05)
    long *= 1 + 0.25 * np.sin(2 * np.pi * 7 * _t(0.85))  # a wobbly blow
    _add(out, long, 0.3)
    return _norm(out)


def _brass(rng, dur, freqs, attack=0.03, release=0.25):
    """A brass section: detuned sawtooths through a lowpass that opens on the attack."""
    t = _t(dur)
    vib = 1 + 0.006 * np.sin(2 * np.pi * 5.5 * t) * np.minimum(1, t / 0.4)
    x = np.zeros(len(t))
    for f in freqs:
        for d in (-0.004, 0.0, 0.004):
            phase = np.cumsum(f * (1 + d) * vib) / RATE + rng.uniform()
            x += 2 * (phase % 1) - 1
    env = np.minimum(1, t / attack) * np.minimum(1, (dur - t) / release).clip(0)
    # brightness: the filter follows the envelope (blown harder = brighter)
    bright = _biquad(x, "lp", 4200, q=0.9)
    dull = _biquad(x, "lp", 1200, q=0.7)
    open_ = np.clip(env * np.exp(-t / 0.6) + 0.35, 0, 1)
    return (bright * open_ + dull * (1 - open_)) * env


def tada():
    rng = np.random.default_rng(15)
    out = np.zeros(int(1.4 * RATE))
    g4, b4, d5, c5, e5, g5, c6 = 392.0, 493.9, 587.3, 523.3, 659.3, 784.0, 1046.5
    _add(out, 0.75 * _brass(rng, 0.16, [g4, b4, d5], release=0.04), 0.0)
    big = _brass(rng, 1.15, [c5, e5, g5, c6], release=0.45)
    big *= 1 + 0.25 * np.minimum(1, _t(1.15) / 0.3)  # swells a little
    _add(out, big, 0.2)
    return _norm(out)


def _snare(rng, gain):
    n = int(0.06 * RATE)
    t = np.arange(n) / RATE
    wires = _biquad(rng.standard_normal(n), "bp", 3500, q=0.6) * np.exp(-t / 0.018)
    body = np.sin(2 * np.pi * 480 * t) * np.exp(-t / 0.012)
    return gain * (wires + 0.5 * body)


def _crash(rng, dur):
    """808-style cymbal: six detuned square waves + noise, high-passed, long decay."""
    t = _t(dur)
    x = np.zeros(len(t))
    for f in (205.3, 304.4, 369.6, 522.7, 540.0, 800.0):
        x += np.sign(np.sin(2 * np.pi * f * 2.3 * t + rng.uniform(0, 6.28)))
    x = 0.4 * x / 6 + rng.standard_normal(len(t))
    x = _biquad(_biquad(x, "hp", 4500, q=0.7), "hp", 3500, q=0.7)
    return x * np.exp(-t / 0.35) * np.minimum(1, t / 0.002)


def drumroll():
    rng = np.random.default_rng(16)
    out = np.zeros(int(1.9 * RATE))
    t, roll_end = 0.0, 1.15
    while t < roll_end:
        _add(out, _snare(rng, 0.25 + 0.6 * (t / roll_end) ** 1.5 + rng.uniform(-0.05, 0.05)), t)
        t += 0.032 + rng.uniform(-0.003, 0.003)
    hit = 1.22
    _add(out, 1.4 * _snare(rng, 1.0), hit)
    tt = _t(0.25)
    _add(out, 0.9 * np.sin(2 * np.pi * (90 + 120 * np.exp(-tt / 0.03)) * tt) * np.exp(-tt / 0.08), hit)  # kick
    _add(out, 1.3 * _crash(rng, 1.9 - hit), hit)
    return _norm(out)


def _square(f, dur, duty=0.25):
    t = _t(dur)
    return np.where((f * t) % 1 < duty, 1.0, -1.0)


def level_up():
    out = np.zeros(int(1.0 * RATE))
    notes = [523.3, 659.3, 784.0, 1046.5, 1318.5, 1568.0]  # C E G C E G
    for i, f in enumerate(notes):
        tone = _square(f, 0.07) * np.minimum(1, (0.07 - _t(0.07)) / 0.01)
        _add(out, 0.6 * tone, i * 0.065)
    top, td = 2093.0, 0.55
    t = _t(td)
    tone = _square(top * (1 + 0.01 * np.sin(2 * np.pi * 12 * t)), td, duty=0.5)
    tone += 0.5 * _square(top * 1.5, td, duty=0.5)  # a fifth on top: shimmer
    _add(out, 0.55 * tone * np.exp(-t / 0.3), len(notes) * 0.065)
    return _norm(out)


def _ping(rng, f, decay):
    """A small coin: a few inharmonic partials, like struck metal."""
    n = int(decay * 5 * RATE)
    t = np.arange(n) / RATE
    x = sum(a * np.sin(2 * np.pi * f * r * t + rng.uniform(0, 6.28)) * np.exp(-t / (decay / r ** 0.5))
            for r, a in ((1.0, 1.0), (2.76, 0.5), (5.4, 0.25)))
    return x * np.minimum(1, t / 0.0015)


def coins():
    rng = np.random.default_rng(17)
    out = np.zeros(int(1.5 * RATE))
    for f, at, d in ((987.8, 0.0, 0.06), (1318.5, 0.07, 0.35)):  # the "bling" (B5 -> E6)
        t = _t(d)
        _add(out, 0.7 * _square(f, d, 0.5) * np.exp(-t / (d * 0.6)), at)
    for _ in range(38):  # the shower: busiest in the middle
        at = 0.15 + rng.beta(2, 2.5) * 1.2
        _add(out, rng.uniform(0.25, 0.7) * _ping(rng, rng.uniform(2200, 4200), rng.uniform(0.04, 0.09)), at)
    return _norm(_fade(out, 0.0, 0.25))


MAKERS = {f.__name__: f for f in (applause, clap, cheer, party_horn, tada, drumroll, level_up, coins)}


def make(name):
    return MAKERS[name]()
