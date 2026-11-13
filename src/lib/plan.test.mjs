// Run: node --experimental-strip-types --test src/lib/plan.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { GLYPH, SAY, asks, head, keep, previewPlan, rows } from "./plan.ts";

const plan = (state, statuses, notes = []) => ({
  id: 7,
  apps: ["TextEdit"],
  state,
  steps: statuses.map((status, i) => ({ text: `Step ${i + 1}`, status, note: notes[i] ?? null })),
});
const marks = (p, live = true) => rows(p, live).map((r) => r.mark);

test("a plan waiting for Start lists every step unticked, names its apps and offers Start and Cancel", () => {
  const p = plan("proposed", ["pending", "pending", "pending"]);
  assert.deepEqual(marks(p), ["todo", "todo", "todo"]);
  assert.deepEqual(rows(p, true).map((r) => r.text), ["Step 1", "Step 2", "Step 3"]);
  assert.equal(head(p, true), "Plan · TextEdit");
  assert.equal(asks(p, true), true);
  assert.equal(asks(p, false), false, "no Start once its turn has ended");
  assert.equal(head(p, false), "Plan not started");
});

test("once started, ticks show as they come and the first unticked step is under way", () => {
  const p = plan("started", ["done", "skipped", "pending", "pending"], ["TextEdit is open"]);
  assert.deepEqual(marks(p), ["done", "skipped", "now", "todo"]);
  assert.equal(rows(p, true)[0].note, "TextEdit is open");
  assert.equal(rows(p, true)[1].note, "");
  assert.equal(head(p, true), "2 of 4 · TextEdit");
  assert.equal(asks(p, true), false);
  assert.deepEqual(marks(plan("started", ["done", "done"])), ["done", "done"]);
  assert.equal(head(plan("started", ["done", "done"]), true), "2 of 2 · TextEdit");
});

test("a failed step means the AI stopped to ask: nothing reads as under way", () => {
  const p = plan("started", ["done", "failed", "pending"], [null, "no text to select"]);
  assert.deepEqual(marks(p), ["done", "failed", "todo"]);
  assert.equal(rows(p, true)[1].note, "no text to select");
});

test("after the turn nothing is under way, and a cancelled plan says so and asks nothing", () => {
  assert.deepEqual(marks(plan("started", ["done", "pending"]), false), ["done", "todo"]);
  const c = plan("cancelled", ["pending", "pending"]);
  assert.deepEqual(marks(c), ["todo", "todo"]);
  assert.equal(head(c, true), "Plan cancelled");
  assert.equal(asks(c, true), false);
});

test("each mark has its own glyph and its own words", () => {
  const all = ["todo", "now", "done", "skipped", "failed"];
  for (const map of [GLYPH, SAY]) {
    assert.deepEqual(Object.keys(map).sort(), [...all].sort());
    assert.equal(new Set(all.map((m) => map[m])).size, all.length);
  }
});

test("the panel keeps the last plan of its turn after the turn lets it go, and drops it when the next turn starts", () => {
  const live = plan("started", ["done", "pending"]);
  let k = keep(null, null, 1);
  assert.equal(k, null, "no plan, nothing shown");
  k = keep(k, live, 1);
  assert.deepEqual(k, { turn: 1, plan: live, live: true });
  const ticked = plan("started", ["done", "done"]);
  k = keep(k, ticked, 1);
  assert.equal(k.plan, ticked, "the newest ticks win");
  k = keep(k, null, 1);
  assert.deepEqual(k, { turn: 1, plan: ticked, live: false }, "the turn ended: it stays, as it ended");
  assert.equal(keep(k, null, 1), k, "and stays put");
  assert.equal(keep(k, null, 2), null, "the next turn starts clean");
  const next = plan("proposed", ["pending"]);
  assert.deepEqual(keep(k, next, 2), { turn: 2, plan: next, live: true });
});

test("the browser preview has a plan for each look, and none unless asked", () => {
  assert.equal(previewPlan(null), null);
  assert.equal(previewPlan("nonsense"), null);
  assert.deepEqual(marks(previewPlan("proposed")), ["todo", "todo", "todo"]);
  assert.deepEqual(marks(previewPlan("started")), ["done", "now", "todo"]);
  assert.deepEqual(marks(previewPlan("failed")), ["done", "failed", "todo"]);
  assert.equal(rows(previewPlan("failed"), true)[1].note, "the text area did not take the text");
});
