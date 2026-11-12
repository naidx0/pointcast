// Run: node --experimental-strip-types --test src/lib/sessions.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { RECENT_SESSIONS, TITLE_MAX, UNTITLED, normalizeSessions, renameValue, searchQuery, sessionRows, turnsLabel } from "./sessions.ts";

// Wednesday 11 November 2026, 15:00 local.
const NOW = new Date(2026, 10, 11, 15, 0);
const at = (d, h = 12) => new Date(2026, 10, d, h).getTime();
const meta = (id, d, over = {}) => ({ id, title: `Session ${id}`, updated_ms: at(d), turns: 2, pictures: 1, provider: "ChatGPT", size_bytes: 1000, ...over });

test("Home lists the three newest sessions, newest first, with their questions and day", () => {
  const rows = sessionRows([meta("a", 4), meta("b", 11), meta("c", 10), meta("d", 9)], NOW, { n: RECENT_SESSIONS });
  assert.deepEqual(
    rows.map((r) => [r.id, r.turns, r.day]),
    [
      ["b", "2 questions", "Today"],
      ["c", "2 questions", "Yesterday"],
      ["d", "2 questions", "Monday"],
    ],
  );
  assert.equal(rows[0].label, "Session b, 2 questions, today");
});

test("the open session is marked, an untitled one says so, and a search keeps the engine's order with its hit", () => {
  const rows = sessionRows([meta("a", 4, { title: "" }), meta("b", 11)], NOW, { current: "a", searched: true });
  assert.deepEqual(rows.map((r) => r.id), ["a", "b"]);
  assert.deepEqual([rows[0].title, rows[0].current, rows[0].hit, rows[1].current], [UNTITLED, true, "", false]);
  const hit = sessionRows(normalizeSessions([{ id: "x", hit: "  divide   by P(B) " }]), NOW, { searched: true })[0];
  assert.equal(hit.hit, "divide by P(B)");
});

test("one question is said in the singular", () => {
  assert.deepEqual([turnsLabel(1), turnsLabel(0), turnsLabel(12)], ["1 question", "0 questions", "12 questions"]);
});

test("whatever the engine sends, the list has one row per id and every field in its type", () => {
  const list = normalizeSessions([{ id: "a", title: " A\n b ", turns: "3", updated_ms: -4 }, { id: "a" }, null, { title: "no id" }, { id: 7, provider: { name: "Codex" } }]);
  assert.deepEqual(
    list.map((s) => [s.id, s.title, s.turns, s.updated_ms, s.provider]),
    [
      ["a", "A b", 0, 0, ""],
      ["7", "", 0, 0, "Codex"],
    ],
  );
  assert.deepEqual(normalizeSessions("x"), []);
});

test("a rename keeps folded words up to the limit; empty or unchanged is no rename", () => {
  assert.equal(renameValue("  Bayes'   rule ", "Old"), "Bayes' rule");
  assert.equal(renameValue("   ", "Old"), null);
  assert.equal(renameValue("Old ", "Old"), null);
  assert.equal(renameValue("x".repeat(TITLE_MAX + 20), "Old").length, TITLE_MAX);
});

test("an empty search shows the recent list; words are searched folded", () => {
  assert.equal(searchQuery("   "), null);
  assert.equal(searchQuery("  p(b)   slide "), "p(b) slide");
});
