// What the mark-up overlay calls its actions, by what it was opened for. Opened by `live_mark` it serves a question
// to Ask (its frame says `ask: true`): its primary action uses the marked picture for the question, and Cancel keeps
// the picture taken by itself. Otherwise the wording is the capture's (or, for a picture from History, Save).
// Pure, so node --test reads it without a window (src/lib/markup.test.mjs).

export type MarkPurpose = "ask" | "capture";

/** What a frame was sent for: `ask: true` (or `purpose: "ask"`) is a question's picture; anything else, or nothing
 * (older engines), is a capture. */
export const markPurpose = (f: { ask?: unknown; purpose?: unknown } | null | undefined): MarkPurpose =>
  f?.ask === true || f?.purpose === "ask" ? "ask" : "capture";

/** A frame the overlay marks whole, from the start (no box to select first): a picture from History, or a question's
 * picture. Its new pins number on from `base`, the highest mark already on it. */
export function wholeFrame(f: { edit?: unknown; ask?: unknown; purpose?: unknown; base?: unknown }): { whole: boolean; base: number } {
  const whole = f.edit === true || markPurpose(f) === "ask";
  const base = typeof f.base === "number" && Number.isFinite(f.base) && f.base > 0 ? Math.floor(f.base) : 0;
  return { whole, base: whole ? base : 0 };
}

export type MarkWording = {
  /** The primary button's words. */
  go: string;
  /** Its tooltip, with its key. */
  goTitle: string;
  /** Cancel's tooltip, with its key. */
  cancelTitle: string;
};

export function markWording(purpose: MarkPurpose, edit: boolean): MarkWording {
  if (purpose === "ask") return { go: "Use for my question", goTitle: "Use for my question (Enter)", cancelTitle: "Cancel and keep the picture taken (Esc)" };
  if (edit) return { go: "Save", goTitle: "Save (Return)", cancelTitle: "Cancel (Esc)" };
  return { go: "Capture", goTitle: "Capture (Enter)", cancelTitle: "Cancel (Esc)" };
}

/** A picture's label in the panel, saying when it was marked up: "PowerPoint · Marked up". */
export function markedLabel(label: string, marked: boolean): string {
  const l = label.trim();
  if (!marked) return l;
  return l ? `${l} · Marked up` : "Marked up";
}

/** M, alone, marks up the question's picture while the panel has focus (never while typing: the caller checks). */
export const isMarkKey = (e: { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; repeat?: boolean }) =>
  e.key.toLowerCase() === "m" && !e.ctrlKey && !e.metaKey && !e.altKey && !e.repeat;

/** The hint line, in the words of the primary action: for Ask, "Enter capture" reads "Enter use it" and the History
 * edit's "Return saves" reads "Enter uses it". */
export function markHint(hint: string, purpose: MarkPurpose): string {
  if (purpose !== "ask") return hint;
  return hint.replace("Enter capture", "Enter use it").replace("Return saves", "Enter uses it").replace("Edit picture", "Mark up the picture");
}
