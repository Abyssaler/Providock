#!/usr/bin/env python3
"""Render the Providock logo: blue gradient rounded square + white anchor."""
from PIL import Image, ImageDraw

S = 1024
SS = 4  # supersampling factor
W = S * SS
RADIUS = 225 * SS

# --- diagonal gradient via 2x2 upscale ---
c0 = (46, 107, 240)   # top-left  #2E6BF0
c1 = (26, 63, 160)    # bottom-right #1A3FA0
tiny = Image.new("RGB", (2, 2))
tiny.putpixel((0, 0), c0)
tiny.putpixel((1, 0), tuple(a + (b - a) // 3 for a, b in zip(c0, c1)))
tiny.putpixel((0, 1), tuple(a + (b - a) // 3 for a, b in zip(c0, c1)))
tiny.putpixel((1, 1), c1)
grad = tiny.resize((W, W), Image.BICUBIC)

img = Image.new("RGBA", (W, W), (0, 0, 0, 0))
mask = Image.new("L", (W, W), 0)
md = ImageDraw.Draw(mask)
md.rounded_rectangle([0, 0, W - 1, W - 1], radius=RADIUS, fill=255)
img.paste(grad, (0, 0), mask)

d = ImageDraw.Draw(img)
white = (255, 255, 255, 255)


def sc(v):
    return v * SS


def thick_line(p1, p2, width):
    d.line([p1, p2], fill=white, width=sc(width))
    r = sc(width) / 2
    for p in (p1, p2):
        d.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=white)


# ring
ring_r = sc(72)
ring_cx, ring_cy = sc(512), sc(330)
ring_w = sc(46)
d.ellipse(
    [ring_cx - ring_r, ring_cy - ring_r, ring_cx + ring_r, ring_cy + ring_r],
    outline=white,
    width=ring_w,
)

# shaft
thick_line((sc(512), sc(402)), (sc(512), sc(768)), 46)
# crossbar
thick_line((sc(382), sc(492)), (sc(642), sc(492)), 42)
# flukes arc (bounding box of the circle the arc belongs to)
# arc from (302,622) curving down through bottom center to (722,622)
# approximate with PIL arc: circle centered (512, 462) radius 360 -> endpoints
# at angles ~122..58 going through 90 (PIL angles: 0=east, 90=down since y down)
arc_cx, arc_cy, arc_r = 512, 462, 360
d.arc(
    [sc(arc_cx - arc_r), sc(arc_cy - arc_r), sc(arc_cx + arc_r), sc(arc_cy + arc_r)],
    start=32,
    end=148,
    fill=white,
    width=sc(46),
)
# fluke tips: short diagonal strokes at arc ends, pointing outward-down
# endpoints of arc at angle 32 and 148 deg
import math

for deg, sign in ((32, 1), (148, -1)):
    ex = sc(arc_cx + arc_r * math.cos(math.radians(deg)))
    ey = sc(arc_cy + arc_r * math.sin(math.radians(deg)))
    thick_line((ex, ey), (ex + sign * sc(48), ey + sc(48)), 44)

img = img.resize((S, S), Image.LANCZOS)
img.save("assets/logo-1024.png")
print("saved assets/logo-1024.png")

# --- tray template (macOS): black silhouette with alpha ---
T = 512  # logical 512 for @3x-scale generation; resize outputs separately
TS = T * 2
timg = Image.new("RGBA", (TS, TS), (0, 0, 0, 0))
td = ImageDraw.Draw(timg)
black = (0, 0, 0, 255)


def tsc(v):
    return v * 2


def tthick(p1, p2, width):
    td.line([p1, p2], fill=black, width=tsc(width))
    r = tsc(width) / 2
    for p in (p1, p2):
        td.ellipse([p[0] - r, p[1] - r, p[0] + r, p[1] + r], fill=black)


rr = tsc(60)
rcx, rcy = tsc(256), tsc(170)
td.ellipse([rcx - rr, rcy - rr, rcx + rr, rcy + rr], outline=black, width=tsc(38))
tthick((tsc(256), tsc(230)), (tsc(256), tsc(392)), 38)
tthick((tsc(191), tsc(246)), (tsc(321), tsc(246)), 34)
acx, acy, ar = 256, 231, 180
td.arc(
    [tsc(acx - ar), tsc(acy - ar), tsc(acx + ar), tsc(acy + ar)],
    start=32,
    end=148,
    fill=black,
    width=tsc(38),
)
for deg, sign in ((32, 1), (148, -1)):
    ex = acx + ar * math.cos(math.radians(deg))
    ey = acy + ar * math.sin(math.radians(deg))
    tthick((tsc(ex), tsc(ey)), (tsc(ex) + sign * tsc(36), tsc(ey) + tsc(36)), 34)

for size, name in ((36, "statusTemplate.png"), (72, "statusTemplate@2x.png"), (108, "statusbar_template_3x.png")):
    timg.resize((size, size), Image.LANCZOS).save(f"src-tauri/icons/tray/macos/{name}")
    print(f"saved tray {name}")
