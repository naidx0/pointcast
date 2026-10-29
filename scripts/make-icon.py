#!/usr/bin/env python3
"""Draw every Pointcast icon from code: the macOS app icon on Apple's grid (824 px body on a
1024 canvas, continuous corners, soft shadow), a full-bleed Windows icon, and the tray glyphs
(a monochrome template for the macOS menu bar, a colour icon for the Windows tray)."""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "src-tauri" / "icons"
SS = 4  # supersampling for smooth edges


def squircle(size, n=5.0):
    """Superellipse mask, the shape of a macOS icon body."""
    big = size * SS
    m = Image.new("L", (big, big), 0)
    px = m.load()
    r = big / 2
    for y in range(big):
        dy = abs((y + 0.5 - r) / r) ** n
        if dy > 1:
            continue
        half = r * (1 - dy) ** (1 / n)
        for x in range(int(r - half), int(r + half)):
            px[x, y] = 255
    return m.resize((size, size), Image.Resampling.LANCZOS)


def gradient(size, top, bottom):
    g = Image.new("RGBA", (1, size))
    for y in range(size):
        t = y / max(1, size - 1)
        g.putpixel((0, y), tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)) + (255,))
    return g.resize((size, size))


def mark(size, cell_frac=0.235, gap_frac=0.05, radius_frac=0.06, fills=(255, 255, 255, 255), rec=False):
    """The four-square mark, centred on a transparent square of `size`."""
    big = size * SS
    im = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    cell, gap, rad = big * cell_frac, big * gap_frac, big * radius_frac
    x0 = (big - (2 * cell + gap)) / 2
    fills = fills if isinstance(fills[0], tuple) else (fills,) * 4
    for i in range(4):
        cx = x0 + (i % 2) * (cell + gap)
        cy = x0 + (i // 2) * (cell + gap)
        if rec and i == 3:
            d.ellipse([cx, cy, cx + cell, cy + cell], fill=fills[i])
        else:
            d.rounded_rectangle([cx, cy, cx + cell, cy + cell], rad, fill=fills[i])
    return im.resize((size, size), Image.Resampling.LANCZOS)


def mac_icon(size=1024):
    body = 824 * size // 1024
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    mask = squircle(body)
    shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    sh = Image.new("RGBA", (body, body), (0, 0, 0, 110))
    shadow.paste(sh, ((size - body) // 2, (size - body) // 2 + size // 80), mask)
    shadow = shadow.filter(ImageFilter.GaussianBlur(size / 64))
    canvas.alpha_composite(shadow)
    fill = gradient(body, (52, 52, 58), (18, 18, 21))
    face = Image.new("RGBA", (body, body), (0, 0, 0, 0))
    face.paste(fill, (0, 0), mask)
    # A hairline of light along the top edge, as on Apple's own dark icons.
    rim = Image.new("RGBA", (body, body), (255, 255, 255, 0))
    inner = squircle(body - 4)
    edge = Image.new("L", (body, body), 0)
    edge.paste(mask, (0, 0))
    hole = Image.new("L", (body, body), 0)
    hole.paste(inner, (2, 3))
    edge = Image.composite(Image.new("L", (body, body), 0), edge, hole)
    rim.putalpha(edge.point(lambda v: v * 38 // 255))
    face.alpha_composite(rim)
    face.alpha_composite(mark(body, cell_frac=0.25, gap_frac=0.06, radius_frac=0.07))
    canvas.alpha_composite(face, ((size - body) // 2, (size - body) // 2))
    return canvas


def win_icon(size=256):
    """Windows draws icons full bleed: a rounded tile edge to edge, no shadow."""
    big = size * SS
    tile = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    m = Image.new("L", (big, big), 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, big - 1, big - 1], big * 0.22, fill=255)
    tile.paste(gradient(big, (52, 52, 58), (18, 18, 21)), (0, 0), m)
    tile = tile.resize((size, size), Image.Resampling.LANCZOS)
    tile.alpha_composite(mark(size, cell_frac=0.29, gap_frac=0.07, radius_frac=0.08))
    return tile


def tray():
    out = OUT / "tray"
    out.mkdir(exist_ok=True)
    # macOS template: black with alpha only; the menu bar tints it for light, dark and selected.
    ink = (0, 0, 0, 255)
    soft = (0, 0, 0, 150)
    mark(36, 0.36, 0.08, 0.09, (ink, soft, soft, ink)).save(out / "mac.png")
    mark(36, 0.36, 0.08, 0.09, (ink, soft, soft, ink), rec=True).save(out / "mac-rec.png")
    icon = win_icon(32)
    icon.save(out / "win.png")
    rec = icon.copy()
    d = ImageDraw.Draw(rec)
    d.ellipse([19, 19, 31, 31], fill=(255, 255, 255, 255))
    d.ellipse([21, 21, 29, 29], fill=(232, 17, 35, 255))
    rec.save(out / "win-rec.png")


def main():
    mac = mac_icon(1024)
    mac.save(OUT / "icon.icns", format="ICNS")
    iconset = OUT / "icon.iconset"
    iconset.mkdir(exist_ok=True)
    for s in (16, 32, 128, 256, 512):
        mac.resize((s, s), Image.Resampling.LANCZOS).save(iconset / f"icon_{s}x{s}.png")
        mac.resize((s * 2, s * 2), Image.Resampling.LANCZOS).save(iconset / f"icon_{s}x{s}@2x.png")
    win = win_icon(1024)
    win.resize((512, 512), Image.Resampling.LANCZOS).save(OUT / "icon.png")
    win.resize((512, 512), Image.Resampling.LANCZOS).save(OUT / "app-icon.png")
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
    icos = [win_icon(s) for s in (16, 24, 32, 48, 64, 128, 256)]
    icos[-1].save(OUT / "icon.ico", format="ICO", sizes=[(i.width, i.height) for i in icos], append_images=icos[:-1])
    tray()
    print("wrote Pointcast icons")


if __name__ == "__main__":
    main()
