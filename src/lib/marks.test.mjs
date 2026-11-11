// Run: node --test src/lib/marks.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { captionMarks, empty, hist, kept, labelsFor, numbers, survives, toScreen, view } from "./marks.ts";

const step = (id, x, y, text = "") => ({ id, kind: "step", c: "#FF3B30", at: { x, y }, text });
const pin = (id, x, y, text = "") => ({ id, kind: "pin", c: "#FF3B30", at: { x, y }, text });
const run = (h, ...as) => as.reduce(hist, h);
const nums = (h, base = 0) => [...numbers(h.now, base).values()];

test("each step counts up by itself, and pins and steps share one sequence in the order they were placed", () => {
  const list = [step(1, 10, 10), step(2, 20, 10), pin(3, 30, 10, "too far left"), step(4, 40, 10), { id: 5, kind: "arrow", a: { x: 0, y: 0 }, b: { x: 5, y: 5 } }];
  assert.deepEqual([...numbers(list).entries()], [[1, 1], [2, 2], [3, 3], [4, 4]]);
  // A picture from History numbers on from the labels already drawn into it.
  assert.deepEqual([...numbers(list, 2).values()], [3, 4, 5, 6]);
});

test("deleting the third of five steps renumbers the later ones, and undo brings the step and its number back", () => {
  let h = run(empty(), ...[1, 2, 3, 4, 5].map((id) => ({ t: "add", s: step(id, id * 20, 50) })));
  assert.deepEqual(nums(h), [1, 2, 3, 4, 5]);
  h = hist(h, { t: "remove", id: 3 });
  assert.deepEqual(h.now.map((s) => s.id), [1, 2, 4, 5]);
  assert.deepEqual([...numbers(h.now).entries()], [[1, 1], [2, 2], [4, 3], [5, 4]], "the old 4 and 5 are now 3 and 4");
  h = hist(h, { t: "undo" });
  assert.deepEqual([...numbers(h.now).entries()], [[1, 1], [2, 2], [3, 3], [4, 4], [5, 5]]);
  h = hist(h, { t: "redo" });
  assert.deepEqual(nums(h), [1, 2, 3, 4]);
});

test("a step's label is set without an undo step of its own per keystroke, and steps without words send none", () => {
  let h = run(empty(), { t: "add", s: step(1, 10, 10) }, { t: "add", s: step(2, 20, 10) }, { t: "add", s: pin(3, 30, 10, "") });
  h = hist(h, { t: "text", id: 2, text: "open the menu" });
  assert.equal(h.past.length, 3, "typing is not an undo step");
  const labels = labelsFor([...h.now, { id: 9, kind: "text", c: "#fff", at: { x: 0, y: 0 }, text: " note " }], 0);
  assert.deepEqual(labels, [
    { n: 1, text: "", kind: "step" },
    { n: 2, text: "open the menu", kind: "step" },
    { n: 3, text: "pin 3" },
    { n: 4, text: "note" },
  ]);
});

test("the caption words steps as one ordered entry apart from the pins, as the prompt does", () => {
  const labels = [
    { n: 1, text: "too far left" },
    { n: 2, text: "open the menu", kind: "step" },
    { n: 3, text: "", kind: "step" },
    { n: 4, text: "save", kind: "step" },
    { n: 0, text: "1 area hidden" },
  ];
  assert.deepEqual(captionMarks(labels), ["pins: 1=too far left", "steps: 2=open the menu, 3, 4=save", "1 area hidden"]);
  assert.deepEqual(captionMarks([{ n: 1, text: "", kind: "step" }, { n: 2, text: "", kind: "step" }]), ["steps: 1, 2"]);
  assert.deepEqual(captionMarks([{ n: 0, text: "1 area hidden" }]), ["1 area hidden"]);
});

test("a hidden note is a note with kind note or stored without a kind, never pin 0", () => {
  const stored = [{ n: 1, text: "Save" }, { n: 0, text: "1 area hidden" }];
  const fresh = [{ n: 1, text: "Save" }, { n: 0, text: "2 areas hidden", kind: "note" }];
  assert.deepEqual(captionMarks(stored), ["pins: 1=Save", "1 area hidden"]);
  assert.deepEqual(captionMarks(fresh), ["pins: 1=Save", "2 areas hidden"]);
  for (const l of [stored, fresh]) assert.ok(!captionMarks(l).join(" ").includes("0="));
});

const box = { x: 100, y: 100, w: 400, h: 300 };
const marks = [
  step(1, 150, 150),
  step(2, 450, 350),
  pin(3, 50, 50, "outside"),
  { id: 4, kind: "arrow", c: "#f00", a: { x: 80, y: 120 }, b: { x: 160, y: 180 } },
  { id: 5, kind: "pen", c: "#f00", pts: [{ x: 10, y: 10 }, { x: 40, y: 30 }] },
  { id: 6, kind: "redact", c: "#f00", a: { x: 480, y: 380 }, b: { x: 560, y: 460 } },
  { id: 7, kind: "element", c: "#f00", r: { x: 90, y: 90, w: 40, h: 30 }, role: "button", name: "Save" },
  { id: 8, kind: "highlight", c: "#f00", pts: [{ x: 600, y: 600 }, { x: 700, y: 600 }] },
];

test("a crop keeps marks inside or across the new edge and drops marks wholly outside it", () => {
  assert.deepEqual(kept(marks, box).map((m) => m.id), [1, 2, 4, 6, 7]);
  assert.equal(survives(marks[3], box), true, "an arrow across the edge is kept and clipped by the picture edge");
  assert.equal(survives(marks[5], box), true, "a hidden area across the edge stays effective inside the kept area");
  assert.equal(survives(marks[4], box), false, "a stroke wholly outside is dropped");
  assert.equal(survives(marks[2], box), false, "a pin whose point is outside is dropped");
  // A horizontal arrow has no height and still counts when it crosses the box.
  assert.equal(survives({ id: 9, kind: "arrow", a: { x: 0, y: 200 }, b: { x: 200, y: 200 } }, box), true);
  // Tightened so the second step falls outside: it is dropped and the numbers close up.
  const tight = { x: 100, y: 100, w: 300, h: 200 };
  const after = kept(marks, tight);
  assert.deepEqual(after.map((m) => m.id), [1, 4, 7]);
  assert.deepEqual([...numbers(after).entries()], [[1, 1]]);
});

test("marks keep their screen positions when the box is cropped or moved", () => {
  const v = view({ w: 2560, h: 1600 }, { w: 1280, h: 800 }, false, 2);
  const before = kept(marks, box).map((m) => (m.at ? toScreen(m.at, v) : null));
  const crops = [{ x: 140, y: 140, w: 330, h: 230 }, { x: 0, y: 0, w: 480, h: 360 }];
  for (const c of crops) {
    const after = kept(marks, c);
    for (const m of after) {
      const was = kept(marks, box).findIndex((x) => x.id === m.id);
      if (was < 0 || !m.at) continue;
      assert.deepEqual(toScreen(m.at, v), before[was], `mark ${m.id} moved on screen`);
      assert.equal(m, kept(marks, box)[was], "the very same mark, not a moved copy");
    }
  }
  // The view is the picture's fit to the screen, the same whatever box is drawn on it.
  assert.deepEqual(v, { fit: 0.5, ox: 0, oy: 0, dw: 1280, dh: 800 });
  const e = view({ w: 800, h: 500 }, { w: 1280, h: 800 }, true, 1);
  assert.equal(e.fit, 1, "a picture from History is never shown larger than its pixels");
});

test("a crop is one undo step: undo brings back the box and with it the marks it had dropped", () => {
  let h = run(empty(), { t: "add", s: step(1, 150, 150) }, { t: "add", s: step(2, 450, 350) }, { t: "frame", r: box });
  assert.deepEqual(h.cut, box);
  assert.ok(h.past.every((p) => p.cut === box), "steps placed before the box was drawn undo to the same box");
  const tight = { x: 100, y: 100, w: 300, h: 200 };
  h = hist(h, { t: "crop", r: tight });
  assert.deepEqual(kept(h.now, h.cut).map((m) => m.id), [1]);
  h = hist(h, { t: "undo" });
  assert.deepEqual(h.cut, box);
  assert.deepEqual(kept(h.now, h.cut).map((m) => m.id), [1, 2]);
  h = hist(h, { t: "redo" });
  assert.deepEqual(h.cut, tight);
  // The same box again is not a step.
  const n = h.past.length;
  assert.equal(hist(h, { t: "crop", r: { ...tight } }).past.length, n);
  // A step dropped and taken back within one press (an empty note) leaves no step behind.
  const d = hist(hist(h, { t: "add", s: step(3, 120, 120) }), { t: "drop", id: 3 });
  assert.equal(d.past.length, n);
});
