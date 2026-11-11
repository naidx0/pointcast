// Run: node --test src/lib/line.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ATTACK_MS,
  RELEASE_MS,
  VOICE_BASELINE,
  VOICE_DX,
  loopsPath,
  marginPath,
  noise,
  pathPoints,
  restPath,
  smoothLevel,
  trackFloor,
  voiceLevel,
  voicePath,
} from "./line.ts";

const box = { w: 240, h: 36 };
const speech = Array.from({ length: 40 }, (_, i) => 0.3 + 0.6 * Math.abs(Math.sin(i * 0.7)));
const inside = (d, b) => pathPoints(d).every((p) => p.x >= 0 && p.x <= b.w && p.y >= 0 && p.y <= b.h);
const base = Math.round(box.h * VOICE_BASELINE);

test("the voice trace is the same for the same levels (no randomness)", () => {
  assert.deepEqual(voicePath(speech, box, { first: 7, shift: 0.4 }), voicePath(speech, box, { first: 7, shift: 0.4 }));
  assert.equal(noise(42), noise(42));
});

test("a silent level history is a flat line on the baseline, with the pen at the right end", () => {
  for (const lv of [[], Array(30).fill(0)]) {
    const { d, tip } = voicePath(lv, box, { shift: 0.5, live: 0 });
    assert.deepEqual([...new Set(pathPoints(d).map((p) => p.y))], [base]);
    assert.deepEqual(tip, { x: box.w - 4, y: base });
  }
});

test("speech lifts the trace off the line, and louder speech lifts it further", () => {
  const spread = (lv) => {
    const ys = pathPoints(voicePath(lv, box).d).map((p) => p.y);
    return Math.max(...ys) - Math.min(...ys);
  };
  assert.ok(spread(speech) > 14, "the tallest strokes use most of the box");
  assert.ok(spread(speech.map((v) => v * 0.3)) < spread(speech));
});

test("the voice trace stays inside its box at any loudness and any shift, even above 1", () => {
  for (const lv of [speech, Array(80).fill(1), Array(80).fill(3), [1, 0, 1, 0, 1, 0], [NaN, -1, 2]])
    for (const shift of [0, 0.3, 0.99]) {
      const { d, tip } = voicePath(lv, box, { shift, live: 1 });
      assert.ok(inside(d, box), d.slice(0, 80));
      assert.ok(tip.x <= box.w && tip.y >= 0 && tip.y <= box.h);
    }
});

test("broad strokes, not a seismograph: at most one lift per two strokes' width", () => {
  const loud = Array(60).fill(0.9);
  const ys = pathPoints(voicePath(loud, box).d).map((p) => p.y);
  let peaks = 0;
  for (let i = 1; i < ys.length - 1; i++) if (ys[i] < ys[i - 1] && ys[i] <= ys[i + 1]) peaks++;
  assert.ok(peaks <= Math.ceil(box.w / (2 * VOICE_DX)) + 1, `${peaks} lifts across ${box.w} px`);
  assert.ok(peaks >= 4, "and it does write");
});

test("the paper moves continuously: just before a stroke lands the line is where it is just after", () => {
  const a = 0.8;
  const before = voicePath(speech, box, { first: 3, shift: 0.999, live: a });
  const after = voicePath([...speech, a], box, { first: 3, shift: 0, live: a });
  assert.ok(Math.abs(before.tip.x - after.tip.x) < 0.2 && Math.abs(before.tip.y - after.tip.y) < 0.2, JSON.stringify([before.tip, after.tip]));
  // A stroke keeps its height as the paper moves: the newest stroke, half a step later, is half a stroke to the left.
  const p = pathPoints(voicePath(speech.slice(0, 30), box, { first: 0 }).d);
  const q = pathPoints(voicePath(speech.slice(0, 30), box, { first: 0, shift: 0.5 }).d);
  const newest = p[p.length - 1];
  const moved = q[q.length - 4];
  assert.equal(moved.y, newest.y);
  // Half a stroke, give or take the hand's unevenness (at most 18 % of a stroke either way).
  assert.ok(Math.abs(newest.x - moved.x - VOICE_DX / 2) <= 0.18 * VOICE_DX + 0.11);
});

test("the drawn level rises in about 60 ms, falls in about 200 ms, and never overshoots", () => {
  const up = smoothLevel(0, 1, ATTACK_MS);
  assert.ok(up > 0.6 && up < 0.65, String(up));
  const down = smoothLevel(1, 0, ATTACK_MS);
  assert.ok(down > 0.7, "falling is slower than rising: " + down);
  assert.ok(Math.abs(smoothLevel(1, 0, RELEASE_MS) - Math.exp(-1)) < 0.01);
  let v = 0;
  for (let t = 0; t < 1000; t += 16) {
    v = smoothLevel(v, 0.5, 16);
    assert.ok(v <= 0.5);
  }
  assert.ok(Math.abs(v - 0.5) < 0.01);
  assert.equal(smoothLevel(0.3, NaN, 16) < 0.3, true, "a bad level is silence");
  assert.equal(smoothLevel(0.3, 1, 0), 0.3, "no time, no change");
});

test("quiet speech is plainly visible, silence is flat, and the mapping only grows with the voice", () => {
  const floor = 0.2;
  assert.equal(voiceLevel(floor, floor), 0, "the room is silence");
  assert.equal(voiceLevel(floor + 0.05, floor), 0, "a little over the room is still silence");
  assert.equal(voiceLevel(0, floor), 0);
  assert.ok(voiceLevel(floor + 0.1, floor) >= 0.25, "quiet speech lifts the pen a quarter of the way at least");
  assert.ok(voiceLevel(floor + 0.3, floor) > 0.6);
  assert.equal(voiceLevel(1, floor), 1);
  let last = 0;
  for (let r = 0; r <= 1.0001; r += 0.01) {
    const v = voiceLevel(r, floor);
    assert.ok(v >= last - 1e-9 && v <= 1, `${r}: ${v}`);
    last = v;
  }
});

test("the room's floor follows the level down fast and up slowly", () => {
  assert.ok(trackFloor(0.2, 0.05, 150) < 0.11, "a quieter room is found in a moment");
  assert.ok(trackFloor(0.2, 0.7, 1000) < 0.3, "a second of speech is not taken for the room");
  assert.equal(trackFloor(0.2, 0.7, 0), 0.2);
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
