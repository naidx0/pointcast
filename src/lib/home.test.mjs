// Run: node --test src/lib/home.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { NO_WORDS, RECENT, dayLabel, firstWords, lengthLabel, recentRows, timeLabel, wordsLabel } from "./home.ts";

// Wednesday 11 November 2026, 15:00 local.
const NOW = new Date(2026, 10, 11, 15, 0);
const at = (y, m, d, h = 12, min = 0) => new Date(y, m - 1, d, h, min).getTime();

test("short text is kept as it is, with its whitespace folded", () => {
  assert.equal(firstWords("move the button"), "move the button");
  assert.equal(firstWords("  move   the\n button  "), "move the button");
  assert.equal(firstWords(""), "");
});

test("long text is cut on a word boundary, never mid-word, with an ellipsis", () => {
  const text = "the button is off by a few pixels so move it a little to the left and make it blue";
  const cut = firstWords(text, 30);
  assert.equal(cut, "the button is off by a few…");
  assert.ok(cut.length <= 31, "at most max characters and the ellipsis");
  assert.ok(text.startsWith(cut.slice(0, -1)), "a prefix of the words");
});

test("a cut drops the punctuation it lands on", () => {
  assert.equal(firstWords("first, second, third, fourth", 14), "first, second…");
});

test("a word cut exactly at the limit keeps the whole word", () => {
  assert.equal(firstWords("aaaa bbbb cccc", 9), "aaaa bbbb…");
});

test("a single word longer than the limit is cut where it must be", () => {
  assert.equal(firstWords("x".repeat(80), 10), `${"x".repeat(10)}…`);
});

test("days read from now: today, yesterday, the weekday within the week, else the date", () => {
  assert.equal(dayLabel(at(2026, 11, 11, 0, 1), NOW, "en-GB"), "Today");
  assert.equal(dayLabel(at(2026, 11, 11, 14, 59), NOW, "en-GB"), "Today");
  assert.equal(dayLabel(at(2026, 11, 10, 23, 59), NOW, "en-GB"), "Yesterday");
  assert.equal(dayLabel(at(2026, 11, 9), NOW, "en-GB"), "Monday");
  assert.equal(dayLabel(at(2026, 11, 5), NOW, "en-GB"), "Thursday", "six days back is still this week");
  assert.equal(dayLabel(at(2026, 11, 4), NOW, "en-GB"), "4 Nov", "seven days back is a date");
  assert.equal(dayLabel(at(2025, 11, 3), NOW, "en-GB"), "3 Nov 2025", "another year says its year");
  assert.equal(dayLabel(at(2026, 11, 12), NOW, "en-GB"), "12 Nov", "a day after today is a date, never Today");
});

test("days are counted by the calendar across a clock change", () => {
  // Late October and early November hold the end of summer time in Europe and North America.
  const now = new Date(2026, 10, 2, 9, 0);
  assert.equal(dayLabel(at(2026, 11, 1, 23, 0), now, "en-GB"), "Yesterday");
  assert.equal(dayLabel(at(2026, 10, 27, 9, 0), now, "en-GB"), "Tuesday", "six calendar days back, an hour more by the clock");
  assert.equal(dayLabel(at(2026, 10, 26, 9, 0), now, "en-GB"), "26 Oct", "seven calendar days back is a date");
});

test("the time of day is written as the locale writes it", () => {
  assert.equal(timeLabel(at(2026, 11, 11, 14, 2), "en-GB"), "14:02");
  assert.equal(timeLabel(at(2026, 11, 11, 9, 5), "en-GB"), "09:05");
  assert.match(timeLabel(at(2026, 11, 11, 14, 2), "en-US"), /^2:02\sPM$/);
});

test("a length is whole seconds under a minute, never zero, then whole minutes", () => {
  assert.equal(lengthLabel(0), "1 s");
  assert.equal(lengthLabel(-5), "1 s");
  assert.equal(lengthLabel(400), "1 s");
  assert.equal(lengthLabel(42_000), "42 s");
  assert.equal(lengthLabel(59_400), "59 s");
  assert.equal(lengthLabel(59_600), "1 min", "rounds up into a minute, never 60 s");
  assert.equal(lengthLabel(89_000), "1 min");
  assert.equal(lengthLabel(200_000), "3 min");
});

const entry = (id, created_ms, preview = "the words", images = 0, duration_ms = 12_000) => ({ id, created_ms, duration_ms, preview, images });

test("recent rows are the newest five, newest first, whatever order they came in", () => {
  const list = [1, 7, 3, 6, 2, 5, 4].map((d) => entry(`e${d}`, at(2026, 11, d)));
  const rows = recentRows(list, NOW, RECENT, "en-GB");
  assert.equal(RECENT, 5);
  assert.deepEqual(rows.map((r) => r.id), ["e7", "e6", "e5", "e4", "e3"]);
  assert.equal(recentRows(list.slice(0, 2), NOW, RECENT, "en-GB").length, 2, "fewer than five lists what there is");
  assert.deepEqual(recentRows([], NOW), []);
});

test("a row says its first words, when, how long, and its pictures", () => {
  const [r] = recentRows([entry("a", at(2026, 11, 11, 14, 2), "move the save button to the right", 2, 42_000)], NOW, RECENT, "en-GB");
  assert.deepEqual(r, {
    id: "a",
    text: "move the save button to the right",
    empty: false,
    when: "Today, 14:02",
    length: "42 s",
    pictures: 2,
    label: "move the save button to the right, Today, 14:02, 42 s, 2 pictures",
  });
  const [one] = recentRows([entry("b", at(2026, 11, 10, 9, 30), "hello", 1)], NOW, RECENT, "en-GB");
  assert.equal(one.label, "hello, Yesterday, 09:30, 12 s, 1 picture");
  const [none] = recentRows([entry("c", at(2026, 11, 10, 9, 30), "hello", 0)], NOW, RECENT, "en-GB");
  assert.equal(none.label, "hello, Yesterday, 09:30, 12 s", "no pictures, nothing said about them");
});

test("a row counts its words when the entry carries them, and falls back to the length when it does not", () => {
  const words = (w) => ({ ...entry("w", at(2026, 11, 11, 14, 2), "move the save button", 0, 42_000), words: w });
  const [r] = recentRows([words(42)], NOW, RECENT, "en-GB");
  assert.equal(r.length, "42 words");
  assert.equal(r.label, "move the save button, Today, 14:02, 42 words");
  assert.equal(recentRows([words(1)], NOW, RECENT, "en-GB")[0].length, "1 word");
  assert.equal(recentRows([words(1204)], NOW, RECENT, "en-GB")[0].length, "1,204 words");
  assert.equal(recentRows([words(undefined)], NOW, RECENT, "en-GB")[0].length, "42 s", "an older build sends no count");
  assert.equal(recentRows([words(0)], NOW, RECENT, "en-GB")[0].length, "42 s", "zero words says how long instead");
  assert.equal(recentRows([words(Number.NaN)], NOW, RECENT, "en-GB")[0].length, "42 s");
  const [none] = recentRows([{ ...entry("n", at(2026, 11, 11, 9), ""), words: 3 }], NOW, RECENT, "en-GB");
  assert.equal(none.length, "12 s", "an entry without words never claims a count");
  assert.equal(wordsLabel(2.6, "en-GB"), "3 words");
});

test("an entry without words says so, whether its text is empty or the list's no-words line", () => {
  const rows = recentRows(
    [entry("a", at(2026, 11, 11, 10), ""), entry("b", at(2026, 11, 11, 9), NO_WORDS), entry("c", at(2026, 11, 11, 8), "  ")],
    NOW,
    RECENT,
    "en-GB",
  );
  assert.deepEqual(rows.map((r) => [r.text, r.empty]), [["No words", true], ["No words", true], ["No words", true]]);
  const [said] = recentRows([entry("d", at(2026, 11, 11, 8), "No words were said here")], NOW, RECENT, "en-GB");
  assert.equal(said.empty, false, "only the exact no-words line counts");
});

test("a long preview is cut for the row", () => {
  const long = Array.from({ length: 24 }, (_, i) => `word${i}`).join(" ");
  const [r] = recentRows([entry("a", at(2026, 11, 11, 10), long)], NOW, RECENT, "en-GB");
  assert.ok(r.text.endsWith("…") && r.text.length <= 57);
  assert.ok(long.startsWith(`${r.text.slice(0, -1)} `), "ends on a whole word");
});
