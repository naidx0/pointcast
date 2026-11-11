// Run: node --test src/lib/line.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { VOICE_BASELINE, VOICE_SLOTS, loopsPath, marginPath, noise, pathPoints, restPath, voicePath } from "./line.ts";

const box = { w: 352, h: 24 };
const speech = Array.from({ length: 90 }, (_, i) => 0.15 + 0.8 * Math.abs(Math.sin(i * 0.7)));
const inside = (d, b) => pathPoints(d).every((p) => p.x >= 0 && p.x <= b.w && p.y >= 0 && p.y <= b.h);

test("the voice trace is the same for the same levels (no randomness)", () => {
  assert.deepEqual(voicePath(speech, box, { first: 7 }), voicePath(speech, box, { first: 7 }));
  assert.equal(noise(42), noise(42));
});

test("a silent level history is a flat line on the baseline", () => {
  const { d, tip } = voicePath(Array(30).fill(0), box);
  const ys = new Set(pathPoints(d).map((p) => p.y));
  assert.deepEqual([...ys], [Math.round(box.h * VOICE_BASELINE)]);
  assert.equal(tip.y, Math.round(box.h * VOICE_BASELINE));
});

test("speech moves the trace off the middle, and louder speech moves it further", () => {
  const spread = (lv) => {
    const ys = pathPoints(voicePath(lv, box).d).map((p) => p.y);
    return Math.max(...ys) - Math.min(...ys);
  };
  assert.ok(spread(speech) > 6);
  assert.ok(spread(speech.map((v) => v * 0.3)) < spread(speech));
});

test("the voice trace stays inside its box at any loudness, even above 1", () => {
  for (const lv of [speech, Array(80).fill(1), Array(80).fill(3), [1, 0, 1, 0, 1, 0]]) {
    const { d, tip } = voicePath(lv, box);
    assert.ok(inside(d, box), d.slice(0, 80));
    assert.ok(tip.x <= box.w && tip.y >= 0 && tip.y <= box.h);
  }
});

test("the pen writes left to right and stops at the right margin once the history fills the line", () => {
  const short = voicePath(speech.slice(0, 10), box).tip.x;
  const full = voicePath(speech, box).tip.x;
  assert.ok(short < full);
  assert.equal(full, box.w - 4);
  assert.equal(voicePath(speech, box).d.slice(0, 5), "M0.8 ");
  // Scrolling keeps each sample's stroke: the same samples drawn from the same running index give the same shape.
  const a = voicePath(speech.slice(0, VOICE_SLOTS + 5), box, { first: 0 });
  const b = voicePath(speech.slice(5, VOICE_SLOTS + 5), box, { first: 5 });
  assert.equal(a.d, b.d);
});

test("the loops are deterministic, stay inside their box and actually loop (the pen goes back on itself)", () => {
  const lb = { w: 200, h: 24 };
  const one = loopsPath(lb);
  assert.deepEqual(one, loopsPath(lb));
  assert.ok(inside(one.d, lb));
  const xs = pathPoints(one.d).map((p) => p.x);
  let back = 0;
  for (let i = 1; i < xs.length; i++) if (xs[i] < xs[i - 1]) back++;
  assert.ok(back > 6 * 4, `went back ${back} times`);
  assert.ok(one.width <= lb.w);
});

test("the margin line runs down, turns, and ends at the caret", () => {
  const d = marginPath({ x: 0.75, top: 2, bottom: 100, end: 180 });
  assert.equal(d, marginPath({ x: 0.75, top: 2, bottom: 100, end: 180 }));
  const pts = pathPoints(d);
  assert.deepEqual(pts[0], { x: 0.8, y: 2 });
  assert.deepEqual(pts[pts.length - 1], { x: 180, y: 100 });
  assert.ok(inside(d, { w: 180, h: 100 }));
  // A caret at the margin (an empty line) is a straight drop with no turn.
  assert.ok(!marginPath({ x: 0.75, top: 2, bottom: 40, end: 0.75 }).includes("Q"));
});

test("the resting stroke is short and level", () => {
  const pts = pathPoints(restPath(0.75, 10));
  assert.equal(pts.length, 2);
  assert.equal(pts[0].y, pts[1].y);
  assert.ok(pts[1].x - pts[0].x <= 16.1);
});
