// Ask's kept sessions as lists show them: the panel's session list and Home's "Recent questions". Pure, so node --test
// reads it without a window (src/lib/sessions.test.mjs). The engine's names live in the SESSIONS adapter in api.ts.

import { dayLabel } from "./home.ts";

/** One kept session (`sessions_list`); `hit` is the matching words when it came from `sessions_search`. */
export type SessionMeta = {
  id: string;
  title: string;
  updated_ms: number;
  turns: number;
  pictures: number;
  provider: string;
  size_bytes: number;
  hit?: string;
};

/** How many sessions Home lists under Ask. */
export const RECENT_SESSIONS = 3;
/** The longest title a rename keeps. */
export const TITLE_MAX = 80;
export const UNTITLED = "Untitled";

const fold = (t: string) => t.replace(/\s+/g, " ").trim();
const count = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.round(v) : 0);

/** The engine's list with every field in its type: rows without an id are dropped, one row per id (the first wins). */
export function normalizeSessions(raw: unknown): SessionMeta[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const out: SessionMeta[] = [];
  for (const r of raw) {
    if (!r || typeof r !== "object") continue;
    const u = r as Record<string, unknown>;
    const id = typeof u.id === "string" || typeof u.id === "number" ? String(u.id) : "";
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const provider = u.provider && typeof u.provider === "object" ? (u.provider as { name?: unknown }).name : u.provider;
    out.push({
      id,
      title: typeof u.title === "string" ? fold(u.title) : "",
      updated_ms: count(u.updated_ms),
      turns: count(u.turns),
      pictures: count(u.pictures),
      provider: typeof provider === "string" ? provider : "",
      size_bytes: count(u.size_bytes),
      ...(typeof u.hit === "string" && fold(u.hit) ? { hit: fold(u.hit) } : {}),
    });
  }
  return out;
}

/** "1 question", "3 questions". */
export function turnsLabel(n: number): string {
  const t = Math.max(0, Math.round(n));
  return `${t} ${t === 1 ? "question" : "questions"}`;
}

export type SessionRow = {
  id: string;
  title: string;
  /** "3 questions". */
  turns: string;
  /** "Today", "Yesterday", "Monday", "3 Nov". */
  day: string;
  /** The words that matched a search, else "". */
  hit: string;
  /** The session open in the panel. */
  current: boolean;
  /** Everything the row says, for a screen reader. */
  label: string;
};

/** Newest first, at most `n` (all when n is 0). A search keeps the engine's order, which is by match. */
export function sessionRows(
  list: SessionMeta[],
  now: Date,
  o: { n?: number; current?: string; searched?: boolean; locale?: string } = {},
): SessionRow[] {
  const sorted = o.searched ? [...list] : [...list].sort((a, b) => b.updated_ms - a.updated_ms);
  return (o.n ? sorted.slice(0, o.n) : sorted).map((s) => {
    const title = s.title || UNTITLED;
    const turns = turnsLabel(s.turns);
    const day = s.updated_ms ? dayLabel(s.updated_ms, now, o.locale) : "";
    const current = !!o.current && s.id === o.current;
    return {
      id: s.id,
      title,
      turns,
      day,
      hit: s.hit ?? "",
      current,
      label: [title, turns, day.toLowerCase(), current ? "open now" : ""].filter(Boolean).join(", "),
    };
  });
}

/** A rename as it will be kept: folded, at most TITLE_MAX characters; null when it would change nothing (empty, or
 * the same title), so Enter on an untouched field just closes it. */
export function renameValue(input: string, was: string): string | null {
  const t = fold(input).slice(0, TITLE_MAX).trim();
  return !t || t === fold(was) ? null : t;
}

// —— Browser preview fixtures: data only; api.ts plays them. ——

/** Five sessions over two weeks; the newest is the panel's three-question preview session. */
export function previewSessions(now: number): SessionMeta[] {
  const day = 86_400_000;
  return [
    { id: "preview", title: "Bayes' rule on the slide", updated_ms: now - 60_000, turns: 3, pictures: 2, provider: "ChatGPT", size_bytes: 412_000 },
    { id: "s-header", title: "Why the header wraps on a small screen", updated_ms: now - day - 3_600_000, turns: 2, pictures: 2, provider: "Codex", size_bytes: 388_000 },
    { id: "s-regex", title: "What this regular expression matches", updated_ms: now - 3 * day, turns: 1, pictures: 1, provider: "ChatGPT", size_bytes: 96_000 },
    { id: "s-loss", title: "Reading the loss curve", updated_ms: now - 9 * day, turns: 4, pictures: 3, provider: "ChatGPT", size_bytes: 702_000 },
    { id: "s-gap", title: "The gap between the toolbar buttons", updated_ms: now - 15 * day, turns: 2, pictures: 1, provider: "Codex", size_bytes: 201_000 },
  ];
}

/** The preview's search: titles that hold every word, in list order, each with the words it matched. */
export function previewSearch(list: SessionMeta[], q: string): SessionMeta[] {
  const words = fold(q).toLowerCase().split(" ").filter(Boolean);
  return list.filter((s) => words.every((w) => s.title.toLowerCase().includes(w))).map((s) => ({ ...s, hit: s.title }));
}

/** What goes to `sessions_search`: the folded words, or null for an empty field (the recent list shows instead). */
export function searchQuery(q: string): string | null {
  const t = fold(q);
  return t ? t : null;
}
