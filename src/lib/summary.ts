// A summary of an Ask session: its key points and the questions still open, each resting on words from one turn (a
// quote, from the person's question or the AI's answer). The engine makes it (`summary_make`), says how it goes
// (`summary://state`) and keeps it (`summary_get`); the names live in the SUMMARY adapter in api.ts. Pure, so node
// --test reads it without a window (src/lib/summary.test.mjs).

export type PointFrom = "question" | "answer";
/** One line of a summary: what it says, the turn it rests on, and the words it rests on there. */
export type SummaryPoint = { text: string; turn: number; from: PointFrom; quote: string };
export type Summary = {
  title: string;
  key_points: SummaryPoint[];
  open_questions: SummaryPoint[];
  /** How many points were kept, and how many left out with why (a point whose quote is not in the conversation is
   * left out: "no source"). */
  kept: number;
  dropped: number;
  why: string[];
  made_ms: number;
  provider: string;
  model: string;
};
export type SummaryCounts = { kept: number; dropped: number; why: string[] };
export type SummaryStateName = "none" | "working" | "done" | "error" | "stopped";
/** The payload of `summary://state`. */
export type SummaryEvent = { id: string; state: Exclude<SummaryStateName, "none">; summary: Summary | null; counts: SummaryCounts | null; error: string };

/** The panel's view of one session's summary. `none`: there is none and none is being made. */
export type SummaryModel = { id: string; state: SummaryStateName; summary: Summary | null; counts: SummaryCounts | null; error: string };

/** A session needs this many turns before a summary is offered. */
export const SUMMARY_MIN_TURNS = 2;

export const canSummarize = (turns: number) => turns >= SUMMARY_MIN_TURNS;

const fold = (t: string) => t.replace(/\s+/g, " ").trim();
const str = (v: unknown) => (typeof v === "string" ? fold(v) : "");
const whole = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v > 0 ? Math.round(v) : 0);
const obj = (v: unknown) => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null);

function pointsOf(raw: unknown): SummaryPoint[] {
  return (Array.isArray(raw) ? raw : []).flatMap((p) => {
    const u = obj(p);
    const text = u ? str(u.text) : "";
    if (!u || !text) return [];
    return [{ text, turn: whole(u.turn), from: u.from === "question" ? "question" : "answer", quote: str(u.quote) } as SummaryPoint];
  });
}

/** A reason as a person says it: "no_source" is "no source". */
const reason = (r: string) => fold(r.replace(/[_-]+/g, " ")).toLowerCase();

/** The reasons points were left out, whatever shape they came in: a string, a list (of strings or of { why } /
 * { reason }), or a map of reason to count. Each once, in order. */
function reasonsOf(...raws: unknown[]): string[] {
  const out: string[] = [];
  const add = (r: unknown) => {
    const t = typeof r === "string" ? reason(r) : "";
    if (t && !out.includes(t)) out.push(t);
  };
  for (const raw of raws) {
    if (typeof raw === "string") add(raw);
    else if (Array.isArray(raw))
      raw.forEach((x) => {
        const u = obj(x);
        add(u ? (u.why ?? u.reason) : x);
      });
    else if (obj(raw)) Object.keys(raw as object).forEach(add);
  }
  return out;
}

/** Counts in any of the shapes above: numbers, or the dropped points themselves. Null when nothing was counted. */
export function normalizeCounts(raw: unknown): SummaryCounts | null {
  const u = obj(raw);
  if (!u) return null;
  const n = (v: unknown) => (Array.isArray(v) ? v.length : whole(v));
  const kept = n(u.kept);
  const dropped = n(u.dropped);
  const why = reasonsOf(u.why, u.reasons, Array.isArray(u.dropped) ? u.dropped : null);
  return kept || dropped ? { kept, dropped, why } : null;
}

/** A summary with every field in its type; null when it has nothing to show. */
export function normalizeSummary(raw: unknown): Summary | null {
  const u = obj(raw);
  if (!u) return null;
  const key_points = pointsOf(u.key_points);
  const open_questions = pointsOf(u.open_questions);
  if (!key_points.length && !open_questions.length && !str(u.title)) return null;
  const c = normalizeCounts(u) ?? { kept: key_points.length + open_questions.length, dropped: 0, why: [] };
  const provider = obj(u.provider) ? str((u.provider as { name?: unknown }).name) : str(u.provider);
  return { title: str(u.title), key_points, open_questions, ...c, made_ms: whole(u.made_ms), provider, model: str(u.model) };
}

const STATES = ["working", "done", "error", "stopped"] as const;

export function normalizeSummaryEvent(raw: unknown): SummaryEvent | null {
  const u = obj(raw);
  const id = u && (typeof u.id === "string" || typeof u.id === "number") ? String(u.id) : "";
  if (!u || !id || !STATES.includes(u.state as SummaryEvent["state"])) return null;
  const err = typeof u.error === "string" ? u.error.trim() : str(obj(u.error)?.message);
  return { id, state: u.state as SummaryEvent["state"], summary: normalizeSummary(u.summary), counts: normalizeCounts(u.counts), error: err };
}

export const noSummary = (id = ""): SummaryModel => ({ id, state: "none", summary: null, counts: null, error: "" });

export type SummaryAction =
  /** The panel is on another session (or none): nothing of the last one's summary stays. */
  | { type: "session"; id: string }
  /** `summary_get`'s reply for a session. */
  | { type: "got"; id: string; summary: unknown }
  /** Summarize or Redo was pressed: working at once, before the engine says so. */
  | { type: "make"; id: string }
  /** `summary_make` or `summary_stop` refused. */
  | { type: "failed"; id: string; error: string }
  | { type: "event"; event: unknown };

export function summaryReduce(m: SummaryModel, a: SummaryAction): SummaryModel {
  switch (a.type) {
    case "session":
      return a.id === m.id ? m : noSummary(a.id);
    case "got": {
      // A late reply for another session, or one overtaken by work under way, changes nothing.
      if (a.id !== m.id || m.state === "working") return m;
      const summary = normalizeSummary(a.summary);
      if (!summary) return m;
      return { ...m, state: m.state === "none" || m.state === "done" ? "done" : m.state, summary, counts: null };
    }
    case "make":
      return a.id === m.id ? { ...m, state: "working", error: "" } : m;
    case "failed":
      return a.id === m.id ? { ...m, state: "error", error: a.error } : m;
    case "event": {
      const e = normalizeSummaryEvent(a.event);
      if (!e || e.id !== m.id) return m;
      // Done brings the new summary; working, stopped and a failure keep the one before, if there was one.
      const summary = e.summary ?? (e.state === "done" ? null : m.summary);
      const counts = e.counts ?? (e.summary || e.state === "done" ? null : m.counts);
      return { id: m.id, state: e.state === "done" && !summary ? "none" : e.state, summary, counts, error: e.state === "error" ? e.error || "The summary could not be made" : "" };
    }
  }
}

/** The quiet line under a summary: "9 kept, 1 dropped: no source". "" when nothing was counted. */
export function countsLine(c: SummaryCounts | null): string {
  if (!c || (!c.kept && !c.dropped)) return "";
  const kept = `${c.kept} kept`;
  if (!c.dropped) return kept;
  return `${kept}, ${c.dropped} dropped${c.why.length ? `: ${c.why.join(", ")}` : ""}`;
}

/** One point as the view shows it. `by`: the one small word that says whose words the quote is ("You" for the
 * person's question, "AI" for an answer), so it never rests on colour alone. */
export type PointView = {
  key: string;
  text: string;
  quote: string;
  turn: number;
  /** "T3"; "" when the point names no turn. */
  ref: string;
  /** The turn is one of the session's: its reference goes there. */
  go: boolean;
  by: "You" | "AI";
  /** Everything the point says, for a screen reader. */
  label: string;
};
export type PointGroup = { id: "points" | "open"; label: string; items: PointView[] };
export type SummaryView = {
  title: string;
  groups: PointGroup[];
  /** The counts and the AI that made it: "9 kept, 1 dropped: no source · ChatGPT". */
  foot: string;
};

function pointView(p: SummaryPoint, i: number, group: string, known: (n: number) => boolean): PointView {
  const by = p.from === "question" ? "You" : "AI";
  const whose = by === "You" ? "your words" : "the AI's words";
  const from = p.turn ? `From question ${p.turn}` : "";
  const quote = p.quote ? `${whose}: ${p.quote}` : "";
  return {
    key: `${group}-${i}`,
    text: p.text,
    quote: p.quote,
    turn: p.turn,
    ref: p.turn ? `T${p.turn}` : "",
    go: !!p.turn && known(p.turn),
    by,
    label: [p.text, [from, quote].filter(Boolean).join(", ")].filter(Boolean).join(". "),
  };
}

/** What the summary view shows: the title, "Key points" and "Still open" (each only when it has points), and the foot.
 * `turns`: the turns the session has, so a reference to one of them can go there. */
export function summaryView(s: Summary, counts: SummaryCounts | null, turns: number[]): SummaryView {
  const known = (n: number) => turns.includes(n);
  const groups: PointGroup[] = [
    { id: "points" as const, label: "Key points", items: s.key_points.map((p, i) => pointView(p, i, "points", known)) },
    { id: "open" as const, label: "Still open", items: s.open_questions.map((p, i) => pointView(p, i, "open", known)) },
  ].filter((g) => g.items.length);
  const c = counts ?? { kept: s.kept, dropped: s.dropped, why: s.why };
  return { title: s.title || "Summary", groups, foot: [countsLine(c), s.provider].filter(Boolean).join(" · ") };
}

/** The summary as plain text, for Copy as text: the title, each group with its points, the turn each rests on and its
 * quote with whose words it is, then the foot. */
export function summaryText(v: SummaryView): string {
  const nl = String.fromCharCode(10);
  const parts = [v.title];
  for (const g of v.groups) {
    const lines = g.items.flatMap((p) => [`- ${p.text}${p.ref ? ` (${p.ref})` : ""}`, ...(p.quote ? [`  ${p.by}: “${p.quote}”`] : [])]);
    parts.push([g.label, ...lines].join(nl));
  }
  if (v.foot) parts.push(v.foot);
  return parts.join(nl + nl);
}

// —— Browser preview fixtures: data only; api.ts plays them. ——

/** A summary of the panel's three-question preview session; one point was left out for want of a source. */
export function previewSummary(provider = "ChatGPT"): Summary {
  return {
    title: "Why Bayes' rule divides by P(B)",
    key_points: [
      { text: "The line over B means not B: every case where B does not happen.", turn: 1, from: "answer", quote: "It means not B: every case where B does not happen." },
      { text: "P(B|A) and P(A|B) are not the same; Bayes' rule turns one into the other.", turn: 2, from: "answer", quote: "Bayes' rule is how you turn one into the other." },
      { text: "Dividing by P(B) turns a count of cases into a share of all the ways B can happen.", turn: 3, from: "answer", quote: "Dividing by P(B) turns that count into a share of all the ways B can happen" },
      { text: "With a 1% base rate, one positive test means about a 16% chance.", turn: 3, from: "answer", quote: "one positive test means about a 16% chance" },
    ],
    open_questions: [{ text: "Whether multiplying the two alone ever gives a probability you asked for.", turn: 3, from: "question", quote: "and not just multiply the two?" }],
    kept: 5,
    dropped: 1,
    why: ["no source"],
    made_ms: 0,
    provider,
    model: "gpt-5.1",
  };
}

/** Any other preview session's summary: two points and nothing left out. */
export function previewSummaryOf(title: string, provider = "ChatGPT"): Summary {
  return {
    title,
    key_points: [
      { text: "The picture shows what the question was about.", turn: 1, from: "answer", quote: "Here is what the picture shows, in short." },
      { text: "Each later part works the same way as the first.", turn: 2, from: "answer", quote: "Part 2 works the same way." },
    ],
    open_questions: [{ text: "What happens after the second part.", turn: 2, from: "question", quote: "And what about part 2?" }],
    kept: 3,
    dropped: 0,
    why: [],
    made_ms: 0,
    provider,
    model: "gpt-5.1",
  };
}
