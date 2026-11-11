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
    // Held between the segment's ends too, so the pen never writes backward past them.
    const c1x = clamp(b.x + (c.x - a.x) / 6, b.x, c.x);
    const c1y = clamp(b.y + (c.y - a.y) / 6, lo, hi);
    const c2x = clamp(c.x - (e.x - b.x) / 6, b.x, c.x);
    const c2y = clamp(c.y - (e.y - b.y) / 6, lo, hi);
    d += `C${n1(c1x)} ${n1(c1y)} ${n1(c2x)} ${n1(c2y)} ${n1(c.x)} ${n1(c.y)}`;
  }
  return d;
}

/** Where the voice trace writes: its baseline, as a share of the box's height from the top (handwriting sits low, with
 * room above for tall strokes and a little below for the odd descender). */
export const VOICE_BASELINE = 0.62;

/** The level rises toward the voice in about this long (a syllable lifts the pen at once)… */
export const ATTACK_MS = 60;
/** …and falls back in about this long (the end of a word lets it down gently). */
export const RELEASE_MS = 200;
/** One stroke of the pen every this many ms, each this many px wide: broad, slow strokes, handwriting and not a
 * seismograph. The paper moves under the pen at VOICE_DX / VOICE_STEP_MS px per ms. */
export const VOICE_STEP_MS = 200;
export const VOICE_DX = 13;

/** The meter's level (0 to 1 over 50 dB) above the room's floor that still counts as quiet: below it, a flat line. */
const GATE = 0.06;
/** Above the gate, this much more (20 dB) draws the tallest strokes. */
const SPAN = 0.4;
/** The room's floor follows the level up slowly (a fan that starts is room, not voice) and down fast. */
const FLOOR_UP_MS = 6000;
const FLOOR_DOWN_MS = 150;
/** Where the floor starts, about -50 dBFS on the meter: a quiet room. */
export const FLOOR_START = 0.2;

const unit = (v: number) => (Number.isFinite(v) ? clamp(v, 0, 1) : 0);

/** One step of the drawn level toward `target` after `dt` ms: an exponential ease whose time constant is the attack
 * when rising and the release when falling. Never overshoots; a bad number is read as silence. */
export function smoothLevel(prev: number, target: number, dt: number, o: { attack?: number; release?: number } = {}): number {
  const p = unit(prev);
  const t = unit(target);
  if (!(dt > 0)) return p;
  const tau = Math.max(1, t > p ? (o.attack ?? ATTACK_MS) : (o.release ?? RELEASE_MS));
  const v = p + (t - p) * (1 - Math.exp(-dt / tau));
  return Math.abs(v - t) < 1e-4 ? t : v;
}

/** The room's noise floor after `dt` ms with the level at `raw`: up slowly, down fast. */
export function trackFloor(floor: number, raw: number, dt: number): number {
  const f = unit(floor);
  const r = unit(raw);
  if (!(dt > 0)) return f;
  return f + (r - f) * (1 - Math.exp(-dt / (r > f ? FLOOR_UP_MS : FLOOR_DOWN_MS)));
}

/** How far the pen lifts (0 to 1) for the meter's level over the room's floor. Perceptual: a square-root curve, and a
 * floor of a quarter of the height reached within 1 dB of the gate, so quiet speech is plainly visible and loud speech
 * does not dwarf it. At or below the gate it is 0: silence is a flat line. */
export function voiceLevel(raw: number, floor = 0): number {
  const v = (unit(raw) - unit(floor) - GATE) / SPAN;
  if (!(v > 0)) return 0;
  return clamp(Math.max(Math.sqrt(Math.min(1, v)), Math.min(0.25, v * 8)), 0, 1);
}

/** Listening: the drawn levels (0 to 1, from voiceLevel and smoothLevel, oldest first, one per VOICE_STEP_MS) as a line
 * of handwriting. The pen stays at the right end, where the red dot is, and the paper moves left under it. The strokes
 * take turns: one lifts the pen from the baseline as high as the voice was loud, the next brings it back to the line.
 * Heights vary a little like a hand's, but are fixed per stroke, so the trace does not shimmer as it moves.
 * `first` is the running index of levels[0]; `shift` (0 to 1) is how far the paper has moved since the newest stroke;
 * `live` is the level now, which the pen is moving toward. Silence is a flat line, because a flat line is the honest
 * picture of no sound. */
export function voicePath(
  levels: number[],
  box: Box,
  o: { first?: number; shift?: number; live?: number; dx?: number; pad?: number } = {},
): { d: string; tip: Pt } {
  const pad = o.pad ?? 4;
  const dx = Math.max(2, o.dx ?? VOICE_DX);
  const first = o.first ?? 0;
  const shift = clamp(Number.isFinite(o.shift) ? (o.shift as number) : 0, 0, 1);
  const base = Math.round(box.h * VOICE_BASELINE);
  const up = Math.max(0, base - pad);
  const down = Math.max(0, box.h - pad - base);
  const sx = 0.75;
  // The pen: short of the right edge by the dot's radius.
  const tipX = box.w - pad;
  const strokeY = (k: number, a: number) => {
    const v = unit(a);
    if (v === 0) return base;
    // A lift reaches most of the height, now and then all of it; a return comes back to the line, at most a little below.
    return k % 2 === 1 ? base - up * v * (0.58 + 0.42 * noise(k * 5 + 1)) : base + down * v * 0.35 * noise(k * 5 + 3);
  };
  const n = levels.length;
  const pts: Pt[] = [];
  for (let j = 0; j < n; j++) {
    const back = n - 1 - j + shift;
    // Strokes are a little uneven in width, like a hand's; the newest settles into its place as it leaves the pen.
    const x = tipX - back * dx + (noise((first + j) * 5 + 4) - 0.5) * 0.36 * dx * Math.min(1, back);
    if (x < sx) continue;
    pts.push({ x, y: strokeY(first + j, levels[j]) });
  }
  // Before the first stroke the line is flat from the left edge.
  if (!pts.length) pts.push({ x: sx, y: base });
  else if (pts[0].x > sx + 0.5) {
    const lead = pts[0].x - dx;
    pts.unshift(...(lead > sx + 0.5 ? [{ x: sx, y: base }, { x: lead, y: base }] : [{ x: sx, y: base }]));
  }
  // The pen itself, on its way from the newest stroke to the next.
  const from = n ? strokeY(first + n - 1, levels[n - 1]) : base;
  const live = o.live ?? (n ? levels[n - 1] : 0);
  const tip = { x: tipX, y: from + (strokeY(first + n, live) - from) * shift };
  if (shift > 0.02 || !n) pts.push(tip);
  else pts[pts.length - 1] = { x: tipX, y: from };
  const end = pts[pts.length - 1];
  return { d: smooth(pts, pad, box.h - pad), tip: { x: Math.round(end.x * 10) / 10, y: Math.round(end.y * 10) / 10 } };
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
