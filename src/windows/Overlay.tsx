import { useEffect, useLayoutEffect, useReducer, useRef, useState, type PointerEvent as PE } from "react";
import {
  Check,
  Highlighter,
  MapPin,
  MousePointer2,
  MoveUpRight,
  Pencil,
  Redo2,
  Square,
  Type,
  Undo2,
  ZoomIn,
  type LucideIcon,
} from "lucide-react";
import { onOverlayFrame, regionCancel, regionCommit, type Label } from "@/lib/api";
import "../styles/overlay.css";

type Layer = "selecting" | "annotating" | "pinning" | "drawing";
type Tool = "move" | "pin" | "arrow" | "rect" | "highlight" | "pen" | "text" | "magnify";
type Pt = { x: number; y: number };
type Rect = { x: number; y: number; w: number; h: number };
type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

// Every shape lives in frame-pixel space (the physical pixels of the frozen PNG).
type Shape =
  | { id: number; kind: "pin" | "text"; c: string; at: Pt; text: string }
  | { id: number; kind: "arrow" | "rect"; c: string; a: Pt; b: Pt }
  | { id: number; kind: "pen" | "highlight"; c: string; pts: Pt[] }
  | { id: number; kind: "magnify"; at: Pt };

// Drawing primitives. Shapes compile to these once; the SVG view and the canvas rasterizer both just interpret them.
type Seg = ["M" | "L", number, number] | ["Q", number, number, number, number] | ["Z"];
type Op =
  | { k: "path"; d: Seg[]; w: number; stroke?: string; fill?: string; alpha?: number }
  | { k: "ell"; x: number; y: number; r: number; fill?: string; stroke?: string; w?: number; shadow?: boolean }
  | { k: "box"; x: number; y: number; w: number; h: number; r: number; fill?: string; stroke?: string; sw?: number; shadow?: boolean }
  | { k: "txt"; x: number; y: number; str: string; px: number; wt: number; fill: string; mid?: boolean }
  | { k: "lens"; x: number; y: number; r: number; z: number };

type Ix =
  | { k: "sel"; a: Pt; cx: number; cy: number; moved: boolean; r: Rect | null }
  | { k: "size"; h: Handle; o: Rect }
  | { k: "move"; a: Pt; o: Rect }
  | { k: "draw"; shape: Shape }
  | { k: "drop"; a: Pt; tool: "pin" | "text" | "magnify" };

type Out = { url: string; w: number; h: number; labels: Label[] };

const PREVIEW = typeof location !== "undefined" && location.search.includes("preview=1");
const COLORS = ["#FF3B30", "#FF9500", "#FFCC00", "#34C759", "#007AFF"];
const HL = "#FFCC00";
const FONT = '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
// Sizes in logical (on-screen) px. Everything drawn into the frame is multiplied by s = frame px per logical px.
const M = { line: 4, rect: 3, pen: 3.5, hl: 18, pin: 11, font: 13, padX: 8, pillH: 26, gap: 6, lens: 60, ring: 3, head: 16 };
const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const HX = { nw: 0, n: 0.5, ne: 1, e: 1, se: 1, s: 0.5, sw: 0, w: 0 };
const HY = { nw: 0, n: 0, ne: 0, e: 0.5, se: 1, s: 1, sw: 1, w: 0.5 };
const TOOLS: { id: Tool; Icon: LucideIcon; label: string; key: string }[] = [
  { id: "move", Icon: MousePointer2, label: "Move selection", key: "V" },
  { id: "pin", Icon: MapPin, label: "Pin", key: "P" },
  { id: "arrow", Icon: MoveUpRight, label: "Arrow", key: "A" },
  { id: "rect", Icon: Square, label: "Rectangle", key: "R" },
  { id: "highlight", Icon: Highlighter, label: "Highlighter", key: "H" },
  { id: "pen", Icon: Pencil, label: "Pen", key: "D" },
  { id: "text", Icon: Type, label: "Text", key: "T" },
  { id: "magnify", Icon: ZoomIn, label: "Magnify 2x", key: "M" },
];
const KEYS = Object.fromEntries(TOOLS.map((t) => [t.key.toLowerCase(), t.id])) as Record<string, Tool>;

function esc(layer: Layer): Layer | "idle" {
  if (layer === "drawing" || layer === "pinning") return "annotating";
  if (layer === "annotating") return "selecting";
  return "idle";
}

const GESTURE: Record<Tool, string> = {
  move: "Drag inside the box to move it",
  pin: "Click to drop a numbered pin",
  arrow: "Drag an arrow",
  rect: "Drag a rectangle",
  highlight: "Drag to highlight",
  pen: "Draw freehand",
  text: "Click to place a label",
  magnify: "Click to drop a 2x loupe",
};

function hint(layer: Layer, tool: Tool): string {
  if (layer === "selecting") return "Drag to select · Click for full screen · Esc cancel";
  if (layer === "pinning") return "Type a label · Enter or Esc to finish";
  if (layer === "drawing") return "Release to finish · Esc discard";
  return `${GESTURE[tool]} · Enter capture · Esc back`;
}

// Undo history: `now` is the shape list; text edits mutate it in place so one keystroke is not one undo step.
type H = { past: Shape[][]; now: Shape[]; future: Shape[][] };
type A =
  | { t: "add"; s: Shape }
  | { t: "text"; id: number; text: string }
  | { t: "drop"; id: number }
  | { t: "undo" }
  | { t: "redo" }
  | { t: "reset"; now?: Shape[] };

function hist(h: H, a: A): H {
  switch (a.t) {
    case "add":
      return { past: [...h.past, h.now], now: [...h.now, a.s], future: [] };
    case "text":
      return { ...h, now: h.now.map((x) => (x.id === a.id && (x.kind === "pin" || x.kind === "text") ? { ...x, text: a.text } : x)) };
    case "drop": {
      const prev = h.past[h.past.length - 1];
      const last = h.now[h.now.length - 1];
      if (prev && last?.id === a.id && prev.length === h.now.length - 1) return { ...h, past: h.past.slice(0, -1), now: prev };
      return { ...h, now: h.now.filter((x) => x.id !== a.id) };
    }
    case "undo":
      return h.past.length ? { past: h.past.slice(0, -1), now: h.past[h.past.length - 1], future: [h.now, ...h.future] } : h;
    case "redo":
      return h.future.length ? { past: [...h.past, h.now], now: h.future[0], future: h.future.slice(1) } : h;
    case "reset":
      return { past: [], now: a.now ?? [], future: [] };
  }
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);
const ink = (c: string) => (c === HL ? "#1c1c1e" : "#fff");
const inside = (p: Pt, b: Rect) => p.x >= b.x && p.y >= b.y && p.x <= b.x + b.w && p.y <= b.y + b.h;
const norm = (a: Pt, b: Pt): Rect => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });

// Anchored shapes only count when their anchor is inside the box; strokes are just clipped by it.
const seen = (sh: Shape, b: Rect) => !("at" in sh) || inside(sh.at, b);

function numbering(list: Shape[]): Map<number, number> {
  const m = new Map<number, number>();
  let n = 0;
  for (const sh of list) if (sh.kind === "pin") m.set(sh.id, ++n);
  return m;
}

function resize(o: Rect, h: Handle, p: Pt, min: number): Rect {
  let x1 = o.x;
  let y1 = o.y;
  let x2 = o.x + o.w;
  let y2 = o.y + o.h;
  if (h.includes("w")) x1 = Math.min(p.x, x2 - min);
  if (h.includes("e")) x2 = Math.max(p.x, x1 + min);
  if (h.includes("n")) y1 = Math.min(p.y, y2 - min);
  if (h.includes("s")) y2 = Math.max(p.y, y1 + min);
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 };
}

let mctx: CanvasRenderingContext2D | null = null;
function textW(str: string, px: number, wt = 600) {
  mctx ??= document.createElement("canvas").getContext("2d")!;
  mctx.font = `${wt} 100px ${FONT}`;
  return (mctx.measureText(str).width * px) / 100;
}

function smooth(pts: Pt[]): Seg[] {
  const d: Seg[] = [["M", pts[0].x, pts[0].y]];
  const end = pts[pts.length - 1];
  if (pts.length === 1) return [...d, ["L", end.x + 0.01, end.y]];
  for (let i = 1; i < pts.length - 1; i++) {
    d.push(["Q", pts[i].x, pts[i].y, (pts[i].x + pts[i + 1].x) / 2, (pts[i].y + pts[i + 1].y) / 2]);
  }
  d.push(["L", end.x, end.y]);
  return d;
}

function pill(x: number, y: number, str: string, s: number, fill: string, fg: string, edge: string): Op[] {
  const h = M.pillH * s;
  return [
    { k: "box", x, y: y - h / 2, w: textW(str, M.font * s) + 2 * M.padX * s, h, r: 7 * s, fill, stroke: edge, sw: 1.5 * s, shadow: true },
    { k: "txt", x: x + M.padX * s, y, str, px: M.font * s, wt: 600, fill: fg },
  ];
}

// The one place a shape becomes pixels. s = frame px per logical px (natural.w / displayed width).
function ops(sh: Shape, s: number, n: number, bare: boolean): Op[] {
  switch (sh.kind) {
    case "pin": {
      const { x, y } = sh.at;
      const r = M.pin * s;
      const o: Op[] = [
        { k: "ell", x, y, r, fill: sh.c, stroke: "#fff", w: 2 * s, shadow: true },
        { k: "txt", x, y, str: String(n), px: 12 * s, wt: 700, fill: ink(sh.c), mid: true },
      ];
      if (sh.text && !bare) o.push(...pill(x + r + M.gap * s, y, sh.text, s, "rgba(255,255,255,.96)", "#1c1c1e", sh.c));
      return o;
    }
    case "text":
      return sh.text && !bare ? pill(sh.at.x, sh.at.y, sh.text, s, sh.c, ink(sh.c), "#fff") : [];
    case "arrow": {
      const { a, b } = sh;
      const len = Math.hypot(b.x - a.x, b.y - a.y) || 1;
      const ux = (b.x - a.x) / len;
      const uy = (b.y - a.y) / len;
      const hl = Math.min(M.head * s, len * 0.7);
      const hw = hl * 0.5;
      const bx = b.x - ux * hl;
      const by = b.y - uy * hl;
      return [
        { k: "path", d: [["M", a.x, a.y], ["L", bx + ux * hl * 0.3, by + uy * hl * 0.3]], stroke: sh.c, w: M.line * s },
        { k: "path", d: [["M", b.x, b.y], ["L", bx - uy * hw, by + ux * hw], ["L", bx + uy * hw, by - ux * hw], ["Z"]], fill: sh.c, stroke: sh.c, w: 2 * s },
      ];
    }
    case "rect":
      return [{ k: "box", ...norm(sh.a, sh.b), r: 4 * s, stroke: sh.c, sw: M.rect * s }];
    case "pen":
      return [{ k: "path", d: smooth(sh.pts), stroke: sh.c, w: M.pen * s }];
    case "highlight":
      return [{ k: "path", d: smooth(sh.pts), stroke: HL, w: M.hl * s, alpha: 0.42 }];
    case "magnify": {
      const { x, y } = sh.at;
      const r = M.lens * s;
      return [
        { k: "ell", x, y, r, fill: "#000", shadow: true },
        { k: "lens", x, y, r, z: 2 },
        { k: "ell", x, y, r, stroke: "#fff", w: M.ring * s },
      ];
    }
  }
}

const dstr = (d: Seg[]) => d.map((g) => g[0] + (g.slice(1) as number[]).join(" ")).join("");

function Ops({ list, id, src, nat }: { list: Op[]; id: number; src: string; nat: { w: number; h: number } }) {
  return (
    <>
      {list.map((o, i) => {
        const key = `${id}-${i}`;
        switch (o.k) {
          case "path":
            return <path key={key} d={dstr(o.d)} fill={o.fill ?? "none"} stroke={o.stroke} strokeWidth={o.w} strokeLinecap="round" strokeLinejoin="round" opacity={o.alpha} />;
          case "ell":
            return <circle key={key} cx={o.x} cy={o.y} r={o.r} fill={o.fill ?? "none"} stroke={o.stroke} strokeWidth={o.w} filter={o.shadow ? "url(#ov-sh)" : undefined} />;
          case "box":
            return <rect key={key} x={o.x} y={o.y} width={o.w} height={o.h} rx={o.r} fill={o.fill ?? "none"} stroke={o.stroke} strokeWidth={o.sw} filter={o.shadow ? "url(#ov-sh)" : undefined} />;
          case "txt":
            return (
              <text key={key} x={o.x} y={o.y + 0.35 * o.px} fontSize={o.px} fontWeight={o.wt} fill={o.fill} textAnchor={o.mid ? "middle" : "start"} fontFamily={FONT} style={{ whiteSpace: "pre" }}>
                {o.str}
              </text>
            );
          case "lens":
            return (
              <g key={key}>
                <clipPath id={`ov-lens-${key}`}>
                  <circle cx={o.x} cy={o.y} r={o.r} />
                </clipPath>
                <g clipPath={`url(#ov-lens-${key})`}>
                  <image href={src} x={0} y={0} width={nat.w} height={nat.h} transform={`translate(${o.x} ${o.y}) scale(${o.z}) translate(${-o.x} ${-o.y})`} />
                </g>
              </g>
            );
        }
      })}
    </>
  );
}

function paint(ctx: CanvasRenderingContext2D, img: HTMLImageElement, o: Op, s: number) {
  ctx.save();
  if ("shadow" in o && o.shadow) {
    ctx.shadowColor = "rgba(0,0,0,.35)";
    ctx.shadowBlur = 6 * s;
    ctx.shadowOffsetY = 2 * s;
  }
  const fin = (fill?: string, stroke?: string, w = 0) => {
    if (fill) {
      ctx.fillStyle = fill;
      ctx.fill();
      ctx.shadowColor = "transparent";
    }
    if (stroke) {
      ctx.strokeStyle = stroke;
      ctx.lineWidth = w;
      ctx.lineCap = "round";
      ctx.lineJoin = "round";
      ctx.stroke();
    }
  };
  switch (o.k) {
    case "path":
      ctx.beginPath();
      for (const g of o.d) {
        if (g[0] === "M") ctx.moveTo(g[1], g[2]);
        else if (g[0] === "L") ctx.lineTo(g[1], g[2]);
        else if (g[0] === "Q") ctx.quadraticCurveTo(g[1], g[2], g[3], g[4]);
        else ctx.closePath();
      }
      ctx.globalAlpha = o.alpha ?? 1;
      fin(o.fill, o.stroke, o.w);
      break;
    case "ell":
      ctx.beginPath();
      ctx.arc(o.x, o.y, o.r, 0, Math.PI * 2);
      fin(o.fill, o.stroke, o.w);
      break;
    case "box":
      roundRect(ctx, o.x, o.y, o.w, o.h, o.r);
      fin(o.fill, o.stroke, o.sw);
      break;
    case "txt":
      ctx.font = `${o.wt} ${o.px}px ${FONT}`;
      ctx.fillStyle = o.fill;
      ctx.textAlign = o.mid ? "center" : "left";
      ctx.textBaseline = "alphabetic";
      ctx.fillText(o.str, o.x, o.y + 0.35 * o.px);
      break;
    case "lens": {
      ctx.beginPath();
      ctx.arc(o.x, o.y, o.r, 0, Math.PI * 2);
      ctx.clip();
      const q = o.r / o.z;
      ctx.drawImage(img, o.x - q, o.y - q, 2 * q, 2 * q, o.x - o.r, o.y - o.r, 2 * o.r, 2 * o.r);
      break;
    }
  }
  ctx.restore();
}

async function rasterize(src: string, b: Rect, list: Shape[], s: number, nums: Map<number, number>): Promise<string> {
  const img = await loadImg(src);
  const x0 = Math.round(b.x);
  const y0 = Math.round(b.y);
  const w = Math.max(1, Math.round(b.x + b.w) - x0);
  const h = Math.max(1, Math.round(b.y + b.h) - y0);
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  const ctx = c.getContext("2d")!;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, x0, y0, w, h, 0, 0, w, h);
  ctx.translate(-x0, -y0);
  ctx.beginPath();
  ctx.rect(x0, y0, w, h);
  ctx.clip();
  for (const sh of list) for (const o of ops(sh, s, nums.get(sh.id) ?? 0, false)) paint(ctx, img, o, s);
  const url = c.toDataURL("image/png");
  return url.slice(url.indexOf(",") + 1);
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  r = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

function loadImg(src: string) {
  return new Promise<HTMLImageElement>((res, rej) => {
    const i = new Image();
    i.onload = () => res(i);
    i.onerror = rej;
    i.src = src;
  });
}

function EditInput(p: { x: number; y: number; w: number; text: string; hold: string; bg: string; fg: string; edge: string; onText: (t: string) => void; onDone: (empty: boolean) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    const f = requestAnimationFrame(() => {
      const el = ref.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    });
    return () => cancelAnimationFrame(f);
  }, []);
  return (
    <input
      ref={ref}
      className="ov-in"
      value={p.text}
      placeholder={p.hold}
      spellCheck={false}
      style={{ left: p.x, top: p.y, width: p.w, ["--ov-in-bg" as string]: p.bg, ["--ov-in-fg" as string]: p.fg, ["--ov-in-edge" as string]: p.edge }}
      onChange={(e) => p.onText(e.target.value)}
      onBlur={(e) => p.onDone(!e.currentTarget.value.trim())}
      onPointerDown={(e) => e.stopPropagation()}
    />
  );
}

export function Overlay() {
  const rootRef = useRef<HTMLDivElement>(null);
  const barRef = useRef<HTMLDivElement>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [natural, setNatural] = useState({ w: 0, h: 0 });
  const [vp, setVp] = useState({ w: window.innerWidth, h: window.innerHeight });
  const [layer, setLayer] = useState<Layer>("selecting");
  const [tool, setTool] = useState<Tool>("pin");
  const [color, setColor] = useState(COLORS[0]);
  const [box, setBox] = useState<Rect | null>(null);
  const [live, setLive] = useState<Shape | null>(null);
  const [editing, setEditing] = useState<number | null>(null);
  const [cur, setCur] = useState<Pt | null>(null);
  const [tb, setTb] = useState({ w: 0, h: 0 });
  const [out, setOut] = useState<Out | null>(null);
  const [hs, dispatch] = useReducer(hist, { past: [], now: [], future: [] });
  const ix = useRef<Ix | null>(null);
  const ids = useRef(0);
  const busy = useRef(false);
  const editRef = useRef<number | null>(null);
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {});

  useEffect(() => {
    const load = (f: { png_b64: string; width: number; height: number }) => {
      setSrc(`data:image/png;base64,${f.png_b64}`);
      setNatural({ w: f.width, h: f.height });
      setLayer("selecting");
      setBox(null);
      setLive(null);
      setEditing(null);
      setOut(null);
      editRef.current = null;
      ix.current = null;
      dispatch({ t: "reset" });
    };
    let u: Promise<(() => void) | null> = Promise.resolve(null);
    try {
      u = onOverlayFrame(load).catch(() => null);
    } catch {
      /* plain browser, no Tauri */
    }
    if (PREVIEW) {
      const img = new Image();
      img.onload = () => {
        setSrc("/freeze.png");
        setNatural({ w: img.naturalWidth || 1280, h: img.naturalHeight || 800 });
        setLayer("annotating");
        setBox({ x: 420, y: 280, w: 360, h: 220 });
        dispatch({ t: "reset", now: [{ id: ++ids.current, kind: "pin", c: COLORS[0], at: { x: 560, y: 360 }, text: "Place order — 28px too far left" }] });
      };
      img.src = "/freeze.png";
    }
    const onResize = () => setVp({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    const onKey = (e: KeyboardEvent) => keyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey);
      void u.then((fn) => fn?.());
    };
  }, []);

  useLayoutEffect(() => {
    const el = barRef.current;
    if (el && (el.offsetWidth !== tb.w || el.offsetHeight !== tb.h)) setTb({ w: el.offsetWidth, h: el.offsetHeight });
  });

  // Frame px <-> logical px. s = natural.w / displayed image width: every size drawn into the frame is multiplied by it.
  const fit = natural.w ? Math.min(vp.w / natural.w, vp.h / natural.h) : 1;
  const s = 1 / fit;
  const dw = natural.w * fit;
  const dh = natural.h * fit;
  const ox = (vp.w - dw) / 2;
  const oy = (vp.h - dh) / 2;
  const toImg = (e: { clientX: number; clientY: number }): Pt => ({ x: (e.clientX - ox) / fit, y: (e.clientY - oy) / fit });
  const clampP = (p: Pt): Pt => ({ x: clamp(p.x, 0, natural.w), y: clamp(p.y, 0, natural.h) });
  const toScr = (r: Rect): Rect => ({ x: ox + r.x * fit, y: oy + r.y * fit, w: r.w * fit, h: r.h * fit });
  const full: Rect = { x: 0, y: 0, w: natural.w, h: natural.h };

  const shapes = hs.now;
  const vis = box ? shapes.filter((x) => seen(x, box)) : [];
  const nums = numbering(vis);
  const sb = box ? toScr(box) : null;
  const grab = (e: PE) => rootRef.current?.setPointerCapture(e.pointerId);

  function startEdit(id: number) {
    editRef.current = id;
    setEditing(id);
    setLayer("pinning");
  }

  function endEdit(id: number, empty: boolean, kind: Shape["kind"]) {
    if (empty && kind === "text") dispatch({ t: "drop", id });
    if (editRef.current !== id) return;
    editRef.current = null;
    setEditing(null);
    setLayer((l) => (l === "pinning" ? "annotating" : l));
  }

  function back() {
    const next = esc(layer);
    if (next === "idle") {
      void regionCancel().catch(() => {});
      return;
    }
    if (layer === "drawing") {
      ix.current = null;
      setLive(null);
    }
    if (layer === "pinning") {
      editRef.current = null;
      setEditing(null);
    }
    if (next === "selecting") {
      setBox(null);
      dispatch({ t: "reset" });
    }
    setLayer(next);
  }

  async function commit() {
    if (!src || !natural.w || busy.current) return;
    busy.current = true;
    try {
      const b = layer === "selecting" || !box ? full : box;
      const list = shapes.filter((x) => seen(x, b) && !(x.kind === "text" && !x.text.trim()));
      const nn = numbering(list);
      const png = await rasterize(src, b, list, s, nn);
      const pins = list.filter((x) => x.kind === "pin");
      const notes = list.filter((x) => x.kind === "text");
      const labels: Label[] = [
        ...pins.map((p, i) => ({ n: i + 1, text: ("text" in p && p.text.trim()) || `pin ${i + 1}` })),
        ...notes.map((t, i) => ({ n: pins.length + i + 1, text: "text" in t ? t.text.trim() : "" })),
      ];
      if (PREVIEW) setOut({ url: `data:image/png;base64,${png}`, w: Math.round(b.w), h: Math.round(b.h), labels });
      else await regionCommit(png, labels);
    } catch (err) {
      console.error("pointcast overlay commit failed", err);
    } finally {
      busy.current = false;
    }
  }

  keyRef.current = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    const t = e.target as HTMLElement | null;
    const typing = !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
    if (e.key === "Escape") {
      if (typing) t.blur();
      else back();
      return;
    }
    if (e.key === "Enter") {
      if (typing) t.blur();
      else if (!e.repeat) void commit();
      return;
    }
    if (typing) return;
    const mod = e.ctrlKey || e.metaKey;
    const k = e.key.toLowerCase();
    if (mod && (k === "z" || k === "y")) {
      e.preventDefault();
      dispatch({ t: k === "y" || e.shiftKey ? "redo" : "undo" });
    } else if (!mod && !e.altKey && KEYS[k]) setTool(KEYS[k]);
  };

  function down(e: PE<HTMLDivElement>) {
    if (e.button !== 0 || !src || !natural.w) return;
    const p = toImg(e);
    if (layer === "selecting") {
      ix.current = { k: "sel", a: clampP(p), cx: e.clientX, cy: e.clientY, moved: false, r: null };
      grab(e);
      return;
    }
    if (!box) return;
    if (tool === "move") {
      if (inside(p, box)) {
        ix.current = { k: "move", a: p, o: box };
        setCur({ x: e.clientX, y: e.clientY });
        grab(e);
      }
      return;
    }
    if (!inside(p, box)) return;
    if (tool === "pin" || tool === "text" || tool === "magnify") {
      ix.current = { k: "drop", a: p, tool };
      grab(e);
      return;
    }
    const id = ++ids.current;
    const shape: Shape =
      tool === "arrow" || tool === "rect" ? { id, kind: tool, c: color, a: p, b: p } : { id, kind: tool, c: tool === "highlight" ? HL : color, pts: [p] };
    ix.current = { k: "draw", shape };
    setLive(shape);
    setLayer("drawing");
    grab(e);
  }

  function startSize(e: PE<HTMLDivElement>, h: Handle) {
    e.stopPropagation();
    if (e.button !== 0 || !box) return;
    ix.current = { k: "size", h, o: box };
    setCur({ x: e.clientX, y: e.clientY });
    grab(e);
  }

  function move(e: PE<HTMLDivElement>) {
    const m = ix.current;
    if (!m) return;
    const p = clampP(toImg(e));
    if (m.k === "sel") {
      if (!m.moved && Math.hypot(e.clientX - m.cx, e.clientY - m.cy) < 4) return;
      m.moved = true;
      m.r = norm(m.a, p);
      setBox(m.r);
      setCur({ x: e.clientX, y: e.clientY });
    } else if (m.k === "size") {
      setBox(resize(m.o, m.h, p, 12 * s));
      setCur({ x: e.clientX, y: e.clientY });
    } else if (m.k === "move") {
      const q = toImg(e);
      setBox({ ...m.o, x: clamp(m.o.x + q.x - m.a.x, 0, natural.w - m.o.w), y: clamp(m.o.y + q.y - m.a.y, 0, natural.h - m.o.h) });
      setCur({ x: e.clientX, y: e.clientY });
    } else if (m.k === "draw") {
      const sh = m.shape;
      if (sh.kind === "arrow" || sh.kind === "rect") m.shape = { ...sh, b: p };
      else if (sh.kind === "pen" || sh.kind === "highlight") {
        const l = sh.pts[sh.pts.length - 1];
        if (Math.hypot(p.x - l.x, p.y - l.y) < 1.2 * s) return;
        m.shape = { ...sh, pts: [...sh.pts, p] };
      }
      setLive(m.shape);
    }
  }

  function up(e: PE<HTMLDivElement>) {
    const m = ix.current;
    ix.current = null;
    setCur(null);
    if (rootRef.current?.hasPointerCapture(e.pointerId)) rootRef.current.releasePointerCapture(e.pointerId);
    if (!m) return;
    if (m.k === "sel") {
      if (!m.moved) {
        setBox(full);
        setLayer("annotating");
      } else if (m.r && m.r.w * fit >= 8 && m.r.h * fit >= 8) setLayer("annotating");
      else setBox(null);
    } else if (m.k === "draw") {
      const sh = m.shape;
      setLive(null);
      setLayer("annotating");
      const long = sh.kind === "arrow" || sh.kind === "rect" ? Math.hypot(sh.b.x - sh.a.x, sh.b.y - sh.a.y) >= 6 * s : true;
      if (long) dispatch({ t: "add", s: sh });
    } else if (m.k === "drop") {
      const id = ++ids.current;
      if (m.tool === "magnify") dispatch({ t: "add", s: { id, kind: "magnify", at: m.a } });
      else {
        dispatch({ t: "add", s: { id, kind: m.tool, c: color, at: m.a, text: "" } });
        startEdit(id);
      }
    }
  }

  function cancel() {
    if (ix.current?.k === "draw") {
      setLive(null);
      setLayer("annotating");
    }
    ix.current = null;
    setCur(null);
  }

  // Toolbar: just below the box, flipped above when it does not fit, clamped on screen and clear of the hint pill.
  let bar = { x: 0, y: 0 };
  if (sb) {
    const m = 8;
    const floor = vp.h - 52;
    let y = sb.y + sb.h + 12;
    if (y + tb.h > floor) {
      y = sb.y - 12 - tb.h;
      if (y < m) y = floor - tb.h;
    }
    bar = { x: clamp(sb.x + sb.w / 2 - tb.w / 2, m, Math.max(m, vp.w - m - tb.w)), y: clamp(y, m, Math.max(m, vp.h - m - tb.h)) };
  }

  const ed = editing != null ? vis.find((x) => x.id === editing) : undefined;
  const readout = cur && box ? `${Math.round(box.w)} × ${Math.round(box.h)} px` : "";
  const rw = readout.length * 7 + 20;
  const cursor = layer === "selecting" ? "crosshair" : tool === "move" ? "move" : tool === "text" ? "text" : "crosshair";
  const dim = `M0 0H${vp.w}V${vp.h}H0Z${sb ? `M${sb.x} ${sb.y}h${sb.w}v${sb.h}h${-sb.w}Z` : ""}`;
  const canEdit = tool === "pin" || tool === "text";

  return (
    <div
      ref={rootRef}
      className="ov-root"
      style={{ cursor }}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      onContextMenu={(e) => e.preventDefault()}
    >
      {src && natural.w ? (
        <>
          <img className="ov-frame" src={src} alt="" draggable={false} style={{ left: ox, top: oy, width: dw, height: dh }} />
          <svg className="ov-dim" width={vp.w} height={vp.h}>
            <path d={dim} fillRule="evenodd" fill="rgba(0,0,0,.42)" />
          </svg>
          <svg className="ov-shapes" style={{ left: ox, top: oy, width: dw, height: dh }} viewBox={`0 0 ${natural.w} ${natural.h}`}>
            <defs>
              <filter id="ov-sh" x="-50%" y="-50%" width="200%" height="200%">
                <feDropShadow dx="0" dy={2 * s} stdDeviation={3 * s} floodColor="#000" floodOpacity=".35" />
              </filter>
              {box ? (
                <clipPath id="ov-clip">
                  <rect x={box.x} y={box.y} width={box.w} height={box.h} />
                </clipPath>
              ) : null}
            </defs>
            <g clipPath={box ? "url(#ov-clip)" : undefined}>
              {[...vis, ...(live ? [live] : [])].map((sh) => (
                <Ops key={sh.id} id={sh.id} src={src} nat={natural} list={ops(sh, s, nums.get(sh.id) ?? 0, sh.id === editing)} />
              ))}
            </g>
          </svg>
          {sb ? <div className="ov-sel" style={{ left: sb.x, top: sb.y, width: sb.w, height: sb.h }} /> : null}
          {sb && layer !== "selecting"
            ? HANDLES.map((h) => (
                <div key={h} className={`ov-h ov-h-${h}`} style={{ left: sb.x + sb.w * HX[h], top: sb.y + sb.h * HY[h] }} onPointerDown={(e) => startSize(e, h)} />
              ))
            : null}
          {canEdit && layer !== "drawing"
            ? vis.map((sh) => {
                if (sh.kind !== "pin" && sh.kind !== "text") return null;
                const x = ox + sh.at.x * fit;
                const y = oy + sh.at.y * fit;
                const pw = sh.text ? textW(sh.text, M.font) + 2 * M.padX : 0;
                const left = sh.kind === "pin" ? x - M.pin : x;
                const width = sh.kind === "pin" ? M.pin * 2 + (pw ? M.gap + pw : 0) : pw;
                if (!width) return null;
                return (
                  <div
                    key={sh.id}
                    className="ov-hit"
                    style={{ left, top: y - M.pillH / 2, width, height: M.pillH }}
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      startEdit(sh.id);
                    }}
                  />
                );
              })
            : null}
          {ed && (ed.kind === "pin" || ed.kind === "text") ? (
            <EditInput
              key={ed.id}
              x={ox + ed.at.x * fit + (ed.kind === "pin" ? M.pin + M.gap : 0)}
              y={oy + ed.at.y * fit - M.pillH / 2}
              w={Math.max(112, textW(ed.text || " ", M.font) + 2 * M.padX + 10)}
              text={ed.text}
              hold={ed.kind === "pin" ? "Label…" : "Type a label…"}
              bg={ed.kind === "pin" ? "#fff" : ed.c}
              fg={ed.kind === "pin" ? "#1c1c1e" : ink(ed.c)}
              edge={ed.kind === "pin" ? ed.c : "#fff"}
              onText={(text) => dispatch({ t: "text", id: ed.id, text })}
              onDone={(empty) => endEdit(ed.id, empty, ed.kind)}
            />
          ) : null}
        </>
      ) : (
        <div className="ov-wait">Capturing freeze-frame…</div>
      )}
      {readout ? (
        <div
          className="ov-hud ov-read"
          style={{ left: cur!.x + 16 + rw > vp.w ? cur!.x - 16 - rw : cur!.x + 16, top: cur!.y + 20 + 28 > vp.h ? cur!.y - 20 - 28 : cur!.y + 20 }}
        >
          {readout}
        </div>
      ) : null}
      {sb && layer !== "selecting" ? (
        <div
          ref={barRef}
          className="ov-hud ov-bar"
          style={{ left: bar.x, top: bar.y }}
          onPointerDown={(e) => e.stopPropagation()}
          onMouseDown={(e) => e.preventDefault()}
        >
          {TOOLS.map((t, i) => (
            <span key={t.id} style={{ display: "contents" }}>
              {i === 1 ? <span className="ov-sep" /> : null}
              <button type="button" className="ov-btn" aria-pressed={tool === t.id} aria-label={t.label} title={`${t.label} (${t.key})`} onClick={() => setTool(t.id)}>
                <t.Icon size={18} strokeWidth={1.75} />
              </button>
            </span>
          ))}
          <span className="ov-sep" />
          {COLORS.map((c) => (
            <button key={c} type="button" className="ov-sw" aria-pressed={color === c} aria-label={`Colour ${c}`} title={c} onClick={() => setColor(c)}>
              <i style={{ ["--c" as string]: c }} />
            </button>
          ))}
          <span className="ov-sep" />
          <button type="button" className="ov-btn" aria-label="Undo" title="Undo (Ctrl/Cmd+Z)" disabled={!hs.past.length} onClick={() => dispatch({ t: "undo" })}>
            <Undo2 size={18} strokeWidth={1.75} />
          </button>
          <button type="button" className="ov-btn" aria-label="Redo" title="Redo (Shift+Ctrl/Cmd+Z)" disabled={!hs.future.length} onClick={() => dispatch({ t: "redo" })}>
            <Redo2 size={18} strokeWidth={1.75} />
          </button>
          <span className="ov-sep" />
          <button type="button" className="ov-go" title="Capture (Enter)" onClick={() => void commit()}>
            <Check size={16} strokeWidth={2.25} />
            Capture
          </button>
        </div>
      ) : null}
      {src ? <div className="ov-hud ov-hint">{hint(layer, tool)}</div> : null}
      {out ? (
        <div className="ov-hud ov-out" onPointerDown={(e) => e.stopPropagation()}>
          <img src={out.url} alt="Captured result" />
          <p>
            Preview capture {out.w} × {out.h} px · {out.labels.map((l) => `${l.n}. ${l.text}`).join(" · ") || "no labels"}
          </p>
        </div>
      ) : null}
    </div>
  );
}
