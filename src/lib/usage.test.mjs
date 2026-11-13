// Run: node --test src/lib/usage.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  KEEP_DAYS,
  NO_WORDS,
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
  readUsage,
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

const one = day(1, 10, 5_000, 6_000);
/** One recording on each of these days of November 2026. */
const novDays = (...ds) => Object.fromEntries(ds.map((d) => [`2026-11-${String(d).padStart(2, "0")}`, one]));
const from = (a, b) => Array.from({ length: b - a + 1 }, (_, i) => a + i);
/** The pane's two streak lines, the streak and the longest. */
function streakLines(u, today = TODAY) {
  const v = usageView(u, today, "week", "en-GB");
  return [v.streak, v.longest];
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
  assert.deepEqual(over(u, TODAY, 7), { recordings: 3, words: 400, speech_ms: 180_000, wall_ms: 220_000, pictures: 3, questions: 0, answer_words: 0 });
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
  // The run is read from the days themselves.
  assert.equal(streakNow(usage(novDays(...from(6, 11))), TODAY), 6);
  assert.equal(streakNow(usage(novDays(...from(5, 10))), TODAY), 6, "today is not over yet");
  assert.equal(streakNow(usage(novDays(...from(4, 9))), TODAY), 0);
  assert.equal(streakNow(usage({}), TODAY), 0);
  const kept = usage({});
  kept.streak = { current: 6, longest: 14, last_day: "2026-11-11" };
  assert.equal(streakNow(kept, TODAY), 0, "a kept run with no days under it is not a streak");
});

test("one recording dated ahead of the clock does not hide the days in a row before it", () => {
  // Made while the clock said the 20th; the 9th to the 11th came after it, as earlier days. The app keeps this:
  const u = usage(novDays(9, 10, 11, 20));
  u.streak = { current: 1, longest: 3, last_day: "2026-11-20" };
  assert.equal(streakNow(u, TODAY), 3);
  assert.deepEqual(streakLines(u), ["3 days in a row", null]);
  assert.equal(streakNow(u, new Date(2026, 10, 13, 9)), 0, "nothing on the 12th ends it, the 20th ahead or not");
});

test("a gap starts the streak again and the longest run is kept", () => {
  // The 2nd to the 6th, nothing on the 7th and 8th, then the 9th to today.
  const u = usage({ ...novDays(...from(2, 6)), ...novDays(...from(9, 11)) });
  // As the app keeps it, and with nothing kept: the days alone say the same.
  for (const kept of [{ current: 3, longest: 5, last_day: "2026-11-11" }, { current: 0, longest: 0, last_day: "" }]) {
    u.streak = kept;
    assert.equal(streakNow(u, TODAY), 3, JSON.stringify(kept));
    assert.deepEqual(streakLines(u), ["3 days in a row", "Longest: 5 days"], JSON.stringify(kept));
  }
});

test("a streak that ended yesterday still shows until today passes without a recording", () => {
  const u = usage(novDays(8, 9, 10));
  assert.deepEqual(streakLines(u), ["3 days in a row", null], "nothing yet today, and today is not over");
  assert.deepEqual(streakLines(u, new Date(2026, 10, 12, 0, 5)), [null, "Longest: 3 days"], "the 11th went by without one");
  u.days["2026-11-11"] = one;
  assert.deepEqual(streakLines(u), ["4 days in a row", null]);
  // By the calendar: across the end of October, and a clock change where there is one.
  const turn = usage({ "2026-10-30": one, "2026-10-31": one, "2026-11-01": one });
  assert.equal(streakNow(turn, new Date(2026, 10, 2, 8)), 3);
});

test("a streak that ended two days ago shows nothing, and is still the longest", () => {
  const u = usage(novDays(7, 8, 9));
  assert.equal(streakNow(u, TODAY), 0);
  assert.deepEqual(streakLines(u), [null, "Longest: 3 days"]);
});

test("the days folded past four hundred never shorten the longest run, nor a streak older than them", () => {
  // stats.rs keeps this many days one by one; the pane counts on the same number.
  const rs = readFileSync(new URL("../../src-tauri/src/stats.rs", import.meta.url), "utf8");
  assert.equal(Number(rs.match(/pub const KEEP_DAYS: usize = (\d+);/)?.[1]), KEEP_DAYS);
  const keep = (keys) => usage(Object.fromEntries(keys.map((k) => [k, one])));
  // 405 days in a row up to today: the five oldest are only in the totals now.
  const long = keep(lastDays(TODAY, 405).slice(0, KEEP_DAYS));
  long.streak = { current: 405, longest: 405, last_day: "2026-11-11" };
  assert.equal(Object.keys(long.days).length, KEEP_DAYS);
  assert.equal(streakNow(long, TODAY), 405);
  assert.deepEqual(streakLines(long), ["405 days in a row", null]);
  // A run of 450 long ago, folded away; the days kept are two runs of 200, the newer one up to today.
  const days = lastDays(TODAY, 401);
  const gap = keep([...days.slice(0, 200), ...days.slice(201)]);
  gap.streak = { current: 200, longest: 450, last_day: "2026-11-11" };
  assert.equal(Object.keys(gap.days).length, KEEP_DAYS);
  assert.deepEqual(streakLines(gap), ["200 days in a row", "Longest: 450 days"]);
  // The kept run is read only where the days run out: a run that starts inside them is what they say.
  gap.streak.current = 999;
  assert.equal(streakNow(gap, TODAY), 200);
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

test("counts that did not arrive or do not read show a fresh install's quiet line, never a blank or an error", () => {
  const now = TODAY.getTime();
  const good = usage(novDays(9, 10, 11));
  assert.equal(readUsage(good, now), good, "counts that read are taken as they are");
  const preview = previewUsage(TODAY);
  assert.equal(readUsage(preview, now), preview);
  const unread = [
    undefined,
    null,
    "",
    "{ not json",
    7,
    [],
    {},
    { ...good, v: 7 },
    { ...good, since_ms: "yesterday" },
    { ...good, total: undefined },
    { ...good, total: { ...good.total, words: Number.NaN } },
    { ...good, days: null },
    { ...good, days: [] },
    { ...good, days: { "2026-11-11": null } },
    { ...good, days: { "2026-11-11": { r: "1", w: 1, speech_ms: 1, wall_ms: 1, p: 0 } } },
    { ...good, hours: [] },
    { ...good, weekdays: good.weekdays.slice(1) },
    { ...good, grid: [] },
    { ...good, grid: good.grid.map((row) => row.slice(1)) },
    { ...good, grid: good.grid.map((row, d) => (d === 3 ? null : row)) },
    { ...good, streak: null },
    { ...good, streak: { ...good.streak, last_day: 11 } },
  ];
  for (const range of ["week", "all"]) {
    const fresh = usageView(emptyUsage(now), TODAY, range, "en-GB");
    assert.deepEqual([fresh.empty, fresh.quiet], [true, NOTHING_YET]);
    for (const raw of unread) {
      assert.deepEqual(usageView(readUsage(raw, now), TODAY, range, "en-GB"), fresh, `${range}: ${JSON.stringify(raw)}`);
    }
  }
});

test("the streak is said only past one day, and the longest only when it says more", () => {
  // The runs are in the days; the longest kept is the run of 14 no longer among them.
  const said = (days, longest = 0) => {
    const u = usage(days);
    u.streak.longest = longest;
    return streakLines(u);
  };
  assert.deepEqual(said({}), [null, null]);
  assert.deepEqual(said(novDays(11), 1), [null, null]);
  assert.deepEqual(said(novDays(...from(6, 11)), 6), ["6 days in a row", null]);
  assert.deepEqual(said(novDays(...from(6, 11)), 14), ["6 days in a row", "Longest: 14 days"]);
  assert.deepEqual(said(novDays(...from(3, 8)), 14), [null, "Longest: 14 days"], "a run that ended is not a streak");
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
  assert.deepEqual(u.total, { recordings: sum((d) => d.r), words: sum((d) => d.w), speech_ms: sum((d) => d.speech_ms), wall_ms: sum((d) => d.wall_ms), pictures: sum((d) => d.p), questions: sum((d) => d.q ?? 0), answer_words: sum((d) => d.aw ?? 0) });
  assert.equal(u.grid.flat().reduce((a, b) => a + b, 0), u.total.recordings);
  assert.deepEqual(previewUsage(TODAY), u, "the same every time");
  const v = usageView(u, TODAY, "week", "en-GB");
  assert.equal(v.figures.length, 4);
  assert.match(v.streak, /^\d+ days in a row$/);
});

test("a week of recordings that caught no words says so, never that nothing was dictated", () => {
  const silent = usage({ "2026-11-11": day(1, 0, 0, 3_000) });
  assert.equal(usageView(silent, TODAY, "week").quiet, NO_WORDS);
  assert.equal(usageView(usage({}), TODAY, "week").quiet, NOTHING_YET);
  assert.equal(usageView(usage({ "2026-10-01": one }), TODAY, "week").quiet, NOTHING_THIS_WEEK);
});
