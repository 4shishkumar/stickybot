"""Draws the StickyBot icon (1024 px PNG): the dashboard's brand mark (the stick, dark, with two
teal eyes) on a macOS-style rounded square. Needs Pillow. The committed .icns came from:

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
TEAL = (46, 196, 182)  # --agent (dark theme) in static/app.css


def box(b):
    return [int(v * SS) for v in b]


def vgrad(size, top, bot):
    g = Image.new("RGBA", size)
    d = ImageDraw.Draw(g)
    for y in range(size[1]):
        t = y / size[1]
        d.line([(0, y), (size[0], y)], fill=tuple(int(top[i] + (bot[i] - top[i]) * t) for i in range(3)) + (255,))
    return g


img = Image.new("RGBA", (W, W), (0, 0, 0, 0))

# the rounded square (macOS grid: 824 px on a 1024 canvas), deep slate
mask = Image.new("L", (W, W), 0)
ImageDraw.Draw(mask).rounded_rectangle(box((100, 100, 924, 924)), radius=185 * SS, fill=255)
img.paste(vgrad((W, W), (31, 40, 47), (13, 17, 20)), (0, 0), mask)

# soft teal glow behind the stick
glow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse(box((262, 230, 762, 790)), fill=TEAL + (70,))
glow = glow.filter(ImageFilter.GaussianBlur(70 * SS))
img.alpha_composite(Image.composite(glow, Image.new("RGBA", (W, W), (0, 0, 0, 0)), mask))

# the stick: a tall rounded body (the brand mark is 26 x 40 with radius 8)
body = (342, 214, 682, 810)
shadow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
ImageDraw.Draw(shadow).rounded_rectangle(box((body[0], body[1] + 26, body[2], body[3] + 26)), radius=104 * SS,
                                         fill=(0, 0, 0, 120))
img.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(22 * SS)))
smask = Image.new("L", (W, W), 0)
ImageDraw.Draw(smask).rounded_rectangle(box(body), radius=104 * SS, fill=255)
img.paste(vgrad((W, W), (16, 21, 25), (8, 10, 12)), (0, 0), smask)
ImageDraw.Draw(img).rounded_rectangle(box(body), radius=104 * SS, outline=(70, 86, 96, 255), width=7 * SS)

# the eyes: two teal pills (5 x 8, 4 apart in the mark), with a little glow
eyes = [(424, 404, 494, 532), (530, 404, 600, 532)]
eglow = Image.new("RGBA", (W, W), (0, 0, 0, 0))
for e in eyes:
    ImageDraw.Draw(eglow).rounded_rectangle(box(e), radius=35 * SS, fill=TEAL + (200,))
img.alpha_composite(eglow.filter(ImageFilter.GaussianBlur(18 * SS)))
d = ImageDraw.Draw(img)
for e in eyes:
    d.rounded_rectangle(box(e), radius=35 * SS, fill=TEAL + (255,))
    d.rounded_rectangle(box((e[0] + 14, e[1] + 14, e[0] + 30, e[1] + 44)), radius=8 * SS, fill=(220, 255, 250, 150))

img.resize((S, S), Image.LANCZOS).save(sys.argv[1] if len(sys.argv) > 1 else "icon.png")
