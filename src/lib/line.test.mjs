// Run: node --test src/lib/line.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ATTACK_MS,
  RELEASE_MS,
  VOICE_BASELINE,
  VOICE_DX,
  VOICE_STEP_MS,
  VOICE_STROKE_STEPS,
  VOICE_TOP,
  loopsPath,
  marginPath,
  meterLevel,
  meterStart,
  meterStep,
  noise,
  pathPoints,
  restPath,
  smoothLevel,
  trackFloor,
  voiceLevel,
  voicePath,
  waveLift,
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
  assert.ok(spread(speech) > 12, "a voice uses most of the room above the line: " + spread(speech));
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

test("a steady voice is one smooth wave: no jump between neighbouring points, one crest per two strokes", () => {
  const steady = Array(160).fill(0.7);
  const up = base - 4;
  for (const shift of [0, 0.5]) {
    const pts = pathPoints(voicePath(steady, box, { first: 11, shift, live: 0.7 }).d);
    for (let i = 1; i < pts.length; i++)
      assert.ok(Math.abs(pts[i].y - pts[i - 1].y) <= 0.4 * up, `jump of ${pts[i].y - pts[i - 1].y} at ${pts[i].x}`);
  }
  const ys = [];
  for (let t = 0; t < 400; t += 0.25) ys.push(waveLift(t, 0.7));
  let peaks = 0;
  for (let i = 1; i < ys.length - 1; i++) if (ys[i] > ys[i - 1] && ys[i] >= ys[i + 1]) peaks++;
  assert.equal(peaks, 400 / (2 * VOICE_STROKE_STEPS), "one crest every two strokes, no ripples");
  // Strokes as wide as before, the paper as slow: one stroke of VOICE_DX px every 200 ms.
  assert.equal(VOICE_STEP_MS * VOICE_STROKE_STEPS, 200);
  assert.equal(VOICE_DX, 13);
});

test("no randomness in the voice: the same level writes the same height in every stroke", () => {
  const crests = [];
  for (let k = 0; k < 20; k++) crests.push(waveLift(k * 2 * VOICE_STROKE_STEPS + VOICE_STROKE_STEPS / 2, 0.6));
  assert.ok(Math.max(...crests) - Math.min(...crests) < 1e-9, crests.join(" "));
});

test("the paper moves continuously: just before a sample lands the line is where it is just after", () => {
  const a = 0.8;
  const before = voicePath(speech, box, { first: 3, shift: 0.999, live: a });
  const after = voicePath([...speech, a], box, { first: 3, shift: 0, live: a });
  assert.ok(Math.abs(before.tip.x - after.tip.x) < 0.2 && Math.abs(before.tip.y - after.tip.y) < 0.2, JSON.stringify([before.tip, after.tip]));
});

test("the pen answers the voice within one frame: a step in the level moves the newest point at once", () => {
  const quiet = Array(40).fill(0);
  const frame = 16;
  // One frame after the voice starts, from silence: the drawn level is already about half way up…
  const target = meterLevel(meterStart(), 0.5);
  const env = smoothLevel(0, target, frame);
  assert.ok(env > 0.45 * target, `one frame: ${env} of ${target}`);
  // …and the pen, wherever the wave is, has lifted off the line by a visible amount.
  const up = base - 4;
  for (const shift of [0, 0.25, 0.5, 0.75]) {
    const was = voicePath(quiet, box, { first: 9, shift, live: 0 }).tip;
    const now = voicePath(quiet, box, { first: 9, shift, live: env }).tip;
    assert.ok(was.y - now.y >= 0.1 * env * up - 0.1 && was.y - now.y > 0.5, `shift ${shift}: ${was.y} to ${now.y}`);
  }
  // Within one sample's time it is all but there, and it falls over a few samples, not at once.
  assert.ok(smoothLevel(0, 1, VOICE_STEP_MS) > 0.85);
  assert.ok(smoothLevel(1, 0, VOICE_STEP_MS) > 0.6);
});

test("the drawn level eases and never overshoots", () => {
  assert.ok(Math.abs(smoothLevel(0, 1, ATTACK_MS) - (1 - Math.exp(-1))) < 0.01);
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

test("honest to the voice: a quiet one moves the line a fifth of its room, a loud one stays under the top, the mapping only grows", () => {
  const m = { floor: 0.02 };
  const up = base - 4;
  const lift = (raw) => {
    const lv = Array(40).fill(meterLevel(m, raw));
    const ys = pathPoints(voicePath(lv, box, { live: lv[0] }).d).map((p) => p.y);
    return (base - Math.min(...ys)) / up;
  };
  assert.ok(lift(0.08) >= 0.2, "quiet: " + lift(0.08));
  assert.ok(lift(1) < 0.95 && meterLevel(m, 1) <= VOICE_TOP, "loud: " + lift(1));
  // Normal speech, 8 to 16 dB over the room (0.2 to 0.4 on the engine's scale), uses roughly 30 to 80 % of the room.
  assert.ok(meterLevel(m, 0.22) >= 0.3 && meterLevel(m, 0.42) <= 0.82, `${meterLevel(m, 0.22)} ${meterLevel(m, 0.42)}`);
  assert.equal(meterLevel(m, 0.02), 0, "the room is silence");
  assert.equal(meterLevel(m, 0.045), 0, "a dB over the room is still silence");
  let last = 0;
  for (let r = 0; r <= 1.0001; r += 0.01) {
    const v = meterLevel(m, r);
    assert.ok(v >= last - 1e-9 && v <= VOICE_TOP, `${r}: ${v}`);
    last = v;
  }
  assert.equal(voiceLevel(0.3, 0.02), meterLevel(m, 0.3));
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

// The meter over a room: 50 ms frames, the room at 0.1 (about -46 dBFS) with its own flutter, a voice over it.
const run = (m, levels) => {
  const out = [];
  for (const r of levels) {
    m = meterStep(m, r, 50);
    out.push(meterLevel(m, r));
  }
  return { m, out };
};
const room = (n) => Array.from({ length: n }, (_, i) => 0.1 + ((i * 7) % 5) * 0.004);
const talk = (n, loud) => Array.from({ length: n }, (_, i) => (i % 6 < 4 ? 0.1 + loud + (i % 3) * 0.02 : 0.11));

test("silence is flat: the room's flutter never lifts the pen, before speech or after it", () => {
  const { m, out } = run(meterStart(), room(80));
  assert.deepEqual(out.slice(10), out.slice(10).map(() => 0));
  const after = run(run(m, talk(60, 0.3)).m, room(40)).out;
  assert.deepEqual(after.slice(4), after.slice(4).map(() => 0));
});

test("a loud word does not squash the quiet words after it, and quiet words stay smaller than loud ones", () => {
  const { m } = run(run(meterStart(), room(40)).m, [0.9, 0.9, 0.9, 0.9]);
  const soft = run(m, talk(60, 0.1)).out.filter((v, i) => i % 6 < 4);
  assert.ok(Math.min(...soft) > 0.3, `soft after loud ${Math.min(...soft)}`);
  const loud = run(m, talk(60, 0.4)).out.filter((v, i) => i % 6 < 4);
  assert.ok(Math.max(...soft) < Math.min(...loud), "a quiet voice draws lower than a loud one");
});

test("on a speech-like level the line keeps moving: off the line most of the time, never frozen while the voice changes", () => {
  // The engine's level at 30 Hz: a second of room, 3.3 s of syllables at 5 Hz between 0.05 and 0.4 with a 300 ms pause,
  // then room again. The panel draws at 60 frames a second, as Line.tsx does.
  const level = (ms) => {
    if (ms < 1000 || ms >= 4300) return 0.02;
    const t = ms - 1000;
    if (t > 1500 && t < 1800) return 0.03;
    const syl = 0.5 - 0.5 * Math.cos((2 * Math.PI * 5 * t) / 1000);
    return 0.05 + 0.35 * syl * (0.6 + 0.4 * Math.sin(t / 370));
  };
  const up = base - 4;
  let m = meterStart();
  let env = 0;
  let acc = 0;
  const hist = [];
  let speech = 0;
  let off = 0;
  let still = 0;
  let longest = 0;
  let lastY = null;
  let quietAfter = [];
  for (let t = 0; t < 5600; t += 16) {
    const raw = level(Math.floor(t / 33) * 33);
    m = meterStep(m, raw, 16);
    env = smoothLevel(env, meterLevel(m, raw), 16);
    acc += 16;
    while (acc >= VOICE_STEP_MS) {
      acc -= VOICE_STEP_MS;
      hist.push(env);
    }
    const first = Math.max(0, hist.length - 120);
    const tip = voicePath(hist.slice(first), box, { first, shift: acc / VOICE_STEP_MS, live: env }).tip;
    const talking = t >= 1000 && t < 4300 && !(t > 2500 && t < 2800);
    if (talking) {
      speech++;
      if (base - tip.y > 0.15 * up) off++;
      if (lastY != null && Math.abs(tip.y - lastY) < 0.05) longest = Math.max(longest, (still += 16));
      else still = 0;
    }
    if (t > 5000) quietAfter.push(tip.y);
    lastY = tip.y;
  }
  assert.ok(off / speech > 0.7, `off the line in ${Math.round((100 * off) / speech)} % of speech frames`);
  assert.ok(longest <= 150, `frozen for ${longest} ms while the voice changed`);
  assert.ok(quietAfter.every((y) => Math.abs(y - base) <= 0.5), "and flat again in the room after");
});

import { BREATH_IDLE, BREATH_LAYERS, breathAt, breathPath, breathStart, breathStep } from "./line.ts";

test("the breathing wave: quiet at both ends, inside its box, and still breathing at silence", () => {
  const box = { w: 300, h: 32 };
  let b = breathStart();
  for (let k = 0; k < 200; k++) b = breathStep(b, k % 7 < 4 ? 1 : 0.2, 16);
  for (let i = 0; i < BREATH_LAYERS.length; i++) {
    const ys = pathPoints(breathPath(box, b, i)).map((p) => p.y);
    assert.equal(ys[0], 16, "the left end sits on the middle");
    assert.equal(ys[ys.length - 1], 16, "so does the right end");
    assert.ok(ys.every((y) => y >= 3 && y <= 29), "never past the padding");
  }
  let quiet = breathStart();
  for (let k = 0; k < 300; k++) quiet = breathStep(quiet, 0, 16);
  assert.ok(Math.abs(quiet.amp - BREATH_IDLE) < 1e-3, "silence keeps a little height");
  assert.equal(breathStep(quiet, NaN, 16).amp <= quiet.amp, true, "a bad level reads as silence");
});

test("the breathing wave moves faster with the voice and its swell wanders along the line", () => {
  let a = breathStart(), z = breathStart();
  for (let k = 0; k < 60; k++) (a = breathStep(a, 1, 16)), (z = breathStep(z, 0, 16));
  assert.ok(a.phase > z.phase * 1.8, "a voice sets it moving faster");
  // Where the main wave's swell is highest, sampled over 30 s: it travels, it does not sit in one spot.
  const crest = (b) => {
    let best = 0, at = 0;
    for (let u = 0; u <= 1; u += 0.01) {
      const v = Math.abs(breathAt(u, b, 0));
      if (v > best) (best = v), (at = u);
    }
    return at;
  };
  const spots = [];
  let b = { ...breathStart(), amp: 0.8 };
  for (let s = 0; s < 30; s += 0.5) spots.push(crest({ ...b, phase: s * 3, drift: s }));
  assert.ok(Math.max(...spots) - Math.min(...spots) > 0.3, `crest spread ${Math.max(...spots) - Math.min(...spots)}`);
  // The layers travel different ways at unrelated speeds, so the picture does not repeat.
  assert.ok(BREATH_LAYERS.some((l) => l.speed < 0) && BREATH_LAYERS.some((l) => l.speed > 0));
});
