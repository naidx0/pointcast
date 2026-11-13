// The line of the Ask panel: one continuous stroke the voice draws, that loops while the AI thinks, and that runs down
// the answer's margin while it is written. Pure: every function returns SVG path data for a box, so node --test reads
// it without a window (src/lib/line.test.mjs). The panel draws it 1.5 px wide with round caps (src/styles/live.css).

export type Box = { w: number; h: number };
/** What the line does: the voice draws it, it loops while the words are finished or the AI thinks, or it rests flat. */
export type LineMode = "listen" | "think" | "rest";
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

/** The drawn level rises toward the voice in about this long: within one frame to the eye (the engine's `loudness`
 * already rises at once, so this only takes the edge off a single sample)… */
export const ATTACK_MS = 24;
/** …and falls back in about this long (the end of a word lets it down gently). */
export const RELEASE_MS = 160;
/** The voice wave: one stroke (half a wave, up or down) is this many px wide, written in VOICE_STROKE_STEPS samples of
 * the level, one every VOICE_STEP_MS. The paper moves under the pen at VOICE_DX / (VOICE_STEP_MS * VOICE_STROKE_STEPS)
 * px per ms, as broad and slow as handwriting; the samples are small, so the pen follows the voice at once. */
export const VOICE_DX = 13;
export const VOICE_STROKE_STEPS = 4;
export const VOICE_STEP_MS = 50;

/** The room's floor follows the level up slowly (a fan that starts is room, not voice) and down fast. */
const FLOOR_UP_MS = 10000;
const FLOOR_DOWN_MS = 150;
/** Where the floor starts, above most rooms (about -42 dBFS): it falls to the room within a moment of quiet, so a room
 * is never drawn as a voice while the floor finds it. */
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

/** The meter reads the level over the room's floor, on one fixed scale (the engine's `loudness` is 0 to 1 over 40 dB,
 * so 0.025 is 1 dB): a quiet voice is plainly visible, a loud one never slams to the top, and nothing adapts to the
 * loudest word (an adaptive top made every voice look the same height, and a cough squashed the words after it). */
export type Meter = { floor: number };
/** Just over the room (1 dB): the room's own flutter stays flat. */
const METER_GATE = 0.025;
/** Where the lift is half way to VOICE_TOP, over the gate (4 dB): steep for a quiet voice, flat toward the top. */
const METER_KNEE = 0.1;
/** The highest the voice lifts the pen, as a share of the room above the baseline: a shout stays under the top. */
export const VOICE_TOP = 0.9;

export const meterStart = (): Meter => ({ floor: FLOOR_START });

/** The meter after `dt` ms with the level at `raw`: its floor as trackFloor follows the room. */
export function meterStep(m: Meter, raw: number, dt: number): Meter {
  return { floor: trackFloor(m.floor, unit(raw), dt) };
}

/** How far the voice lifts the pen (0 to VOICE_TOP) for `raw` over the meter's floor: 0 within the gate, then a soft
 * saturating curve, steep for a quiet voice and flat near the top. Never decreases as the voice gets louder. */
export function meterLevel(m: Meter, raw: number): number {
  const v = unit(raw) - unit(m.floor) - METER_GATE;
  return v > 0 ? (VOICE_TOP * v) / (v + METER_KNEE) : 0;
}

/** The older fixed mapping, kept for callers that read the level without a meter: the same curve over `floor`. */
export function voiceLevel(raw: number, floor = 0): number {
  return meterLevel({ floor }, raw);
}

/** Where the voice wave stands at running sample index `t` (fractional) for the drawn level `a` (0 to 1): its height
 * above the baseline as a share of the room above it. A wave between 0.3 of the level and all of it, one stroke up and
 * one down per wavelength, so the pen is plainly off the line the whole time there is voice (never at a crossing, never
 * "bounce, stop, bounce") and flat at silence. */
export function waveLift(t: number, a: number): number {
  const v = unit(a);
  if (v === 0) return 0;
  return v * (0.65 + 0.35 * Math.sin((Math.PI * t) / VOICE_STROKE_STEPS));
}

/** Listening: the drawn levels (0 to 1, from meterLevel and smoothLevel, oldest first, one per VOICE_STEP_MS) as one
 * flowing wave, its height following the voice. The pen stays at the right end, where the red dot is, and the paper
 * moves left under it. `first` is the running index of levels[0]; `shift` (0 to 1) is how far the paper has moved
 * since the newest sample; `live` is the level now, which the pen itself shows at once. `dx` is one stroke's width.
 * Silence is a flat line, because a flat line is the honest picture of no sound. */
export function voicePath(
  levels: number[],
  box: Box,
  o: { first?: number; shift?: number; live?: number; dx?: number; pad?: number } = {},
): { d: string; tip: Pt } {
  const pad = o.pad ?? 4;
  const step = Math.max(0.5, Math.max(2, o.dx ?? VOICE_DX) / VOICE_STROKE_STEPS);
  const first = o.first ?? 0;
  const shift = clamp(Number.isFinite(o.shift) ? (o.shift as number) : 0, 0, 1);
  const base = Math.round(box.h * VOICE_BASELINE);
  const up = Math.max(0, base - pad);
  const sx = 0.75;
  // The pen: short of the right edge by the dot's radius.
  const tipX = box.w - pad;
  const y = (t: number, a: number) => base - up * waveLift(t, a);
  const n = levels.length;
  const pts: Pt[] = [];
  for (let j = 0; j < n; j++) {
    const x = tipX - (n - 1 - j + shift) * step;
    if (x < sx) continue;
    pts.push({ x, y: y(first + j, levels[j]) });
  }
  // Before the first sample the line is flat from the left edge; a history that runs off the edge continues to it.
  if (!pts.length) pts.push({ x: sx, y: base });
  else if (pts[0].x > sx + 0.5) pts.unshift({ x: sx, y: n > pts.length ? pts[0].y : base });
  // The pen itself: the level now, where the wave is now.
  const live = o.live ?? (n ? levels[n - 1] : 0);
  const tip = { x: tipX, y: y(first + n - 1 + shift, live) };
  if (n && shift < 0.02) pts[pts.length - 1] = tip;
  else pts.push(tip);
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

/** The Ask panel's voice (owner, 2026-11-12: the breathing wave, "more waving, more action", never one spot repeating):
 * three soft waves layered, no head and no tail, quiet at both ends. Each layer travels its own way at its own speed,
 * the frequencies are unrelated (golden ratio, silver ratio) and wander slowly, and the swell drifts along the line, so
 * no frame repeats. The voice sets how high it swells and how fast it moves; at silence it still breathes, barely. */
export type Breath = { phase: number; drift: number; amp: number };
export const breathStart = (): Breath => ({ phase: 0, drift: 0, amp: 0 });
/** At silence the waves keep this much of their height: alive, not flat. */
export const BREATH_IDLE = 0.06;
/** How fast the waves travel, in radians per second, at silence and added at full voice. */
export const BREATH_SPEED = 1.8;
export const BREATH_SPEED_VOICE = 4.2;
/** The layers: height share, opacity, stroke width, frequency, and speed (its sign is the way it travels). */
export const BREATH_LAYERS = [
  { k: 1, alpha: 0.95, width: 1.8, f: 1, speed: 1 },
  { k: 0.72, alpha: 0.5, width: 1.3, f: 1.618, speed: -1.37 },
  { k: 0.48, alpha: 0.28, width: 1.1, f: 2.414, speed: 1.83 },
] as const;

/** The waves after `dt` ms with the drawn level at `level` (0 to 1): the height eases toward the voice (quick up,
 * gentle down), the travel speeds up with it, and the slow wander keeps its own clock. A bad number reads as silence. */
export function breathStep(b: Breath, level: number, dt: number): Breath {
  if (!(dt > 0)) return b;
  const s = Math.min(dt, 100) / 1000;
  const want = Math.max(BREATH_IDLE, unit(level));
  const amp = smoothLevel(b.amp, want, dt, { attack: 60, release: 260 });
  return { phase: b.phase + s * (BREATH_SPEED + BREATH_SPEED_VOICE * amp), drift: b.drift + s, amp };
}

/** Where layer `i` stands at `u` (0 to 1 along the line), as a share of the half-height: a sum of two travelling waves
 * under a swell whose centre and width wander, and zero at both ends. */
export function breathAt(u: number, b: Breath, i: number): number {
  const L = BREATH_LAYERS[i] ?? BREATH_LAYERS[0];
  const t = b.drift;
  const p = b.phase * L.speed;
  const fm = 1 + 0.22 * Math.sin(t * 0.37 * L.f + i * 2.1);
  const wave = 0.62 * Math.sin(2 * Math.PI * 1.25 * L.f * fm * u + p) + 0.38 * Math.sin(2 * Math.PI * 2.07 * L.f * u - p * 0.61 + 1.3 * L.f);
  const c = 0.5 + 0.17 * Math.sin(t * 0.53 + L.f) + 0.07 * Math.sin(t * 1.31 + i);
  const wide = 0.36 + 0.09 * Math.sin(t * 0.29 + i * 1.7);
  const swell = Math.exp(-(((u - c) / wide) ** 2));
  // A soft gain: a normal voice reads clearly, a loud one rounds off under the edge instead of hitting it.
  return Math.tanh(1.8 * unit(b.amp) * L.k * wave * swell * Math.sin(Math.PI * clamp(u, 0, 1)));
}

/** Layer `i` as SVG path data across `box`, about every 2 px, centred on the box's middle with `pad` kept clear. */
export function breathPath(box: Box, b: Breath, i: number, pad = 3): string {
  const mid = box.h / 2;
  const half = Math.max(0, mid - pad);
  const n = Math.max(2, Math.ceil(box.w / 2));
  let d = "";
  for (let j = 0; j <= n; j++) {
    const x = (box.w * j) / n;
    const y = mid - half * breathAt(j / n, b, i);
    d += `${j ? "L" : "M"}${n1(x)} ${n1(y)}`;
  }
  return d;
}
