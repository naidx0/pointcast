// Run: node --test src/lib/timeline.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { timelineRows } from "./timeline.ts";

// Wednesday 11 November 2026, 15:00 local.
const NOW = new Date(2026, 10, 11, 15, 0);
const at = (h = 12, min = 0) => new Date(2026, 10, 11, h, min).getTime();

const dict = (id, created_ms, preview = "move the button left", extra = {}) => ({
  id,
  created_ms,
  duration_ms: 5_000,
  preview,
  images: 0,
  words: 5,
  ...extra,
});
const sess = (id, updated_ms, title = "Bayes' rule on the slide", extra = {}) => ({
  id,
  title,
  updated_ms,
  turns: 3,
  pictures: 0,
  provider: "ChatGPT",
  ...extra,
});

test("dictations and sessions merge into one list, newest first, and nothing is lost", () => {
  const rows = timelineRows(
    [dict("d1", at(14), "first words"), dict("d2", at(9), "earlier today")],
    [sess("s1", at(14, 30)), sess("s2", at(10))],
    NOW,
  );
  assert.deepEqual(
    rows.map((r) => r.id),
    ["s1", "d1", "s2", "d2"],
    "interleaved by time, not grouped by kind",
  );
  assert.equal(rows.filter((r) => r.kind === "dictation").length, 2);
  assert.equal(rows.filter((r) => r.kind === "ask").length, 2);
});

test("the cap keeps the newest across both stores", () => {
  const dicts = Array.from({ length: 5 }, (_, i) => dict(`d${i}`, at(10, i)));
  const sessions = Array.from({ length: 5 }, (_, i) => sess(`s${i}`, at(11, i)));
  const rows = timelineRows(dicts, sessions, NOW, 6);
  assert.equal(rows.length, 6);
  assert.equal(rows[0].id, "s4", "the newest of either store first");
  assert.ok(!rows.some((r) => r.id === "d0"), "the oldest is what is cut");
});

test("an ask row names the AI that answered and its questions; a dictation row its words", () => {
  const [ask] = timelineRows([], [sess("s", at(), "The plan", { provider: "Codex", turns: 3 })], NOW);
  assert.equal(ask.provider, "Codex");
  assert.equal(ask.meta, "3 questions");
  assert.match(ask.label, /^Asked: The plan, Codex, 3 questions/);
  const [d] = timelineRows([dict("d", at(), "hello there", { words: 2 })], [], NOW);
  assert.equal(d.meta, "2 words");
  assert.match(d.label, /^Dictated: hello there/);
});

test("a dictation with no words says so quietly, and an untitled session is Untitled", () => {
  const [d] = timelineRows([dict("d", at(), "")], [], NOW);
  assert.equal(d.text, "No words");
  assert.equal(d.empty, true);
  const [a] = timelineRows([], [sess("s", at(), "")], NOW);
  assert.equal(a.text, "Untitled");
});
