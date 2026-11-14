import { smoothLevel } from "./line.ts";

/** Bars: a row of small rounded bars that rise with the voice, the newest at the middle and older ones moving outward
 * to both sides (the look of Wispr Flow's listening indicator). Pure: no DOM, so it is tested. */

export type Bars = {
  /** The smoothed level now, never below BARS_FLOOR. */
  amp: number;
  /** Levels newest first: slot 0 is the middle bar, slot i the pair i bars out. */
  trail: number[];
  /** Ms since the trail last took a new sample. */
  since: number;
};

export const BARS_FLOOR = 0.04;
export const BARS_ATTACK_MS = 45;
export const BARS_RELEASE_MS = 170;
/** A new sample enters the middle this often; the rest move one bar out. */
export const BARS_STEP_MS = 55;
export const BARS_BAR = 3;
export const BARS_GAP = 3;
/** Shortest a bar is (a dot at rest), as a share of the height. */
export const BARS_DOT = 0.14;

const unit = (x: unknown): number => (typeof x === "number" && Number.isFinite(x) ? Math.min(1, Math.max(0, x)) : 0);

/** How many bars fit in `width` px (always odd, so one is exactly in the middle). */
export function barCount(width: number): number {
  const n = Math.floor((Math.max(0, unit(0) + (Number.isFinite(width) ? width : 0)) + BARS_GAP) / (BARS_BAR + BARS_GAP));
  const odd = n % 2 === 0 ? n - 1 : n;
  return Math.max(1, odd);
}

export const barsStart = (): Bars => ({ amp: BARS_FLOOR, trail: [], since: 0 });

/** Advance by dt ms at the level (0 to 1): the level eases in 45 ms and out in 170 ms, and every BARS_STEP_MS the
 * trail takes the current level and pushes the older ones outward. `keep` is how many slots are ever needed. */
export function barsStep(s: Bars, level: number, dtMs: number, keep: number): Bars {
  const dt = Math.max(0, Number.isFinite(dtMs) ? dtMs : 0);
  const want = Math.max(BARS_FLOOR, unit(level));
  const amp = Math.max(BARS_FLOOR, smoothLevel(unit(s.amp), want, dt, { attack: BARS_ATTACK_MS, release: BARS_RELEASE_MS }));
  let since = s.since + dt;
  let trail = s.trail;
  let steps = Math.floor(since / BARS_STEP_MS);
  if (steps > 0) {
    since -= steps * BARS_STEP_MS;
    steps = Math.min(steps, Math.max(1, keep));
    trail = [...Array(steps).fill(amp), ...s.trail].slice(0, Math.max(1, keep));
  }
  return { amp, trail, since };
}

/** The height of the bar `i` places out from the middle, as a share of the full height (BARS_DOT up to 1): the level
 * it holds, shaped so the middle runs tallest and the far ends fall away. */
export function barHeight(i: number, trail: readonly number[], count: number): number {
  const half = Math.max(1, (count - 1) / 2);
  const lvl = unit(trail[i] ?? BARS_FLOOR);
  const reach = Math.cos((Math.PI / 2) * Math.min(1, i / (half + 1))) ** 0.7;
  return Math.min(1, BARS_DOT + (1 - BARS_DOT) * lvl * reach);
}

/** Draw the bars into a `w` x `h` canvas context in `ink` (rgb): rounded, centred vertically, the middle one in the
 * middle of the width. */
export function drawBars(g: CanvasRenderingContext2D, w: number, h: number, s: Bars, ink: [number, number, number]) {
  const count = barCount(w);
  const used = count * BARS_BAR + (count - 1) * BARS_GAP;
  const x0 = (w - used) / 2;
  const mid = (count - 1) / 2;
  g.fillStyle = `rgb(${ink[0]}, ${ink[1]}, ${ink[2]})`;
  for (let k = 0; k < count; k++) {
    const i = Math.round(Math.abs(k - mid));
    const bh = Math.max(BARS_BAR, barHeight(i, s.trail, count) * h);
    const x = x0 + k * (BARS_BAR + BARS_GAP);
    g.beginPath();
    g.roundRect(x, (h - bh) / 2, BARS_BAR, bh, BARS_BAR / 2);
    g.fill();
  }
}
