// Run: node --test src/lib/undo.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { UNDO_MS, close, due, startDelete, undo, undoView, visible } from "./undo.ts";

const list = ["a", "b", "c"].map((id) => ({ id }));
const ids = (l) => l.map((e) => e.id);

test("a delete takes the entry out of the list at once and shows the bar, with nothing committed", () => {
  const r = startDelete(null, list, "b", 1000);
  assert.deepEqual(ids(visible(list, r.pending)), ["a", "c"]);
  assert.equal(r.commit, null);
  assert.deepEqual(undoView(r.pending), { text: "Deleted", action: "Undo" });
  assert.equal(undoView(null), null);
});

test("the delete is real only after 5 seconds", () => {
  const { pending } = startDelete(null, list, "b", 1000);
  assert.equal(UNDO_MS, 5000);
  assert.equal(due(pending, 1000 + UNDO_MS - 1), false);
  assert.equal(due(pending, 1000 + UNDO_MS), true);
  assert.equal(due(null, 1e12), false);
});

test("Undo puts the entry back where it was and selects it", () => {
  const { pending } = startDelete(null, list, "b", 0);
  const r = undo(pending);
  assert.equal(r.pending, null);
  assert.equal(r.restore, "b");
  assert.deepEqual(ids(visible(list, r.pending)), ["a", "b", "c"]);
  assert.equal(undo(null).restore, null);
});

test("deleting selects the next entry, or the one before when it was last, or none", () => {
  assert.equal(startDelete(null, list, "a", 0).next, "b");
  assert.equal(startDelete(null, list, "b", 0).next, "c");
  assert.equal(startDelete(null, list, "c", 0).next, "b");
  assert.equal(startDelete(null, [{ id: "a" }], "a", 0).next, null);
});

test("a second delete commits the first and starts its own bar", () => {
  const first = startDelete(null, list, "a", 0);
  const second = startDelete(first.pending, list, "c", 2000);
  assert.equal(second.commit, "a");
  assert.equal(second.pending.id, "c");
  assert.equal(second.pending.at, 2000);
  assert.deepEqual(ids(visible(list, second.pending)), ["a", "b"]);
  // the neighbour is chosen among what is still shown, never the entry waiting to commit
  assert.equal(second.next, "b");
});

test("closing the window commits what is pending, and nothing when nothing is", () => {
  const { pending } = startDelete(null, list, "b", 0);
  assert.deepEqual(close(pending), { pending: null, commit: "b" });
  assert.deepEqual(close(null), { pending: null, commit: null });
});
