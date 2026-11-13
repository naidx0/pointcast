// Home's one Recent list: dictations and Ask sessions in a single timeline, newest first (the next-layer plan's S1).
// Both stores are only merged here, for the list; nothing is moved or rewritten. Pure, so node --test reads it without
// a window (src/lib/timeline.test.mjs).

import type { HistoryMeta } from "./api";
import { dayLabel, firstWords, lengthLabel, timeLabel, wordsLabel, NO_WORDS } from "./home.ts";
import { turnsLabel, UNTITLED, type SessionMeta } from "./sessions.ts";

/** How many entries the timeline lists. */
export const TIMELINE = 8;

export type TimelineRow = {
  /** Which store it came from: a recording ("dictation") or an Ask session ("ask"). */
  kind: "dictation" | "ask";
  id: string;
  /** The first words, or the session's title ("Untitled" when it has none). */
  text: string;
  /** A dictation with no words: the row says so quietly. */
  empty: boolean;
  /** "42 words" or "42 s"; "3 questions". */
  meta: string;
  /** The AI that answered (Ask rows only). */
  provider: string;
  /** "Today, 14:02". */
  when: string;
  /** For ordering and tests: when it happened, Unix ms. */
  at_ms: number;
  pictures: number;
  /** Everything the row says, for a screen reader. */
  label: string;
};

type Dictation = Pick<HistoryMeta, "id" | "created_ms" | "duration_ms" | "preview" | "images" | "words">;
type Session = Pick<SessionMeta, "id" | "title" | "updated_ms" | "turns" | "pictures" | "provider">;

const when = (ms: number, now: Date, locale?: string) => `${dayLabel(ms, now, locale)}, ${timeLabel(ms, locale)}`;
const pics = (n: number) => (n ? `, ${n} ${n === 1 ? "picture" : "pictures"}` : "");

/** Both stores as one list, newest first, at most `n`. Nothing is dropped by kind: whichever is newer leads. */
export function timelineRows(dictations: Dictation[], sessions: Session[], now: Date, n = TIMELINE, locale?: string): TimelineRow[] {
  const rows: TimelineRow[] = [];
  for (const e of dictations) {
    const said = e.preview.trim();
    const empty = !said || said === NO_WORDS;
    const text = empty ? "No words" : firstWords(said);
    const words = typeof e.words === "number" && Number.isFinite(e.words) && e.words > 0 ? e.words : 0;
    const meta = words && !empty ? wordsLabel(words, locale) : lengthLabel(e.duration_ms);
    const pictures = Math.max(0, e.images);
    rows.push({
      kind: "dictation",
      id: e.id,
      text,
      empty,
      meta,
      provider: "",
      when: when(e.created_ms, now, locale),
      at_ms: e.created_ms,
      pictures,
      label: `Dictated: ${text}, ${when(e.created_ms, now, locale)}, ${meta}${pics(pictures)}`,
    });
  }
  for (const s of sessions) {
    const text = s.title || UNTITLED;
    const meta = turnsLabel(s.turns);
    const provider = s.provider.trim();
    const pictures = Math.max(0, s.pictures);
    rows.push({
      kind: "ask",
      id: s.id,
      text,
      empty: false,
      meta,
      provider,
      when: when(s.updated_ms, now, locale),
      at_ms: s.updated_ms,
      pictures,
      label: `Asked: ${text}, ${provider ? `${provider}, ` : ""}${meta}, ${when(s.updated_ms, now, locale)}${pics(pictures)}`,
    });
  }
  return rows.sort((a, b) => b.at_ms - a.at_ms).slice(0, n);
}
