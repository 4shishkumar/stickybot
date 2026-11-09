#!/usr/bin/env python3
"""Draws the Cat Nap example face's pixel art (faces/examples/catnap/img/*.png) as indexed
PNGs. The PNGs are the source a creator would normally draw in Aseprite or Piskel; this
script only exists so the example's art is reproducible. Standard library only.

    python3 tools/faces/make_example_art.py
"""
from __future__ import annotations

import math
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import facepkg  # noqa: E402

OUT = facepkg.ROOT / "faces" / "examples" / "catnap" / "img"

CLEAR, LINE, FUR, STRIPE, CREAM, PINK, EYE, WHITE = range(8)
CAT_PALETTE = [(0, 0, 0, 0), (0x3a, 0x24, 0x2c, 255), (0xf2, 0xa6, 0x5a, 255), (0xcc, 0x76, 0x33, 255),
               (0xff, 0xf1, 0xdc, 255), (0xff, 0x8f, 0xa3, 255), (0x1b, 0x14, 0x18, 255), (0xff, 0xff, 0xff, 255)]


class Canvas:
    def __init__(self, w, h):
        self.w, self.h = w, h
        self.px = [CLEAR] * (w * h)

    def set(self, x, y, c):
        x, y = int(round(x)), int(round(y))
        if 0 <= x < self.w and 0 <= y < self.h:
            self.px[y * self.w + x] = c

    def get(self, x, y):
        return self.px[y * self.w + x] if 0 <= x < self.w and 0 <= y < self.h else CLEAR

    def ellipse(self, cx, cy, rx, ry, c, only_on=None):
        for y in range(self.h):
            for x in range(self.w):
                inside = ((x + 0.5 - cx) / rx) ** 2 + ((y + 0.5 - cy) / ry) ** 2 <= 1
                if inside and (only_on is None or self.get(x, y) in only_on):
                    self.set(x, y, c)

    def tri(self, a, b, c, col):
        def side(p, q, r):
            return (p[0] - r[0]) * (q[1] - r[1]) - (q[0] - r[0]) * (p[1] - r[1])
        for y in range(self.h):
            for x in range(self.w):
                p = (x + 0.5, y + 0.5)
                d1, d2, d3 = side(p, a, b), side(p, b, c), side(p, c, a)
                if not ((d1 < 0 or d2 < 0 or d3 < 0) and (d1 > 0 or d2 > 0 or d3 > 0)):
                    self.set(x, y, col)

    def thick(self, pts, r, c):
        for (x0, y0), (x1, y1) in zip(pts, pts[1:]):  # noqa: B905
            n = int(max(abs(x1 - x0), abs(y1 - y0)) * 3) + 1
            for i in range(n + 1):
                t = i / n
                self.ellipse(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, r, r, c)

    def outline(self, col=LINE):
        edge = []
        for y in range(self.h):
            for x in range(self.w):
                if self.get(x, y) == CLEAR and any(self.get(x + dx, y + dy) not in (CLEAR, col)
                                                    for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
                    edge.append((x, y))
        for x, y in edge:
            self.set(x, y, col)


def sitting(tail, eyes="open", paws_up=False):
    c = Canvas(32, 25)
    c.thick(tail, 1.3, FUR)
    c.ellipse(15.5, 17.5, 7.5, 6, FUR)
    c.ellipse(15.5, 9.5, 6.8, 5.6, FUR)
    c.tri((9, 8), (9.5, 1), (14.5, 5), FUR)
    c.tri((22, 8), (21.5, 1), (16.5, 5), FUR)
    c.ellipse(15.5, 19.5, 3.6, 3.8, CREAM, only_on={FUR})
    if paws_up:
        c.ellipse(7.5, 13, 2, 2.2, CREAM)
        c.ellipse(23.5, 13, 2, 2.2, CREAM)
    else:
        c.ellipse(12.5, 22.6, 2.2, 1.3, CREAM)
        c.ellipse(18.5, 22.6, 2.2, 1.3, CREAM)
    c.outline()
    face(c, eyes)
    c.set(tail[-1][0], tail[-1][1], STRIPE)
    return c


def face(c, eyes, ox=0, oy=0):
    for x, y in ((10, 3), (11, 4), (21, 3), (20, 4)):
        c.set(x + ox, y + oy, PINK)
    for x, y in ((15, 5), (16, 5), (15, 6), (12, 5), (19, 5)):  # forehead stripes
        c.set(x + ox, y + oy, STRIPE)
    if eyes == "open":
        for x in (12, 19):
            c.set(x + ox, 9 + oy, EYE)
            c.set(x + ox, 10 + oy, EYE)
    elif eyes == "blink":
        for x in (11, 12, 19, 20):
            c.set(x + ox, 10 + oy, EYE)
    elif eyes == "happy":
        for x, y in ((11, 10), (12, 9), (13, 10), (18, 10), (19, 9), (20, 10)):
            c.set(x + ox, y + oy, EYE)
    elif eyes == "wide":
        for x in (11, 12, 19, 20):
            c.set(x + ox, 9 + oy, WHITE)
            c.set(x + ox, 10 + oy, WHITE)
        c.set(12 + ox, 10 + oy, EYE)
        c.set(19 + ox, 10 + oy, EYE)
    c.set(15 + ox, 11 + oy, PINK)
    c.set(16 + ox, 11 + oy, PINK)
    if eyes == "happy":
        c.set(15 + ox, 12 + oy, LINE)
        c.set(16 + ox, 12 + oy, LINE)
        c.set(15 + ox, 13 + oy, PINK)
        c.set(16 + ox, 13 + oy, PINK)
    elif eyes == "wide":
        for x, y in ((14, 13), (15, 12), (16, 12), (17, 13)):
            c.set(x + ox, y + oy, LINE)
    else:
        c.set(14 + ox, 12 + oy, LINE)
        c.set(17 + ox, 12 + oy, LINE)
    if eyes != "wide":
        c.set(10 + ox, 11 + oy, PINK)
        c.set(21 + ox, 11 + oy, PINK)


def sleeping():
    c = Canvas(32, 25)
    c.ellipse(17, 19, 11, 4.6, FUR)
    c.ellipse(10.5, 16, 6, 5, FUR)
    c.tri((5, 15), (5.5, 9.5), (9, 12), FUR)
    c.tri((15.5, 14), (15, 9.5), (11.5, 12), FUR)
    c.thick([(27, 18), (25, 22), (18, 22.5), (10, 22.5)], 1.3, FUR)
    c.ellipse(19, 18, 4, 2, STRIPE, only_on={FUR})
    c.ellipse(19, 17.6, 3.2, 1.4, FUR, only_on={STRIPE})
    c.outline()
    for x, y in ((6, 12), (14, 12)):
        c.set(x, y, PINK)
    for x in (7, 8, 12, 13):
        c.set(x, 17, EYE)
    c.set(10, 18, PINK)
    c.set(9, 13, STRIPE)
    c.set(10, 13, STRIPE)
    c.set(11, 13, STRIPE)
    c.set(10, 22, STRIPE)
    return c


def puffed():
    c = Canvas(32, 25)
    c.thick([(23, 20), (27, 15), (28, 7)], 2.4, FUR)
    for i in range(14):  # fur on end
        a = i / 14 * 2 * math.pi
        c.tri((15.5 + math.cos(a) * 6, 16.5 + math.sin(a) * 5),
              (15.5 + math.cos(a + 0.25) * 6, 16.5 + math.sin(a + 0.25) * 5),
              (15.5 + math.cos(a + 0.12) * 9.6, 16.5 + math.sin(a + 0.12) * 7.6), FUR)
    c.ellipse(15.5, 17, 8.4, 6.6, FUR)
    c.ellipse(15.5, 9.5, 6.8, 5.6, FUR)
    c.tri((7, 9), (4.5, 1.5), (12.5, 6), FUR)
    c.tri((24, 9), (26.5, 1.5), (18.5, 6), FUR)
    c.ellipse(15.5, 19.5, 3.6, 3.8, CREAM, only_on={FUR})
    c.ellipse(12.5, 22.6, 2.2, 1.3, CREAM)
    c.ellipse(18.5, 22.6, 2.2, 1.3, CREAM)
    c.outline()
    face(c, "wide")
    for x, y in ((10, 3), (11, 4), (21, 3), (20, 4), (12, 5), (19, 5)):
        c.set(x, y, FUR)
    c.set(6, 4, PINK)
    c.set(25, 4, PINK)
    return c


def strip(frames, path, palette):
    w = sum(f.w for f in frames)
    h = frames[0].h
    px = []
    for y in range(h):
        for f in frames:
            px += f.px[y * f.w:(y + 1) * f.w]
    facepkg.write_png(path, w, h, palette, px)
    print(path.relative_to(facepkg.ROOT), f"{w}x{h}")


def hills():
    w, h = 120, 30
    pal = [(0, 0, 0, 0), (0x3f, 0x8f, 0x5a, 255), (0x5d, 0xb0, 0x70, 255), (0x2f, 0x72, 0x47, 255)]
    px = [0] * (w * h)
    for x in range(w):
        top = 12 + 5 * math.sin(2 * math.pi * x / w) + 3 * math.sin(2 * math.pi * 2 * x / w + 1.2)
        for y in range(h):
            if y >= top:
                px[y * w + x] = 2 if y < top + 2 else 1
        for tx in (14, 52, 88):  # a few round trees
            for y in range(h):
                if (x - tx) ** 2 + ((y - (top - 1)) * 1.3) ** 2 < 10 and y < top + 2:
                    px[y * w + x] = 3
    facepkg.write_png(OUT / "hills.png", w, h, pal, px)
    print((OUT / "hills.png").relative_to(facepkg.ROOT), f"{w}x{h}")


def clouds():
    c = Canvas(80, 16)
    WHITE_, SHADE = 1, 2
    for cx, cy, rx, ry in ((14, 9, 9, 4.5), (22, 6, 7, 5), (30, 10, 8, 4), (58, 10, 7, 3.5), (65, 8, 6, 4.2)):
        c.ellipse(cx, cy, rx, ry, WHITE_)
    for y in range(c.h):
        for x in range(c.w):
            if c.get(x, y) == WHITE_ and c.get(x, y + 2) == CLEAR:
                c.set(x, y, SHADE)
    pal = [(0, 0, 0, 0), (0xff, 0xff, 0xff, 255), (0xd6, 0xe8, 0xf5, 255)]
    facepkg.write_png(OUT / "clouds.png", c.w, c.h, pal, c.px)
    print((OUT / "clouds.png").relative_to(facepkg.ROOT), f"{c.w}x{c.h}")


def fish():
    c = Canvas(14, 9)
    BODY, FIN = 2, 3
    c.ellipse(6, 4.5, 5, 3.2, BODY)
    c.tri((10, 4.5), (13.6, 1), (13.6, 8), FIN)
    c.outline(1)
    c.set(3, 3, 1)
    c.set(6, 3, FIN)
    c.set(7, 4, FIN)
    c.set(6, 5, FIN)
    pal = [(0, 0, 0, 0), (0x1d, 0x35, 0x57, 255), (0x8e, 0xc5, 0xff, 255), (0x5a, 0x8f, 0xd8, 255)]
    facepkg.write_png(OUT / "fish.png", c.w, c.h, pal, c.px)
    print((OUT / "fish.png").relative_to(facepkg.ROOT), f"{c.w}x{c.h}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    tail_down = [(22, 21), (26, 19), (28, 15), (28, 11)]
    tail_flick = [(22, 21), (26, 18), (27, 13), (25, 9)]
    tail_up = [(22, 19), (27, 13), (28, 6)]
    frames = [
        sitting(tail_down),                      # 0 sitting
        sitting(tail_flick),                     # 1 tail flick
        sitting(tail_down, "blink"),             # 2 blink
        sleeping(),                              # 3 asleep (night)
        sitting(tail_up, "happy", paws_up=True),  # 4 happy (PR merged, CI green)
        puffed(),                                # 5 fur on end (CI failed)
    ]
    strip(frames, OUT / "cat.png", CAT_PALETTE)
    hills()
    clouds()
    fish()


if __name__ == "__main__":
    main()
