// The Ask panel as a conversation: the earlier turns as one-line rows, the rules of the typed follow-up, and the Sent
// view (exactly what left the computer for one turn). Pure, so node --test reads it without a window
// (src/lib/conversation.test.mjs). The engine's names live in the adapters in api.ts.

import { earlierTurns, safePicture, type LivePicture, type LiveSnapshot, type LiveTurn, type LiveView } from "./live.ts";

/** Whitespace folded to single spaces. */
const fold = (t: string) => t.replace(/\s+/g, " ").trim();

/** One earlier turn, collapsed to its question; its answer opens in place. */
export type TurnRow = {
  n: number;
  /** The one line the row shows: the question, or what stands in for it. CSS cuts it to the row. */
  line: string;
  /** The question as asked, whitespace folded ("" when only a picture went). */
  question: string;
  answer: string;
  picture: LivePicture | null;
  /** "Stopped", "Not answered" or "" (answered). */
  note: string;
  /** Why it was not answered, when the engine said. */
  error: string;
  at_ms: number;
};

export function turnRow(t: LiveTurn): TurnRow {
  const question = fold(t.question);
  const picture = safePicture(t.picture);
  const line = question || (picture ? "A picture, without words" : "No words");
  const note = t.error ? "Not answered" : t.stopped ? "Stopped" : t.answer.trim() ? "" : "No answer";
  return { n: t.n, line, question, answer: t.answer, picture, note, error: t.error ?? "", at_ms: t.at_ms };
}

/** The rows above the open turn, oldest first. */
export function earlierRows(s: LiveSnapshot): TurnRow[] {
  return earlierTurns(s).map(turnRow);
}

/** What a screen reader hears for a collapsed row. */
export function rowLabel(r: TurnRow, open: boolean): string {
  return `Question ${r.n}: ${r.line}${r.note ? `, ${r.note.toLowerCase()}` : ""}. ${open ? "Hide the answer" : "Show the answer"}`;
}

// —— The typed follow-up ——

/** The longest follow-up the field takes. */
export const FOLLOW_MAX = 4000;

/** The field shows once an answer is shown in full, and in a new session waiting for its first question: never while
 * a question is heard, asked or answered. */
export function followUpShown(v: Pick<LiveView, "phase" | "complete" | "empty">): boolean {
  return v.phase === "shown" && (v.complete || v.empty);
}

/** What the field asks for: the first question of a new session, or a follow-up. */
export const fieldPlaceholder = (empty: boolean) => (empty ? "Type a question" : "Ask a follow-up");

export function canSend(text: string, busy: boolean): boolean {
  const t = text.trim();
  return !busy && t.length > 0 && t.length <= FOLLOW_MAX;
}

/** The words that go: the field's text without the blank edges, lines inside kept. */
export const sendText = (text: string) => text.trim();

export type KeyLike = { key: string; shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; isComposing?: boolean };
/** send: Enter with words; hold: Enter with nothing to send (no new line either); newline: Shift+Enter; leave: Esc
 * (the field lets go, the panel stays); pass: everything else, typed as usual. */
export type FieldAct = "send" | "hold" | "newline" | "leave" | "pass";

export function followKey(e: KeyLike, text: string, busy: boolean): FieldAct {
  // A key that finishes an input method's composition is the composition's, not the field's.
  if (e.isComposing) return "pass";
  if (e.key === "Escape") return "leave";
  if (e.key !== "Enter") return "pass";
  if (e.shiftKey) return "newline";
  if (e.ctrlKey || e.metaKey || e.altKey) return "hold";
  return canSend(text, busy) ? "send" : "hold";
}

/** Ctrl+K (Cmd+K on a Mac) puts the cursor in the field. */
export function isFocusKey(e: KeyLike, mac: boolean): boolean {
  if (e.key.toLowerCase() !== "k" || e.shiftKey || e.altKey) return false;
  return mac ? !!e.metaKey && !e.ctrlKey : !!e.ctrlKey && !e.metaKey;
}

// —— Sent: exactly what left the computer for one turn (`live_sent`) ——

/** `turn`: the turn it is of, by its place in the session. */
export type SentPicture = { n: number; turn: number; w: number; h: number; bytes: number; mime: string; src: string };
/** `replay`: the transcript of the earlier turns as it was sent when the conversation had to be reopened ("" when the
 * AI still held them); `again`: their pictures sent again with it. */
export type SentPayload = {
  text: string;
  pictures: SentPicture[];
  earlier: number[];
  provider: string;
  model: string;
  adds: string[];
  replay: string;
  again: SentPicture[];
};

const num = (v: unknown) => (typeof v === "number" && Number.isFinite(v) && v >= 0 ? v : 0);
const str = (v: unknown) => (typeof v === "string" ? v : "");

function sentPictures(raw: unknown): SentPicture[] {
  return (Array.isArray(raw) ? raw : [])
    .filter((p): p is Record<string, unknown> => !!p && typeof p === "object")
    .map((p, i) => ({ n: num(p.n) || i + 1, turn: num(p.turn), w: num(p.w), h: num(p.h), bytes: num(p.bytes), mime: str(p.mime), src: str(p.src) }));
}

/** The engine's reply with every field in its type; null when nothing usable came. */
export function normalizeSent(raw: unknown): SentPayload | null {
  if (!raw || typeof raw !== "object") return null;
  const u = raw as Record<string, unknown>;
  const earlier = [...new Set((Array.isArray(u.earlier) ? u.earlier : []).filter((n): n is number => typeof n === "number" && Number.isFinite(n)))].sort(
    (a, b) => a - b,
  );
  const provider = u.provider && typeof u.provider === "object" ? str((u.provider as { name?: unknown }).name) : str(u.provider);
  const adds = (Array.isArray(u.adds) ? u.adds : []).map((a) => fold(str(a))).filter(Boolean);
  const again = sentPictures(u.again).sort((a, b) => a.turn - b.turn || a.n - b.n);
  return { text: str(u.text), pictures: sentPictures(u.pictures), earlier, provider: provider.trim(), model: str(u.model).trim(), adds, replay: str(u.replay).trim(), again };
}

/** "214 KB", "1.2 MB", "870 bytes": decimal units, as the system's file sizes read. */
export function sizeLabel(bytes: number): string {
  const b = Math.max(0, Math.round(bytes));
  if (b < 1000) return `${b} ${b === 1 ? "byte" : "bytes"}`;
  if (b < 1_000_000) return `${Math.round(b / 1000)} KB`;
  const mb = b / 1_000_000;
  return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`;
}

/** "PNG", "JPEG", "WebP" from a MIME type; "" when there is none. */
export function kindLabel(mime: string): string {
  const sub = mime.toLowerCase().split("/")[1]?.split(/[;+]/)[0] ?? "";
  if (!sub) return "";
  if (sub === "jpeg" || sub === "jpg") return "JPEG";
  if (sub === "webp") return "WebP";
  return sub.toUpperCase();
}

/** Which earlier questions went along: "Questions 1 and 2", "Questions 1 to 5", "Question 3"; "" for none. */
export function earlierLabel(ns: number[]): string {
  if (!ns.length) return "";
  if (ns.length === 1) return `Question ${ns[0]}`;
  const run = ns.every((n, i) => i === 0 || n === ns[i - 1] + 1);
  if (run && ns.length > 2) return `Questions ${ns[0]} to ${ns[ns.length - 1]}`;
  return `Questions ${ns.slice(0, -1).join(", ")} and ${ns[ns.length - 1]}`;
}

export type SentRow = { label: string; value: string };
export type SentPictureView = { n: number; src: string | null; label: string; size: string };
/** The earlier turns sent again because the conversation had to be reopened: their transcript as it went (shown on
 * request), and their pictures, small. */
export type ReplayView = {
  text: string;
  /** One quiet line for the closed row: "Their words and 2 pictures". */
  meta: string;
  pictures: SentPictureView[];
};
export type SentView = {
  /** The words exactly as they went ("" when none did: `noWords` says so). */
  words: string;
  noWords: boolean;
  pictures: SentPictureView[];
  /** The other rows, in reading order: the earlier questions that went along, and the AI with its model. */
  rows: SentRow[];
  /** Null when the AI still held the conversation and nothing was sent again. */
  replay: ReplayView | null;
  /** What the AI's own tool added on the way, under its name ("Codex added"); MCP servers it started are among them. */
  addsBy: string;
  adds: string[];
};

/** The preview's `live_sent` for a turn: its words, its picture as sent, every earlier question, and, for Codex, what
 * Codex adds on the way. `replay`: the conversation was reopened, so the earlier turns went again as a transcript with
 * their pictures. */
export function previewSent(t: Pick<LiveTurn, "n" | "question" | "picture"> | null, provider: string, o: { replay?: LiveTurn[] } = {}): SentPayload {
  const n = t?.n ?? 1;
  const codex = /codex/i.test(provider);
  const before = (o.replay ?? []).filter((x) => x.n < n);
  const nl = String.fromCharCode(10);
  return {
    text: t?.question ?? "",
    pictures: t?.picture ? [{ n: 1, turn: n, w: 1280, h: 720, bytes: 214_300, mime: "image/jpeg", src: t.picture.src }] : [],
    earlier: Array.from({ length: Math.max(0, n - 1) }, (_, i) => i + 1),
    provider: provider || "ChatGPT",
    model: codex ? "gpt-5.1-codex" : "gpt-5.1",
    adds: codex ? ["Codex's own instructions for answering", "The AGENTS.md file in the folder it runs in", "MCP server started by Codex: github"] : [],
    replay: before.map((x) => `Question ${x.n}: ${x.question}${nl}Answer ${x.n}: ${x.answer}`).join(nl + nl),
    again: before.flatMap((x) => (x.picture ? [{ n: 1, turn: x.n, w: 1280, h: 720, bytes: 188_400, mime: "image/jpeg", src: x.picture.src }] : [])),
  };
}

function pictureView(x: SentPicture, label: string): SentPictureView {
  const dims = x.w && x.h ? `${x.w} × ${x.h}` : "";
  const size = [dims, x.bytes ? sizeLabel(x.bytes) : "", kindLabel(x.mime)].filter(Boolean).join(" · ");
  const src = safePicture({ src: x.src, w: x.w, h: x.h })?.src ?? null;
  return { n: x.n, src, label, size };
}

/** "Their words", "Their words and 1 picture", "Their words and 2 pictures". */
export function replayMeta(pictures: number): string {
  return pictures ? `Their words and ${pictures} ${pictures === 1 ? "picture" : "pictures"}` : "Their words";
}

export function sentView(raw: unknown): SentView | null {
  const p = normalizeSent(raw);
  if (!p) return null;
  const words = p.text.trim();
  const pictures = p.pictures.map((x) => pictureView(x, p.pictures.length > 1 ? `Picture ${x.n}` : "Picture"));
  const rows: SentRow[] = [];
  const earlier = earlierLabel(p.earlier);
  rows.push({ label: "Earlier", value: earlier ? `${earlier}, with ${p.earlier.length === 1 ? "its answer" : "their answers"}` : "None" });
  const to = [p.provider, p.model].filter(Boolean).join(" · ");
  if (to) rows.push({ label: "To", value: to });
  // A picture of a turn that had several says which one it was.
  const many = (turn: number) => p.again.filter((x) => x.turn === turn).length > 1;
  const again = p.again.map((x) => pictureView(x, x.turn ? `Question ${x.turn}${many(x.turn) ? `, picture ${x.n}` : ""}` : `Picture ${x.n}`));
  const replay = p.replay || again.length ? { text: p.replay, meta: replayMeta(again.length), pictures: again } : null;
  return { words, noWords: !words, pictures, rows, replay, addsBy: p.adds.length ? `${p.provider || "The AI"} added` : "", adds: p.adds };
}
