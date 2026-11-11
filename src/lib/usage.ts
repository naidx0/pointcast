// What the Usage pane draws from the running counts the app keeps (stats.json): four numbers for a week or for all
// time, how many recordings a day, when in the week they were made, the streak. Pure, so node --test reads it without
// a window (src/lib/usage.test.mjs).

export type UsageDay = { r: number; w: number; speech_ms: number; wall_ms: number; p: number };

/** The counts as the app keeps them. Days are local ("2026-11-11"); `grid` is recordings by weekday (Monday first),
 * then by hour. `streak.current` is the run that ends on `last_day`, which may be days ago. */
export type Usage = {
  v: number;
  since_ms: number;
  total: { recordings: number; words: number; speech_ms: number; wall_ms: number; pictures: number };
  days: Record<string, UsageDay>;
  hours: { r: number; w: number }[];
  weekdays: { r: number; w: number }[];
  grid: number[][];
  streak: { current: number; longest: number; last_day: string };
};

export type Range = "week" | "all";

/** Time saved is measured against typing at this many words a minute. */
export const TYPING_WPM = 40;
export const SAVED_NOTE = `Time saved is against typing at ${TYPING_WPM} wpm.`;

export const NOTHING_YET = "Nothing dictated yet. Your numbers show up here after your first recording.";
export const NOTHING_THIS_WEEK = "Nothing dictated in the last 7 days.";

/** One large number: "3 h 20 min" is two parts, each a number and its small unit. */
export type Figure = { label: string; parts: { n: string; u?: string }[]; title?: string };

export type UsageView = {
  /** Nothing was ever counted: the pane is one quiet line. */
  empty: boolean;
  /** Of Words, Speed, Saved and Pictures, in that order, the ones that are not zero. */
  figures: Figure[];
  /** Said in place of the figures when there are none for the range. */
  quiet: string | null;
  /** Recordings a day over the last 7 and 30 days, a rate of zero left unsaid; null with none in 30. */
  rate: string | null;
  /** Seven rows (Monday first) of 24 hours: a shade from 0 (none) to 4, and the exact figure for a tooltip. */
  cells: { level: number; title: string }[][];
  /** "6 days in a row", only for a run longer than a day that is still going. */
  streak: string | null;
  /** "Longest: 14 days", only when it says more than the line above. */
  longest: string | null;
  /** "Since 3 September 2026". */
  since: string;
};

type Sum = { recordings: number; words: number; speech_ms: number; wall_ms: number; pictures: number };

const pad = (n: number) => String(n).padStart(2, "0");

/** A local day as the counts name it. */
export const dayKey = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;

/** The `n` local days that end today, newest first. By the calendar, not by 24 hours: a clock change loses no day. */
export function lastDays(today: Date, n: number): string[] {
  return Array.from({ length: n }, (_, i) => dayKey(new Date(today.getFullYear(), today.getMonth(), today.getDate() - i)));
}

/** What was counted over the `n` days that end today. */
export function over(u: Usage, today: Date, n: number): Sum {
  const s: Sum = { recordings: 0, words: 0, speech_ms: 0, wall_ms: 0, pictures: 0 };
  for (const k of lastDays(today, n)) {
    const d = u.days[k];
    if (!d) continue;
    s.recordings += d.r;
    s.words += d.w;
    s.speech_ms += d.speech_ms;
    s.wall_ms += d.wall_ms;
    s.pictures += d.p;
  }
  return s;
}

/** Words a minute while speaking; 0 with no speech. */
export const speed = (s: Sum) => (s.speech_ms > 0 ? Math.round(s.words / (s.speech_ms / 60_000)) : 0);

/** Minutes saved: typing the words at 40 wpm, less the time the recordings took. Never below zero. */
export const savedMinutes = (s: Sum) => Math.max(0, s.words / TYPING_WPM - s.wall_ms / 60_000);

/** Recordings a day over the `n` days that end today. Days before counting began are not days without recordings:
 * three days in, the 30-day figure is over three days. */
export function perDay(u: Usage, today: Date, n: number): number {
  const began = new Date(u.since_ms);
  const from = Date.UTC(began.getFullYear(), began.getMonth(), began.getDate());
  const counted = Math.round((Date.UTC(today.getFullYear(), today.getMonth(), today.getDate()) - from) / 86_400_000) + 1;
  return over(u, today, n).recordings / Math.min(n, Math.max(1, counted));
}

/** The streak as it stands today: the kept run still counts if its last day is today or yesterday, else it is over. */
export function streakNow(u: Usage, today: Date): number {
  return lastDays(today, 2).includes(u.streak.last_day) ? u.streak.current : 0;
}

/** Shades for the grid: 0 for none, else 1 to 4 by share of the busiest hour. */
export function shades(grid: number[][]): number[][] {
  const max = Math.max(0, ...grid.flat());
  return grid.map((row) => row.map((v) => (v > 0 ? Math.min(4, Math.max(1, Math.ceil((v / max) * 4))) : 0)));
}

/** 1240 as "1,240", as the tray's line writes it. */
export const grouped = (n: number) => Math.round(n).toLocaleString("en-US");

const plural = (n: number, one: string) => `${grouped(n)} ${one}${n === 1 ? "" : "s"}`;

/** A rate that is not zero, to a tenth and never rounded away: 4.25 as "4.3", 0.03 as "0.1". */
const tenth = (x: number) => String(Math.max(0.1, Math.round(x * 10) / 10));

const aDay = (x: number) => `${tenth(x)} ${tenth(x) === "1" ? "recording" : "recordings"} a day`;

/** Whole minutes as number and unit pairs: 200 is 3 h 20 min, 45 is 45 min, 120 is 2 h. */
export function clockParts(minutes: number): { n: string; u: string }[] {
  const m = Math.round(minutes);
  const parts: { n: string; u: string }[] = [];
  if (m >= 60) parts.push({ n: grouped(Math.floor(m / 60)), u: "h" });
  if (m % 60 > 0) parts.push({ n: String(m % 60), u: "min" });
  return parts;
}

/** Monday first, as the grid's rows are. 1 January 2024 was a Monday. */
export function weekdayNames(width: "short" | "long", locale?: string): string[] {
  return Array.from({ length: 7 }, (_, i) => new Date(2024, 0, 1 + i).toLocaleDateString(locale, { weekday: width }));
}

/** Everything the pane draws, for the range picked. A number that is zero is left out, never drawn as 0. */
export function usageView(u: Usage, today: Date, range: Range, locale?: string): UsageView {
  const s: Sum = range === "week" ? over(u, today, 7) : u.total;
  const figures: Figure[] = [];
  if (s.words > 0) figures.push({ label: "Words", parts: [{ n: grouped(s.words) }] });
  if (speed(s) > 0) figures.push({ label: "Speed", parts: [{ n: grouped(speed(s)), u: "wpm" }], title: "Words a minute while speaking" });
  const saved = clockParts(savedMinutes(s));
  if (saved.length) figures.push({ label: "Saved", parts: saved, title: SAVED_NOTE });
  if (s.pictures > 0) figures.push({ label: "Pictures", parts: [{ n: grouped(s.pictures) }] });

  const [week, month] = [perDay(u, today, 7), perDay(u, today, 30)];
  const rate = month <= 0 ? null : week > 0 ? `${aDay(week)} this week, ${tenth(month)} over 30 days.` : `${aDay(month)} over the last 30 days.`;

  const names = weekdayNames("long", locale);
  const level = shades(u.grid);
  const cells = u.grid.map((row, d) => row.map((v, h) => ({ level: level[d][h], title: `${names[d]} ${pad(h)}:00, ${v ? plural(v, "recording") : "no recordings"}` })));

  const now = streakNow(u, today);
  const empty = u.total.recordings === 0;
  return {
    empty,
    figures,
    quiet: figures.length ? null : empty ? NOTHING_YET : NOTHING_THIS_WEEK,
    rate,
    cells,
    streak: now > 1 ? `${grouped(now)} days in a row` : null,
    longest: u.streak.longest > 1 && u.streak.longest > now ? `Longest: ${grouped(u.streak.longest)} days` : null,
    since: `Since ${new Date(u.since_ms).toLocaleDateString(locale, { day: "numeric", month: "long", year: "numeric" })}`,
  };
}

/** The counts of an install that has recorded nothing, counting from `since_ms`. */
export function emptyUsage(since_ms: number): Usage {
  return {
    v: 1,
    since_ms,
    total: { recordings: 0, words: 0, speech_ms: 0, wall_ms: 0, pictures: 0 },
    days: {},
    hours: Array.from({ length: 24 }, () => ({ r: 0, w: 0 })),
    weekdays: Array.from({ length: 7 }, () => ({ r: 0, w: 0 })),
    grid: Array.from({ length: 7 }, () => Array.from({ length: 24 }, () => 0)),
    streak: { current: 0, longest: 0, last_day: "" },
  };
}

/** Browser preview: ten weeks of a person who dictates most working days, the same every time for a given day. */
export function previewUsage(today: Date): Usage {
  const u = emptyUsage(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 69, 9, 12).getTime());
  let seed = 7;
  const rnd = () => (seed = (seed * 16807) % 2147483647) / 2147483647;
  let run = 0;
  for (let back = 69; back >= 0; back--) {
    const day = new Date(today.getFullYear(), today.getMonth(), today.getDate() - back);
    const wd = (day.getDay() + 6) % 7;
    // Weekends are mostly off, and so is the odd weekday; the last six days are all on, for a streak.
    const n = back < 6 ? 4 + Math.floor(rnd() * 6) : rnd() < (wd > 4 ? 0.25 : 0.86) ? 1 + Math.floor(rnd() * 9) : 0;
    run = n ? run + 1 : 0;
    u.streak.longest = Math.max(u.streak.longest, run, 14);
    if (!n) continue;
    const d: UsageDay = { r: 0, w: 0, speech_ms: 0, wall_ms: 0, p: 0 };
    for (let i = 0; i < n; i++) {
      // Late morning and mid afternoon, a little in the evening.
      const h = Math.min(23, Math.round((rnd() < 0.55 ? 10 : rnd() < 0.8 ? 15 : 20) + (rnd() - 0.5) * 5));
      const w = 20 + Math.floor(rnd() * rnd() * 420);
      const speech = Math.round((w / (128 + rnd() * 30)) * 60_000);
      const p = rnd() < 0.3 ? 1 + Math.floor(rnd() * 3) : 0;
      d.r += 1;
      d.w += w;
      d.speech_ms += speech;
      d.wall_ms += Math.round(speech * 1.18) + p * 2_500;
      d.p += p;
      u.hours[h].r += 1;
      u.hours[h].w += w;
      u.weekdays[wd].r += 1;
      u.weekdays[wd].w += w;
      u.grid[wd][h] += 1;
    }
    u.days[dayKey(day)] = d;
    u.total.recordings += d.r;
    u.total.words += d.w;
    u.total.speech_ms += d.speech_ms;
    u.total.wall_ms += d.wall_ms;
    u.total.pictures += d.p;
    u.streak.last_day = dayKey(day);
  }
  u.streak.current = run;
  return u;
}
