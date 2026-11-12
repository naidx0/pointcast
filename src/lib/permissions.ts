import type { Access, PermissionKind, Permissions } from "./api";

/** What a permission row offers: Allowed; Allowed but waiting for Pointcast to reopen (macOS applies a Screen Recording
 * grant only to a fresh process); Allow (the OS prompt); its switch in Settings; or both. */
export type RowAction = "allowed" | "reopen" | "allow" | "open" | "allow-open";

export function rowAction(o: { kind: PermissionKind; access: Access; asked: boolean; pending: boolean; mac: boolean }): RowAction {
  if (o.access === "granted") return "allowed";
  if (o.kind === "screen" && o.pending) return "reopen";
  if (o.access === "denied" || o.asked) return o.access === "denied" && !o.mac ? "allow-open" : "open";
  return "allow";
}

/** After Allow on the microphone the OS prompt is still up and the answer is "unknown": read again every `every` ms
 * until the person answers it (or `max` ms pass), handing each read to `seen`, so the row changes as the prompt closes. */
export async function untilAnswered(
  get: () => Promise<Permissions>,
  seen: (p: Permissions) => void,
  kind: PermissionKind,
  o: { every?: number; max?: number; sleep?: (ms: number) => Promise<void> } = {},
): Promise<Permissions | null> {
  const every = o.every ?? 250;
  const max = o.max ?? 60_000;
  const sleep = o.sleep ?? ((ms: number) => new Promise<void>((r) => setTimeout(r, ms)));
  for (let waited = 0; waited < max; waited += every) {
    await sleep(every);
    const p = await get().catch(() => null);
    if (!p) continue;
    seen(p);
    if (p[kind] !== "unknown") return p;
  }
  return null;
}
