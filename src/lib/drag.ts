// Moving a window by its own surface. The `.drag` class marks the areas a person can hold to move the window (the
// sidebar, the title bar, onboarding's band). CSS `app-region` does that only in Chromium-based web views; the macOS
// web view ignores it, so Settings and onboarding could not be moved on a Mac (owner, 2026-11-12). One listener per
// window starts a native drag instead, on both systems.

/** One element on the way from the pressed element up to the page: its tag, classes and role. */
export type Hop = { tag: string; classes: readonly string[]; role?: string | null; editable?: boolean };

/** What a press does: nothing, move the window, or (a double press) zoom it, as a title bar does. */
export type DragAct = "none" | "move" | "zoom";

const CONTROL_TAGS = new Set(["button", "a", "input", "select", "textarea", "label", "summary", "option"]);
const CONTROL_ROLES = new Set(["button", "switch", "radio", "checkbox", "tab", "slider", "link", "menuitem", "option", "textbox"]);

/** The press on `chain` (the pressed element first): it moves the window only from inside a `.drag` area and never
 * from a control, a field or anything marked `.no-drag` between the press and that area. */
export function dragAct(chain: readonly Hop[], button: number, presses: number): DragAct {
  if (button !== 0) return "none";
  for (const h of chain) {
    if (h.classes.includes("no-drag") || h.editable || CONTROL_TAGS.has(h.tag.toLowerCase()) || (h.role && CONTROL_ROLES.has(h.role))) return "none";
    if (h.classes.includes("drag")) return presses === 2 ? "zoom" : "move";
  }
  return "none";
}

/** The chain from `el` up to the page, as `dragAct` reads it. */
export function chainOf(el: Element | null): Hop[] {
  const out: Hop[] = [];
  for (let e = el; e && e !== document.documentElement; e = e.parentElement) {
    out.push({ tag: e.tagName, classes: Array.from(e.classList), role: e.getAttribute("role"), editable: (e as HTMLElement).isContentEditable });
  }
  return out;
}
