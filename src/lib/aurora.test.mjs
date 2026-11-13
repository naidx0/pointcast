// Run: node --experimental-strip-types --test src/lib/aurora.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { AURORA_FLOOR, AURORA_SPEED, BANDS, auroraSpeed, auroraStart, auroraStep, isLight, lens, parseInk, waveAt } from "./aurora.ts";

const grid = (n) => Array.from({ length: n + 1 }, (_, i) => i / n);
const times = [0, 0.7, 3.3, 41, 600, 86400];
const amps = [0, 0.12, 0.5, 1];

test("every lens is zero-thick at both ends", () => {
  for (const b of BANDS) for (const t of times) for (const a of amps) for (const phase of [0, 5, 1234.5]) {
    for (const u of [0, 1]) {
      const [, th] = lens(u, t, a, phase, b.f, b.sp, b.k);
      assert.ok(Math.abs(th) < 1e-9, `thickness ${th} at u=${u}`);
    }
  }
});

test("centre +- thickness never leaves -0.96..0.96, even at level 1", () => {
  for (const b of BANDS) for (const t of times) for (const a of [...amps, 5]) for (const phase of [0, 2, 9, 777]) {
    for (const u of grid(200)) {
      const [c, th] = lens(u, t, a, phase, b.f, b.sp, b.k);
      assert.ok(th >= 0);
      assert.ok(c - th >= -0.96 - 1e-12 && c + th <= 0.96 + 1e-12, `${c} +- ${th}`);
    }
  }
});

test("the swell wave is zero at both ends", () => {
  for (const t of times) for (const u of [0, 1]) assert.ok(Math.abs(waveAt(u, t, 1, 3.3)) < 1e-9);
});

test("the phase advances by dt x speed, not by total time x speed", () => {
  let s = auroraStart();
  for (let i = 0; i < 100; i++) s = auroraStep(s, 0.5, 16);
  const before = s;
  const next = auroraStep(s, 0.5, 16);
  const dt = 16 * AURORA_SPEED;
  assert.ok(Math.abs(next.phase - before.phase - (dt / 1000) * auroraSpeed(next.amp)) < 1e-12);
  assert.ok(Math.abs(next.drift - before.drift - dt / 1000) < 1e-12);
});

test("a level change after 600 s moves the phase no more than after 1 s", () => {
  const run = (seconds) => {
    let s = auroraStart();
    for (let i = 0; i < seconds * 60; i++) s = auroraStep(s, 0.3, 1000 / 60);
    const p = s.phase;
    s = auroraStep(s, 1, 16); // the voice jumps to full
    return s.phase - p;
  };
  const early = run(1);
  const late = run(600);
  assert.ok(late <= early * 1.05 + 1e-9, `late ${late} vs early ${early}`);
  assert.ok(late < 0.1);
});

test("the level is eased: 60 ms in, 260 ms out, never under the floor", () => {
  let s = auroraStart();
  assert.equal(s.amp, AURORA_FLOOR);
  const up = auroraStep({ ...s, amp: 0 + AURORA_FLOOR }, 1, 60 / AURORA_SPEED);
  assert.ok(Math.abs(up.amp - (AURORA_FLOOR + (1 - AURORA_FLOOR) * (1 - Math.exp(-1)))) < 0.01);
  const down = auroraStep({ phase: 0, drift: 0, amp: 1 }, 0, 260 / AURORA_SPEED);
  assert.ok(Math.abs(down.amp - (AURORA_FLOOR + (1 - AURORA_FLOOR) * Math.exp(-1))) < 0.01);
  for (let i = 0; i < 500; i++) s = auroraStep(s, 0, 16);
  assert.equal(s.amp, AURORA_FLOOR);
});

test("a bad number reads as silence", () => {
  for (const bad of [NaN, Infinity, -Infinity, undefined, null, "0.9", {}]) {
    const s = auroraStep(auroraStart(), bad, 16);
    assert.equal(s.amp, AURORA_FLOOR);
    assert.ok(Number.isFinite(s.phase) && Number.isFinite(s.drift));
  }
  const s = auroraStep({ phase: NaN, drift: NaN, amp: NaN }, 0.5, NaN);
  assert.ok([s.phase, s.drift, s.amp].every(Number.isFinite));
  assert.ok(Number.isFinite(waveAt(0.5, NaN, NaN, NaN)));
  const [c, th] = lens(0.5, NaN, NaN, NaN, 1, 1, 0);
  assert.ok(Number.isFinite(c) && Number.isFinite(th));
});

test("ink: parsed from CSS, light ink adds light, dark ink mixes normally", () => {
  assert.deepEqual(parseInk("#fff"), [255, 255, 255]);
  assert.deepEqual(parseInk(" #101424 "), [16, 20, 36]);
  assert.deepEqual(parseInk("rgba(255, 255, 255, 0.9)"), [255, 255, 255]);
  assert.equal(parseInk("var(--x)"), null);
  assert.ok(isLight([255, 255, 255]));
  assert.ok(!isLight([16, 20, 36]));
});

test("the chosen wave: an unknown or missing value reads as Aurora", async () => {
  const { waveStyleOf, WAVE_STYLES } = await import("./aurora.ts");
  assert.deepEqual([...WAVE_STYLES], ["pen", "breath", "aurora"]);
  for (const s of WAVE_STYLES) assert.equal(waveStyleOf(s), s);
  for (const bad of [undefined, null, "", "Pen", "sparkles", 3, {}]) assert.equal(waveStyleOf(bad), "aurora");
});
