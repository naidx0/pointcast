import { smoothLevel } from "./line.ts";

/** Aurora: three soft lenses of light that pass through each other, each with a fine bright core. The motion maths and
 * the drawing, ported 1:1 from docs/design/waves-draft-3.html ("Aurora · Glow"). Pure: no DOM, so it is tested. */

export type Aurora = {
  /** Accumulated travel of the waves: advanced by dt, never computed from the time since start. */
  phase: number;
  /** Seconds of the slow wander of the swell (the reference's `t`). */
  drift: number;
  /** The smoothed level, never below AURORA_FLOOR. */
  amp: number;
};

export type Ink = [number, number, number];
export type Band = { f: number; sp: number; k: number; a: number };

export const AURORA_FLOOR = 0.14;
export const AURORA_ATTACK_MS = 60;
export const AURORA_RELEASE_MS = 260;
/** The reference was chosen at its "Slow" setting: every dt is scaled by this. */
export const AURORA_SPEED = 0.5;
/** A lens and its centre never leave this share of the half-height. */
export const AURORA_EDGE = 0.96;

export const BANDS: readonly Band[] = [
  { f: 1, sp: 1, k: 0, a: 0.34 },
  { f: 1.618, sp: -1.37, k: 2.1, a: 0.24 },
  { f: 2.414, sp: 1.83, k: 4.2, a: 0.16 },
];

/** A number the maths can use: a bad one (NaN, infinite, not a number) reads as 0, silence. */
export const num = (x: unknown): number => (typeof x === "number" && Number.isFinite(x) ? x : 0);
const unit = (x: unknown): number => Math.min(1, Math.max(0, num(x)));
const taper = (u: number) => Math.sin(Math.PI * u);

export const auroraStart = (): Aurora => ({ phase: 0, drift: 0, amp: AURORA_FLOOR });

/** How fast the waves travel at this level (radians per second). */
export const auroraSpeed = (amp: number): number => 1.8 + 4.2 * amp;

/** Advance by dt ms at the level (0 to 1, after the meter): the level is eased in 60 ms and out in 260 ms over a floor
 * of 0.14, and the phase gains dt times the speed, so a louder voice runs the waves faster and never jumps them. */
export function auroraStep(s: Aurora, level: number, dtMs: number, speed = AURORA_SPEED): Aurora {
  const dt = Math.max(0, num(dtMs)) * Math.max(0, num(speed));
  const want = Math.max(AURORA_FLOOR, unit(level));
  const amp = Math.max(AURORA_FLOOR, smoothLevel(unit(s.amp), want, dt, { attack: AURORA_ATTACK_MS, release: AURORA_RELEASE_MS }));
  return { phase: num(s.phase) + (dt / 1000) * auroraSpeed(amp), drift: num(s.drift) + dt / 1000, amp };
}

/** Travelling waves under a wandering swell, zero at both ends. */
export function waveAt(u: number, tt: number, amp: number, phase: number, f = 1, sp = 1): number {
  const t = num(tt);
  const p = num(phase) * sp;
  const a = unit(amp);
  const w = 0.62 * Math.sin(2 * Math.PI * 1.25 * f * u + p) + 0.38 * Math.sin(2 * Math.PI * 2.07 * f * u - p * 0.61 + 1.3 * f);
  const c = 0.5 + 0.17 * Math.sin(t * 0.53 + f);
  const wide = 0.36 + 0.09 * Math.sin(t * 0.29 + f);
  return Math.tanh(1.8 * a * w * Math.exp(-(((u - c) / wide) ** 2)) * taper(u));
}

/** A lens: its centre line and half-thickness at u, smooth, zero-thick at both ends, never past the edge. */
export function lens(u: number, tt: number, amp: number, phase: number, f: number, sp: number, k: number): [number, number] {
  const t = num(tt);
  const a = unit(amp);
  const centre = 0.5 * waveAt(u, t, a, phase, f, sp);
  const swell = Math.exp(-(((u - (0.5 + 0.2 * Math.sin(t * 0.41 * f + k))) / (0.3 + 0.08 * Math.sin(t * 0.33 + k))) ** 2));
  const th = Math.tanh((0.1 + 1.6 * a) * swell * Math.max(0, taper(u)) ** 1.1 * (0.72 + 0.28 * Math.sin(2 * Math.PI * f * u + t * sp * 1.3 + k)));
  return [centre, Math.max(0, Math.min(th, AURORA_EDGE - Math.abs(centre)))];
}

/** The ink of the surface, from a CSS colour (#rgb, #rrggbb, rgb(), rgba()); null when it is not one of those. */
export function parseInk(css: string): Ink | null {
  const s = css.trim().toLowerCase();
  let m = /^#([0-9a-f]{3})$/.exec(s);
  if (m) return [...m[1]].map((h) => parseInt(h + h, 16)) as Ink;
  m = /^#([0-9a-f]{6})(?:[0-9a-f]{2})?$/.exec(s);
  if (m) return [0, 2, 4].map((i) => parseInt(m![1].slice(i, i + 2), 16)) as Ink;
  m = /^rgba?\(\s*([\d.]+)[,\s]+([\d.]+)[,\s]+([\d.]+)/.exec(s);
  if (m) return [1, 2, 3].map((i) => Math.min(255, Math.max(0, Math.round(Number(m![i]))))) as Ink;
  return null;
}

/** Relative luminance, 0 (black) to 1 (white). */
export const luminance = ([r, g, b]: Ink): number => (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;

/** Light ink on a dark surface adds light ("lighter"); dark ink on a light one would vanish that way, so it mixes normally. */
export const isLight = (ink: Ink): boolean => luminance(ink) > 0.5;

const rgba = (c: Ink, a: number) => `rgba(${c[0]},${c[1]},${c[2]},${a})`;

type Ctx = CanvasRenderingContext2D;

/** A smooth outline through points, using midpoints so there are no corners at any size. */
function smooth(g: Ctx, pts: [number, number][], move = true) {
  if (move) g.moveTo(pts[0][0], pts[0][1]);
  for (let i = 1; i < pts.length - 1; i++) {
    const mx = (pts[i][0] + pts[i + 1][0]) / 2;
    const my = (pts[i][1] + pts[i + 1][1]) / 2;
    g.quadraticCurveTo(pts[i][0], pts[i][1], mx, my);
  }
  g.lineTo(pts[pts.length - 1][0], pts[pts.length - 1][1]);
}

function lensPath(g: Ctx, w: number, h: number, s: Aurora, amp: number, b: Band): [number, number][] {
  const mid = h / 2;
  const H = mid - 1.5;
  const n = Math.max(2, Math.ceil(w));
  const top: [number, number][] = [];
  const bot: [number, number][] = [];
  const core: [number, number][] = [];
  for (let j = 0; j <= n; j++) {
    const u = j / n;
    const [c, th] = lens(u, s.drift, amp, s.phase, b.f, b.sp, b.k);
    top.push([u * w, mid + H * (c - th)]);
    bot.push([u * w, mid + H * (c + th)]);
    core.push([u * w, mid + H * c]);
  }
  g.beginPath();
  smooth(g, top);
  smooth(g, bot.reverse(), false);
  g.closePath();
  return core;
}

/** One frame of Aurora · Glow in the box w by h (CSS px; the caller has scaled for the pixel ratio and cleared it).
 * `add` is "lighter" compositing (a light ink); otherwise the lenses mix normally. Ends faded like the reference. */
export function drawAurora(g: Ctx, w: number, h: number, s: Aurora, ink: Ink, add = isLight(ink)) {
  const amp = Math.max(unit(s.amp), 0.12);
  g.globalCompositeOperation = add ? "lighter" : "source-over";
  for (const b of BANDS) {
    const core = lensPath(g, w, h, s, amp, b);
    g.shadowColor = rgba(ink, b.a * 1.4);
    g.shadowBlur = 8;
    g.fillStyle = rgba(ink, b.a);
    g.fill();
    g.shadowBlur = 0;
    g.beginPath();
    smooth(g, core);
    g.lineWidth = 1;
    g.strokeStyle = rgba(ink, Math.min(1, b.a * 2.4));
    g.stroke();
  }
  g.globalCompositeOperation = "destination-in";
  const m = g.createLinearGradient(0, 0, w, 0);
  m.addColorStop(0, "rgba(0,0,0,0)");
  m.addColorStop(0.1, "#000");
  m.addColorStop(0.9, "#000");
  m.addColorStop(1, "rgba(0,0,0,0)");
  g.fillStyle = m;
  g.fillRect(0, 0, w, h);
  g.globalCompositeOperation = "source-over";
}

/** The voice wave the person picked: Classic (the pen), Strands (breath) or Aurora. */
export type WaveStyle = "pen" | "breath" | "aurora";
export const WAVE_STYLES: readonly WaveStyle[] = ["pen", "breath", "aurora"];
/** What a saved or sent value means: Aurora unless it names one of the others exactly. */
export const waveStyleOf = (x: unknown): WaveStyle => (x === "pen" || x === "breath" ? x : "aurora");
