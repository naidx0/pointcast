// Hidden areas in a marked picture. The pixels under one are written over, not blended, so nothing of them is left
// in the PNG: every pixel the area touches, even in part, becomes the fill or the thin lighter edge, fully opaque.
export type Box = { x: number; y: number; w: number; h: number };

export const REDACT_FILL = [20, 20, 22] as const;
export const REDACT_EDGE = [96, 96, 102] as const;
export const REDACT_FILL_CSS = `rgb(${REDACT_FILL.join(",")})`;
export const REDACT_EDGE_CSS = `rgb(${REDACT_EDGE.join(",")})`;

/** The edge in picture px: one logical px, never less than one pixel. s = picture px per logical px. */
export const edgeWidth = (s: number) => Math.max(1, Math.round(s));

/** The whole pixels an area touches, clipped to a w x h picture; null when it covers none. */
export function redactBounds(r: Box, w: number, h: number): { x0: number; y0: number; x1: number; y1: number } | null {
  if (!(r.w > 0 && r.h > 0)) return null;
  const x0 = Math.max(0, Math.floor(r.x));
  const y0 = Math.max(0, Math.floor(r.y));
  const x1 = Math.min(w, Math.ceil(r.x + r.w));
  const y1 = Math.min(h, Math.ceil(r.y + r.h));
  return x1 > x0 && y1 > y0 ? { x0, y0, x1, y1 } : null;
}

/** Write every area into an RGBA buffer (w x h, row-major), in picture px. */
export function redact(px: Uint8ClampedArray | Uint8Array, w: number, h: number, rects: Box[], s: number) {
  const e = edgeWidth(s);
  for (const r of rects) {
    const b = redactBounds(r, w, h);
    if (!b) continue;
    for (let y = b.y0; y < b.y1; y++) {
      const rimRow = y < b.y0 + e || y >= b.y1 - e;
      for (let x = b.x0; x < b.x1; x++) {
        const c = rimRow || x < b.x0 + e || x >= b.x1 - e ? REDACT_EDGE : REDACT_FILL;
        const i = (y * w + x) * 4;
        px[i] = c[0];
        px[i + 1] = c[1];
        px[i + 2] = c[2];
        px[i + 3] = 255;
      }
    }
  }
}

/** The words the prompt carries for the picture: "1 area hidden", "3 areas hidden", or nothing. */
export const hiddenNote = (n: number) => (n > 0 ? `${n} ${n === 1 ? "area" : "areas"} hidden` : "");

export const overlaps = (a: Box, b: Box) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
