// Run: node --test src/lib/bars.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { BARS_DOT, BARS_FLOOR, BARS_STEP_MS, barCount, barHeight, barsStart, barsStep } from "./bars.ts";

test("the bar count is odd, so one bar is in the middle, and never below one", () => {
  for (const w of [0, 1, 5, 40, 100, 301, NaN]) {
    const n = barCount(w);
    assert.ok(n >= 1 && n % 2 === 1, `${w} -> ${n}`);
  }
  assert.ok(barCount(200) > barCount(100));
});

test("silence is a row of dots: every bar at the shortest height, none taller", () => {
  let s = barsStart();
  for (let i = 0; i < 60; i++) s = barsStep(s, 0, 16, 11);
  for (let i = 0; i < 11; i++) assert.ok(barHeight(i, s.trail, 21) <= BARS_DOT + 0.05, `bar ${i}`);
});

test("a voice lifts the middle most and the ends least, and older levels move outward", () => {
  let s = barsStart();
  for (let i = 0; i < 40; i++) s = barsStep(s, 0.9, 16, 11);
  const mid = barHeight(0, s.trail, 21);
  const far = barHeight(10, s.trail, 21);
  assert.ok(mid > 0.8 && far < mid * 0.5, `mid ${mid} far ${far}`);
  // Then silence: the loud bars travel outward (slot i holds what slot 0 held i steps ago).
  for (let i = 0; i < 4; i++) s = barsStep(s, 0, BARS_STEP_MS, 11);
  assert.ok(s.trail[0] < s.trail[3], "the newest is already quieter than the one 3 steps out");
});

test("the trail takes one sample per step, however the frames fall, and never grows past what is shown", () => {
  let s = barsStart();
  s = barsStep(s, 0.5, BARS_STEP_MS - 1, 9);
  assert.equal(s.trail.length, 0);
  s = barsStep(s, 0.5, 1, 9);
  assert.equal(s.trail.length, 1);
  s = barsStep(s, 0.5, 10_000, 9);
  assert.equal(s.trail.length, 9);
});

test("bad numbers read as silence, never NaN", () => {
  const s = barsStep(barsStart(), NaN, NaN, 5);
  assert.ok(Number.isFinite(s.amp) && s.amp >= BARS_FLOOR);
  assert.ok(Number.isFinite(barHeight(0, [NaN], 5)));
});
