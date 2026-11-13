// What the Home pane draws from History: the last few recordings as plain rows. Pure, so node --test reads it without
// a window (src/lib/home.test.mjs). The usage numbers and the grid come from usage.ts.

import type { HistoryMeta } from "./api";
import type { SessionMeta } from "./sessions";

/** Home's one-time "New: Prompt with your screen" card: for someone who finished setup before Ask existed and has not
 * seen it introduced. Dismissed, it is remembered in the settings (`ask_intro_seen`). */
export const askNews = (cfg: { onboarded: boolean; ask_intro_seen?: boolean }) => cfg.onboarded && !cfg.ask_intro_seen;

/** How many recordings Home lists. */
export const RECENT = 5;

/** The list's text for an entry that kept its audio and has no words (history.rs NO_WORDS). */
export const NO_WORDS = "No words were transcribed. Transcribe again to try.";

export type RecentRow = {
  id: string;
  /** The first words, cut on a word boundary; "No words" for an entry without any. */
  text: string;
  /** The entry has no words: the row says so quietly. */
  empty: boolean;
  /** "Today, 14:02", "Yesterday, 09:30", "Monday, 16:40", "3 Nov, 10:15". */
  when: string;
  /** How many words it kept ("42 words"); how long the recording was ("42 s", "3 min") for an entry from a build that
   * did not count words, or one without words. */
  length: string;
  pictures: number;
  /** Everything the row says, for a screen reader. */
  label: string;
};

/** Whitespace folded, then cut to at most `max` characters on a word boundary, with an ellipsis when cut. A single
 * word longer than `max` is cut where it must be. */
export function firstWords(text: string, max = 56): string {
  const t = text.replace(/\s+/g, " ").trim();
  if (t.length <= max) return t;
  const at = t.slice(0, max + 1).lastIndexOf(" ");
  const head = at > 0 ? t.slice(0, at) : t.slice(0, max);
  return `${head.replace(/[\s,.;:!?-]+$/, "")}…`;
}

const startOf = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();

/** A day as a person says it from `now`: Today, Yesterday, the weekday within the week, else the date (with the year
 * only when it is not this year). By the calendar, so a clock change moves nothing. A day after today is a date. */
export function dayLabel(ms: number, now: Date, locale?: string): string {
  const d = new Date(ms);
  const back = Math.round((startOf(now) - startOf(d)) / 86_400_000);
  if (back === 0) return "Today";
  if (back === 1) return "Yesterday";
  if (back > 1 && back < 7) return d.toLocaleDateString(locale, { weekday: "long" });
  const year = d.getFullYear() === now.getFullYear() ? {} : { year: "numeric" as const };
  return d.toLocaleDateString(locale, { day: "numeric", month: "short", ...year });
}

/** The time of day as the system writes it: "09:05" and "14:02" on a 24-hour clock, "9:05 AM" on a 12-hour one. */
export function timeLabel(ms: number, locale?: string): string {
  const h24 = new Intl.DateTimeFormat(locale, { hour: "numeric" }).resolvedOptions().hourCycle?.startsWith("h2");
  return new Date(ms).toLocaleTimeString(locale, { hour: h24 ? "2-digit" : "numeric", minute: "2-digit" });
}

/** A recording's length: whole seconds under a minute ("42 s", never "0 s"), whole minutes from there ("3 min"). */
export function lengthLabel(ms: number): string {
  const s = Math.round(Math.max(0, ms) / 1000);
  return s < 60 ? `${Math.max(1, s)} s` : `${Math.round(s / 60)} min`;
}

/** A word count as the row says it: "1 word", "42 words", "1,204 words" (grouped as the system writes numbers). */
export function wordsLabel(n: number, locale?: string): string {
  const w = Math.max(0, Math.round(n));
  return `${w.toLocaleString(locale)} ${w === 1 ? "word" : "words"}`;
}

/** The newest `n` entries, newest first, as Home lists them. */
export function recentRows(
  entries: Pick<HistoryMeta, "id" | "created_ms" | "duration_ms" | "preview" | "images" | "words">[],
  now: Date,
  n = RECENT,
  locale?: string,
): RecentRow[] {
  return [...entries]
    .sort((a, b) => b.created_ms - a.created_ms)
    .slice(0, n)
    .map((e) => {
      const said = e.preview.trim();
      const empty = !said || said === NO_WORDS;
      const text = empty ? "No words" : firstWords(said);
      const when = `${dayLabel(e.created_ms, now, locale)}, ${timeLabel(e.created_ms, locale)}`;
      const words = typeof e.words === "number" && Number.isFinite(e.words) && e.words > 0 ? e.words : 0;
      const length = words && !empty ? wordsLabel(words, locale) : lengthLabel(e.duration_ms);
      const pictures = Math.max(0, e.images);
      const pics = pictures ? `, ${pictures} ${pictures === 1 ? "picture" : "pictures"}` : "";
      return { id: e.id, text, empty, when, length, pictures, label: `${text}, ${when}, ${length}${pics}` };
    });
}
