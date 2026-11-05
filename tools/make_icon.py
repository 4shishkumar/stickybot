"""Draws the StickyBot icon (1024 px PNG). Needs Pillow. The committed .icns came from:

    python tools/make_icon.py /tmp/icon.png
    mkdir StickyBot.iconset; for s in 16 32 128 256 512; do
      sips -z $s $s /tmp/icon.png --out StickyBot.iconset/icon_${s}x${s}.png
      sips -z $((s*2)) $((s*2)) /tmp/icon.png --out StickyBot.iconset/icon_${s}x${s}@2x.png; done
    iconutil -c icns StickyBot.iconset -o mac-agent/stick_agent/icon/StickyBot.icns
"""
import sys

from PIL import Image, ImageDraw, ImageFilter

S = 1024
SS = 4  # supersample
W = S * SS


def box(b):
    return [int(v * SS) for v in b]


img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
grad = Image.new("RGBA", (W, W))
top, bot = (255, 184, 77), (247, 120, 52)  # warm amber -> orange
gd = ImageDraw.Draw(grad)
for y in range(W):
    t = y / W
    gd.line([(0, y), (W, y)], fill=tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3)) + (255,))
mask = Image.new("L", (W, W), 0)
ImageDraw.Draw(mask).rounded_rectangle(box((100, 100, 924, 924)), radius=185 * SS, fill=255)
sh = Image.new("RGBA", (W, W), (0, 0, 0, 0))
ImageDraw.Draw(sh).rounded_rectangle(box((100, 114, 924, 938)), radius=185 * SS, fill=(0, 0, 0, 80))
sh = sh.filter(ImageFilter.GaussianBlur(14 * SS))
img.alpha_composite(sh)
img.paste(grad, (0, 0), mask)
d = ImageDraw.Draw(img)
cream, ink, mint, orange = (255, 250, 242, 255), (38, 36, 52, 255), (120, 230, 200, 255), (247, 120, 52, 255)
d.rounded_rectangle(box((352, 196, 672, 828)), radius=86 * SS, fill=cream)       # the stick
d.rounded_rectangle(box((392, 246, 632, 556)), radius=42 * SS, fill=ink)         # its screen
for cx in (466, 558):                                                            # eyes
    d.ellipse(box((cx - 28, 352, cx + 28, 436)), fill=mint)
d.arc(box((470, 410, 554, 494)), start=35, end=145, fill=mint, width=13 * SS)    # smile
d.ellipse(box((462, 624, 562, 724)), fill=orange)                               # button A
d.rounded_rectangle(box((672, 330, 690, 430)), radius=9 * SS, fill=cream)        # side button
img.resize((S, S), Image.LANCZOS).save(sys.argv[1])
