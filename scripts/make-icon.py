#!/usr/bin/env python3
"""Draw every Pointcast icon from code: the pointing glove with its sticker shadow and click spark.

The macOS app icon sits on Apple's grid (824 px body on a 1024 canvas, continuous corners, soft
shadow); Windows gets a full-bleed rounded tile; the tray gets a monochrome template for the macOS
menu bar and a colour icon for the Windows tray. Small sizes are drawn heavier, not just shrunk:
thin lines vanish at 16 px, so each size class has its own line weight and detail."""
import math
from pathlib import Path
from PIL import Image, ImageChops, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"
SS = 4  # supersampling for the tile shapes

INK = (10, 10, 10, 255)
PAPER = (255, 255, 255, 255)

# The glove, pointing straight up, in its own units (the fingertip's top edge is TIP).
TIP = (470, 150)
PARTS = [
    ("rect", 405, 150, 130, 470, 65),  # index finger
    ("rect", 522, 410, 128, 260, 64),  # first curled finger
    ("rect", 630, 452, 122, 220, 61),  # second curled finger
    ("rect", 395, 540, 357, 300, 130),  # back of the hand
    ("line", 452, 720, 330, 596, 118),  # thumb
]
ANGLE = 12  # degrees anticlockwise: the finger leans to the upper left like a pointer
OUTLINE, CREASE = 40, 26
SHADOW = 32  # hard sticker shadow, down and to the right
SPARK_GAP, SPARK_LEN, SPARK_SPREAD = 40, 72, 50


def cubic(p0, p1, p2, p3, n=40):
    return [tuple((1 - t) ** 3 * a + 3 * (1 - t) ** 2 * t * b + 3 * (1 - t) * t * t * c + t ** 3 * d
                  for a, b, c, d in zip(p0, p1, p2, p3)) for t in (i / n for i in range(n + 1))]


def quad(p0, p1, p2, n=30):
    return [tuple((1 - t) ** 2 * a + 2 * (1 - t) * t * b + t * t * c for a, b, c in zip(p0, p1, p2))
            for t in (i / n for i in range(n + 1))]


CREASES = [
    cubic((528, 452), (522, 520), (524, 580), (532, 626)),  # index meets the curled fingers
    cubic((640, 492), (634, 548), (636, 596), (642, 640)),  # between the curled fingers
    quad((404, 640), (430, 668), (462, 690)),  # thumb
]


def stroke(d, pts, width, fill):
    d.line(pts, fill=fill, width=round(width), joint="curve")
    r = width / 2
    for x, y in (pts[0], pts[-1]):
        d.ellipse([x - r, y - r, x + r, y + r], fill=fill)


def mark(weight=1.0, creases=True, spark=True, shadow=True, style="sticker", ink=INK, paper=PAPER):
    """The glove at high resolution, cropped to what it draws. style: "sticker" (paper glove, ink line
    and shadow), "line" (ink outline only, for a menu bar template) or "solid" (an ink silhouette)."""
    K = 3  # pixels per glove unit
    ox, oy = -260, -260  # glove units at pixel 0
    size = 1500 * K // 1
    P = lambda x, y: ((x - ox) * K, (y - oy) * K)  # noqa: E731
    w, c = OUTLINE * weight, CREASE * weight
    outer, inner, folds = (Image.new("L", (size, size), 0) for _ in range(3))
    do, di, df = ImageDraw.Draw(outer), ImageDraw.Draw(inner), ImageDraw.Draw(folds)
    for p in PARTS:
        if p[0] == "rect":
            _, x, y, pw, ph, r = p
            for d, e in ((do, w), (di, 0)):
                d.rounded_rectangle([*P(x - e, y - e), *P(x + pw + e, y + ph + e)], (r + e) * K, fill=255)
        else:
            _, x1, y1, x2, y2, lw = p
            for d, e in ((do, w), (di, 0)):
                stroke(d, [P(x1, y1), P(x2, y2)], (lw + 2 * e) * K, 255)
    if creases:
        for pts in CREASES:
            stroke(df, [P(*q) for q in pts], c * K, 255)
    face = ImageChops.subtract(inner, folds)
    glove = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    if style == "sticker":
        glove.paste(ink, (0, 0), outer)
        glove.paste(paper, (0, 0), face)
    elif style == "line":
        glove.paste(ink, (0, 0), ImageChops.add(ImageChops.subtract(outer, inner), ImageChops.multiply(inner, folds)))
    else:
        glove.paste(ink, (0, 0), ImageChops.subtract(outer, folds))
    tip = P(*TIP)
    glove = glove.rotate(ANGLE, resample=Image.Resampling.BICUBIC, center=tip)
    out = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    if shadow:
        sil = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        sil.paste(ink, (0, 0), outer)
        sil = sil.rotate(ANGLE, resample=Image.Resampling.BICUBIC, center=tip)
        off = round(SHADOW * math.sqrt(weight) * K)
        out.alpha_composite(sil, (off, off))
    if spark:
        d = ImageDraw.Draw(out)
        start, length = w + SPARK_GAP, SPARK_LEN * (0.75 + weight / 4)
        for a in (-SPARK_SPREAD, 0, SPARK_SPREAD):
            t = math.radians(-ANGLE + a)
            dx, dy = math.sin(t), -math.cos(t)
            stroke(d, [(tip[0] + dx * start * K, tip[1] + dy * start * K),
                       (tip[0] + dx * (start + length) * K, tip[1] + dy * (start + length) * K)], w * K, ink)
    out.alpha_composite(glove)
    return out.crop(out.getbbox())


_cache = {}


def cut(px):
    """The glove drawn for a target size in pixels: heavier lines and less detail as it shrinks."""
    if px >= 96:
        key = dict(weight=1.0)
    elif px >= 48:
        key = dict(weight=1.2)
    elif px >= 28:
        key = dict(weight=1.55, creases=False)
    else:  # 16-24 px: the spark would be noise, so the glove gets the whole tile
        key = dict(weight=1.9, creases=False, spark=False)
    k = tuple(sorted(key.items()))
    if k not in _cache:
        _cache[k] = mark(**key)
    return _cache[k]


def fit(im, box):
    """Scale so the longer side is `box` pixels."""
    s = box / max(im.size)
    return im.resize((max(1, round(im.width * s)), max(1, round(im.height * s))), Image.Resampling.LANCZOS)


def centred(canvas, im, cx, cy):
    canvas.alpha_composite(im, (round(cx - im.width / 2), round(cy - im.height / 2)))


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
    centred(face, fit(cut(size), body * (0.66 if size >= 48 else 0.72 if size >= 28 else 0.80)), body / 2, body / 2)
    canvas.alpha_composite(face, at)
    return canvas


def win_icon(size=256, detail=None):
    """Windows draws icons full bleed: a rounded white tile edge to edge, a thin grey rim, no shadow.
    `detail` draws the glove as for another size (the tray icon is 32 px but shown at 16)."""
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
    # 36 px is the 18 pt menu bar slot at 2x; the glove is drawn as a line, and solid while recording.
    for name, style in (("mac.png", "line"), ("mac-rec.png", "solid")):
        g = Image.new("RGBA", (36, 36), (0, 0, 0, 0))
        centred(g, fit(mark(weight=1.6, creases=False, shadow=False, style=style, ink=(0, 0, 0, 255)), 34), 18, 18)
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
