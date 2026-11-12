"""Pip, the Pointcast character: a chubby P whose stem is a speech-bubble tail, with two eyes that carry the app's
states. This file is the drawing's source of truth; make-icon.py rasters every icon from it.

Run it on its own to write the SVGs (app tile, one-colour marks, the four states) next to it, for design work:
    python scripts/pip.py <folder>
"""
import os
import sys

NL = chr(10)

# Pip's body: a chubby P whose stem is the speech-bubble tail, in a 512 tile's units.
BODY = ("M128 216 A112 112 0 0 1 240 104 H286 A112 112 0 0 1 398 216 V240 A112 112 0 0 1 286 352 H262 "
        "Q228 352 228 386 V392 A50 50 0 0 1 128 392 Z")

# The icon is centred on the tile by moving the drawing this much.
SHIFT = (-7, -17)

STATES = ("idle", "listen", "think", "done")


def pill(cx, cy, w, h):
    r = w / 2
    return (f"M{cx - r:.1f} {cy - h / 2 + r:.1f} A{r} {r} 0 0 1 {cx + r:.1f} {cy - h / 2 + r:.1f} "
            f"V{cy + h / 2 - r:.1f} A{r} {r} 0 0 1 {cx - r:.1f} {cy + h / 2 - r:.1f} Z")


# The eyes as numbers, so a raster can draw them: pills (cx, cy, w, h) or, for done, two arcs (x0, y0, cx, cy, x1, y1).
EYES = {
    "idle": ("fill", [(252, 222, 38, 70), (318, 222, 38, 70)]),
    "listen": ("fill", [(248, 226, 46, 82), (318, 226, 46, 82)]),
    "think": ("fill", [(232, 200, 36, 56), (294, 200, 36, 56)]),
    "done": ("stroke", [(226, 236, 250, 206, 274, 236), (296, 236, 320, 206, 344, 236)]),
}
DONE_STROKE = 20
EYE_INK = "#141A2B"

# The small white catch-lights in each eye (cx, cy, r); none while thinking or done.
HIGHLIGHTS = {
    "idle": [(258, 198, 7), (324, 198, 7)],
    "listen": [(256, 196, 7), (326, 196, 7)],
}


def eyes(state):
    """The eye shapes for a state, as (path, kind). kind 'fill' or 'stroke'."""
    kind, parts = EYES[state]
    if kind == "fill":
        return " ".join(pill(*p) for p in parts), kind
    return " ".join(f"M{a} {b} Q{c} {d} {e} {f}" for a, b, c, d, e, f in parts), kind


TILES = {
    "blue": ("#4F8BFF", "#1F5BFF"),
}
MARK_INK = "#141414"


def icon(tile="blue", state="idle"):
    top, bot = TILES[tile]
    d, kind = eyes(state)
    if kind == "fill":
        eye_svg = f'<path d="{d}" fill="{EYE_INK}"/>'
        hl = "".join(f'<circle cx="{x}" cy="{y}" r="{r}" fill="#FFFFFF" opacity="0.9"/>' for x, y, r in HIGHLIGHTS.get(state, []))
    else:
        eye_svg = f'<path d="{d}" fill="none" stroke="{EYE_INK}" stroke-width="{DONE_STROKE}" stroke-linecap="round"/>'
        hl = ""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
<defs>
  <linearGradient id="t" x1="0.2" y1="0" x2="0.8" y2="1"><stop offset="0" stop-color="{top}"/><stop offset="1" stop-color="{bot}"/></linearGradient>
  <radialGradient id="glow" cx="0.3" cy="0.15" r="0.75"><stop offset="0" stop-color="#FFFFFF" stop-opacity="0.28"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/></radialGradient>
  <linearGradient id="b" x1="0.25" y1="0" x2="0.65" y2="1"><stop offset="0" stop-color="#FFFFFF"/><stop offset="0.7" stop-color="#F4F6FA"/><stop offset="1" stop-color="#E1E6EF"/></linearGradient>
  <radialGradient id="shade" cx="0.62" cy="0.9" r="0.7"><stop offset="0" stop-color="#8E9AB5" stop-opacity="0.28"/><stop offset="1" stop-color="#8E9AB5" stop-opacity="0"/></radialGradient>
  <radialGradient id="shine" cx="0.32" cy="0.2" r="0.32"><stop offset="0" stop-color="#FFFFFF" stop-opacity="1"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0"/></radialGradient>
  <clipPath id="bc"><path d="{BODY}"/></clipPath>
  <filter id="sh" x="-30%" y="-30%" width="160%" height="160%"><feGaussianBlur stdDeviation="14"/></filter>
</defs>
<rect width="512" height="512" rx="116" fill="url(#t)"/>
<rect width="512" height="512" rx="116" fill="url(#glow)"/>
<g transform="translate({SHIFT[0]} {SHIFT[1]})">
  <path d="{BODY}" fill="#0A1F66" opacity="0.24" filter="url(#sh)" transform="translate(0 16)"/>
  <path d="{BODY}" fill="url(#b)"/>
  <g clip-path="url(#bc)"><rect width="512" height="512" fill="url(#shade)"/><rect x="128" y="104" width="270" height="250" fill="url(#shine)"/></g>
  {eye_svg}{hl}
</g>
</svg>
"""


def mark(color="#FFFFFF", state="idle"):
    """One-colour mark: the body with the eyes cut out."""
    d, kind = eyes(state)
    if kind == "fill":
        return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="118 94 290 358" width="290" height="358">'
                f'<path d="{BODY} {d}" fill="{color}" fill-rule="evenodd"/></svg>{NL}')
    return (f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="118 94 290 358" width="290" height="358">'
            f'<defs><mask id="m"><rect x="0" y="0" width="512" height="512" fill="#fff"/>'
            f'<path d="{d}" fill="none" stroke="#000" stroke-width="{DONE_STROKE}" stroke-linecap="round"/></mask></defs>'
            f'<path d="{BODY}" fill="{color}" mask="url(#m)"/></svg>{NL}')


def main(folder):
    def write(name, text):
        with open(os.path.join(folder, name), "wb") as f:
            f.write(text.encode("utf-8"))

    for t in TILES:
        write(f"pip-icon-{t}.svg", icon(t))
    for s in STATES:
        write(f"pip-state-{s}.svg", icon("blue", s))
        write(f"pip-mark-{s}-white.svg", mark("#FFFFFF", s))
        write(f"pip-mark-{s}-ink.svg", mark(MARK_INK, s))
    print("ok")


if __name__ == "__main__":
    main(sys.argv[1] if len(sys.argv) > 1 else ".")
