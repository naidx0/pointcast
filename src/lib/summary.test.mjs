// Run: node --experimental-strip-types --test src/lib/summary.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ALREADY_MAKING,
  canSummarize,
  countsLine,
  noSummary,
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
  const v = summaryView(s, "", [1, 2, 3]);
  assert.deepEqual(v.groups.map((g) => [g.label, g.items.length]), [["Key points", 2], ["Still open", 1]], "a point without words is dropped");
  const [a, q] = v.groups[0].items;
  assert.deepEqual([a.by, a.ref, a.go], ["AI", "T3", true]);
  assert.deepEqual([q.by, q.ref], ["You", "T1"]);
  assert.match(a.label, /From question 3, the AI's words: turns that count into a share/);
  assert.match(q.label, /your words: What does the line/);
  assert.equal(v.groups[1].items[0].go, false, "a turn the session does not have is not a place to go");
  const empty = summaryView({ ...s, open_questions: [] }, "", [1, 2, 3]);
  assert.deepEqual(empty.groups.map((g) => g.id), ["points"], "a group with no points is left out");
});

test("the counts line says what was kept and what was dropped for want of a source, with the AI's name", () => {
  assert.equal(countsLine(9, 1), "9 points, every quote found in the session; 1 left out, its quote was not");
  assert.equal(countsLine(4, 0), "4 points, every quote found in the session");
  assert.equal(countsLine(1, 2), "1 point, every quote found in the session; 2 left out, their quotes were not");
  assert.equal(countsLine(0, 0), "");
  const s = normalizeSummary(raw);
  assert.deepEqual([s.kept, s.dropped, s.model], [3, 1, ""], "a model the engine leaves null is none");
  assert.equal(summaryView(s, "9 kept, 1 dropped: no source", []).foot, "9 kept, 1 dropped: no source · ChatGPT", "the engine's line wins");
  assert.equal(summaryView(s, "", []).foot, "3 kept, 1 dropped: no source · ChatGPT", "else the same words from the summary's counts");
});

test("Copy as text is the title, each group's points with their turn and whose words, and the foot", () => {
  const nl = String.fromCharCode(10);
  const text = summaryText(summaryView(previewSummary(), "", [1, 2, 3]));
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
  const failed = summaryReduce(m, { type: "event", event: { id: "a", state: "error", summary: null, counts: "", error: null } });
  assert.deepEqual([failed.state, failed.error], ["error", "The summary could not be made"]);
  const done = summaryReduce(m, { type: "event", event: { id: "a", state: "done", summary: { ...raw, title: "New" }, counts: "9 kept, 1 dropped: no source", error: null } });
  assert.deepEqual([done.state, done.summary.title, done.counts], ["done", "New", "9 kept, 1 dropped: no source"]);
  assert.equal(summaryReduce(done, { type: "make", id: "a" }).counts, "9 kept, 1 dropped: no source", "working keeps the last line until done brings a new one");
  assert.equal(summaryReduce(m, { type: "failed", id: "a", error: ALREADY_MAKING }).state, "working", "already being made is under way, not a failure");
  const refused = summaryReduce(m, { type: "failed", id: "a", error: "no such session" });
  assert.deepEqual([refused.state, refused.error], ["error", "no such session"]);
  assert.deepEqual(summaryReduce(done, { type: "session", id: "b" }), noSummary("b"), "another session starts with nothing");
  assert.equal(summaryReduce(m, { type: "event", event: { id: "a", state: "bogus" } }), m);
});

test("Summarize is offered from two turns", () => {
  assert.deepEqual([0, 1, 2, 5].map(canSummarize), [false, false, true, true]);
});
