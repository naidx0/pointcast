// What the capsule says and does at the end of a recording, while it is written down, and when Discard is pressed.
// Pure, so node --test reads it without a window (src/lib/capsule.test.mjs).
import type { UiState } from "./api";
import type { LineMode } from "./line";

export type Notice = NonNullable<UiState["notice"]>;

/** What a click on the end notice does: open History on the saved entry, transcribe the rest of it there, or ask macOS
 * for the Accessibility permission auto-paste needs. null: nothing was saved, so nothing to open. */
export type NoticeAction = "history" | "transcribe" | "permission" | null;

export type NoticeView = {
  /** The first line, and the second when the notice has one: what a click does. */
  lines: [string, string?];
  mark: "ok" | "part" | "warn" | "none";
  action: NoticeAction;
};

export function noticeView(
  notice: Notice,
  o: { mac: boolean; pasteBlocked: boolean; entry: string | null; error: string | null },
): NoticeView {
  const keys = o.mac ? "⌘V" : "Ctrl+V";
  switch (notice) {
    case "pasted":
      return { lines: ["Pasted"], mark: "ok", action: "history" };
    case "copied":
      // Auto-paste was the choice and did not happen: say so, and on macOS (Accessibility) offer to allow it.
      if (!o.pasteBlocked) return { lines: [`Copied · ${keys} to paste`], mark: "ok", action: "history" };
      return o.mac
        ? { lines: [`Not pasted · on the clipboard, press ${keys}`, "Allow auto-paste"], mark: "warn", action: "permission" }
        : { lines: [`Not pasted · on the clipboard, press ${keys}`], mark: "warn", action: "history" };
    case "saved":
      return { lines: ["Saved to your folder"], mark: "ok", action: "history" };
    case "kept":
      return { lines: ["No words yet · audio kept", "Open History"], mark: "warn", action: "history" };
    case "partial":
      return { lines: ["Pasted what's done · rest kept in History", "Transcribe the rest"], mark: "part", action: "transcribe" };
    case "cut":
      return { lines: ["Saved what's done · the rest was not transcribed"], mark: "part", action: "history" };
    default:
      return { lines: [o.error || "Nothing was recorded"], mark: "none", action: null };
  }
}

/** The line while the last words are written down. No engine or device here: History's detail line names them. */
export function finishingLine(p: { percent: number; stalled: boolean } | null, cutting: boolean): string {
  if (cutting) return "Saving what's done…";
  if (!p) return "Finishing…";
  const pct = Math.min(100, Math.max(0, Math.round(p.percent)));
  return p.stalled ? `Still working… ${pct}%` : `Writing it down… ${pct}%`;
}

/** How long Discard waits for its second click. */
export const DISCARD_ARM_MS = 3000;

/** Whether Discard is waiting for its second click at `now`. */
export function discardLive(armedUntil: number | null, now: number): boolean {
  return armedUntil != null && now < armedUntil;
}

/** A click on Discard: the first one asks ("Discard?"), a second one while it asks discards. */
export function discardClick(armedUntil: number | null, now: number): { discard: boolean; armedUntil: number | null } {
  if (discardLive(armedUntil, now)) return { discard: true, armedUntil: null };
  return { discard: false, armedUntil: now + DISCARD_ARM_MS };
}

/** The microphone warning cut to one line under the timer and the level bars, which stay. */
export function micCaption(quiet: NonNullable<UiState["quiet_mic"]>): string {
  return quiet === "silent" ? "No sound from the mic" : "Mic is very quiet";
}

/** What the capsule's line does, the same line as Ask's: the voice draws it once the microphone is live, it rests flat
 * before that (nothing is recorded yet), and it loops while the last words are written down. */
export function capsuleLine(s: { phase: string; mic: boolean }): LineMode {
  if (s.phase === "transcribing") return "think";
  return (s.phase === "recording" || s.phase === "loading") && s.mic ? "listen" : "rest";
}

/** How long the microphone may take to start before the capsule says anything about it. */
export const READY_WAIT_MS = 600;

/** What the capsule says while a recording gets ready, or null for the plain recording look. Once the microphone is
 * live it looks like recording at once: the audio is kept and written down when the speech model has loaded, so there
 * is nothing for the person to wait for. Before that, only a microphone slow to start (a Bluetooth headset) is said. */
export function readyNote(s: { phase: string; mic: boolean; preparing_gpu?: boolean }, waitedMs: number): { lines: [string, string?]; full: string } | null {
  if (!(s.phase === "loading" || s.phase === "recording")) return null;
  if (s.mic || waitedMs < READY_WAIT_MS) return null;
  if (s.preparing_gpu) return { lines: ["Setting up graphics card", "(first time only)"], full: "Setting up your graphics card (first time only)" };
  return { lines: ["Getting ready…"], full: "Getting ready…" };
}

/** Whether the person is looking at the time: the pointer is over the capsule, or the keyboard is on one of its
 * buttons. Kept by the page and not left to :hover or :focus-within, which stuck: a click on a picture button kept its
 * focus, and a capsule hidden with the pointer over it came back "hovered" (no pointer-leave reaches a hidden window),
 * so the time stayed for the whole of the next recording (owner, 2026-11-12: "sometimes the timer stays"). */
export type Peek = { pointer: boolean; keyboard: boolean };
export const NO_PEEK: Peek = { pointer: false, keyboard: false };
/** enter / leave: the pointer. key-focus: a button reached by the keyboard (a click's focus is not a look). blur: the
 * focus left the capsule. hidden: the window went off screen. start: a new recording began. */
export type PeekEvent = "enter" | "leave" | "key-focus" | "blur" | "hidden" | "start";

export function peekStep(p: Peek, e: PeekEvent): Peek {
  switch (e) {
    case "enter":
      return { ...p, pointer: true };
    case "leave":
      return { ...p, pointer: false };
    case "key-focus":
      return { ...p, keyboard: true };
    case "blur":
      return { ...p, keyboard: false };
    default:
      return NO_PEEK;
  }
}

/** Whether the recording's time shows beside the line: only while the microphone is live, and then with the
 * microphone warning or while the person looks (Peek). Never once the recording has ended, and never on its own:
 * the owner asked for the clock on a look only (2026-11-13). */
export function timeShown(s: { live: boolean; durationMs: number; warned: boolean; peek: Peek }): boolean {
  return s.live && (s.warned || s.peek.pointer || s.peek.keyboard);
}
