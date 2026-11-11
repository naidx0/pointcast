// Run: node --test src/lib/redact.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { REDACT_EDGE, REDACT_FILL, edgeWidth, hiddenNote, overlaps, redact, redactBounds } from "./redact.ts";

// A picture where every pixel is different, so a pixel left alone is told apart from one written over.
function picture(w, h) {
  const px = new Uint8ClampedArray(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    px[i * 4] = (i * 7) % 251;
    px[i * 4 + 1] = (i * 13) % 241;
    px[i * 4 + 2] = (i * 29) % 239;
    px[i * 4 + 3] = 200 + (i % 50);
  }
  return px;
}

const at = (px, w, x, y) => Array.from(px.subarray((y * w + x) * 4, (y * w + x) * 4 + 4));

for (const s of [1, 1.5, 2]) {
  test(`at scale ${s} every pixel the area touches is the fill or the edge, opaque, and every other pixel is untouched`, () => {
    const w = Math.round(120 * s);
    const h = Math.round(80 * s);
    // In logical px, off the pixel grid once scaled, so the area cuts pixels in part.
    const logical = { x: 10.3, y: 7.7, w: 41.45, h: 23.2 };
    const r = { x: logical.x * s, y: logical.y * s, w: logical.w * s, h: logical.h * s };
    const before = picture(w, h);
    const px = before.slice();
    redact(px, w, h, [r], s);
    const b = redactBounds(r, w, h);
    assert.ok(b);
    // The bounds cover every pixel the area overlaps, even in part.
    assert.equal(b.x0, Math.floor(r.x));
    assert.equal(b.y0, Math.floor(r.y));
    assert.equal(b.x1, Math.ceil(r.x + r.w));
    assert.equal(b.y1, Math.ceil(r.y + r.h));
    const e = edgeWidth(s);
    let fill = 0;
    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        const got = at(px, w, x, y);
        const inArea = x >= b.x0 && x < b.x1 && y >= b.y0 && y < b.y1;
        if (!inArea) {
          assert.deepEqual(got, at(before, w, x, y), `outside (${x},${y}) changed`);
          continue;
        }
        const rim = x < b.x0 + e || x >= b.x1 - e || y < b.y0 + e || y >= b.y1 - e;
        assert.deepEqual(got, [...(rim ? REDACT_EDGE : REDACT_FILL), 255], `inside (${x},${y})`);
        if (!rim) fill++;
      }
    }
    assert.ok(fill > 0, "the area has a filled middle");
  });
}

test("an area off the picture is clipped, one wholly off it changes nothing, and an empty one is skipped", () => {
  const w = 40;
  const h = 30;
  const before = picture(w, h);
  const px = before.slice();
  redact(px, w, h, [{ x: -10, y: -5, w: 20, h: 12 }, { x: 50, y: 50, w: 10, h: 10 }, { x: 5, y: 5, w: 0, h: 9 }], 1);
  assert.deepEqual(redactBounds({ x: -10, y: -5, w: 20, h: 12 }, w, h), { x0: 0, y0: 0, x1: 10, y1: 7 });
  assert.equal(redactBounds({ x: 50, y: 50, w: 10, h: 10 }, w, h), null);
  assert.equal(redactBounds({ x: 5, y: 5, w: 0, h: 9 }, w, h), null);
  for (let y = 0; y < h; y++)
    for (let x = 0; x < w; x++) if (x >= 10 || y >= 7) assert.deepEqual(at(px, w, x, y), at(before, w, x, y));
  assert.deepEqual(at(px, w, 4, 3), [...REDACT_FILL, 255]);
});

test("the caption counts the hidden areas in plain words", () => {
  assert.equal(hiddenNote(0), "");
  assert.equal(hiddenNote(1), "1 area hidden");
  assert.equal(hiddenNote(3), "3 areas hidden");
});

test("overlap is true only for rectangles that share some area", () => {
  assert.equal(overlaps({ x: 0, y: 0, w: 10, h: 10 }, { x: 9, y: 9, w: 5, h: 5 }), true);
  assert.equal(overlaps({ x: 0, y: 0, w: 10, h: 10 }, { x: 10, y: 0, w: 5, h: 5 }), false);
  assert.equal(overlaps({ x: 0, y: 0, w: 10, h: 10 }, { x: 2, y: 2, w: 1, h: 1 }), true);
});
