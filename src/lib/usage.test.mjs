// Run: node --test src/lib/usage.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  NOTHING_THIS_WEEK,
  NOTHING_YET,
  SAVED_NOTE,
  clockParts,
  dayKey,
  emptyUsage,
  grouped,
  lastDays,
  over,
  perDay,
  previewUsage,
  savedMinutes,
  shades,
  speed,
  streakNow,
  usageView,
  weekdayNames,
} from "./usage.ts";

// Wednesday 11 November 2026, mid afternoon, local.
const TODAY = new Date(2026, 10, 11, 15, 30);
const day = (r, w, speech_ms, wall_ms, p = 0) => ({ r, w, speech_ms, wall_ms, p });
const read = (f) => f.parts.map((p) => p.n + (p.u ? " " + p.u : "")).join(" ");

/** Counts with the given days, the totals being their sum plus `before` (days long folded away). */
function usage(days, before = {}) {
  const u = emptyUsage(new Date(2026, 8, 3, 10).getTime());
  u.days = days;
  const t = { recordings: 0, words: 0, speech_ms: 0, wall_ms: 0, pictures: 0, ...before };
  for (const d of Object.values(days)) {
    t.recordings += d.r;
    t.words += d.w;
    t.speech_ms += d.speech_ms;
    t.wall_ms += d.wall_ms;
    t.pictures += d.p;
  }
  u.total = t;
  return u;
}

test("a day is named by the local calendar, and the last days are counted by it, newest first", () => {
  assert.equal(dayKey(new Date(2026, 0, 5, 23, 59)), "2026-01-05");
  assert.deepEqual(lastDays(TODAY, 3), ["2026-11-11", "2026-11-10", "2026-11-09"]);
  assert.deepEqual(lastDays(new Date(2026, 2, 1, 0, 5), 2), ["2026-03-01", "2026-02-28"], "across the end of a month");
  assert.equal(new Set(lastDays(TODAY, 400)).size, 400, "no day twice across clock changes");
});

test("this week is the last seven local days and nothing older", () => {
  const u = usage({
    "2026-11-11": day(2, 300, 120_000, 150_000, 1),
    "2026-11-05": day(1, 100, 60_000, 70_000, 2),
    "2026-11-04": day(9, 9_000, 900_000, 990_000, 9),
  });
  assert.deepEqual(over(u, TODAY, 7), { recordings: 3, words: 400, speech_ms: 180_000, wall_ms: 220_000, pictures: 3 });
  assert.equal(over(u, TODAY, 8).recordings, 12);
  assert.equal(over(u, new Date(2026, 10, 20), 7).recordings, 0);
});

test("speed is words over minutes speaking; time saved is against 40 wpm and never below zero", () => {
  const s = { recordings: 1, words: 400, speech_ms: 180_000, wall_ms: 220_000, pictures: 0 };
  assert.equal(speed(s), 133);
  assert.equal(speed({ ...s, speech_ms: 0 }), 0, "no speech, no speed, no division by zero");
  assert.equal(savedMinutes({ ...s, wall_ms: 240_000 }), 6, "400 words is 10 minutes of typing, less 4 of recording");
  assert.equal(savedMinutes({ ...s, words: 40, wall_ms: 600_000 }), 0, "a slow recording saves nothing, not less than nothing");
  assert.match(SAVED_NOTE, /40 wpm/);
});

test("time reads as hours and minutes with no zero part", () => {
  assert.deepEqual(clockParts(200), [{ n: "3", u: "h" }, { n: "20", u: "min" }]);
  assert.deepEqual(clockParts(45.4), [{ n: "45", u: "min" }]);
  assert.deepEqual(clockParts(120), [{ n: "2", u: "h" }]);
  assert.deepEqual(clockParts(0.4), []);
  assert.equal(grouped(1240), "1,240");
});

test("recordings a day do not count the days before counting began", () => {
  const u = usage({ "2026-11-11": day(4, 1, 1, 1), "2026-11-10": day(2, 1, 1, 1), "2026-10-20": day(6, 1, 1, 1) });
  assert.equal(perDay(u, TODAY, 7), 6 / 7);
  assert.equal(perDay(u, TODAY, 30), 12 / 30);
  // Counting began yesterday: two days, not seven or thirty.
  u.since_ms = new Date(2026, 10, 10, 23, 50).getTime();
  assert.equal(perDay(u, TODAY, 7), 3);
  assert.equal(perDay(u, TODAY, 30), 6);
  u.since_ms = TODAY.getTime();
  assert.equal(perDay(u, TODAY, 7), 6, "the first day is one day");
});

test("a streak still counts if its last day is today or yesterday, and is over after that", () => {
  const u = usage({});
  u.streak = { current: 6, longest: 14, last_day: "2026-11-11" };
  assert.equal(streakNow(u, TODAY), 6);
  u.streak.last_day = "2026-11-10";
  assert.equal(streakNow(u, TODAY), 6, "today is not over yet");
  u.streak.last_day = "2026-11-09";
  assert.equal(streakNow(u, TODAY), 0);
  u.streak.last_day = "";
  assert.equal(streakNow(u, TODAY), 0);
});

test("the grid has five shades: none, then four by share of the busiest hour", () => {
  const g = emptyUsage(0).grid;
  assert.ok(shades(g).flat().every((l) => l === 0), "an empty grid divides by nothing");
  [g[0][9], g[0][10], g[2][14], g[4][15], g[6][20]] = [1, 5, 10, 15, 20];
  const l = shades(g);
  assert.deepEqual([l[0][9], l[0][10], l[2][14], l[4][15], l[6][20], l[1][1]], [1, 1, 2, 3, 4, 0]);
  assert.deepEqual([...new Set(l.flat())].sort(), [0, 1, 2, 3, 4]);
  g[3][3] = 400;
  assert.equal(shades(g)[0][9], 1, "one recording is never shaded as none");
});

test("the view leads with the four numbers and leaves out any that is zero", () => {
  const before = { recordings: 500, words: 61_000, speech_ms: 27_000_000, wall_ms: 30_000_000, pictures: 0 };
  const u = usage({ "2026-11-11": day(2, 300, 120_000, 150_000, 1), "2026-11-05": day(1, 100, 60_000, 70_000) }, before);
  const week = usageView(u, TODAY, "week", "en-GB");
  assert.deepEqual(week.figures.map((f) => f.label), ["Words", "Speed", "Saved", "Pictures"]);
  assert.deepEqual(week.figures.map(read), ["400", "133 wpm", "6 min", "1"]);
  assert.equal(week.figures[2].title, SAVED_NOTE);
  assert.equal(week.quiet, null);
  assert.equal(week.rate, "0.4 recordings a day this week, 0.1 over 30 days.");
  const all = usageView(u, TODAY, "all", "en-GB");
  assert.deepEqual(all.figures.map(read), ["61,400", "136 wpm", "17 h 11 min", "1"]);
  assert.equal(all.since, "Since 3 September 2026");
  // No pictures, and so slow that nothing was saved: two numbers, no zeros.
  const slow = usageView(usage({ "2026-11-11": day(1, 40, 60_000, 600_000) }), TODAY, "week", "en-GB");
  assert.deepEqual(slow.figures.map((f) => f.label), ["Words", "Speed"]);
  for (const v of [week, all, slow]) assert.ok(v.figures.every((f) => f.parts.every((p) => p.n !== "0")));
});

test("a fresh install is one quiet line, and a week off says so without zeros", () => {
  const fresh = usageView(emptyUsage(TODAY.getTime()), TODAY, "week", "en-GB");
  assert.deepEqual([fresh.empty, fresh.figures, fresh.quiet, fresh.rate, fresh.streak, fresh.longest], [true, [], NOTHING_YET, null, null, null]);
  assert.equal(fresh.since, "Since 11 November 2026");
  const away = usage({ "2026-10-20": day(6, 900, 400_000, 450_000) });
  const week = usageView(away, TODAY, "week", "en-GB");
  assert.deepEqual([week.empty, week.figures, week.quiet], [false, [], NOTHING_THIS_WEEK]);
  assert.equal(week.rate, "0.2 recordings a day over the last 30 days.");
  assert.equal(usageView(away, TODAY, "all", "en-GB").quiet, null);
});

test("the streak is said only past one day, and the longest only when it says more", () => {
  const u = usage({ "2026-11-11": day(1, 10, 5_000, 6_000) });
  const said = (current, longest, last_day = "2026-11-11") => {
    u.streak = { current, longest, last_day };
    const v = usageView(u, TODAY, "week", "en-GB");
    return [v.streak, v.longest];
  };
  assert.deepEqual(said(0, 0, ""), [null, null]);
  assert.deepEqual(said(1, 1), [null, null]);
  assert.deepEqual(said(6, 6), ["6 days in a row", null]);
  assert.deepEqual(said(6, 14), ["6 days in a row", "Longest: 14 days"]);
  assert.deepEqual(said(6, 14, "2026-11-08"), [null, "Longest: 14 days"], "a run that ended is not a streak");
});

test("each hour of the grid says its weekday, hour and exact count", () => {
  const u = usage({});
  u.grid[0][9] = 12;
  u.grid[6][23] = 1;
  const v = usageView(u, TODAY, "all", "en-GB");
  assert.deepEqual([v.cells.length, ...new Set(v.cells.map((r) => r.length))], [7, 24]);
  assert.deepEqual(v.cells[0][9], { level: 4, title: "Monday 09:00, 12 recordings" });
  assert.deepEqual(v.cells[6][23], { level: 1, title: "Sunday 23:00, 1 recording" });
  assert.deepEqual(v.cells[3][3], { level: 0, title: "Thursday 03:00, no recordings" });
  assert.deepEqual(weekdayNames("short", "en-GB"), ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]);
});

test("the preview's totals are the sum of its days, and it has a streak and a week to show", () => {
  const u = previewUsage(TODAY);
  const sum = (f) => Object.values(u.days).reduce((a, d) => a + f(d), 0);
  assert.deepEqual(u.total, { recordings: sum((d) => d.r), words: sum((d) => d.w), speech_ms: sum((d) => d.speech_ms), wall_ms: sum((d) => d.wall_ms), pictures: sum((d) => d.p) });
  assert.equal(u.grid.flat().reduce((a, b) => a + b, 0), u.total.recordings);
  assert.deepEqual(previewUsage(TODAY), u, "the same every time");
  const v = usageView(u, TODAY, "week", "en-GB");
  assert.equal(v.figures.length, 4);
  assert.match(v.streak, /^\d+ days in a row$/);
});
