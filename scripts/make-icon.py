#!/usr/bin/env python3
"""Draw every Pointcast icon from code: a cursor landing in the middle of an orange click spark, as a sticker with a hard shadow.

The macOS app icon sits on Apple's grid (824 px body on a 1024 canvas, continuous corners, soft
shadow); Windows gets a full-bleed rounded tile; the tray gets a monochrome template for the macOS
menu bar and a colour icon for the Windows tray. Small sizes are drawn heavier, not just shrunk:
thin lines vanish at 16 px, so each size class has its own line weight. Every size is centred by the
weight of the drawing, not only its bounding box, so the mark does not read as off-centre when it is small."""
import math
from pathlib import Path
from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"
SS = 4  # supersampling for the tile shapes

INK = (10, 10, 10, 255)
PAPER = (255, 255, 255, 255)
ORANGE = (255, 138, 31, 255)

# The mark in its own units: a four-point spark centred on (400, 400) and a cursor whose tip sits at its centre.
SPARK_AT, SPARK_R, SPARK_PINCH = (400, 400), 360, 0.26
ARROW = [(0, 0), (0, 520), (150, 400), (250, 610), (340, 566), (238, 358), (430, 358)]
ARROW_AT, ARROW_SCALE = (400, 400), 0.8
SPARK_LINE, ARROW_LINE = 32, 48  # outline growth on every side
SPARK_SHADOW, ARROW_SHADOW = 40, 34  # hard sticker shadow, down and to the right
SOFT = 14  # rounds every corner of every shape by about this many units (0 keeps them sharp)
GLINTS = [(740, 120, 74), (90, 800, 52)]  # two small extra sparks (x, y, radius), drawn behind the cursor like the big one
MAC_FRAC = 0.78  # how much of the macOS icon body the mark's longer side fills at 48 px and up


def quad(p0, p1, p2, n=30):
    return [tuple((1 - t) ** 2 * a + 2 * (1 - t) * t * b + t * t * c for a, b, c in zip(p0, p1, p2))
            for t in (i / n for i in range(n + 1))]


def spark_points(cx=None, cy=None, r=None):
    if cx is None:
        (cx, cy), r = SPARK_AT, SPARK_R
    a = r * SPARK_PINCH
    tips = [(cx, cy - r), (cx + r, cy), (cx, cy + r), (cx - r, cy)]
    bends = [(cx + a, cy - a), (cx + a, cy + a), (cx - a, cy + a), (cx - a, cy - a)]
    pts = []
    for i in range(4):
        pts += quad(tips[i], bends[i], tips[(i + 1) % 4])[:-1]
    return pts


def arrow_points():
    return [(ARROW_AT[0] + x * ARROW_SCALE, ARROW_AT[1] + y * ARROW_SCALE) for x, y in ARROW]


def shape_mask(size, pts, grow, K, P, shift=0):
    """A polygon grown by `grow` on every side with round joins, shifted down and right by `shift`."""
    m = Image.new("L", (size, size), 0)
    d = ImageDraw.Draw(m)
    q = [P(x + shift, y + shift) for x, y in pts]
    d.polygon(q, fill=255)
    if grow:
        r = grow * K
        d.line(q + [q[0]], fill=255, width=round(2 * r), joint="curve")
        for x, y in q:
            d.ellipse([x - r, y - r, x + r, y + r], fill=255)
    return m


def soften(m, radius):
    """Round the corners of a mask: blur it, then cut it back to a hard edge."""
    if not radius:
        return m
    return m.filter(ImageFilter.GaussianBlur(radius)).point(lambda v: 255 if v > 127 else 0)


def mark(weight=1.0, shadow=True, style="sticker", ink=INK, paper=PAPER, accent=ORANGE):
    """The mark at high resolution, cropped to what it draws. style: "sticker" (orange spark, paper
    cursor, ink line and shadow), "line" (ink outline only, for a menu bar template) or "solid" (as
    "line" with the cursor filled in, for the recording state)."""
    K = 3  # pixels per mark unit
    size = 1000 * K
    P = lambda x, y: (x * K, y * K)  # noqa: E731
    sg, ag = SPARK_LINE * weight, ARROW_LINE * weight
    spark, arrow = spark_points(), arrow_points()
    mask = lambda pts, grow, shift=0: soften(shape_mask(size, pts, grow, K, P, shift), SOFT * K)  # noqa: E731
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    if style == "sticker":
        off = math.sqrt(weight)
        out.paste(ink, (0, 0), mask(spark, sg, SPARK_SHADOW * off) if shadow else Image.new("L", (size, size), 0))
        out.paste(ink, (0, 0), mask(spark, sg))
        out.paste(accent, (0, 0), mask(spark, 0))
        for gx, gy, gr in GLINTS if weight < 1.5 else []:  # at 16 to 24 px the glints would be noise
            glint = spark_points(gx, gy, gr)
            out.paste(ink, (0, 0), mask(glint, sg * 0.8))
            out.paste(accent, (0, 0), mask(glint, 0))
        if shadow:
            out.paste(ink, (0, 0), mask(arrow, ag, ARROW_SHADOW * off))
        out.paste(ink, (0, 0), mask(arrow, ag))
        out.paste(paper, (0, 0), mask(arrow, 0))
    else:
        arrow_outer = mask(arrow, ag)
        spark_ring = ImageChops.subtract(ImageChops.subtract(mask(spark, sg), mask(spark, 0)), arrow_outer)
        arrow_ring = arrow_outer if style == "solid" else ImageChops.subtract(arrow_outer, mask(arrow, 0))
        out.paste(ink, (0, 0), ImageChops.add(spark_ring, arrow_ring))
    return out.crop(out.getbbox())


_cache = {}


def cut(px):
    """The mark drawn for a target size in pixels: heavier lines as it shrinks."""
    weight = 1.0 if px >= 96 else 1.2 if px >= 48 else 1.55 if px >= 28 else 1.6
    if weight not in _cache:
        _cache[weight] = mark(weight=weight)
    return _cache[weight]


def fit(im, box):
    """Scale so the longer side is `box` pixels."""
    s = box / max(im.size)
    return im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.Resampling.LANCZOS)


def optical_centre(im):
    """Halfway between the drawing's box centre and its centre of mass, so a heavy side does not read as off-centre."""
    n = 64
    a = im.getchannel("A").resize((n, n), Image.Resampling.BOX).point(lambda v: 255 if v > 127 else 0)
    px = a.load()
    pts = [(x, y) for y in range(n) for x in range(n) if px[x, y]]
    mx = sum(x + 0.5 for x, _ in pts) / len(pts) / n
    my = sum(y + 0.5 for _, y in pts) / len(pts) / n
    return im.width * (0.5 + mx) / 2, im.height * (0.5 + my) / 2


def centred(canvas, im, cx, cy):
    ox, oy = optical_centre(im)
    canvas.alpha_composite(im, (round(cx - ox), round(cy - oy)))


def squircle(size, n=5.0):
    """Superellipse mask, the shape of a macOS icon body."""
    big = size * SS
    m = Image.new("L", (big, big), 0)
    d = ImageDraw.Draw(m)
    r = big / 2
    for y in range(big):
        dy = abs((y + 0.5 - r) / r) ** n
        if dy > 1:
            continue
        half = r * (1 - dy) ** (1 / n)
        d.line([(r - half, y), (r + half - 1, y)], fill=255)
    return m.resize((size, size), Image.Resampling.LANCZOS)


def gradient(size, top, bottom):
    g = Image.new("RGBA", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        g.putpixel((0, y), tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    return g.resize((size, size))


FACE_TOP, FACE_BOTTOM = (255, 255, 255), (236, 236, 240)


def mac_icon(size=1024):
    body = round(824 * size / 1024)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    mask = squircle(body)
    at = ((size - body) // 2, (size - body) // 2)
    if size >= 64:
        shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        shadow.paste(Image.new("RGBA", (body, body), (0, 0, 0, 90)), (at[0], at[1] + size // 80), mask)
        canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(size / 64)))
    face = Image.new("RGBA", (body, body), (0, 0, 0, 0))
    face.paste(gradient(body, FACE_TOP, FACE_BOTTOM), (0, 0), mask)
    # A faint edge so the white body holds its shape on a white desktop.
    e = max(1, body // 200)
    inner = Image.new("L", (body, body), 0)
    inner.paste(squircle(body - 2 * e), (e, e))
    face.paste((0, 0, 0, 34), (0, 0), ImageChops.subtract(mask, inner))
    centred(face, fit(cut(size), body * (MAC_FRAC if size >= 48 else MAC_FRAC + 0.04 if size >= 28 else MAC_FRAC + 0.12)), body / 2, body / 2)
    canvas.alpha_composite(face, at)
    return canvas


def win_icon(size=256, detail=None):
    """Windows draws icons full bleed: a rounded white tile edge to edge, a thin grey rim, no shadow.
    `detail` draws the mark as for another size (the tray icon is 32 px but shown at 16)."""
    big = size * SS
    m = Image.new("L", (big, big), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, big - 1, big - 1], big * 0.22, fill=255)
    rim = max(SS, round(big * 0.012))
    hole = Image.new("L", (big, big), 0)
    ImageDraw.Draw(hole).rounded_rectangle([rim, rim, big - 1 - rim, big - 1 - rim], big * 0.22 - rim, fill=255)
    tile = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    tile.paste((200, 200, 206, 255), (0, 0), m)
    tile.paste(gradient(big, FACE_TOP, FACE_BOTTOM), (0, 0), hole)
    tile = tile.resize((size, size), Image.Resampling.LANCZOS)
    px = detail or size
    centred(tile, fit(cut(px), size * (0.70 if px >= 48 else 0.76 if px >= 28 else 0.84)), size / 2, size / 2)
    return tile


def tray():
    out = OUT / "tray"
    out.mkdir(exist_ok=True)
    # macOS template: black with alpha only; the menu bar tints it for light, dark and selected.
    # 36 px is the 18 pt menu bar slot at 2x; the mark is drawn as a line, and with the cursor filled while recording.
    for name, style in (("mac.png", "line"), ("mac-rec.png", "solid")):
        g = Image.new("RGBA", (36, 36), (0, 0, 0, 0))
        centred(g, fit(mark(weight=1.6, shadow=False, style=style, ink=(0, 0, 0, 255)), 34), 18, 18)
        g.save(out / name)
    icon = win_icon(32, detail=16)
    icon.save(out / "win.png")
    rec = icon.copy()
    d = ImageDraw.Draw(rec)
    d.ellipse([19, 19, 31, 31], fill=(255, 255, 255, 255))
    d.ellipse([21, 21, 29, 29], fill=(232, 17, 35, 255))
    rec.save(out / "win-rec.png")


def main():
    mac = {s: mac_icon(s) for s in (16, 32, 64, 128, 256, 512, 1024)}
    mac[1024].save(OUT / "icon.icns", format="ICNS", append_images=[mac[s] for s in (16, 32, 64, 128, 256, 512)])
    iconset = OUT / "icon.iconset"
    iconset.mkdir(exist_ok=True)
    for s in (16, 32, 128, 256, 512):
        mac[s].save(iconset / f"icon_{s}x{s}.png")
        mac[s * 2].save(iconset / f"icon_{s}x{s}@2x.png")
    win512 = win_icon(512)
    win512.save(OUT / "icon.png")
    win512.save(OUT / "app-icon.png")
    for name, s in {
        "32x32.png": 32,
        "64x64.png": 64,
        "128x128.png": 128,
        "128x128@2x.png": 256,
        "Square30x30Logo.png": 30,
        "Square44x44Logo.png": 44,
        "Square71x71Logo.png": 71,
        "Square89x89Logo.png": 89,
        "Square107x107Logo.png": 107,
        "Square142x142Logo.png": 142,
        "Square150x150Logo.png": 150,
        "Square284x284Logo.png": 284,
        "Square310x310Logo.png": 310,
        "StoreLogo.png": 50,
    }.items():
        win_icon(s).save(OUT / name)
    # Windows picks the frame nearest the size it draws (24 in the taskbar at 100%, 30 at 125%, 36 at 150%).
    icos = [win_icon(s) for s in (16, 20, 24, 30, 32, 36, 40, 48, 64, 128, 256)]
    icos[-1].save(OUT / "icon.ico", format="ICO", sizes=[(i.width, i.height) for i in icos], append_images=icos[:-1])
    tray()
    # The pictures the setup assistant and Settings show.
    mac[512].save(PUBLIC / "icon-mac.png")
    win_icon(256).save(PUBLIC / "icon-win.png")
    print("wrote Pointcast icons")


if __name__ == "__main__":
    main()
