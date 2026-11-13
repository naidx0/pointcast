// The AI's plan for a task of several steps, as the Ask panel shows it: a short checklist with Start and Cancel while
// it waits, then each step ticked as the AI says how it ended. Pure, so node --test reads it (src/lib/plan.test.mjs).

export type StepStatus = "pending" | "done" | "skipped" | "failed";
export type PlanStep = { text: string; status: StepStatus; note: string | null };
export type PlanState = "proposed" | "started" | "cancelled";
/** What the engine sends in `computer://status` (`computer::plan::Plan`). */
export type ComputerPlan = { id: number; steps: PlanStep[]; apps: string[]; state: PlanState };

/** How a row reads: still to do, under way now, or how it ended. */
export type Mark = "todo" | "now" | "done" | "skipped" | "failed";
export type Row = { mark: Mark; text: string; note: string };

/** The glyph each mark draws in front of its step. */
export const GLYPH: Record<Mark, string> = { todo: "○", now: "◐", done: "✓", skipped: "–", failed: "✕" };
/** What a screen reader says for each mark. */
export const SAY: Record<Mark, string> = { todo: "to do", now: "under way", done: "done", skipped: "skipped", failed: "failed" };

/** The checklist's rows. While a started plan's turn runs, the first step not ticked yet is the one under way, unless
 * a step failed: then the AI has stopped to ask, and nothing reads as under way. */
export function rows(plan: ComputerPlan, live: boolean): Row[] {
  const going = live && plan.state === "started" && !plan.steps.some((s) => s.status === "failed");
  const now = going ? plan.steps.findIndex((s) => s.status === "pending") : -1;
  return plan.steps.map((s, i) => ({
    mark: s.status === "pending" ? (i === now ? "now" : "todo") : s.status,
    text: s.text,
    note: s.note ?? "",
  }));
}

/** The line over the checklist: the apps it acts in, with how far it got once started. */
export function head(plan: ComputerPlan, live: boolean): string {
  const apps = plan.apps.join(", ");
  if (plan.state === "cancelled") return "Plan cancelled";
  if (plan.state === "proposed") return live ? `Plan · ${apps}` : "Plan not started";
  const ticked = plan.steps.filter((s) => s.status !== "pending").length;
  return `${ticked} of ${plan.steps.length} · ${apps}`;
}

/** Whether Start and Cancel show: only while the plan waits and its turn still runs. */
export function asks(plan: ComputerPlan, live: boolean): boolean {
  return live && plan.state === "proposed";
}

/** The browser preview's plan (`plan=proposed|started|failed` on live.html?preview=1): a three-step TextEdit task. */
export function previewPlan(k: string | null): ComputerPlan | null {
  const steps = ["Open TextEdit and make a new document", "Type a three-item shopping list", "Select all the text"];
  const marks: Record<string, [PlanState, StepStatus[]]> = {
    proposed: ["proposed", ["pending", "pending", "pending"]],
    started: ["started", ["done", "pending", "pending"]],
    failed: ["started", ["done", "failed", "pending"]],
  };
  const m = k ? marks[k] : undefined;
  if (!m) return null;
  const notes = [null, m[1][1] === "failed" ? "the text area did not take the text" : null, null];
  return { id: 1, apps: ["TextEdit"], state: m[0], steps: steps.map((text, i) => ({ text, status: m[1][i], note: notes[i] })) };
}

/** The plan the panel shows, and the turn it belongs to. */
export type Kept = { turn: number; plan: ComputerPlan; live: boolean };

/** The live plan while the turn holds one; once the turn lets it go, the last one seen stays (as it ended) until the
 * next turn starts. */
export function keep(prev: Kept | null, live: ComputerPlan | null, turn: number): Kept | null {
  if (live) return { turn, plan: live, live: true };
  if (prev && prev.turn === turn) return prev.live ? { ...prev, live: false } : prev;
  return null;
}
