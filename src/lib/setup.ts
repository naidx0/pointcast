// First-run setup's step logic, kept free of React and Tauri so `node --test src/lib/setup.test.mjs` can run it.

/** `ask`: one calm step for Ask about your screen, its key and where its AI is chosen; nothing on it is required. */
export const STEPS = ["welcome", "access", "try", "ask", "done"] as const;
export type Step = (typeof STEPS)[number];

/** Where getting the speech model stands, as the footer line says it. */
export type SpeechLine = "downloading" | "ready" | "failed" | "none";

/** The footer's main button on each step. Only the practice run holds Continue back, and it offers Skip; the speech
 * download never does, and neither does choosing an AI. */
export function nextButton(step: Step, practiced: boolean): { label: string; enabled?: boolean; skip?: string } {
  switch (step) {
    case "welcome":
      return { label: "Get started" };
    case "access":
    case "ask":
      return { label: "Continue" };
    case "try":
      return { label: "Continue", enabled: practiced, skip: "Skip" };
    case "done":
      return { label: "Start using Pointcast" };
  }
}

/** The one model whose Download is the primary button: the recommended one, while no model can run yet (first setup).
 * Once one runs, every Download is plain: a list of blue buttons asks for nothing in particular. */
export function primaryDownload(models: { id: string; ready: boolean }[], recommended: string | null | undefined): string | null {
  if (!recommended || models.some((m) => m.ready)) return null;
  return models.some((m) => m.id === recommended) ? recommended : null;
}

export function speechLine(s: { ready: boolean; busy: boolean; error: string | null }): SpeechLine {
  if (s.busy) return "downloading";
  if (s.ready) return "ready";
  if (s.error && s.error !== "Cancelled.") return "failed";
  return "none";
}

/** 0 to 100 from a download's bytes, or null before its size is known. */
export function percent(received: number | undefined, total: number | undefined): number | null {
  if (!total) return null;
  return Math.max(0, Math.min(100, Math.floor(((received ?? 0) / total) * 100)));
}

/** Starts the recommended download the first time setup asks while nothing is installed or downloading, and never
 * again: going Back to the welcome step and on again does not start a second one. */
export function downloadOnce() {
  let asked = false;
  return (need: { installed: boolean; busy: boolean }, start: () => void) => {
    if (asked) return false;
    asked = true;
    if (need.installed || need.busy) return false;
    start();
    return true;
  };
}
