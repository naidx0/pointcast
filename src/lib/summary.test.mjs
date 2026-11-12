// Run: node --experimental-strip-types --test src/lib/summary.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  canSummarize,
  countsLine,
  noSummary,
  normalizeCounts,
  normalizeSummary,
  previewSummary,
  summaryReduce,
  summaryText,
  summaryView,
} from "./summary.ts";

const raw = {
  title: "  Why Bayes' rule\n divides ",
  key_points: [
    { text: "Dividing turns a count into a share.", turn: 3, from: "answer", quote: "turns that count into a share" },
    { text: "", turn: 2, from: "answer", quote: "x" },
    { text: "Not B is every case without B.", turn: 1, from: "question", quote: "What does the line over the B mean?" },
  ],
  open_questions: [{ text: "Whether multiplying ever works.", turn: 9, from: "question", quote: "not just multiply" }],
  kept: 3,
  dropped: 1,
  provider: "ChatGPT",
};

test("a summary groups its points, marks whose words each quote is by a word, and refers to its turn", () => {
  const s = normalizeSummary(raw);
  assert.equal(s.title, "Why Bayes' rule divides");
  const v = summaryView(s, null, [1, 2, 3]);
  assert.deepEqual(v.groups.map((g) => [g.label, g.items.length]), [["Key points", 2], ["Still open", 1]], "a point without words is dropped");
  const [a, q] = v.groups[0].items;
  assert.deepEqual([a.by, a.ref, a.go], ["AI", "T3", true]);
  assert.deepEqual([q.by, q.ref], ["You", "T1"]);
  assert.match(a.label, /From question 3, the AI's words: turns that count into a share/);
  assert.match(q.label, /your words: What does the line/);
  assert.equal(v.groups[1].items[0].go, false, "a turn the session does not have is not a place to go");
  const empty = summaryView({ ...s, open_questions: [] }, null, [1, 2, 3]);
  assert.deepEqual(empty.groups.map((g) => g.id), ["points"], "a group with no points is left out");
});

test("the counts line says what was kept and what was dropped and why, with the AI's name", () => {
  assert.equal(countsLine({ kept: 9, dropped: 1, why: ["no source"] }), "9 kept, 1 dropped: no source");
  assert.equal(countsLine({ kept: 4, dropped: 0, why: [] }), "4 kept");
  assert.equal(countsLine({ kept: 3, dropped: 2, why: [] }), "3 kept, 2 dropped");
  assert.equal(countsLine(null), "");
  assert.deepEqual(normalizeCounts({ kept: 9, dropped: [{ why: "no_source" }, { reason: "no-source" }, "repeated"] }), { kept: 9, dropped: 3, why: ["no source", "repeated"] });
  assert.deepEqual(normalizeCounts({ kept: 2, dropped: 1, reasons: { no_source: 1 } }), { kept: 2, dropped: 1, why: ["no source"] });
  assert.equal(normalizeCounts({}), null);
  const s = normalizeSummary(raw);
  assert.equal(summaryView(s, { kept: 9, dropped: 1, why: ["no source"] }, []).foot, "9 kept, 1 dropped: no source · ChatGPT", "the event's counts win");
  assert.equal(summaryView(s, null, []).foot, "3 kept, 1 dropped · ChatGPT", "else the summary's own");
});

test("Copy as text is the title, each group's points with their turn and whose words, and the foot", () => {
  const nl = String.fromCharCode(10);
  const text = summaryText(summaryView(previewSummary(), null, [1, 2, 3]));
  const lines = text.split(nl);
  assert.equal(lines[0], "Why Bayes' rule divides by P(B)");
  assert.equal(lines[2], "Key points");
  assert.equal(lines[3], "- The line over B means not B: every case where B does not happen. (T1)");
  assert.equal(lines[4], "  AI: “It means not B: every case where B does not happen.”");
  assert.ok(text.includes(nl + nl + "Still open" + nl + "- Whether multiplying"));
  assert.ok(text.includes("  You: “and not just multiply the two?”"));
  assert.ok(text.endsWith("5 kept, 1 dropped: no source · ChatGPT"));
});

test("the summary state follows its session's events only; working keeps the one before until done replaces it", () => {
  let m = summaryReduce(noSummary(), { type: "session", id: "a" });
  m = summaryReduce(m, { type: "got", id: "a", summary: null });
  assert.equal(m.state, "none");
  m = summaryReduce(m, { type: "got", id: "a", summary: raw });
  assert.deepEqual([m.state, m.summary.key_points.length], ["done", 2]);
  m = summaryReduce(m, { type: "make", id: "a" });
  assert.deepEqual([m.state, !!m.summary], ["working", true], "Redo keeps the old one while it works");
  assert.equal(summaryReduce(m, { type: "got", id: "a", summary: raw }).state, "working", "a late get does not end the work");
  assert.equal(summaryReduce(m, { type: "event", event: { id: "b", state: "done", summary: raw } }), m, "another session's event changes nothing");
  const stopped = summaryReduce(m, { type: "event", event: { id: "a", state: "stopped", summary: null } });
  assert.deepEqual([stopped.state, !!stopped.summary], ["stopped", true]);
  const failed = summaryReduce(m, { type: "event", event: { id: "a", state: "error", summary: null, error: "" } });
  assert.deepEqual([failed.state, failed.error], ["error", "The summary could not be made"]);
  const done = summaryReduce(m, { type: "event", event: { id: "a", state: "done", summary: { ...raw, title: "New" }, counts: { kept: 9, dropped: 1, why: "no_source" } } });
  assert.deepEqual([done.state, done.summary.title, done.counts], ["done", "New", { kept: 9, dropped: 1, why: ["no source"] }]);
  assert.deepEqual(summaryReduce(done, { type: "session", id: "b" }), noSummary("b"), "another session starts with nothing");
  assert.equal(summaryReduce(m, { type: "event", event: { id: "a", state: "bogus" } }), m);
});

test("Summarize is offered from two turns", () => {
  assert.deepEqual([0, 1, 2, 5].map(canSummarize), [false, false, true, true]);
});
