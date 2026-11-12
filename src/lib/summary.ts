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
  /** How many points kept their source, and how many were left out for want of one (a point whose quote is not in
   * the turn it cites). */
  kept: number;
  dropped: number;
  made_ms: number;
  /** The AI that made it, as the person knows it. */
  provider: string;
  model: string;
};
export type SummaryStateName = "none" | "working" | "done" | "error" | "stopped";
/** The payload of `summary://state`. `summary` and `counts` (the engine's own line, "9 kept, 1 dropped: no source")
 * come only with done. */
export type SummaryEvent = { id: string; state: Exclude<SummaryStateName, "none">; summary: Summary | null; counts: string; error: string };

/** The panel's view of one session's summary. `none`: there is none and none is being made. `counts`: the engine's
 * line from the last done ("" until then). */
export type SummaryModel = { id: string; state: SummaryStateName; summary: Summary | null; counts: string; error: string };

/** A session needs this many turns before a summary is offered. */
export const SUMMARY_MIN_TURNS = 2;

/** `summary_make`'s refusal while one is already being made: the work goes on, it is not a failure. */
export const ALREADY_MAKING = "A summary of this session is already being made.";

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

/** A summary with every field in its type; null when it has nothing to show. */
export function normalizeSummary(raw: unknown): Summary | null {
  const u = obj(raw);
  if (!u) return null;
  const key_points = pointsOf(u.key_points);
  const open_questions = pointsOf(u.open_questions);
  if (!key_points.length && !open_questions.length && !str(u.title)) return null;
  const kept = typeof u.kept === "number" ? whole(u.kept) : key_points.length + open_questions.length;
  const provider = obj(u.provider) ? str((u.provider as { name?: unknown }).name) : str(u.provider);
  return { title: str(u.title), key_points, open_questions, kept, dropped: whole(u.dropped), made_ms: whole(u.made_ms), provider, model: str(u.model) };
}

const STATES = ["working", "done", "error", "stopped"] as const;

export function normalizeSummaryEvent(raw: unknown): SummaryEvent | null {
  const u = obj(raw);
  const id = u && (typeof u.id === "string" || typeof u.id === "number") ? String(u.id) : "";
  if (!u || !id || !STATES.includes(u.state as SummaryEvent["state"])) return null;
  const err = typeof u.error === "string" ? u.error.trim() : str(obj(u.error)?.message);
  return { id, state: u.state as SummaryEvent["state"], summary: normalizeSummary(u.summary), counts: str(u.counts), error: err };
}

export const noSummary = (id = ""): SummaryModel => ({ id, state: "none", summary: null, counts: "", error: "" });

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
      return { ...m, state: m.state === "none" || m.state === "done" ? "done" : m.state, summary, counts: "" };
    }
    case "make":
      return a.id === m.id ? { ...m, state: "working", error: "" } : m;
    case "failed":
      // Already being made (another window started it): it is under way, and its events will say how it goes.
      if (a.id !== m.id) return m;
      return a.error.trim() === ALREADY_MAKING ? { ...m, state: "working", error: "" } : { ...m, state: "error", error: a.error.trim() || "The summary could not be made" };
    case "event": {
      const e = normalizeSummaryEvent(a.event);
      if (!e || e.id !== m.id) return m;
      // Done brings the new summary; working, stopped and a failure keep the one before, if there was one.
      const summary = e.summary ?? (e.state === "done" ? null : m.summary);
      const counts = e.state === "done" ? e.counts : m.counts;
      return { id: m.id, state: e.state === "done" && !summary ? "none" : e.state, summary, counts, error: e.state === "error" ? e.error || "The summary could not be made" : "" };
    }
  }
}

/** The quiet line under a summary, as the engine words it (summary.rs `counts`): "9 kept, 1 dropped: no source", or
 * "4 kept". "" when nothing was counted. */
export function countsLine(kept: number, dropped: number): string {
  if (!kept && !dropped) return "";
  const points = `${kept} ${kept === 1 ? "point" : "points"}, every quote found in the session`;
  return dropped ? `${points}; ${dropped} left out, ${dropped === 1 ? "its quote was" : "their quotes were"} not` : points;
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

/** What the summary view shows: the title, "Key points" and "Still open" (each only when it has points), and the foot
 * (the engine's counts line when it sent one, else the same words from the summary's own counts). `turns`: the turns
 * the session has, so a reference to one of them can go there. */
export function summaryView(s: Summary, counts: string, turns: number[]): SummaryView {
  const known = (n: number) => turns.includes(n);
  const groups: PointGroup[] = [
    { id: "points" as const, label: "Key points", items: s.key_points.map((p, i) => pointView(p, i, "points", known)) },
    { id: "open" as const, label: "Still open", items: s.open_questions.map((p, i) => pointView(p, i, "open", known)) },
  ].filter((g) => g.items.length);
  return { title: s.title || "Summary", groups, foot: [counts || countsLine(s.kept, s.dropped), s.provider].filter(Boolean).join(" · ") };
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
    made_ms: 0,
    provider,
    model: "gpt-5.1",
  };
}
