// Marks on a frozen picture: their numbers, which ones a crop keeps, the labels and caption words they become, and
// the undo history that holds them and the box. Pure, so node --test reads it without a window (marks.test.mjs).
// Every position is in picture pixels; the box (the crop) only decides what is kept, it never moves a mark.

export type Pt = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };
/** The parts of a mark these helpers read; the overlay's shapes carry more. */
export type Mark = { id: number; kind: string; at?: Pt; a?: Pt; b?: Pt; pts?: Pt[]; r?: Rect; text?: string };
export type LabelOut = { n: number; text: string; kind?: "step" };

/** Pins and steps share one number sequence, so each number drawn into the picture names one mark. */
const NUMBERED = new Set(["pin", "step"]);

/** Each pin's and step's number, in the order they were placed, counting on from `base`. */
export function numbers(list: readonly Mark[], base = 0): Map<number, number> {
  const m = new Map<number, number>();
  let n = base;
  for (const sh of list) if (NUMBERED.has(sh.kind)) m.set(sh.id, ++n);
  return m;
}

const inside = (p: Pt, b: Rect) => p.x >= b.x && p.y >= b.y && p.x <= b.x + b.w && p.y <= b.y + b.h;
// Touching counts: a line with no height across the box is still in it.
const meets = (r: Rect, b: Rect) => r.x <= b.x + b.w && b.x <= r.x + r.w && r.y <= b.y + b.h && b.y <= r.y + r.h;

function span(pts: Pt[]): Rect {
  const xs = pts.map((p) => p.x);
  const ys = pts.map((p) => p.y);
  const x = Math.min(...xs);
  const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
}

/**
 * Whether a mark is kept by the box. A mark placed at a point (pin, step, note, loupe, ring) is kept when its point is
 * in the box; an outlined control when its middle is; a stroke, arrow, rectangle or hidden area when any of it is, and
 * the picture edge clips the rest.
 */
export function survives(m: Mark, box: Rect): boolean {
  if (m.kind === "element" && m.r) return inside({ x: m.r.x + m.r.w / 2, y: m.r.y + m.r.h / 2 }, box);
  if (m.at) return inside(m.at, box);
  if (m.a && m.b) return meets(span([m.a, m.b]), box);
  if (m.pts?.length) return meets(span(m.pts), box);
  return true;
}

/** The marks a box keeps, the same objects in the same order: nothing is moved or copied. */
export const kept = <T extends Mark>(list: readonly T[], box: Rect): T[] => list.filter((m) => survives(m, box));

/**
 * The labels a picture sends: pins and steps by their numbers (a pin without words is named "pin n", a step without
 * words sends none), then the typed notes numbered on.
 */
export function labelsFor(list: readonly Mark[], base = 0): LabelOut[] {
  const out: LabelOut[] = [];
  let n = base;
  for (const sh of list) {
    const t = (sh.text ?? "").trim();
    if (sh.kind === "pin") out.push({ n: ++n, text: t || `pin ${n}` });
    else if (sh.kind === "step") out.push({ n: ++n, text: t, kind: "step" });
  }
  for (const sh of list) if (sh.kind === "text" && (sh.text ?? "").trim()) out.push({ n: ++n, text: (sh.text ?? "").trim() });
  return out;
}

/**
 * The labels in the words the prompt's picture list uses (prompt.rs `caption_marks`): `pins: 1=too far left; 2`,
 * `steps: 3, 4=save`, then each note on the whole picture (kind "note") as its words. A label numbered 0 without a
 * kind (stored by builds before notes had one) is a note too, never pin 0.
 */
export const isNote = (l: { n: number; kind?: string }) => l.kind === "note" || (!l.n && !l.kind);

export function captionMarks(labels: readonly { n: number; text: string; kind?: string }[]): string[] {
  const one = (l: { n: number; text: string }) => (l.text.trim() ? `${l.n}=${l.text.trim()}` : `${l.n}`);
  const pins = labels.filter((l) => !isNote(l) && l.kind !== "step").map(one);
  const steps = labels.filter((l) => !isNote(l) && l.kind === "step").map(one);
  const notes = labels.filter(isNote).map((l) => l.text.trim()).filter(Boolean);
  return [...(pins.length ? [`pins: ${pins.join("; ")}`] : []), ...(steps.length ? [`steps: ${steps.join(", ")}`] : []), ...notes];
}

// Room a picture from History leaves around it (logical px) for the toolbar below and the hint under that.
export const EDGE = { side: 40, top: 40, bottom: 128 };

export type View = { fit: number; ox: number; oy: number; dw: number; dh: number };

/**
 * Where the picture sits on the screen: `fit` logical px per picture px and its top left corner. It depends on the
 * picture and the screen only, never on the box, so cropping or moving the box leaves every mark where it was.
 */
export function view(natural: { w: number; h: number }, vp: { w: number; h: number }, edit: boolean, dpr: number): View {
  const room = edit ? { w: vp.w - 2 * EDGE.side, h: vp.h - EDGE.top - EDGE.bottom } : vp;
  // A picture from History is never shown larger than its own pixels.
  const most = edit ? 1 / (dpr || 1) : Infinity;
  const fit = natural.w && room.w > 0 && room.h > 0 ? Math.min(room.w / natural.w, room.h / natural.h, most) : 1;
  const dw = natural.w * fit;
  const dh = natural.h * fit;
  return { fit, ox: (vp.w - dw) / 2, oy: edit ? EDGE.top + (room.h - dh) / 2 : (vp.h - dh) / 2, dw, dh };
}

export const toScreen = (p: Pt, v: View): Pt => ({ x: v.ox + p.x * v.fit, y: v.oy + p.y * v.fit });

// Undo history. `now` is the mark list and `cut` the box; a crop is a step like a mark. Text edits change `now` in
// place so one keystroke is not one undo step.
export type Snap<S> = { now: S[]; cut: Rect | null };
export type H<S> = { past: Snap<S>[]; now: S[]; cut: Rect | null; future: Snap<S>[] };
export type A<S> =
  | { t: "add"; s: S }
  | { t: "text"; id: number; text: string }
  | { t: "drop"; id: number }
  | { t: "move"; id: number; at: Pt }
  | { t: "remove"; id: number }
  | { t: "put"; s: S }
  | { t: "seed"; base: S[]; s: S[] }
  // The box changed by hand: one undo step.
  | { t: "crop"; r: Rect }
  // The box first drawn: no step, and every step before it undoes to it.
  | { t: "frame"; r: Rect }
  | { t: "undo" }
  | { t: "redo" }
  | { t: "reset"; now?: S[] };

export const empty = <S>(): H<S> => ({ past: [], now: [], cut: null, future: [] });

const same = (a: Rect | null, b: Rect | null) => !!a && !!b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;

export function hist<S extends Mark>(h: H<S>, a: A<S>): H<S> {
  const snap: Snap<S> = { now: h.now, cut: h.cut };
  const step = (now: S[], cut = h.cut): H<S> => ({ past: [...h.past, snap], now, cut, future: [] });
  switch (a.t) {
    case "add":
      return step([...h.now, a.s]);
    case "text":
      return { ...h, now: h.now.map((x) => (x.id === a.id && "text" in x ? ({ ...x, text: a.text } as S) : x)) };
    case "drop": {
      const prev = h.past[h.past.length - 1];
      const last = h.now[h.now.length - 1];
      if (prev && last?.id === a.id && prev.now.length === h.now.length - 1) return { ...h, past: h.past.slice(0, -1), now: prev.now };
      return { ...h, now: h.now.filter((x) => x.id !== a.id) };
    }
    case "move":
      return step(h.now.map((x) => (x.id === a.id && "at" in x ? ({ ...x, at: a.at } as S) : x)));
    case "put":
      return step(h.now.map((x) => (x.id === a.s.id ? a.s : x)));
    case "remove":
      return step(h.now.filter((x) => x.id !== a.id));
    case "crop":
      return same(a.r, h.cut) ? h : step(h.now, a.r);
    case "frame": {
      const fill = (s: Snap<S>) => (s.cut ? s : { ...s, cut: a.r });
      return { past: h.past.map(fill), now: h.now, cut: a.r, future: h.future.map(fill) };
    }
    // Each mark placed for the person counts as one step, so undo takes them away one at a time.
    case "seed": {
      const past: Snap<S>[] = [{ now: a.base, cut: null }];
      for (const x of a.s.slice(0, -1)) past.push({ now: [...past[past.length - 1].now, x], cut: null });
      return { past, now: [...a.base, ...a.s], cut: null, future: [] };
    }
    case "undo": {
      const p = h.past[h.past.length - 1];
      return p ? { past: h.past.slice(0, -1), now: p.now, cut: p.cut, future: [snap, ...h.future] } : h;
    }
    case "redo": {
      const f = h.future[0];
      return f ? { past: [...h.past, snap], now: f.now, cut: f.cut, future: h.future.slice(1) } : h;
    }
    case "reset":
      return { past: [], now: a.now ?? [], cut: null, future: [] };
  }
}
