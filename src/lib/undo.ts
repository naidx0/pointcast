// A History delete waits 5 seconds behind an Undo bar before it is real. Pure, so node --test reads it without a
// window (src/lib/undo.test.mjs). The window keeps one `Pending` and calls these; the list it draws is `visible`.

export const UNDO_MS = 5000;

/** The entry taken out of the list but not yet deleted: its id, where it sat among the shown entries, and when. */
export type Pending = { id: string; index: number; at: number };

/** The list as drawn: the pending entry is gone from it, but is still on disk until the delete commits. */
export function visible<T extends { id: string }>(entries: T[], p: Pending | null): T[] {
  return p ? entries.filter((e) => e.id !== p.id) : entries;
}

/** Delete `id` now. `commit` is the earlier pending entry, which this delete makes real; `next` is what to select. */
export function startDelete<T extends { id: string }>(
  p: Pending | null,
  entries: T[],
  id: string,
  now: number,
): { pending: Pending; commit: string | null; next: string | null } {
  const shown = visible(entries, p);
  const index = Math.max(0, shown.findIndex((e) => e.id === id));
  const rest = shown.filter((e) => e.id !== id);
  return {
    pending: { id, index, at: now },
    commit: p ? p.id : null,
    next: rest[Math.min(index, rest.length - 1)]?.id ?? null,
  };
}

/** Undo: nothing pending any more, and `restore` is the entry to put back and select. */
export function undo(p: Pending | null): { pending: null; restore: string | null } {
  return { pending: null, restore: p ? p.id : null };
}

/** The 5 seconds are over: the delete is to be made real. */
export function due(p: Pending | null, now: number): boolean {
  return !!p && now - p.at >= UNDO_MS;
}

/** The window closes: whatever is pending is made real, since nothing is left to undo it from. */
export function close(p: Pending | null): { pending: null; commit: string | null } {
  return { pending: null, commit: p ? p.id : null };
}

/** The words of the bar, or null when no bar shows. */
export function undoView(p: Pending | null): { text: string; action: string } | null {
  return p ? { text: "Deleted", action: "Undo" } : null;
}
