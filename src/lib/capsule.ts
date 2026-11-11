// What the capsule says and does at the end of a recording, while it is written down, and when Discard is pressed.
// Pure, so node --test reads it without a window (src/lib/capsule.test.mjs).
import type { UiState } from "./api";

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
      return o.pasteBlocked
        ? { lines: [`Copied · ${keys} to paste`, "Turn on auto-paste"], mark: "ok", action: "permission" }
        : { lines: [`Copied · ${keys} to paste`], mark: "ok", action: "history" };
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
