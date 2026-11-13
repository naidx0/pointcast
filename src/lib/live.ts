// The Ask panel's state. It mirrors the engine: one snapshot event, one delta event and four commands, whose names live
// in one place only (the adapter at the end of api.ts). Pure: the reducer, what each state shows, the 8 s and 20 s
// wait lines on an injected clock, and the browser preview's fixtures, so node --test reads it without a window
// (src/lib/live.test.mjs).

export type LiveStateName = "closed" | "listening" | "finishing" | "sending" | "answering" | "shown" | "error" | "discarded";
/** `label`: the app or window the picture is of, when the engine names it ("" otherwise). `marked`: the person marked
 * it up in the overlay (`live_mark`) and the marked one is what goes. */
export type LivePicture = { src: string; w: number; h: number; label?: string; marked?: boolean };
/** The engine's error: a message, or a code with an optional message. `no_provider` asks for Settings. */
export type LiveErrorPayload = string | { code?: string; message?: string } | null;

/** A finished turn of the open session (`turns` on the snapshot, oldest first): what was asked, what came back, the
 * picture that went with it, and whether it was stopped or failed. */
export type LiveTurn = {
  n: number;
  question: string;
  answer: string;
  picture: LivePicture | null;
  stopped: boolean;
  /** Why it was not answered, as one line; null when it was. */
  error: string | null;
  at_ms: number;
  /** What the AI did on the computer while it answered, one plain line each ("Clicked button "Send" in Mail"). */
  acts?: string[];
};

/** The payload of `live://state` and the reply of `live_state`. */
export type LiveSnapshot = {
  state: LiveStateName;
  /** `turns`: how many turns the session has kept, the one in the fields below included once it is kept (so then
   * that one's number). */
  session: { id: string; title: string; turns: number };
  /** The session's finished turns, oldest first, never the one in the fields below. After `sessions_open` every turn
   * is here and the fields below are empty. Joining a closed session, they arrive a moment after the first snapshot. */
  turns: LiveTurn[];
  /** Which question of this panel opening the fields below belong to (what deltas carry), counted from 1; 0 before
   * the first. Not the turn's number in the session: that is `openNumber`. */
  turn: number;
  /** The person's words: as they are heard while listening, then the question. */
  words: string;
  /** Exactly the picture that will be sent (a data: or blob: URL), or null. */
  picture: LivePicture | null;
  /** The person took the picture out: nothing but the words is sent. */
  dropped: boolean;
  /** Why the question has no picture of the screen: Pointcast is not allowed to see it (macOS Screen Recording), or
   * reading it failed. Null when it has one, or when none was taken. */
  no_picture?: NoPicture | null;
  /** A picture marked up with no question under way: it goes with the next question, spoken or typed, whatever the
   * field's picture switch says; `live_drop_picture` takes it off. */
  next_picture: LivePicture | null;
  provider: string | { id?: string; name?: string } | null;
  error: LiveErrorPayload;
  /** True while only reasoning has arrived for the current turn: the AI is working, nothing to show yet. */
  thinking?: boolean;
  /** The answer so far, if the engine sends it with the snapshot (a window opened mid-answer). Optional. */
  answer?: string;
  /** What the AI did on the computer for the turn in the fields above, once it ended. */
  acts?: string[];
};

export type NoPicture = "not_allowed" | "failed";

/** The payload of `live://delta`: text to append to the answer of `turn`. */
export type LiveDelta = { turn: number; text: string };

export type LiveModel = {
  snap: LiveSnapshot;
  /** The answer of snap.turn, as streamed so far. */
  text: string;
  /** The question of snap.turn has left the computer: the engine said sending or answering for it, or an answer
   * arrived. Only then does the panel say it was sent. A new question (listening) starts unsent. */
  sent: boolean;
  /** Clock time at which the request to the AI started waiting, while it waits. */
  waitFrom: number | null;
};

export type LiveAction = { type: "snapshot"; snap: LiveSnapshot; now: number } | { type: "deltas"; deltas: LiveDelta[] };

export const CLOSED: LiveSnapshot = {
  state: "closed",
  session: { id: "", title: "", turns: 0 },
  turns: [],
  turn: 0,
  words: "",
  picture: null,
  dropped: false,
  next_picture: null,
  provider: null,
  error: null,
};

export const initialLive = (): LiveModel => ({ snap: CLOSED, text: "", sent: false, waitFrom: null });

const STATES: LiveStateName[] = ["closed", "listening", "finishing", "sending", "answering", "shown", "error", "discarded"];

/** A picture with every field in its type, or null when it has no address. */
function pictureOf(p: unknown): LivePicture | null {
  const v = (p && typeof p === "object" ? p : {}) as Record<string, unknown>;
  if (typeof v.src !== "string") return null;
  return {
    src: v.src,
    w: Number(v.w) || 0,
    h: Number(v.h) || 0,
    label: typeof v.label === "string" ? v.label.trim() : "",
    ...(v.marked === true ? { marked: true } : {}),
  };
}

/** An engine error as one line: its message, else a plain sentence when only a code came; null for none. */
function errorLine(e: unknown): string | null {
  if (typeof e === "string") return e.trim() || null;
  if (e && typeof e === "object") {
    const o = e as { message?: unknown; code?: unknown };
    const m = typeof o.message === "string" ? o.message.trim() : "";
    return m || (o.code ? "Not answered" : null);
  }
  return null;
}

/** The session's finished turns, oldest first, one per number (the last word wins), whatever arrived. */
export function normalizeTurns(raw: unknown): LiveTurn[] {
  if (!Array.isArray(raw)) return [];
  const by = new Map<number, LiveTurn>();
  raw.forEach((t, i) => {
    if (!t || typeof t !== "object") return;
    const u = t as Record<string, unknown>;
    const n = typeof u.n === "number" && Number.isFinite(u.n) ? u.n : i + 1;
    by.set(n, {
      n,
      question: typeof u.question === "string" ? u.question : "",
      answer: typeof u.answer === "string" ? u.answer : "",
      picture: pictureOf(u.picture),
      stopped: u.stopped === true,
      error: errorLine(u.error),
      at_ms: typeof u.at_ms === "number" && Number.isFinite(u.at_ms) ? u.at_ms : 0,
      ...withActs(u.acts),
    });
  });
  return [...by.values()].sort((a, b) => a.n - b.n);
}

/** A snapshot with every field in its type, whatever arrived. */
export function normalizeSnapshot(s: Partial<LiveSnapshot> | null | undefined): LiveSnapshot {
  const v = s ?? {};
  const turns = normalizeTurns((v as { turns?: unknown }).turns);
  const kept = Number((v.session as { turns?: unknown } | undefined)?.turns);
  return {
    state: STATES.includes(v.state as LiveStateName) ? (v.state as LiveStateName) : "closed",
    session: {
      id: String(v.session?.id ?? ""),
      title: String(v.session?.title ?? ""),
      turns: Number.isFinite(kept) && kept >= 0 ? kept : turns.length,
    },
    turns,
    turn: Number.isFinite(v.turn) ? Number(v.turn) : 0,
    words: typeof v.words === "string" ? v.words : "",
    picture: pictureOf(v.picture),
    dropped: !!v.dropped,
    no_picture: v.no_picture === "not_allowed" || v.no_picture === "failed" ? v.no_picture : null,
    next_picture: pictureOf((v as { next_picture?: unknown }).next_picture),
    provider: v.provider ?? null,
    error: v.error ?? null,
    thinking: v.thinking === true,
    answer: typeof v.answer === "string" ? v.answer : undefined,
    ...withActs((v as { acts?: unknown }).acts),
  };
}

/** `acts` only when the AI did something on the computer, so a turn without any reads as it always did. */
function withActs(raw: unknown): { acts?: string[] } {
  const acts = actsOf(raw);
  return acts.length ? { acts } : {};
}

/** The lines of what the AI did, whatever arrived: strings only, none when there are none. */
export function actsOf(raw: unknown): string[] {
  return Array.isArray(raw) ? raw.filter((a): a is string => typeof a === "string" && a.trim() !== "") : [];
}

export function liveReduce(m: LiveModel, a: LiveAction): LiveModel {
  if (a.type === "deltas") {
    let text = m.text;
    // A delta for any turn but the current one is late or early: dropped, never appended to the wrong answer.
    for (const d of a.deltas) if (d && d.turn === m.snap.turn && typeof d.text === "string") text += d.text;
    return text === m.text ? m : { ...m, text, sent: true };
  }
  const s = normalizeSnapshot(a.snap);
  // The session id is "" until the first question is kept: that is the same session, not a new one.
  const sameTurn = s.turn === m.snap.turn && (s.session.id === m.snap.session.id || !s.session.id || !m.snap.session.id);
  // Listening is a new question: whatever was answered before is not this one's.
  const fresh = !sameTurn || s.state === "listening";
  let text = fresh ? "" : m.text;
  if (s.answer !== undefined) text = s.answer;
  // A session opened finished: its open turn's answer is the one kept with it.
  if (!text && s.state === "shown") text = currentTurn(s)?.answer ?? "";
  const sent = (!fresh && m.sent) || s.state === "sending" || s.state === "answering" || text !== "";
  const waitFrom =
    s.state === "sending"
      ? m.snap.state === "sending" && sameTurn && m.waitFrom != null
        ? m.waitFrom
        : a.now
      : null;
  return { snap: s, text, sent, waitFrom };
}

/** The microphone level in the session stream's payload (UiState), 0 to 1: the first of `fields` that is a number, so
 * an engine-smoothed level is preferred when the engine sends one. 0 when the microphone is off or nothing fits. */
/** The fields of `session://state` the line reads, first found wins: the engine's Ask loudness (RMS over 50 ms,
 * rising at once and falling over about 150 ms), then older names, then the raw level. */
export const LEVEL_FIELDS = ["loudness", "level_smooth", "voice", "level"] as const;

export function levelOf(p: unknown, fields: readonly string[]): number {
  const u = (p && typeof p === "object" ? p : {}) as Record<string, unknown>;
  if (u.mic === false) return 0;
  for (const f of fields) {
    const v = u[f];
    if (typeof v === "number" && Number.isFinite(v)) return Math.min(1, Math.max(0, v));
  }
  return 0;
}

/** After this long waiting for the AI, one quiet line says so. */
export const SLOW_MS = 8_000;
/** After this long with nothing back from the AI, the wait is an error with Stop. */
export const STALL_MS = 20_000;

/** discarded: the question was stopped or thrown away before an answer; the panel says so for a moment and hides. */
export type Phase = "closed" | "listening" | "finishing" | "thinking" | "answering" | "shown" | "error" | "discarded";
/** The line's behaviour: drawn by the voice, looping, down the answer's margin, or resting. Never two at once. */
export type LineMode = "voice" | "loops" | "margin" | "rest" | "none";
/** ink: the work is on this computer. accent: going to or coming from the AI. warn: a wait ran long or failed. */
export type Tone = "ink" | "accent" | "warn" | "quiet";
/** The one action an error offers: open Settings (no AI chosen, or one not allowed yet), ask the kept question again,
 * stop the request still in flight, or close the panel. */
export type ErrorAction = "settings" | "retry" | "stop" | "close" | null;
export type LiveErrorView = { text: string; action: ErrorAction; label: string | null };

export type LiveView = {
  phase: Phase;
  line: LineMode;
  tone: Tone;
  /** The red dot: the microphone is live. Nothing else is red. */
  mic: boolean;
  /** A request to the AI is in flight: its name takes the accent. */
  inFlight: boolean;
  provider: string;
  /** The session's title: empty while a question is still being heard (the title is not the transcript). */
  title: string;
  /** One row and nothing else: the line, the dot, the picture. While listening and finishing, and the brief "Not sent". */
  compact: boolean;
  /** The question as asked, once it is asked; empty while it is being heard. */
  question: string;
  thumb: LivePicture | null;
  /** The app or window the picture is of ("" when the engine does not say). */
  label: string;
  /** The picture can still be taken out (it has not left the computer). */
  canDrop: boolean;
  note: string | null;
  error: LiveErrorView | null;
  /** Stop is offered and Esc stops; otherwise Esc closes. */
  stop: boolean;
  /** The answer is complete: the foot row and Copy. Only for a question that was sent and answered. */
  complete: boolean;
  /** This turn's question left the computer. */
  sent: boolean;
  /** The panel hides itself after a moment (discarded). */
  hide: boolean;
  /** What a screen reader hears, politely, when it changes. */
  announce: string;
  /** A new session, open on nothing yet (`sessions_new`): one quiet line and the field, ready. */
  empty: boolean;
  /** The marked picture waiting for the next question (shown by the field), or null. */
  next: LivePicture | null;
  /** The question goes without a picture of the screen, and why: one quiet line, with Allow when only a permission
   * is missing. Null when it has a picture or the person took it out. */
  noPicture: NoPictureView | null;
};

export type NoPictureView = { text: string; allow: boolean };

/** What the panel says when the screen could not be seen. The question still goes with its words. */
export function noPictureView(why: NoPicture | null | undefined): NoPictureView | null {
  if (why === "not_allowed") return { text: "No picture: Pointcast isn't allowed to see your screen.", allow: true };
  if (why === "failed") return { text: "No picture: the screen could not be read.", allow: false };
  return null;
}

export function providerName(p: LiveSnapshot["provider"]): string {
  if (!p) return "";
  if (typeof p === "string") return p;
  return p.name || p.id || "";
}

/** Only a picture held on this computer is shown: a data: image or a blob: URL. Anything else (a web address) is never
 * requested. */
export function safePicture(p: LivePicture | null): LivePicture | null {
  if (!p) return null;
  const src = p.src.trim().toLowerCase();
  return src.startsWith("data:image/") || src.startsWith("blob:") ? p : null;
}

/** The one quiet line under the listening row: what the picture that will go is of. */
export function pictureCaption(label: string): string {
  const of = label.trim();
  return of ? `Picture of ${of}` : "Picture of your screen";
}

/** `kept`: the engine still holds the question, so Try again asks it again. */
export function errorView(e: LiveErrorPayload, provider: string, kept = false): LiveErrorView {
  const code = typeof e === "object" && e ? e.code ?? "" : "";
  const message = (typeof e === "string" ? e : e?.message ?? "").trim();
  if (code === "no_provider" || (!provider && !message) || /^choose an ai/i.test(message))
    return { text: "Choose an AI in Settings first", action: "settings", label: "Open Settings" };
  if (code === "not_allowed" || /^allow .+ in settings first/i.test(message))
    return { text: message || `Allow ${provider || "the AI"} in Settings first`, action: "settings", label: "Open Settings" };
  const text = message || `${provider || "The AI"} could not answer`;
  return kept ? { text, action: "retry", label: "Try again" } : { text, action: "close", label: "Close" };
}

/** The first words of the question, for the header until the engine gives the session a title. */
export function firstWords(words: string, n = 6): string {
  const w = words.trim().split(" ").filter(Boolean);
  return w.length > n ? w.slice(0, n).join(" ") + "…" : w.join(" ");
}

/** Shown with nothing in the panel's own fields: no words, no answer, no picture, no error. A session opened from the
 * list (its turns kept) or a new one (none yet). */
function settledBare(s: LiveSnapshot): boolean {
  return s.state === "shown" && !s.words.trim() && !(s.answer ?? "").trim() && !s.picture && !s.error;
}

/** The turn the panel shows open when it is one of `turns`: after `sessions_open`, the newest kept turn. Null while
 * the open turn is the one in the panel's own fields (the engine never lists that one in `turns`). */
export function currentTurn(s: LiveSnapshot): LiveTurn | null {
  return settledBare(s) && s.turns.length ? s.turns[s.turns.length - 1] : null;
}

/** A new session open on nothing (`sessions_new`): shown, no turns, nothing in the fields. */
export function isEmptySession(s: LiveSnapshot): boolean {
  return settledBare(s) && !s.turns.length;
}

/** The kept turns before the open one, oldest first: every kept turn, less the one opened from the list. */
export function earlierTurns(s: LiveSnapshot): LiveTurn[] {
  return currentTurn(s) ? s.turns.slice(0, -1) : s.turns;
}

/** The open turn's number in the session (what `live_sent` and a summary's references use). The engine counts it in
 * `session.turns` once it keeps it (at sending), and never lists it in `turns`; a question still being heard is not
 * kept yet, and joining a closed session its earlier turns may not have arrived. 0 for a new session. */
export function openNumber(s: LiveSnapshot): number {
  const opened = currentTurn(s);
  if (opened) return opened.n;
  if (isEmptySession(s)) return 0;
  const last = s.turns.reduce((n, t) => Math.max(n, t.n), 0);
  if (s.state === "listening" || s.state === "finishing") return Math.max(s.session.turns, last) + 1;
  return Math.max(s.session.turns, last + 1);
}

export function liveView(m: LiveModel, now: number): LiveView {
  const s = m.snap;
  const provider = providerName(s.provider);
  // A finished turn shows what was kept with it when the top-level fields are empty (a session opened from the list).
  const kept = s.state === "shown" ? currentTurn(s) : null;
  const thumb = s.dropped ? null : safePicture(s.picture ?? kept?.picture ?? null);
  const base: LiveView = {
    phase: "closed",
    line: "none",
    tone: "ink",
    mic: false,
    inFlight: false,
    provider,
    title: s.session.title || firstWords(s.words),
    compact: false,
    question: s.words.trim() || (kept?.question.trim() ?? ""),
    thumb,
    label: thumb?.label ?? "",
    canDrop: false,
    note: null,
    error: null,
    stop: false,
    complete: false,
    sent: m.sent,
    hide: false,
    announce: "",
    empty: false,
    next: safePicture(s.next_picture),
    noPicture: thumb || s.dropped ? null : noPictureView(s.no_picture),
  };
  // A brand-new question is heard in one quiet row. A spoken follow-up (the session already has a turn: the engine
  // lists the one just answered in `turns` as soon as the press listens) keeps the conversation in view, its title and
  // the earlier turns above, the listening row where the field was, so the person sees it goes on.
  const follow = s.turns.length > 0;
  const hearing = follow ? { compact: false, question: "" } : { compact: true, title: "", question: "" };
  const name = provider || "the AI";
  switch (s.state) {
    case "closed":
      return { ...base, thumb: null, noPicture: null };
    case "listening":
      return { ...base, ...hearing, phase: "listening", line: "voice", mic: true, canDrop: !!thumb, stop: true, announce: "Listening" };
    case "finishing":
      return { ...base, ...hearing, phase: "finishing", line: "loops", canDrop: !!thumb, stop: true, announce: "Finishing your words" };
    case "sending": {
      // A first delta can arrive before the snapshot that says so: the answer has started.
      if (m.text) return { ...base, phase: "answering", line: "margin", tone: "accent", inFlight: true, stop: true, announce: `${provider || "The AI"} is answering` };
      const waited = m.waitFrom == null ? 0 : now - m.waitFrom;
      // Reasoning arriving is the AI at work, not a stall: the loops go on and the long-wait line says so.
      if (s.thinking) return thinkingView(base, name, waited >= SLOW_MS ? `${provider || "The AI"} is thinking` : null);
      if (waited >= STALL_MS)
        return {
          ...base,
          phase: "error",
          line: "rest",
          tone: "warn",
          inFlight: true,
          stop: true,
          error: { text: `${provider || "The AI"} has not answered`, action: "stop", label: "Stop" },
          announce: `${provider || "The AI"} has not answered`,
        };
      return thinkingView(base, name, waited >= SLOW_MS ? `Waiting for ${name}` : null);
    }
    case "answering":
      if (s.thinking && !m.text) return thinkingView(base, name, null);
      return { ...base, phase: "answering", line: "margin", tone: "accent", inFlight: true, stop: true, announce: `${provider || "The AI"} is answering` };
    case "shown":
      // A new session: nothing yet, the field ready. Never "Not sent", never hidden.
      if (isEmptySession(s)) return { ...base, phase: "shown", line: "none", tone: "quiet", title: s.session.title, empty: true, announce: "New session" };
      // A session opened from the list whose newest turn has no answer: it says why, and the panel stays.
      if (!m.text && kept)
        return { ...base, phase: "shown", line: "none", tone: "quiet", complete: true, note: kept.error ? "Not answered" : "Stopped", announce: "Session opened" };
      // A follow-up stopped before any answer: the conversation stays, the turn says so.
      if (!m.text && (earlierTurns(s).length || openNumber(s) > 1))
        return { ...base, phase: "shown", line: "none", tone: "quiet", complete: true, note: m.sent ? "Stopped" : "Not sent", announce: "Stopped" };
      // Stopped before any answer came: nothing is ready, and nothing is said to have been sent unless it was.
      if (!m.text) return endedView(base, m.sent);
      return { ...base, phase: "shown", line: "rest", tone: "quiet", complete: true, announce: "Answer ready" };
    case "discarded":
      return endedView(base, false);
    case "error": {
      const err = errorView(s.error, provider, !!s.words.trim());
      return { ...base, phase: "error", line: "rest", tone: "warn", error: err, announce: err.text };
    }
  }
}

function endedView(base: LiveView, sent: boolean): LiveView {
  const note = sent ? "Stopped" : "Not sent";
  return { ...base, phase: "discarded", compact: true, title: "", question: "", thumb: null, label: "", sent, note, hide: true, announce: note, noPicture: null };
}

function thinkingView(base: LiveView, name: string, note: string | null): LiveView {
  return { ...base, phase: "thinking", line: "loops", tone: "accent", inFlight: true, stop: true, note, announce: note ?? `Asking ${name}` };
}

/** Whether the person is reading the newest line: the view follows the text only then, and leaves it alone once they
 * scroll up. */
export function isPinned(scrollTop: number, scrollHeight: number, clientHeight: number, slack = 24): boolean {
  return scrollHeight - scrollTop - clientHeight <= slack;
}

// —— The answer as it streams: what arrived is the truth; what shows follows it smoothly ——

/** The text shown catches up with what arrived at this share of the backlog per ms: everything waiting is shown in
 * about this long, so the reveal runs a little behind the AI and its uneven chunks become an even flow. */
export const REVEAL_CATCH_MS = 250;
/** The slowest the text is revealed, in characters a second, so a short tail never crawls. */
export const REVEAL_MIN_CPS = 45;
/** Text that has waited this long is shown at once (a hidden window, a stalled frame). */
export const REVEAL_MAX_LAG_MS = 1500;

/** How much of the answer shows (a count of characters, fractional between frames) after `dt` ms. `total`: how much
 * has arrived. `lag`: how long the oldest character not yet shown has waited. `done`: the answer is complete (or the
 * person asked for less motion): everything shows. Never more than arrived, never less than was shown, and it lands
 * exactly on `total`. */
export function revealStep(p: { shown: number; total: number; dt: number; lag?: number; done?: boolean }): number {
  const total = Math.max(0, p.total);
  const shown = Number.isFinite(p.shown) ? Math.max(0, p.shown) : 0;
  if (p.done || shown >= total || (p.lag ?? 0) >= REVEAL_MAX_LAG_MS) return total;
  if (!(p.dt > 0)) return shown;
  const backlog = total - shown;
  const add = Math.max(REVEAL_MIN_CPS / 1000, backlog / REVEAL_CATCH_MS) * p.dt;
  return add >= backlog ? total : shown + add;
}

/** The first `n` characters of `s`, never ending inside a surrogate pair. */
export function revealSlice(s: string, n: number): string {
  let k = Math.max(0, Math.min(s.length, Math.floor(n)));
  const c = s.charCodeAt(k - 1);
  if (k > 0 && k < s.length && c >= 0xd800 && c <= 0xdbff) k++;
  return s.slice(0, k);
}

const NL = String.fromCharCode(10);
const LIST_OR_INDENT = /^(\s|[-*+]\s|\d+[.)]\s|>)/;
const FENCE = /^ {0,3}(```|~~~)/;

/** Where the streamed answer splits into blocks that are settled (rendered once) and the tail still being written:
 * just after the last blank line outside a code fence or a math block that starts a new block of its own (not a list
 * item, an indented line or a quote, which could belong to the block above). The settled part parses the same alone
 * as it does in the whole. 0 while there is no such line. */
export function settledSplit(src: string): number {
  const lines = src.split(NL);
  let fence = "";
  let math = false;
  let at = 0;
  let split = 0;
  for (let i = 0; i < lines.length; i++) {
    const l = lines[i];
    const t = l.trim();
    const f = FENCE.exec(l);
    if (fence) {
      if (f && f[1] === fence && t.replace(/[`~]/g, "") === "") fence = "";
    } else if (math) {
      if (t.endsWith("$$")) math = false;
    } else if (f) fence = f[1];
    else if (t.startsWith("$$") && !(t.length > 4 && t.endsWith("$$"))) math = true;
    else if (t === "" && i > 0 && i + 1 < lines.length) {
      const next = lines[i + 1];
      if (next.trim() && !LIST_OR_INDENT.test(next)) split = at + l.length + 1;
    }
    at += l.length + 1;
  }
  return split;
}

/** The panel's width in logical pixels: as platform.rs PANEL_WIDTH docks it (owner, 2026-11-12: "it should open up
 * bigger"; 2026-11-13: bigger still). */
export const PANEL_W = 640;
/** Once an answer shows, the panel is at least this tall, so a short answer opens as a reading surface, not a strip. */
export const ANSWER_MIN_H = 400;
/** The most of the screen's height the panel takes. */
export const PANEL_MAX_SHARE = 0.6;

/** The height the panel asks for: its content's, at least ANSWER_MIN_H while an answer shows, and never more than
 * PANEL_MAX_SHARE of the screen. Once the person dragged it, an answer shows at their `chosen` height instead. */
export function panelHeight(content: number, screenH: number, answer: boolean, chosen?: number | null): number {
  if (answer && chosen != null && Number.isFinite(chosen) && chosen > 0) return chosenHeight(chosen, screenH);
  const most = Math.round(Math.max(0, screenH) * PANEL_MAX_SHARE);
  const want = Math.max(0, Number.isFinite(content) ? content : 0);
  return Math.min(most, answer ? Math.max(want, ANSWER_MIN_H) : want);
}

/** The narrowest and widest the person can drag the panel, and the shortest an answer can be made. */
export const PANEL_MIN_W = 380;
export const PANEL_MAX_W = 900;
export const PANEL_MIN_H = 200;
/** The space kept between the panel and the screen's edges (platform.rs PANEL_MARGIN), on both sides. */
const EDGES = 24;

/** The panel's width: the one the person chose, readable and on screen; PANEL_W until they choose. */
export function panelWidth(chosen: number | null | undefined, screenW: number): number {
  const w = chosen != null && Number.isFinite(chosen) ? Math.round(chosen) : PANEL_W;
  return Math.max(0, Math.min(Math.max(PANEL_MIN_W, w), PANEL_MAX_W, Math.round(screenW) - EDGES));
}

/** An answer's height once the person chose one: that height, readable and on screen. Past PANEL_MAX_SHARE is fine. */
function chosenHeight(chosen: number, screenH: number): number {
  return Math.max(0, Math.min(Math.max(PANEL_MIN_H, Math.round(chosen)), Math.round(screenH) - EDGES));
}

/** The size while the corner grip (bottom left) is dragged by (dx, dy): the panel stays docked at the top right, so
 * dragging left widens it and dragging down makes it taller. */
export function dragSize(start: { w: number; h: number }, dx: number, dy: number, screen: { w: number; h: number }) {
  return { w: panelWidth(start.w - dx, screen.w), h: chosenHeight(start.h + dy, screen.h) };
}

/** The size the person last chose, as saved; null when nothing real was saved. */
export function parsePanelSize(raw: string | null): { w: number; h: number | null } | null {
  try {
    const v = raw ? JSON.parse(raw) : null;
    if (!v || typeof v !== "object" || Array.isArray(v) || typeof v.w !== "number" || !(v.w > 0)) return null;
    return { w: v.w, h: typeof v.h === "number" && v.h > 0 ? v.h : null };
  } catch {
    return null;
  }
}

/** How long "Not sent" stays before the panel hides. */
export const ENDED_MS = 1100;

/** The foot row's receipt: what left the computer. Nothing for a question that never left. */
export function sentLine(v: Pick<LiveView, "thumb" | "provider" | "sent">): string {
  if (!v.sent) return "";
  const to = v.provider ? ` to ${v.provider}` : "";
  // Never "only": a follow-up into a reopened conversation also carries the earlier questions, which Sent lists.
  return v.thumb ? `Sent your words and the picture${to}` : `Sent your words${to}`;
}

// —— Browser preview fixtures (live.html?preview=1&state=…): data only; api.ts plays them. ——

export const PREVIEW_WORDS = "Why do we divide by P(B) here, and not just multiply the two?";

export const PREVIEW_ANSWER = [
  "The top, **P(B|A) P(A)**, counts only the cases where A and B happen together. Dividing by `P(B)` turns that count into a *share* of all the ways B can happen:",
  "",
  "$$P(B) = P(B|A) P(A) + P(B|not A) P(not A)$$",
  "",
  "With the numbers on the next slide:",
  "",
  "```python",
  "p, tp, fp = 0.01, 0.95, 0.05",
  "p_b = tp * p + fp * (1 - p)",
  "print(tp * p / p_b)  # 0.161",
  "```",
  "",
  "So one positive test means about a **16%** chance:",
  "",
  "1. Most positives come from the 99% without it.",
  "2. Multiplying alone gives 0.0095, which is not a probability of anything you asked.",
  "",
  "More in [Bayes' theorem](https://en.wikipedia.org/wiki/Bayes%27_theorem).",
].join(String.fromCharCode(10));

/** A slide-like picture drawn inline, so the preview requests nothing; `code` is another window to switch to. */
export function previewPicture(kind: "slide" | "code" = "slide"): LivePicture {
  if (kind === "code") {
    const bars = [28, 44, 60, 76, 92, 108, 124, 140]
      .map((y, i) => `<rect x="${80 + (i % 3) * 14}" y="${y}" width="${60 + ((i * 37) % 110)}" height="6" rx="3" fill="${["#c77dbb", "#6aab73", "#56a8f5"][i % 3]}"/>`)
      .join("");
    const svg =
      '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">' +
      '<rect width="320" height="180" fill="#1e1f22"/><rect width="64" height="180" fill="#2b2d30"/>' +
      bars +
      "</svg>";
    return { src: "data:image/svg+xml," + encodeURIComponent(svg), w: 320, h: 180, label: "Visual Studio Code" };
  }
  const svg =
    '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">' +
    '<rect width="320" height="180" fill="#fbfbfa"/><rect width="320" height="30" fill="#24314d"/>' +
    '<text x="14" y="20" font-family="Segoe UI, sans-serif" font-size="12" font-weight="600" fill="#fff">Lecture 4 · Bayes&#39; rule</text>' +
    '<text x="22" y="86" font-family="Georgia, serif" font-style="italic" font-size="19" fill="#1d1d1f">P(A|B) =</text>' +
    '<text x="118" y="74" font-family="Georgia, serif" font-style="italic" font-size="16" fill="#1d1d1f">P(B|A) P(A)</text>' +
    '<path d="M116 82H222" stroke="#1d1d1f" stroke-width="1.4"/>' +
    '<text x="148" y="102" font-family="Georgia, serif" font-style="italic" font-size="16" fill="#1d1d1f">P(B)</text>' +
    '<rect x="22" y="128" width="190" height="7" rx="3.5" fill="#c9ccd6"/><rect x="22" y="146" width="140" height="7" rx="3.5" fill="#c9ccd6"/></svg>';
  return { src: "data:image/svg+xml," + encodeURIComponent(svg), w: 320, h: 180, label: "PowerPoint" };
}

/** A session of three questions about one slide, the newest still open: two short exchanges, then the long answer. */
export function previewTurns(now = Date.now()): LiveTurn[] {
  return [
    {
      n: 1,
      question: "What does the line over the B mean?",
      answer: "It means **not B**: every case where B does not happen. Some courses write it as ¬B or Bᶜ.",
      picture: previewPicture(),
      stopped: false,
      error: null,
      at_ms: now - 6 * 60_000,
    },
    {
      n: 2,
      question: "Is P(B|A) the same thing as P(A|B)?",
      answer:
        "No. **P(B|A)** is how often B happens among the cases where A does; **P(A|B)** is the reverse. Bayes' rule is how you turn one into the other.",
      picture: null,
      stopped: false,
      error: null,
      at_ms: now - 4 * 60_000,
    },
    { n: 3, question: PREVIEW_WORDS, answer: PREVIEW_ANSWER, picture: previewPicture(), stopped: false, error: null, at_ms: now - 60_000 },
  ];
}

/** The preview's snapshot for a state named in the address, shaped as the engine sends it. `conversation` is a session
 * of three turns, the third in the panel's own fields (asked as this panel opening's first question, so `turn` is 1);
 * `opened` is that session opened from the list (every turn kept, the fields empty, no AI named yet); `empty` is a new
 * session; `follow` is a spoken follow-up being heard in that session. `marked`: the picture was marked up. `next`: a picture marked with no question under way waits for the
 * next one. */
export function previewSnapshot(kind: string, o: { error?: string | null; provider?: string; marked?: boolean; next?: boolean; blind?: boolean } = {}): LiveSnapshot {
  const provider = o.provider ?? "ChatGPT";
  const session = { id: "preview", title: "Why do we divide by P(B)", turns: 1 };
  const picture = o.blind ? null : o.marked ? { ...previewPicture(), marked: true } : previewPicture();
  const next_picture = o.next ? { ...previewPicture("code"), marked: true } : null;
  const no_picture = o.blind ? ("not_allowed" as const) : null;
  const common = { ...CLOSED, session, turn: 1, picture, no_picture, provider, words: PREVIEW_WORDS, next_picture };
  const three = { id: "preview", title: "Bayes' rule on the slide", turns: 3 };
  switch (kind) {
    case "conversation":
      return { ...common, state: "shown", answer: PREVIEW_ANSWER, session: three, turns: previewTurns().slice(0, 2) };
    case "opened":
      return { ...CLOSED, state: "shown", session: three, turns: previewTurns(), next_picture };
    case "empty":
      return { ...CLOSED, state: "shown", next_picture };
    case "listening":
      return { ...common, state: "listening", session: { ...session, title: "" }, words: "" };
    case "follow":
      // Ctrl+Shift+/ again after an answer: listening for a follow-up, the turns so far kept.
      return { ...common, state: "listening", session: three, turns: previewTurns(), words: "" };
    case "finishing":
      return { ...common, state: "finishing" };
    case "answering":
      return { ...common, state: "answering" };
    case "shown":
      return { ...common, state: "shown", answer: PREVIEW_ANSWER };
    case "discarded":
      return { ...common, state: "discarded", words: "" };
    case "error":
      return o.error === "provider"
        ? { ...common, state: "error", error: { code: "provider", message: "ChatGPT could not be reached" } }
        : o.error === "allow"
          ? { ...common, state: "error", error: { code: "not_allowed", message: "Allow ChatGPT in Settings first" } }
          : { ...common, state: "error", provider: "", error: "Choose an AI in Settings first" };
    default:
      return { ...common, state: "sending" };
  }
}

/** The answer cut into the uneven pieces a provider streams, always the same. */
export function previewChunks(text: string): string[] {
  const out: string[] = [];
  let i = 0;
  let k = 0;
  while (i < text.length) {
    const n = 2 + ((k * 7 + 3) % 9);
    out.push(text.slice(i, i + n));
    i += n;
    k++;
  }
  return out;
}

/** A speech-like microphone level for sample i (one every 90 ms): syllables, words and short pauses. */
export function previewLevel(i: number): number {
  const word = Math.floor(i / 7);
  const pause = word % 5 === 4 && i % 7 > 3;
  if (pause) return 0.02;
  const syl = Math.abs(Math.sin(i * 1.3 + word));
  return Math.min(1, 0.18 + 0.7 * syl * (0.6 + 0.4 * Math.abs(Math.sin(word * 2.1))));
}
