#!/usr/bin/env python3
"""Raster every Pointcast icon from Pip (scripts/pip.py): a chubby white P whose stem is a speech-bubble tail, with
two dark eyes, on a blue tile.

pip.py holds the drawing (the body path, the eyes per state, the tile colours) and writes it as SVG; this script
draws the same geometry with Pillow, so building the icons needs nothing but Pillow and numpy. The macOS app icon sits
on Apple's grid (824 px body on a 1024 canvas, continuous corners, soft shadow); Windows gets the tile full bleed; the
tray gets one-colour marks per state: a black template for the macOS menu bar, white for a dark Windows taskbar and
ink for a light one. Small sizes are drawn for their size, not just shrunk: under 28 px Pip fills more of the tile and
his eyes grow, and at 16 to 24 px the catch-lights in the eyes are left out (they would be a smudge)."""
import importlib.util
import math
from pathlib import Path

import numpy as np
from PIL import Image, ImageChops, ImageDraw, ImageFilter

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("pip", HERE / "pip.py")
pip = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(pip)

ROOT = HERE.parent
OUT = ROOT / "src-tauri" / "icons"
PUBLIC = ROOT / "public"
SS = 4  # supersampling for the macOS body shape
UNITS = 512  # pip.py draws on a 512 tile


# --- Paths -----------------------------------------------------------------------------------------------------------

def _arc(p0, p1, r, large, sweep, n=48):
    """An SVG endpoint arc of a circle (no rotation) as points after p0."""
    (x0, y0), (x1, y1) = p0, p1
    xp, yp = (x0 - x1) / 2, (y0 - y1) / 2
    lam = (xp * xp + yp * yp) / (r * r)
    if lam > 1:
        r *= math.sqrt(lam)
    num = r * r * r * r - r * r * yp * yp - r * r * xp * xp
    coef = math.sqrt(max(0.0, num / (r * r * yp * yp + r * r * xp * xp)))
    if large == sweep:
        coef = -coef
    cxp, cyp = coef * yp, -coef * xp
    cx, cy = cxp + (x0 + x1) / 2, cyp + (y0 + y1) / 2
    t0 = math.atan2((yp - cyp) / r, (xp - cxp) / r)
    t1 = math.atan2((-yp - cyp) / r, (-xp - cxp) / r)
    dt = t1 - t0
    if sweep and dt < 0:
        dt += 2 * math.pi
    if not sweep and dt > 0:
        dt -= 2 * math.pi
    return [(cx + r * math.cos(t0 + dt * i / n), cy + r * math.sin(t0 + dt * i / n)) for i in range(1, n + 1)]


def _quad(p0, p1, p2, n=32):
    return [tuple((1 - t) ** 2 * a + 2 * (1 - t) * t * b + t * t * c for a, b, c in zip(p0, p1, p2))
            for t in (i / n for i in range(1, n + 1))]


def flatten(d):
    """Absolute M, H, V, Q, A, Z path data (what pip.py writes) as a list of point lists, one per subpath."""
    tok = d.replace(",", " ").split()
    out, pts, at, i = [], [], (0.0, 0.0), 0

    def num():
        nonlocal i
        i += 1
        return float(tok[i - 1])

    cmd = None
    while i < len(tok):
        t = tok[i]
        if t[0].isalpha():
            cmd = t[0]
            if len(t) > 1:
                tok[i] = t[1:]
            else:
                i += 1
            if cmd == "Z":
                out.append(pts)
                pts = []
                continue
        if cmd == "M":
            if pts:
                out.append(pts)
            at = (num(), num())
            pts = [at]
        elif cmd == "H":
            at = (num(), at[1])
            pts.append(at)
        elif cmd == "V":
            at = (at[0], num())
            pts.append(at)
        elif cmd == "Q":
            c, e = (num(), num()), (num(), num())
            pts += _quad(at, c, e)
            at = e
        elif cmd == "A":
            r, _, _rot, large, sweep = num(), num(), num(), num(), num()
            e = (num(), num())
            pts += _arc(at, e, r, int(large), int(sweep))
            at = e
        else:
            raise ValueError(f"path command {cmd}")
    if pts:
        out.append(pts)
    return out


BODY = flatten(pip.BODY)[0]


# --- Drawing ---------------------------------------------------------------------------------------------------------

def hexc(h):
    return np.array([int(h[i:i + 2], 16) / 255 for i in (1, 3, 5)])


class Canvas:
    """A square of S pixels showing the 512-unit tile; group coordinates go through SHIFT and a zoom about the centre."""

    def __init__(self, S, zoom=1.0):
        self.S, self.k, self.zoom = S, S / UNITS, zoom
        self.rgb = np.zeros((S, S, 3))  # premultiplied
        self.a = np.zeros((S, S))
        ys, xs = np.mgrid[0:S, 0:S]
        self.tx, self.ty = (xs + 0.5) / self.k, (ys + 0.5) / self.k  # tile units
        self.gx = (self.tx - 256) / zoom + 256 - pip.SHIFT[0]  # group units
        self.gy = (self.ty - 256) / zoom + 256 - pip.SHIFT[1]

    def g2px(self, x, y):
        return (((x + pip.SHIFT[0] - 256) * self.zoom + 256) * self.k, ((y + pip.SHIFT[1] - 256) * self.zoom + 256) * self.k)

    def mask(self, polys, dy=0):
        m = Image.new("L", (self.S, self.S), 0)
        d = ImageDraw.Draw(m)
        for p in polys:
            d.polygon([self.g2px(x, y + dy) for x, y in p], fill=255)
        return np.asarray(m, dtype=np.float64) / 255

    def over(self, color, alpha):
        alpha = np.clip(alpha, 0, 1)
        if np.ndim(color) == 1:
            color = np.broadcast_to(color, (self.S, self.S, 3))
        self.rgb = color * alpha[..., None] + self.rgb * (1 - alpha[..., None])
        self.a = alpha + self.a * (1 - alpha)

    def image(self):
        a = np.maximum(self.a, 1e-9)[..., None]
        rgb = np.where(self.a[..., None] > 0, self.rgb / a, 0)
        px = np.dstack([rgb, self.a[..., None]]) * 255
        return Image.fromarray(np.clip(px + 0.5, 0, 255).astype(np.uint8), "RGBA")


def ramp(t, stops):
    """Colour (or opacity) at t along SVG gradient stops [(offset, value)], values as arrays."""
    t = np.clip(t, 0, 1)
    out = np.zeros(t.shape + np.shape(stops[0][1]))
    for (o0, v0), (o1, v1) in zip(stops, stops[1:]):
        sel = (t >= o0) & (t <= o1)
        f = ((t - o0) / max(o1 - o0, 1e-9))[sel]
        out[sel] = np.asarray(v0) + (np.asarray(v1) - np.asarray(v0)) * (f[..., None] if np.ndim(v0) else f)
    return out


def linear_bbox(x, y, box, x1, y1, x2, y2):
    """t of an objectBoundingBox linear gradient at user points."""
    bx, by, bw, bh = box
    u, v = (x - bx) / bw, (y - by) / bh
    dx, dy = x2 - x1, y2 - y1
    return ((u - x1) * dx + (v - y1) * dy) / (dx * dx + dy * dy)


def radial_bbox(x, y, box, cx, cy, r):
    bx, by, bw, bh = box
    return np.sqrt(((x - bx - cx * bw) / (r * bw)) ** 2 + ((y - by - cy * bh) / (r * bh)) ** 2)


def eye_centre(state):
    """The point between the two eyes."""
    kind, parts = pip.EYES[state]
    if kind == "fill":
        return sum(p[0] for p in parts) / 2, sum(p[1] for p in parts) / 2
    return sum(p[0] + p[4] for p in parts) / 4, sum(p[1] + p[3] for p in parts) / 4


def eye_grow(state, scale):
    """Grow the eyes about the point between them, so the gap between them grows with them (a gap closes first)."""
    ex, ey = eye_centre(state)
    return lambda x, y: (ex + (x - ex) * scale, ey + (y - ey) * scale)


def eye_polys(state, scale=1.0):
    """The eyes as polygons (pills) or arcs (done), grown by `scale` (small sizes)."""
    kind, parts = pip.EYES[state]
    g = eye_grow(state, scale)
    if kind == "fill":
        out = []
        for cx, cy, w, h in parts:
            out += flatten(pip.pill(*g(cx, cy), w * scale, h * scale))
        return out, kind
    return [g(a, b) + g(c, d) + g(e, f) for a, b, c, d, e, f in parts], kind


def brush(d, pts, w, fill):
    """A round-capped stroke of width w along densely sampled points: a round brush stamped at each one."""
    for x, y in pts:
        d.ellipse([x - w / 2, y - w / 2, x + w / 2, y + w / 2], fill=fill)


def stroke_mask(cv, parts, width, scale=1.0):
    """The done eyes: quadratic arcs with round caps, as a mask."""
    m = Image.new("L", (cv.S, cv.S), 0)
    d = ImageDraw.Draw(m)
    w = width * scale * cv.k * cv.zoom
    for x0, y0, qx, qy, x1, y1 in parts:
        brush(d, [cv.g2px(x, y) for x, y in [(x0, y0)] + _quad((x0, y0), (qx, qy), (x1, y1), 400)], w, 255)
    return np.asarray(m, dtype=np.float64) / 255


def tile_art(S, state="idle", tile_mask=None, zoom=1.0, eye=1.0, highlights=True, tile="blue"):
    """Pip on his tile at S pixels, as pip.icon() draws it. tile_mask: the tile's shape (default the 116/512 rounded
    square); zoom grows Pip about the tile's centre; eye grows the eyes; highlights draws the catch-lights."""
    cv = Canvas(S, zoom)
    if tile_mask is None:
        m = Image.new("L", (S * 2, S * 2), 0)
        ImageDraw.Draw(m).rounded_rectangle([0, 0, S * 2 - 1, S * 2 - 1], 116 / UNITS * S * 2, fill=255)
        tile_mask = np.asarray(m.resize((S, S), Image.Resampling.LANCZOS), dtype=np.float64) / 255
    top, bot = (hexc(c) for c in pip.TILES[tile])
    full = (0, 0, UNITS, UNITS)
    cv.over(ramp(linear_bbox(cv.tx, cv.ty, full, 0.2, 0, 0.8, 1), [(0, top), (1, bot)]), tile_mask)
    white = np.ones(3)
    cv.over(white, tile_mask * ramp(radial_bbox(cv.tx, cv.ty, full, 0.3, 0.15, 0.75), [(0, 0.28), (1, 0.0)]))
    # The soft shadow under Pip.
    sh = Image.fromarray((cv.mask([BODY], dy=16) * 255).astype(np.uint8))
    sh = np.asarray(sh.filter(ImageFilter.GaussianBlur(14 * cv.k * zoom)), dtype=np.float64) / 255
    cv.over(hexc("#0A1F66"), sh * 0.24 * tile_mask)
    body = cv.mask([BODY])
    xs, ys = [p[0] for p in BODY], [p[1] for p in BODY]
    bbox = (min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys))
    cv.over(ramp(linear_bbox(cv.gx, cv.gy, bbox, 0.25, 0, 0.65, 1),
                 [(0, hexc("#FFFFFF")), (0.7, hexc("#F4F6FA")), (1, hexc("#E1E6EF"))]), body)
    cv.over(hexc("#8E9AB5"), body * ramp(radial_bbox(cv.gx, cv.gy, full, 0.62, 0.9, 0.7), [(0, 0.28), (1, 0.0)]))
    in_rect = (cv.gx >= 128) & (cv.gx <= 398) & (cv.gy >= 104) & (cv.gy <= 354)
    cv.over(white, body * in_rect * ramp(radial_bbox(cv.gx, cv.gy, (128, 104, 270, 250), 0.32, 0.2, 0.32), [(0, 1.0), (1, 0.0)]))
    polys, kind = eye_polys(state, eye)
    eyes = cv.mask(polys) if kind == "fill" else stroke_mask(cv, polys, pip.DONE_STROKE, eye)
    cv.over(hexc(pip.EYE_INK), eyes)
    if highlights:
        for x, y, r in pip.HIGHLIGHTS.get(state, []):
            hl = Image.new("L", (S, S), 0)
            rr = r * cv.k * zoom * eye
            # The catch-light keeps its place in the eye as the eye grows.
            px, py = cv.g2px(*eye_grow(state, eye)(x, y))
            ImageDraw.Draw(hl).ellipse([px - rr, py - rr, px + rr, py + rr], fill=255)
            cv.over(white, np.asarray(hl, dtype=np.float64) / 255 * 0.9)
    return cv.image()


def master(px):
    """The resolution to draw at for a target of px pixels: big enough to anti-alias, then Lanczos down."""
    return min(2048, max(1024, px * 4))


def size_class(px):
    """(zoom, eye scale, highlights) for an icon drawn at px pixels."""
    if px >= 48:
        return 1.0, 1.0, True
    if px >= 28:
        return 1.05, 1.1, True
    return 1.12, 1.25, False


def pip_tile(px, state="idle", tile_mask=None):
    zoom, eye, hl = size_class(px)
    S = master(px)
    im = tile_art(S, state, tile_mask, zoom, eye, hl)
    return im if S == px else im.resize((px, px), Image.Resampling.LANCZOS)


# --- One-colour marks ------------------------------------------------------------------------------------------------

def mark_mask(state, H, eye=1.0):
    """Pip's body with the eyes cut out, the body `H` pixels tall, as an L image cropped to the body."""
    xs, ys = [p[0] for p in BODY], [p[1] for p in BODY]
    x0, y0, w, h = min(xs), min(ys), max(xs) - min(xs), max(ys) - min(ys)
    k = H / h
    W = math.ceil(w * k)
    m = Image.new("L", (W, H), 0)
    d = ImageDraw.Draw(m)
    P = lambda x, y: ((x - x0) * k, (y - y0) * k)  # noqa: E731
    d.polygon([P(x, y) for x, y in BODY], fill=255)
    polys, kind = eye_polys(state, eye)
    if kind == "fill":
        for p in polys:
            d.polygon([P(x, y) for x, y in p], fill=0)
    else:
        sw = pip.DONE_STROKE * eye * k
        for x0_, y0_, qx, qy, x1_, y1_ in polys:
            brush(d, [P(x, y) for x, y in [(x0_, y0_)] + _quad((x0_, y0_), (qx, qy), (x1_, y1_), 400)], sw, 0)
    return m


def mark_eye(px):
    """Eye growth for a mark whose body is px tall: the cut-out eyes close up below about 24 px otherwise."""
    return 1.0 if px >= 32 else 1.15 if px >= 24 else 1.3


def mark_png(state, box, height, color):
    """A box x box image with the mark `height` pixels tall, centred, in `color`."""
    big = mark_mask(state, height * 16, mark_eye(height))
    small = big.resize((max(1, round(big.width / 16)), height), Image.Resampling.LANCZOS)
    out = Image.new("RGBA", (box, box), (0, 0, 0, 0))
    out.paste(Image.new("RGBA", small.size, color), ((box - small.width) // 2, (box - height) // 2), small)
    return out


TRAY_SIZES = (16, 20, 24, 32)  # the tray's small icon at 100, 125, 150 and 200 %
TRAY_COLOURS = {"white": (255, 255, 255, 255), "ink": tuple(int(pip.MARK_INK[i:i + 2], 16) for i in (1, 3, 5)) + (255,)}


def tray():
    out = OUT / "tray"
    out.mkdir(exist_ok=True)
    for old in out.glob("*.png"):
        old.unlink()
    for state in pip.STATES:
        # macOS template: black with alpha only; the menu bar tints it. 36 px is the 18 pt slot at 2x.
        mark_png(state, 36, 32, (0, 0, 0, 255)).save(out / f"mac-{state}.png")
        # Windows: white on a dark taskbar, ink on a light one, one image per scale so none is resampled by the shell.
        for colour, rgba in TRAY_COLOURS.items():
            for px in TRAY_SIZES:
                mark_png(state, px, px, rgba).save(out / f"win-{state}-{colour}-{px}.png")


# --- App icons -------------------------------------------------------------------------------------------------------

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


def mac_icon(size=1024):
    """Apple's grid: an 824/1024 body with continuous corners and a soft shadow, Pip's tile filling the body."""
    body = round(824 * size / 1024)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    mask = squircle(body)
    at = ((size - body) // 2, (size - body) // 2)
    if size >= 64:
        shadow = Image.new("RGBA", (size, size), (0, 0, 0, 0))
        shadow.paste(Image.new("RGBA", (body, body), (0, 0, 0, 90)), (at[0], at[1] + size // 80), mask)
        canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(size / 64)))
    S = master(body)
    zoom, eye, hl = size_class(size)
    art = tile_art(S, "idle", np.asarray(squircle(S), dtype=np.float64) / 255, zoom, eye, hl)
    art = art.resize((body, body), Image.Resampling.LANCZOS)
    face = Image.new("RGBA", (body, body), (0, 0, 0, 0))
    face.paste(art, (0, 0), ImageChops.multiply(art.getchannel("A"), mask))
    canvas.alpha_composite(face, at)
    return canvas


def win_icon(size=256):
    """Windows draws icons full bleed: Pip's rounded tile edge to edge, no shadow."""
    return pip_tile(size)


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
