// The line of the Ask panel: one continuous stroke the voice draws, that loops while the AI thinks, and that runs down
// the answer's margin while it is written. Pure: every function returns SVG path data for a box, so node --test reads
// it without a window (src/lib/line.test.mjs). The panel draws it 1.5 px wide with round caps (src/styles/live.css).

export type Box = { w: number; h: number };
export type Pt = { x: number; y: number };

/** One decimal, and never "-0". */
const n1 = (v: number) => {
  const r = Math.round(v * 10) / 10;
  return String(r === 0 ? 0 : r);
};
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

/** A fixed pseudo-random number in [0, 1) for an integer: the same sample always wobbles the same way. */
export function noise(i: number): number {
  let x = Math.imul((i | 0) ^ 0x9e3779b9, 0x85ebca6b);
  x = Math.imul(x ^ (x >>> 13), 0xc2b2ae35);
  x ^= x >>> 16;
  return (x >>> 0) / 4294967296;
}

/** A smooth curve through the points (Catmull-Rom as cubic Béziers), its control points held inside [lo, hi] so the
 * curve never leaves the box. */
function smooth(p: Pt[], lo: number, hi: number): string {
  if (p.length === 0) return "";
  let d = `M${n1(p[0].x)} ${n1(p[0].y)}`;
  if (p.length === 1) return d + `L${n1(p[0].x)} ${n1(p[0].y)}`;
  for (let i = 0; i < p.length - 1; i++) {
    const a = p[Math.max(0, i - 1)];
    const b = p[i];
    const c = p[i + 1];
    const e = p[Math.min(p.length - 1, i + 2)];
    const c1x = b.x + (c.x - a.x) / 6;
    const c1y = clamp(b.y + (c.y - a.y) / 6, lo, hi);
    const c2x = c.x - (e.x - b.x) / 6;
    const c2y = clamp(c.y - (e.y - b.y) / 6, lo, hi);
    d += `C${n1(c1x)} ${n1(c1y)} ${n1(c2x)} ${n1(c2y)} ${n1(c.x)} ${n1(c.y)}`;
  }
  return d;
}

/** How many microphone samples fit across the line: at one sample every 90 ms the pen crosses the panel in about 4 s,
 * then the paper moves under it. */
export const VOICE_SLOTS = 48;

/** Where the voice trace writes: its baseline, as a share of the box's height from the top (handwriting sits low, with
 * room above for tall strokes and a little below for the odd descender). */
export const VOICE_BASELINE = 0.64;

/** Listening: the microphone level history (0 to 1, oldest first) as a line of handwriting written left to right. The
 * samples take turns: one lifts the pen from the baseline (as high as the voice was loud, now and then a tall stroke),
 * the next brings it back (now and then below the line). Widths and heights are uneven like a hand's, but fixed per
 * sample, so the trace does not shimmer as it moves. `first` is the running index of levels[0], kept as the history
 * scrolls. Silence is a flat line, because a flat line is the honest picture of no sound. */
export function voicePath(levels: number[], box: Box, o: { slots?: number; pad?: number; first?: number } = {}): { d: string; tip: Pt } {
  const slots = Math.max(2, o.slots ?? VOICE_SLOTS);
  const pad = o.pad ?? 4;
  const take = levels.slice(-slots);
  const first = (o.first ?? 0) + (levels.length - take.length);
  const base = Math.round(box.h * VOICE_BASELINE);
  const up = Math.max(0, base - pad);
  const down = Math.max(0, box.h - pad - base);
  // The pen starts on the text's left edge (half the stroke in) and stops short of the right for the tip's dot.
  const sx = 0.75;
  const dx = (box.w - pad - sx) / slots;
  const pts: Pt[] = [{ x: sx, y: base }];
  for (let i = 0; i < take.length; i++) {
    const k = first + i;
    // The envelope: this sample with a little of its neighbours, so one loud sample is a stroke, not a spike.
    const prev = take[i - 1] ?? take[i];
    const next = take[i + 1] ?? take[i];
    const v = clamp(0.25 * prev + 0.5 * take[i] + 0.25 * next, 0, 1);
    const env = v > 0 ? Math.pow(v, 0.75) : 0;
    const r = noise(k * 5 + 1);
    let y = base;
    if (k % 2 === 1) {
      // A lift: mostly letter height, sometimes a tall stroke, sometimes hardly any.
      const h = r > 0.84 ? 1 : r < 0.12 ? 0.3 : 0.5 + 0.22 * noise(k * 5 + 2);
      y = base - up * env * h;
    } else if (r < 0.14 && env > 0.35) {
      // A return that dips below the line, like a descender.
      y = base + down * env * (0.6 + 0.4 * noise(k * 5 + 3));
    } else y = base + down * env * 0.12 * noise(k * 5 + 3);
    const jitter = (noise(k * 5 + 4) - 0.5) * 0.7 * dx;
    pts.push({ x: sx + (i + 1) * dx + (i === take.length - 1 ? 0 : jitter), y });
  }
  const tip = pts[pts.length - 1];
  return { d: smooth(pts, pad, box.h - pad), tip: { x: Math.round(tip.x * 10) / 10, y: Math.round(tip.y * 10) / 10 } };
}

export type LoopsOpts = {
  /** How many loops. */
  n?: number;
  /** Horizontal distance from one loop to the next. */
  pitch?: number;
  /** The straight run before the first loop and after the last. */
  lead?: number;
  /** Lean of the loops, as x per unit of height (a hand's slant). */
  slant?: number;
  pad?: number;
};

/** Thinking: the pen keeps writing a row of loops while it waits (a prolate cycloid, slanted like handwriting, each loop
 * a little different and always the same). Returns the path and its width. The travelling light is drawn by the
 * stylesheet along this same path. */
export function loopsPath(box: Box, o: LoopsOpts = {}): { d: string; width: number } {
  const pad = o.pad ?? 4;
  const n = Math.max(1, o.n ?? 6);
  const lead = o.lead ?? 12;
  const slant = o.slant ?? 0.32;
  const base = box.h - pad - 1;
  const top = pad + 1;
  const bMax = (base - top) / 2;
  const pitch = o.pitch ?? 20;
  const a = pitch / (2 * Math.PI);
  // Starts on the text's left edge, half the stroke in.
  const sx = 0.75;
  const x0 = sx + lead;
  const pts: string[] = [`M${n1(sx)} ${n1(base)}`, `L${n1(x0)} ${n1(base)}`];
  const steps = 32;
  let xe = x0;
  for (let k = 0; k < n; k++) {
    const b = Math.max(a * 1.15, bMax * (0.86 + 0.14 * noise(k + 101)));
    for (let s = 1; s <= steps; s++) {
      const th = Math.PI + 2 * Math.PI * k + (2 * Math.PI * s) / steps;
      const y = base - b - b * Math.cos(th);
      const x = x0 + a * (th - Math.PI) - b * Math.sin(th) + slant * (base - y);
      pts.push(`L${n1(x)} ${n1(y)}`);
      xe = x0 + a * (th - Math.PI);
    }
  }
  const end = Math.min(box.w - sx, xe + lead);
  pts.push(`L${n1(end)} ${n1(base)}`);
  return { d: pts.join(""), width: Math.ceil(end + sx) };
}

/** Answering: the line down the left margin of the answer, turning under the line being written and ending where the
 * text ends: that end is the caret. `x` is the margin, `top` where it starts, `bottom` the underline's height and
 * `end` the caret's x. */
export function marginPath(p: { x: number; top: number; bottom: number; end: number; r?: number }): string {
  const r = Math.max(0, Math.min(p.r ?? 8, p.bottom - p.top, Math.max(0, p.end - p.x)));
  const bottom = Math.max(p.top, p.bottom);
  if (r === 0) return `M${n1(p.x)} ${n1(p.top)}V${n1(bottom)}` + (p.end > p.x ? `H${n1(p.end)}` : "");
  return `M${n1(p.x)} ${n1(p.top)}V${n1(bottom - r)}Q${n1(p.x)} ${n1(bottom)} ${n1(p.x + r)} ${n1(bottom)}H${n1(Math.max(p.x + r, p.end))}`;
}

/** At rest: a short quiet stroke. */
export function restPath(x: number, y: number, len = 16): string {
  return `M${n1(x)} ${n1(y)}H${n1(x + len)}`;
}

/** Every coordinate pair of a path made by this module (M, L, C, Q, H and V commands), for tests and bounds. */
export function pathPoints(d: string): Pt[] {
  const out: Pt[] = [];
  let x = 0;
  let y = 0;
  let cmd = "";
  let nums: number[] = [];
  const flush = () => {
    if (cmd === "H") nums.forEach((v) => out.push({ x: (x = v), y }));
    else if (cmd === "V") nums.forEach((v) => out.push({ x, y: (y = v) }));
    else
      for (let i = 0; i + 1 < nums.length; i += 2) {
        x = nums[i];
        y = nums[i + 1];
        out.push({ x, y });
      }
    nums = [];
  };
  let tok = "";
  const push = () => {
    if (tok) nums.push(Number(tok));
    tok = "";
  };
  for (const c of d) {
    if ("MLCQHV".includes(c)) {
      push();
      flush();
      cmd = c;
    } else if (c === " " || c === ",") push();
    else if (c === "-" && tok) {
      push();
      tok = c;
    } else tok += c;
  }
  push();
  flush();
  return out;
}
