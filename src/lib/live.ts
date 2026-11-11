// The Ask panel's state. It mirrors the engine: one snapshot event, one delta event and four commands, whose names live
// in one place only (the adapter at the end of api.ts). Pure: the reducer, what each state shows, the 8 s and 20 s
// wait lines on an injected clock, and the browser preview's fixtures, so node --test reads it without a window
// (src/lib/live.test.mjs).

export type LiveStateName = "closed" | "listening" | "finishing" | "sending" | "answering" | "shown" | "error" | "discarded";
/** `label`: the app or window the picture is of, when the engine names it ("" otherwise). */
export type LivePicture = { src: string; w: number; h: number; label?: string };
/** The engine's error: a message, or a code with an optional message. `no_provider` asks for Settings. */
export type LiveErrorPayload = string | { code?: string; message?: string } | null;

/** The payload of `live://state` and the reply of `live_state`. */
export type LiveSnapshot = {
  state: LiveStateName;
  session: { id: string; title: string };
  turn: number;
  /** The person's words: as they are heard while listening, then the question. */
  words: string;
  /** Exactly the picture that will be sent (a data: or blob: URL), or null. */
  picture: LivePicture | null;
  /** The person took the picture out: nothing but the words is sent. */
  dropped: boolean;
  provider: string | { id?: string; name?: string } | null;
  error: LiveErrorPayload;
  /** True while only reasoning has arrived for the current turn: the AI is working, nothing to show yet. */
  thinking?: boolean;
  /** The answer so far, if the engine sends it with the snapshot (a window opened mid-answer). Optional. */
  answer?: string;
};

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
  session: { id: "", title: "" },
  turn: 0,
  words: "",
  picture: null,
  dropped: false,
  provider: null,
  error: null,
};

export const initialLive = (): LiveModel => ({ snap: CLOSED, text: "", sent: false, waitFrom: null });

const STATES: LiveStateName[] = ["closed", "listening", "finishing", "sending", "answering", "shown", "error", "discarded"];

/** A snapshot with every field in its type, whatever arrived. */
export function normalizeSnapshot(s: Partial<LiveSnapshot> | null | undefined): LiveSnapshot {
  const v = s ?? {};
  return {
    state: STATES.includes(v.state as LiveStateName) ? (v.state as LiveStateName) : "closed",
    session: { id: String(v.session?.id ?? ""), title: String(v.session?.title ?? "") },
    turn: Number.isFinite(v.turn) ? Number(v.turn) : 0,
    words: typeof v.words === "string" ? v.words : "",
    picture:
      v.picture && typeof v.picture.src === "string"
        ? {
            src: v.picture.src,
            w: Number(v.picture.w) || 0,
            h: Number(v.picture.h) || 0,
            label: typeof v.picture.label === "string" ? v.picture.label.trim() : "",
          }
        : null,
    dropped: !!v.dropped,
    provider: v.provider ?? null,
    error: v.error ?? null,
    thinking: v.thinking === true,
    answer: typeof v.answer === "string" ? v.answer : undefined,
  };
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
};

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

export function liveView(m: LiveModel, now: number): LiveView {
  const s = m.snap;
  const provider = providerName(s.provider);
  const thumb = s.dropped ? null : safePicture(s.picture);
  const base: LiveView = {
    phase: "closed",
    line: "none",
    tone: "ink",
    mic: false,
    inFlight: false,
    provider,
    title: s.session.title || firstWords(s.words),
    compact: false,
    question: s.words.trim(),
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
  };
  const hearing = { compact: true, title: "", question: "" };
  const name = provider || "the AI";
  switch (s.state) {
    case "closed":
      return { ...base, thumb: null };
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
  return { ...base, phase: "discarded", compact: true, title: "", question: "", thumb: null, label: "", sent, note, hide: true, announce: note };
}

function thinkingView(base: LiveView, name: string, note: string | null): LiveView {
  return { ...base, phase: "thinking", line: "loops", tone: "accent", inFlight: true, stop: true, note, announce: note ?? `Asking ${name}` };
}

/** Whether the person is reading the newest line: the view follows the text only then, and leaves it alone once they
 * scroll up. */
export function isPinned(scrollTop: number, scrollHeight: number, clientHeight: number, slack = 24): boolean {
  return scrollHeight - scrollTop - clientHeight <= slack;
}

/** How long "Not sent" stays before the panel hides. */
export const ENDED_MS = 1100;

/** The foot row's receipt: what left the computer. Nothing for a question that never left. */
export function sentLine(v: Pick<LiveView, "thumb" | "provider" | "sent">): string {
  if (!v.sent) return "";
  const to = v.provider ? ` to ${v.provider}` : "";
  return v.thumb ? `Sent your words and the picture${to}` : `Sent your words only${to}`;
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

/** The preview's snapshot for a state named in the address. */
export function previewSnapshot(kind: string, o: { error?: string | null; provider?: string } = {}): LiveSnapshot {
  const provider = o.provider ?? "ChatGPT";
  const session = { id: "preview", title: "Why do we divide by P(B)" };
  const common = { ...CLOSED, session, turn: 1, picture: previewPicture(), provider, words: PREVIEW_WORDS };
  switch (kind) {
    case "listening":
      return { ...common, state: "listening", session: { ...session, title: "" }, words: "" };
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
