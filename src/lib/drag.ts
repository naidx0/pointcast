// Moving a window by its own surface: one rule for every Pointcast window (owner, 2026-11-12: "make every element easy
// and consistent to move"). The `.drag` class marks the areas a person can hold to move the window (Settings' sidebar,
// title bars, onboarding, the Ask panel's header and listening row, the recording capsule); `.drag-self` marks a box
// that moves the window only from its own empty space, never from the text or pictures inside it (the Ask panel's
// body, whose answer stays selectable). CSS `app-region` would hand the press to Windows at once, with no room for a
// click, and the macOS web view ignores it: one listener per window (api.ts startWindowDrag) runs this rule instead.
//
// A press moves the window once the pointer has gone DRAG_SLOP px from where it went down; a press and release short of
// that is a click and keeps its meaning. A press that moved the window never also clicks. Controls never move it.

/** One element on the way from the pressed element up to the page: its tag, classes and role. */
export type Hop = { tag: string; classes: readonly string[]; role?: string | null; editable?: boolean };

/** What a press does: nothing, move the window (once the pointer travels), or (a double press) zoom it, as a title bar
 * does. */
export type DragAct = "none" | "move" | "zoom";

/** How far, in px, the pointer travels from its press before the window moves with it. */
export const DRAG_SLOP = 3;

const CONTROL_TAGS = new Set(["button", "a", "input", "select", "textarea", "label", "summary", "option"]);
const CONTROL_ROLES = new Set(["button", "switch", "radio", "checkbox", "tab", "slider", "link", "menuitem", "option", "textbox", "separator"]);

/** The press on `chain` (the pressed element first): it moves the window only from inside a `.drag` area (or on a
 * `.drag-self` box itself) and never from a control, a field or anything marked `.no-drag` between the press and that
 * area. A double press zooms only a window that can zoom (`canZoom`: resizable and maximizable, as Settings is); on any
 * other it is one more press. */
export function dragAct(chain: readonly Hop[], button: number, presses: number, canZoom = true): DragAct {
  if (button !== 0) return "none";
  for (let i = 0; i < chain.length; i++) {
    const h = chain[i];
    if (h.classes.includes("no-drag") || h.editable || CONTROL_TAGS.has(h.tag.toLowerCase()) || (h.role && CONTROL_ROLES.has(h.role))) return "none";
    const area = h.classes.includes("drag") || (i === 0 && h.classes.includes("drag-self"));
    if (area) return presses === 2 && canZoom ? "zoom" : "move";
  }
  return "none";
}

/** A press that may become a move: where it went down (screen px) and whether the window has started to follow. */
export type Hold = { x: number; y: number; moved: boolean } | null;

/** What a step of a press asks for: nothing, start the window's move, or zoom it. */
export type Run = "none" | "move" | "zoom";

/** Mouse-down: `act` is `dragAct` for this press. A move waits for the pointer to travel; a zoom happens now. */
export function holdDown(act: DragAct, x: number, y: number): { hold: Hold; run: Run } {
  if (act === "zoom") return { hold: null, run: "zoom" };
  if (act === "move") return { hold: { x, y, moved: false }, run: "none" };
  return { hold: null, run: "none" };
}

/** Mouse-move with `buttons` held: the window starts to follow once the pointer is DRAG_SLOP px or more from the press,
 * and only once per press. A move with the left button up ends a press whose release the page never saw. */
export function holdMove(hold: Hold, x: number, y: number, buttons: number): { hold: Hold; run: Run } {
  if (!hold || hold.moved) return { hold, run: "none" };
  if ((buttons & 1) === 0) return { hold: null, run: "none" };
  if (Math.hypot(x - hold.x, y - hold.y) >= DRAG_SLOP) return { hold: { ...hold, moved: true }, run: "move" };
  return { hold, run: "none" };
}

/** Mouse-up: a press that never moved the window is over (its click comes next and keeps its meaning); one that moved it
 * waits for that click, to swallow it. */
export function holdUp(hold: Hold): Hold {
  return hold?.moved ? hold : null;
}

/** Click: swallowed when its press moved the window. `detail` 0 is a keyboard click, never part of a press. */
export function holdClick(hold: Hold, detail: number): { hold: Hold; swallow: boolean } {
  return { hold: null, swallow: detail > 0 && !!hold?.moved };
}

/** After a move, where the window is from one look to the next: once it has stood still for SETTLE_LOOKS looks after
 * moving, that is where the person put it (`put`); a later move and stop is put again. */
export const SETTLE_LOOKS = 2;
export type Settle = { at: string; still: number; moving: boolean };
export function settleStep(s: Settle | null, at: string): { s: Settle; put: boolean } {
  if (!s) return { s: { at, still: 0, moving: true }, put: false };
  if (at !== s.at) return { s: { at, still: 0, moving: true }, put: false };
  const still = s.still + 1;
  if (s.moving && still >= SETTLE_LOOKS) return { s: { at, still, moving: false }, put: true };
  return { s: { ...s, still }, put: false };
}

/** The chain from `el` up to the page, as `dragAct` reads it. */
export function chainOf(el: Element | null): Hop[] {
  const out: Hop[] = [];
  for (let e = el; e && e !== document.documentElement; e = e.parentElement) {
    out.push({ tag: e.tagName, classes: Array.from(e.classList), role: e.getAttribute("role"), editable: (e as HTMLElement).isContentEditable });
  }
  return out;
}
