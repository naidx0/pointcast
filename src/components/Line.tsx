import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { BREATH_LAYERS, VOICE_DX, VOICE_STEP_MS, VOICE_STROKE_STEPS, breathPath, breathStart, breathStep, loopsPath, meterLevel, meterStart, meterStep, smoothLevel, voicePath, type LineMode } from "@/lib/line";

/** Where the line hears the voice: called with a setter for the level (0 to 1, `loudness`), returns the unsubscribe. */
export type LevelSource = (set: (raw: number) => void) => () => void;

export type LineTone = "ink" | "accent" | "warn" | "quiet";

/** The one line of Pointcast, drawn the same way in the Ask panel and the dictation capsule. Listening: the voice draws
 * it (one flowing wave whose height follows the voice at once); thinking: it loops while it waits; at rest: flat. The
 * place it sits in sets its colour (ink, or Ask's accent) and size. */
export function Line({
  mode,
  height,
  className = "",
  tone = "ink",
  levels,
  dx = VOICE_DX,
  tip = "red",
  loops = 6,
  pitch = 19,
  lead,
  look = "pen",
}: {
  mode: LineMode;
  height: number;
  className?: string;
  tone?: LineTone;
  levels?: LevelSource;
  /** Listening: px per stroke. */
  dx?: number;
  /** Listening: the pen's dot, red where it is the live microphone, ink where a red dot sits beside the line. */
  tip?: "red" | "ink";
  /** Thinking: how many loops, how far apart, and the straight run before and after them. */
  loops?: number;
  pitch?: number;
  lead?: number;
  /** Listening: the pen's trace (the dictation capsule), or the breathing wave (the Ask panel). */
  look?: "pen" | "breath";
}) {
  if (mode === "listen" && look === "breath") return <BreathWave className={className} tone={tone} height={height} levels={levels} />;
  if (mode === "think") return <Loops className={className} tone={tone} height={height} n={loops} pitch={pitch} lead={lead} />;
  return <Voice className={className} tone={tone} height={height} levels={levels} dx={dx} tip={tip} still={mode === "rest"} />;
}

/** The voice's line. The level is read over the room's floor and eased toward the voice every frame (within a frame to
 * rise, gently to fall); the pen shows it at once, and a sample is laid down every VOICE_STEP_MS while the paper moves
 * smoothly under the pen. Drawn straight into the SVG, so React
 * does not render 60 times a second. At rest, or with reduced motion: a still flat line. */
function Voice({
  className,
  tone,
  height,
  levels,
  dx,
  tip,
  still,
}: {
  className: string;
  tone: LineTone;
  height: number;
  levels?: LevelSource;
  dx: number;
  tip: "red" | "ink";
  still: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const path = useRef<SVGPathElement>(null);
  const dot = useRef<SVGCircleElement>(null);
  const [w, setW] = useState(0);
  // What the pen has written survives a change of width (the picture taken out, a longer app name).
  const pen = useRef({ raw: 0, meter: meterStart(), env: 0, acc: 0, first: 0, hist: [] as number[] });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const size = () => setW(Math.max(40, Math.round(el.clientWidth)));
    size();
    const ro = new ResizeObserver(size);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    if (!w) return;
    const box = { w, h: height };
    const paint = (d: string, at: { x: number; y: number }) => {
      path.current?.setAttribute("d", d);
      dot.current?.setAttribute("cx", String(at.x));
      dot.current?.setAttribute("cy", String(at.y));
    };
    const flat = voicePath([], box, { dx });
    paint(flat.d, flat.tip);
    if (still || !levels || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const s = pen.current;
    let last = performance.now();
    const keep = Math.ceil((w * VOICE_STROKE_STEPS) / Math.max(2, dx)) + 2;
    const off = levels((x) => (s.raw = x));
    let raf = 0;
    const draw = (t: number) => {
      raf = requestAnimationFrame(draw);
      const dt = Math.min(100, Math.max(0, t - last));
      last = t;
      if (document.hidden) return;
      s.meter = meterStep(s.meter, s.raw, dt);
      s.env = smoothLevel(s.env, meterLevel(s.meter, s.raw), dt);
      s.acc += dt;
      while (s.acc >= VOICE_STEP_MS) {
        s.acc -= VOICE_STEP_MS;
        s.hist.push(s.env);
      }
      while (s.hist.length > keep) {
        s.hist.shift();
        s.first++;
      }
      const { d, tip: at } = voicePath(s.hist, box, { first: s.first, shift: s.acc / VOICE_STEP_MS, live: s.env, dx });
      paint(d, at);
    };
    raf = requestAnimationFrame(draw);
    return () => {
      off();
      cancelAnimationFrame(raf);
    };
  }, [w, height, dx, still, levels]);
  return (
    <div className={`line-voice ${className}`} ref={ref} aria-hidden="true">
      {w ? (
        <svg className={`ln ${tone}`} width={w} height={height} viewBox={`0 0 ${w} ${height}`}>
          <path ref={path} />
          {still ? null : <circle className={`tip ${tip}`} ref={dot} r={tip === "red" ? 3 : 2} />}
        </svg>
      ) : null}
    </div>
  );
}

/** The Ask panel's voice: three soft waves layered (line.ts breathStep, breathPath), faded at both ends, each travelling
 * its own way. Drawn straight into the SVG every frame; still and flat with reduced motion or without a level. */
function BreathWave({ className, tone, height, levels }: { className: string; tone: LineTone; height: number; levels?: LevelSource }) {
  const ref = useRef<HTMLDivElement>(null);
  const paths = useRef<(SVGPathElement | null)[]>([]);
  const fade = `breath-fade-${useId().replace(/:/g, "")}`;
  const [w, setW] = useState(0);
  const st = useRef({ raw: 0, meter: meterStart(), env: 0, b: breathStart() });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const size = () => setW(Math.max(40, Math.round(el.clientWidth)));
    size();
    const ro = new ResizeObserver(size);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => {
    if (!w) return;
    const box = { w, h: height };
    const s = st.current;
    const paint = () => BREATH_LAYERS.forEach((_, i) => paths.current[i]?.setAttribute("d", breathPath(box, s.b, i)));
    paint();
    if (!levels || matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const off = levels((x) => (s.raw = x));
    let last = performance.now();
    let raf = 0;
    const draw = (t: number) => {
      raf = requestAnimationFrame(draw);
      const dt = Math.min(100, Math.max(0, t - last));
      last = t;
      if (document.hidden) return;
      s.meter = meterStep(s.meter, s.raw, dt);
      s.env = smoothLevel(s.env, meterLevel(s.meter, s.raw), dt);
      s.b = breathStep(s.b, s.env, dt);
      paint();
    };
    raf = requestAnimationFrame(draw);
    return () => {
      off();
      cancelAnimationFrame(raf);
    };
  }, [w, height, levels]);
  return (
    <div className={`line-voice line-breath ${className}`} ref={ref} aria-hidden="true">
      {w ? (
        <svg className={`ln breath ${tone}`} width={w} height={height} viewBox={`0 0 ${w} ${height}`}>
          <defs>
            <linearGradient id={`${fade}-g`} x1="0" x2="1" y1="0" y2="0">
              <stop offset="0" stopColor="#fff" stopOpacity="0" />
              <stop offset="0.14" stopColor="#fff" stopOpacity="1" />
              <stop offset="0.86" stopColor="#fff" stopOpacity="1" />
              <stop offset="1" stopColor="#fff" stopOpacity="0" />
            </linearGradient>
            <mask id={fade} maskUnits="userSpaceOnUse" x="0" y="0" width={w} height={height}>
              <rect width={w} height={height} fill={`url(#${fade}-g)`} />
            </mask>
          </defs>
          <g mask={`url(#${fade})`}>
            {BREATH_LAYERS.map((l, i) => (
              <path key={i} ref={(el) => void (paths.current[i] = el)} style={{ opacity: l.alpha, strokeWidth: l.width }} />
            ))}
          </g>
        </svg>
      ) : null}
    </div>
  );
}

/** Thinking, or the words being finished: the pen writes a row of loops and light travels through them. */
function Loops(p: { className: string; tone: LineTone; height: number; n: number; pitch: number; lead?: number }) {
  const { className, tone, height, n, pitch, lead } = p;
  const { d, width } = loopsPath({ w: 40 + n * pitch, h: height }, { n, pitch, lead });
  return (
    <div className={`line-loops ${className}`} aria-hidden="true">
      <svg className={`ln loops ${tone}`} width={width} height={height} viewBox={`0 0 ${width} ${height}`}>
        <path className="ghost" d={d} />
        <path className="tail" pathLength={100} d={d} />
        <path className="head" pathLength={100} d={d} />
      </svg>
    </div>
  );
}
