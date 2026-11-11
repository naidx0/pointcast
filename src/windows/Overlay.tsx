import { useEffect, useLayoutEffect, useReducer, useRef, useState, type PointerEvent as PE } from "react";
import {
  Check,
  EyeOff,
  Highlighter,
  ListOrdered,
  MapPin,
  MoveUpRight,
  Pencil,
  Redo2,
  Square,
  Type,
  Undo2,
  X,
  ZoomIn,
  type LucideIcon,
} from "lucide-react";
import {
  inTauri,
  onOverlayFinish,
  onOverlayFrame,
  regionCancel,
  regionCommit,
  regionFrame,
  sessionToggle,
  type ElementMark,
  type Label,
} from "@/lib/api";
import { REDACT_EDGE_CSS, REDACT_FILL_CSS, edgeWidth, hiddenNote, overlaps, redact } from "@/lib/redact";
import { captionMarks, empty, hist, kept, labelsFor, numbers, view, type A, type H } from "@/lib/marks";
import "../styles/overlay.css";

type Layer = "selecting" | "annotating" | "pinning" | "drawing";
type Tool = "move" | "pin" | "step" | "arrow" | "rect" | "redact" | "highlight" | "pen" | "text" | "magnify";
type Pt = { x: number; y: number };
type Rect = { x: number; y: number; w: number; h: number };
type Handle = "nw" | "n" | "ne" | "e" | "se" | "s" | "sw" | "w";

// Every shape lives in frame-pixel space (the physical pixels of the frozen PNG).
type Shape =
  | { id: number; kind: "pin" | "text"; c: string; at: Pt; text: string }
  // A numbered step: counts up by itself, asks for no words (a label can be added by double-clicking it). `to`: where a
  // short leader from the number points, when it was dragged out.
  | { id: number; kind: "step"; c: string; at: Pt; text: string; to?: Pt }
  // A redaction ("redact") is a box drawn solid over everything else; the pixels under it are written over when saved.
  | { id: number; kind: "arrow" | "rect" | "redact"; c: string; a: Pt; b: Pt }
  | { id: number; kind: "pen" | "highlight"; c: string; pts: Pt[] }
  | { id: number; kind: "magnify"; at: Pt }
  // Where the person pointed when the frame froze; placed for them, theirs to move or remove.
  | { id: number; kind: "ring"; c: string; at: Pt }
  // The control under that spot, outlined for them; theirs to remove.
  | { id: number; kind: "element"; c: string; r: Rect; role: string; name: string };

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
  // The box resized or moved; `d` is where it is now, a crop on release.
  | { k: "size"; h: Handle; o: Rect; d?: Rect }
  | { k: "move"; a: Pt; o: Rect; d?: Rect }
  | { k: "draw"; shape: Shape }
  | { k: "drop"; a: Pt; tool: "pin" | "step" | "text" | "magnify"; to?: Pt }
  | { k: "shift"; id: number; a: Pt; o: Pt; at: Pt; kind: Shape["kind"] }
  // A rectangle or a hidden area: moved by its body, resized by a handle.
  | { k: "grab"; o: Box2; a: Pt; cx: number; cy: number; moved: boolean; d?: Box2 }
  | { k: "reshape"; o: Box2; h: Handle; d?: Box2 };

type Box2 = Extract<Shape, { a: Pt }> & { kind: "rect" | "redact" };
const isBox = (x: Shape | undefined): x is Box2 => x?.kind === "rect" || x?.kind === "redact";

// `edit`: a picture from History, its labels already drawn in; new pins are numbered on from `base`.
type Frame = { png_b64: string; width: number; height: number; pointer?: Pt | null; element?: ElementMark | null; edit?: boolean; base?: number };

type Out = { url: string; w: number; h: number; labels: Label[]; element: ElementMark | null; hidden: number };

const PREVIEW = typeof location !== "undefined" && location.search.includes("preview=1");
const PREVIEW_EDIT = PREVIEW && location.search.includes("edit=1");
const COLORS = ["#FF3B30", "#FF9500", "#FFCC00", "#34C759", "#007AFF"];
const COLOR_NAMES = ["Red", "Orange", "Yellow", "Green", "Blue"];
const HL = "#FFCC00";
const FONT = '-apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif';
// Sizes in logical (on-screen) px. Everything drawn into the frame is multiplied by s = frame px per logical px.
const M = { line: 4, rect: 3, pen: 3.5, hl: 18, pin: 11, step: 12, lead: 2.5, leadHead: 11, font: 13, padX: 8, pillH: 26, gap: 6, lens: 60, ring: 3, head: 16, mark: 16, markW: 3, grip: 16, out: 2, outGap: 3 };
const HANDLES: Handle[] = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
const HX = { nw: 0, n: 0.5, ne: 1, e: 1, se: 1, s: 0.5, sw: 0, w: 0 };
const HY = { nw: 0, n: 0, ne: 0, e: 0.5, se: 1, s: 1, sw: 1, w: 0.5 };
// Moving the box needs no tool: its border always moves it, its handles resize it, and Space or the middle button moves
// it from inside. V still picks a move mode for those who used it, with no button of its own.
const TOOLS: { id: Tool; Icon: LucideIcon; label: string; key: string }[] = [
  { id: "pin", Icon: MapPin, label: "Pin", key: "P" },
  { id: "step", Icon: ListOrdered, label: "Step", key: "S" },
  { id: "arrow", Icon: MoveUpRight, label: "Arrow", key: "A" },
  { id: "rect", Icon: Square, label: "Rectangle", key: "R" },
  { id: "redact", Icon: EyeOff, label: "Redact", key: "X" },
  { id: "highlight", Icon: Highlighter, label: "Highlighter", key: "H" },
  { id: "pen", Icon: Pencil, label: "Pen", key: "D" },
  { id: "text", Icon: Type, label: "Text", key: "T" },
  { id: "magnify", Icon: ZoomIn, label: "Magnify 2x", key: "M" },
];
// Tools that draw where they are dragged; pressing with one starts a stroke, not a grab.
const DRAWS = new Set<Tool>(["arrow", "rect", "redact", "highlight", "pen"]);
const KEYS = { ...Object.fromEntries(TOOLS.map((t) => [t.key.toLowerCase(), t.id])), v: "move" } as Record<string, Tool>;
const SIDES = ["n", "e", "s", "w"] as const;

function esc(layer: Layer): Layer | "idle" {
  if (layer === "drawing" || layer === "pinning") return "annotating";
  if (layer === "annotating") return "selecting";
  return "idle";
}

const GESTURE: Record<Tool, string> = {
  move: "Drag inside the box to move it · Click a mark to pick it",
  pin: "Click to drop a numbered pin",
  step: "Click to drop the next step · Drag for a leader",
  arrow: "Drag an arrow",
  rect: "Drag a rectangle",
  redact: "Drag over what to hide",
  highlight: "Drag to highlight",
  pen: "Draw freehand",
  text: "Click to place a label",
  magnify: "Click to drop a 2x loupe",
};

const CROP_HINT = "Drag the edges to crop · Marks stay put · C or Esc done";

function hint(layer: Layer, tool: Tool, picked?: Shape["kind"], edit = false, crop = false): string {
  if (crop && layer !== "selecting") return CROP_HINT;
  if (layer === "selecting") return "Drag to select · Click for full screen · Esc cancel";
  if (picked === "step" && layer === "annotating") return "Drag to move · Double-click to label · Delete removes it · Enter capture";
  if (edit && layer === "annotating") return "Edit picture · C crop · Return saves · Esc cancels";
  if (picked === "ring" && layer === "annotating") return "Drag the ring to move it · Delete removes it · Enter capture";
  if (picked === "element" && layer === "annotating") return "Delete removes the outline · Enter capture";
  if ((picked === "redact" || picked === "rect") && layer === "annotating") return "Drag to move · Drag a handle to resize · Delete removes it · Enter capture";
  if (layer === "pinning") return "Type a label · Enter or Esc to finish";
  if (layer === "drawing") return "Release to finish · Esc discard";
  return `${GESTURE[tool]} · C crop · Enter capture · Esc back`;
}

const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), hi);
const ink = (c: string) => (c === HL ? "#1c1c1e" : "#fff");
const inside = (p: Pt, b: Rect) => p.x >= b.x && p.y >= b.y && p.x <= b.x + b.w && p.y <= b.y + b.h;
const norm = (a: Pt, b: Pt): Rect => ({ x: Math.min(a.x, b.x), y: Math.min(a.y, b.y), w: Math.abs(a.x - b.x), h: Math.abs(a.y - b.y) });

// The outlined control in the captured picture's pixels, clipped to it; null when none of it is in the box.
function within(r: Rect, b: Rect): ElementMark["rect"] | null {
  const [x0, y0, x1, y1] = [Math.round(b.x), Math.round(b.y), Math.round(b.x + b.w), Math.round(b.y + b.h)];
  const l = clamp(Math.round(r.x), x0, x1);
  const t = clamp(Math.round(r.y), y0, y1);
  const rt = clamp(Math.round(r.x + r.w), x0, x1);
  const bt = clamp(Math.round(r.y + r.h), y0, y1);
  return rt > l && bt > t ? { x: l - x0, y: t - y0, w: rt - l, h: bt - t } : null;
}

// Around the control, a little outside it so its own edge stays visible. s = frame px per logical px.
const ringed = (r: Rect, s: number): Rect => {
  const g = M.outGap * s;
  return { x: r.x - g, y: r.y - g, w: r.w + 2 * g, h: r.h + 2 * g };
};
const svgRect = (r: Rect) => ({ x: r.x, y: r.y, width: r.w, height: r.h });

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

const pillW = (str: string, s: number) => textW(str, M.font * s) + 2 * M.padX * s;

function pill(x: number, y: number, str: string, s: number, fill: string, fg: string, edge: string): Op[] {
  const h = M.pillH * s;
  return [
    { k: "box", x, y: y - h / 2, w: pillW(str, s), h, r: 7 * s, fill, stroke: edge, sw: 1.5 * s, shadow: true },
    { k: "txt", x: x + M.padX * s, y, str, px: M.font * s, wt: 600, fill: fg },
  ];
}

// Left edge of a label of width w (frame px): right of the pin, flipped left or pulled in so it stays inside the box.
function labelX(sh: Extract<Shape, { text: string }>, w: number, s: number, b: Rect) {
  const flush = Math.max(b.x, b.x + b.w - w);
  if (sh.kind === "text") return clamp(sh.at.x, b.x, flush);
  const off = ((sh.kind === "step" ? M.step : M.pin) + M.gap) * s;
  const right = sh.at.x + off;
  const left = sh.at.x - off - w;
  if (right + w <= b.x + b.w) return right;
  if (left >= b.x) return left;
  return b.x + b.w - right >= sh.at.x - off - b.x ? right : left;
}

// The one place a shape becomes pixels. s = frame px per logical px (natural.w / displayed width); b = the box.
function ops(sh: Shape, s: number, n: number, bare: boolean, b: Rect): Op[] {
  switch (sh.kind) {
    case "pin": {
      const { x, y } = sh.at;
      const r = M.pin * s;
      const o: Op[] = [
        { k: "ell", x, y, r, fill: sh.c, stroke: "#fff", w: 2 * s, shadow: true },
        { k: "txt", x, y, str: String(n), px: 12 * s, wt: 700, fill: ink(sh.c), mid: true },
      ];
      if (sh.text && !bare) o.push(...pill(labelX(sh, pillW(sh.text, s), s, b), y, sh.text, s, "rgba(255,255,255,.96)", "#1c1c1e", sh.c));
      return o;
    }
    case "text":
      return sh.text && !bare ? pill(labelX(sh, pillW(sh.text, s), s, b), sh.at.y, sh.text, s, sh.c, ink(sh.c), "#fff") : [];
    case "step": {
      const { x, y } = sh.at;
      const r = M.step * s;
      const o: Op[] = [];
      // The leader starts at the circle's edge and points where it was dragged to.
      if (sh.to) {
        const len = Math.hypot(sh.to.x - x, sh.to.y - y);
        if (len > r + 4 * s) {
          const a = { x: x + ((sh.to.x - x) / len) * r, y: y + ((sh.to.y - y) / len) * r };
          o.push(...arrow(a, sh.to, "rgba(0,0,0,.45)", (M.lead + 2) * s, (M.leadHead + 1.5) * s, s), ...arrow(a, sh.to, sh.c, M.lead * s, M.leadHead * s, s));
        }
      }
      // A dark rim outside the white ring keeps the circle apart from light and dark screens alike.
      o.push(
        { k: "ell", x, y, r: r + 2.25 * s, fill: "rgba(0,0,0,.55)" },
        { k: "ell", x, y, r, fill: sh.c, stroke: "#fff", w: 2.5 * s, shadow: true },
        { k: "txt", x, y, str: String(n), px: (n >= 10 ? 11.5 : 13.5) * s, wt: 700, fill: ink(sh.c), mid: true },
      );
      if (sh.text && !bare) o.push(...pill(labelX(sh, pillW(sh.text, s), s, b), y, sh.text, s, "rgba(255,255,255,.96)", "#1c1c1e", sh.c));
      return o;
    }
    case "arrow":
      return arrow(sh.a, sh.b, sh.c, M.line * s, M.head * s, s);
    case "rect":
      return [{ k: "box", ...norm(sh.a, sh.b), r: 4 * s, stroke: sh.c, sw: M.rect * s }];
    case "redact": {
      const r = norm(sh.a, sh.b);
      const e = edgeWidth(s);
      return [
        { k: "box", ...r, r: 0, fill: REDACT_FILL_CSS },
        { k: "box", x: r.x + e / 2, y: r.y + e / 2, w: Math.max(0, r.w - e), h: Math.max(0, r.h - e), r: 0, stroke: REDACT_EDGE_CSS, sw: e },
      ];
    }
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
    case "ring": {
      const { x, y } = sh.at;
      const r = M.mark * s;
      return [
        { k: "ell", x, y, r, stroke: "rgba(0,0,0,.5)", w: (M.markW + 2.5) * s },
        { k: "ell", x, y, r, stroke: sh.c, w: M.markW * s },
      ];
    }
    case "element": {
      const o = ringed(sh.r, s);
      return [
        { k: "box", ...o, r: 4 * s, stroke: "rgba(0,0,0,.5)", sw: (M.out + 2) * s },
        { k: "box", ...o, r: 4 * s, stroke: sh.c, sw: M.out * s },
      ];
    }
  }
}

// A line from a to b with a filled head at b: w the line's width, head the head's length, both in frame px.
function arrow(a: Pt, b: Pt, c: string, w: number, head: number, s: number): Op[] {
  const len = Math.hypot(b.x - a.x, b.y - a.y) || 1;
  const ux = (b.x - a.x) / len;
  const uy = (b.y - a.y) / len;
  const hl = Math.min(head, len * 0.7);
  const hw = hl * 0.5;
  const bx = b.x - ux * hl;
  const by = b.y - uy * hl;
  return [
    { k: "path", d: [["M", a.x, a.y], ["L", bx + ux * hl * 0.3, by + uy * hl * 0.3]], stroke: c, w },
    { k: "path", d: [["M", b.x, b.y], ["L", bx - uy * hw, by + ux * hw], ["L", bx + uy * hw, by - ux * hw], ["Z"]], fill: c, stroke: c, w: 2 * s },
  ];
}

// Drawing order: marks, then pins and steps with a label (so a label is never under the next step's circle), then
// hidden areas over everything.
const tier = (sh: Shape) => (sh.kind === "redact" ? 2 : (sh.kind === "pin" || sh.kind === "step") && sh.text ? 1 : 0);

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

function paint(ctx: CanvasRenderingContext2D, img: CanvasImageSource, o: Op, s: number) {
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

// Hidden areas last, written into the pixels; the loupes read a frame they are already written into, so none can
// show what is under one.
async function rasterize(src: string, b: Rect, list: Shape[], s: number, nums: Map<number, number>): Promise<string> {
  const raw = await loadImg(src);
  const hide = list.flatMap((x) => (x.kind === "redact" ? [norm(x.a, x.b)] : []));
  let img: CanvasImageSource = raw;
  if (hide.length) {
    const f = document.createElement("canvas");
    f.width = raw.naturalWidth;
    f.height = raw.naturalHeight;
    const fx = f.getContext("2d")!;
    fx.drawImage(raw, 0, 0);
    const d = fx.getImageData(0, 0, f.width, f.height);
    redact(d.data, f.width, f.height, hide, s);
    fx.putImageData(d, 0, 0);
    img = f;
  }
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
  for (const sh of [...list].sort((p, q) => tier(p) - tier(q))) if (sh.kind !== "redact") for (const o of ops(sh, s, nums.get(sh.id) ?? 0, false, b)) paint(ctx, img, o, s);
  if (hide.length) {
    const d = ctx.getImageData(0, 0, w, h);
    redact(d.data, w, h, hide.map((r) => ({ ...r, x: r.x - x0, y: r.y - y0 })), s);
    ctx.putImageData(d, 0, 0);
  }
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

function EditInput(p: { now: boolean; x: number; y: number; w: number; text: string; hold: string; bg: string; fg: string; edge: string; onText: (t: string) => void; onDone: (empty: boolean) => void }) {
  const ref = useRef<HTMLInputElement>(null);
  // Placed on pointerup: focus at once. Reopened on pointerdown: wait a frame, the press would blur it right back.
  useLayoutEffect(() => {
    const go = () => {
      const el = ref.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(el.value.length, el.value.length);
    };
    if (p.now) go();
    else {
      const f = requestAnimationFrame(go);
      return () => cancelAnimationFrame(f);
    }
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
  const [picked, setPicked] = useState<number | null>(null);
  const [nudge, setNudge] = useState<{ id: number; at: Pt } | null>(null);
  // A rectangle or hidden area being moved or resized, shown in place of the stored one until release.
  const [drag, setDrag] = useState<Box2 | null>(null);
  const [pal, setPal] = useState(false);
  const [space, setSpace] = useState(false);
  const [gone, setGone] = useState(false);
  // Cropping (key C): the box's edges resize it instead of moving it, and marks cannot be picked meanwhile.
  const [crop, setCrop] = useState(false);
  // The last press on a step that did not move it, to tell a double-click (edit its label) from a pick.
  const tap = useRef<{ id: number; t: number } | null>(null);
  const spaceRef = useRef(false);
  const [edit, setEdit] = useState(false);
  const [base, setBase] = useState(0);
  const [hs, dispatch] = useReducer((h: H<Shape>, a: A<Shape>) => hist(h, a), empty<Shape>());
  const ix = useRef<Ix | null>(null);
  const ids = useRef(0);
  const busy = useRef<Promise<void> | null>(null);
  const editRef = useRef<number | null>(null);
  const editNow = useRef(true);
  const keyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  const finishRef = useRef<() => void>(() => {});
  const colorRef = useRef(color);
  colorRef.current = color;
  // The outline and the ring placed where the person pointed; starting over brings them back.
  const seed = useRef<Shape[]>([]);
  const restart = (base: Shape[] = []) => dispatch(seed.current.length ? { t: "seed", base, s: seed.current } : { t: "reset", now: base });

  useEffect(() => {
    const load = (f: Frame) => {
      setSrc(`data:image/png;base64,${f.png_b64}`);
      setNatural({ w: f.width, h: f.height });
      // A picture from History is the whole box from the start: there is nothing around it to select.
      setEdit(!!f.edit);
      setBase(f.edit ? (f.base ?? 0) : 0);
      setLayer(f.edit ? "annotating" : "selecting");
      setBox(f.edit ? { x: 0, y: 0, w: f.width, h: f.height } : null);
      setLive(null);
      setEditing(null);
      setOut(null);
      setPicked(null);
      setNudge(null);
      setDrag(null);
      editRef.current = null;
      ix.current = null;
      setCrop(false);
      seed.current = f.edit ? [] : placed(f.element, f.pointer, colorRef.current);
      restart();
      if (f.edit) dispatch({ t: "frame", r: { x: 0, y: 0, w: f.width, h: f.height } });
    };
    let u: Promise<(() => void) | null> = Promise.resolve(null);
    let f: Promise<(() => void) | null> = Promise.resolve(null);
    try {
      u = onOverlayFrame(load).catch(() => null);
      f = onOverlayFinish(() => finishRef.current()).catch(() => null);
      // Built on demand, this page may have missed the event: ask for the frame it should show.
      if (inTauri) regionFrame().then((f) => f && load(f)).catch(() => undefined);
    } catch {
      /* plain browser, no Tauri */
    }
    if (PREVIEW) {
      const img = new Image();
      img.onload = () => {
        setSrc("/freeze.png");
        setNatural({ w: img.naturalWidth || 1280, h: img.naturalHeight || 800 });
        setLayer("annotating");
        if (PREVIEW_EDIT) {
          setEdit(true);
          setBase(1);
          const r = { x: 0, y: 0, w: img.naturalWidth || 1280, h: img.naturalHeight || 800 };
          setBox(r);
          seed.current = [];
          restart();
          dispatch({ t: "frame", r });
          return;
        }
        const r = { x: 975, y: 240, w: 380, h: 310 };
        setBox(r);
        seed.current = placed({ role: "button", name: "Place order", rect: { x: 987, y: 435, w: 116, h: 41 } }, { x: 1060, y: 456 }, COLORS[0]);
        restart([{ id: ++ids.current, kind: "pin", c: COLORS[0], at: { x: 1128, y: 456 }, text: "28px too far left" }]);
        dispatch({ t: "frame", r });
      };
      img.src = "/freeze.png";
    }
    const onResize = () => setVp({ w: window.innerWidth, h: window.innerHeight });
    window.addEventListener("resize", onResize);
    const onKey = (e: KeyboardEvent) => keyRef.current(e);
    window.addEventListener("keydown", onKey);
    const spaceUp = (e: KeyboardEvent | FocusEvent) => {
      if (e.type === "blur" || (e as KeyboardEvent).key === " ") {
        spaceRef.current = false;
        setSpace(false);
      }
    };
    window.addEventListener("keyup", spaceUp);
    window.addEventListener("blur", spaceUp);
    return () => {
      window.removeEventListener("resize", onResize);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("keyup", spaceUp);
      window.removeEventListener("blur", spaceUp);
      void u.then((fn) => fn?.());
      void f.then((fn) => fn?.());
    };
  }, []);

  // The box is part of the undo history once drawn: undoing a crop brings back the box and the marks it dropped.
  useEffect(() => {
    if (hs.cut) setBox(hs.cut);
  }, [hs.cut]);

  useLayoutEffect(() => {
    const el = barRef.current;
    if (el && (el.offsetWidth !== tb.w || el.offsetHeight !== tb.h)) setTb({ w: el.offsetWidth, h: el.offsetHeight });
  });

  function placed(el: ElementMark | null | undefined, at: Pt | null | undefined, c: string): Shape[] {
    const out: Shape[] = [];
    if (el) out.push({ id: ++ids.current, kind: "element", c, r: el.rect, role: el.role, name: el.name });
    if (at) out.push({ id: ++ids.current, kind: "ring", c, at });
    return out;
  }

  // Frame px <-> logical px. s = natural.w / displayed image width: every size drawn into the frame is multiplied by it.
  // A picture from History sits clear of the toolbar and hint, and never larger than its own pixels.
  // It depends on the picture and the screen, never on the box: a crop leaves every mark where it was on screen.
  const { fit, ox, oy, dw, dh } = view(natural, vp, edit, window.devicePixelRatio);
  const s = 1 / fit;
  const toImg = (e: { clientX: number; clientY: number }): Pt => ({ x: (e.clientX - ox) / fit, y: (e.clientY - oy) / fit });
  const clampP = (p: Pt): Pt => ({ x: clamp(p.x, 0, natural.w), y: clamp(p.y, 0, natural.h) });
  const toScr = (r: Rect): Rect => ({ x: ox + r.x * fit, y: oy + r.y * fit, w: r.w * fit, h: r.h * fit });
  const full: Rect = { x: 0, y: 0, w: natural.w, h: natural.h };

  const shapes = hs.now.map((x) => (nudge && x.id === nudge.id && "at" in x ? { ...x, at: nudge.at } : drag && x.id === drag.id ? drag : x));
  const vis = box ? kept(shapes, box) : shapes;
  const nums = numbers(vis, base);
  const sb = box ? toScr(box) : null;
  const grab = (e: PE) => {
    try {
      rootRef.current?.setPointerCapture(e.pointerId);
    } catch {
      /* pointer already gone */
    }
  };

  function startEdit(id: number, now: boolean) {
    editRef.current = id;
    editNow.current = now;
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

  // Leave without capturing: what Esc does from the selecting layer, and the toolbar's Cancel.
  function leave() {
    setPal(false);
    if (PREVIEW) setGone(true);
    try {
      void regionCancel().catch(() => {});
    } catch {
      /* plain browser, no Tauri */
    }
  }

  function back() {
    const next = edit && layer === "annotating" ? "idle" : esc(layer);
    if (next === "idle") {
      leave();
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
      setOut(null);
      setPicked(null);
      restart();
    }
    setLayer(next);
  }

  // A second call while one is running waits for that one, so a stop never overtakes the region.
  function commit(): Promise<void> {
    if (!src || !natural.w) return Promise.resolve();
    busy.current ??= send(src).finally(() => {
      busy.current = null;
    });
    return busy.current;
  }

  async function send(src: string) {
    try {
      const b = layer === "selecting" || !box ? full : box;
      // Marks wholly outside the box are left out of the picture and the caption; the box edge clips the rest.
      const list = kept(shapes, b).filter((x) => !(x.kind === "text" && !x.text.trim()));
      const png = await rasterize(src, b, list, s, numbers(list, base));
      const labels: Label[] = labelsFor(list, base);
      const hidden = list.flatMap((x) => (x.kind === "redact" ? [norm(x.a, x.b)] : [])).filter((r) => r.w > 0 && r.h > 0 && overlaps(r, b));
      const el = list.find((x) => x.kind === "element");
      const rect = el?.kind === "element" ? within(el.r, b) : null;
      // A control under a hidden area keeps its name to itself: its words could be what was hidden.
      const element = el?.kind === "element" && rect && !hidden.some((r) => overlaps(r, el.r)) ? { role: el.role, name: el.name, rect } : null;
      if (PREVIEW) setOut({ url: `data:image/png;base64,${png}`, w: Math.round(b.w), h: Math.round(b.h), labels, element, hidden: hidden.length });
      else await regionCommit(png, labels, element, hidden.length);
    } catch (err) {
      console.error("pointcast overlay commit failed", err);
    }
  }

  finishRef.current = () => void commit().then(() => sessionToggle().catch(() => undefined));

  keyRef.current = (e: KeyboardEvent) => {
    if (e.isComposing) return;
    const t = e.target as HTMLElement | null;
    const typing = !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA");
    if (e.key === "Escape") {
      if (typing) t.blur();
      else if (pal) setPal(false);
      else if (crop) setCrop(false);
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
    // Space held: drag inside the box to move it. Kept from pressing a focused toolbar button.
    if (e.key === " ") {
      e.preventDefault();
      if (!spaceRef.current) {
        spaceRef.current = true;
        setSpace(true);
      }
      return;
    }
    const n = "12345".indexOf(e.key);
    if (!mod && !e.altKey && n >= 0 && e.key.length === 1) {
      setColor(COLORS[n]);
      return;
    }
    if (mod && (k === "z" || k === "y")) {
      e.preventDefault();
      dispatch({ t: k === "y" || e.shiftKey ? "redo" : "undo" });
    } else if ((e.key === "Delete" || e.key === "Backspace") && picked != null) {
      e.preventDefault();
      dispatch({ t: "remove", id: picked });
      setPicked(null);
    } else if (!mod && !e.altKey && k === "c" && layer !== "selecting") {
      setPicked(null);
      setCrop((c) => !c);
    } else if (!mod && !e.altKey && KEYS[k]) pickTool(KEYS[k]);
  };

  function pickTool(t: Tool) {
    setTool(t);
    setCrop(false);
  }

  function down(e: PE<HTMLDivElement>) {
    setPal(false);
    const middle = e.button === 1;
    if ((e.button !== 0 && !middle) || !src || !natural.w) return;
    setPicked(null);
    const p = toImg(e);
    if (layer === "selecting") {
      ix.current = { k: "sel", a: clampP(p), cx: e.clientX, cy: e.clientY, moved: false, r: null };
      grab(e);
      return;
    }
    if (!box) return;
    if (tool === "move" || middle || spaceRef.current || crop) {
      if (inside(p, box)) {
        ix.current = { k: "move", a: p, o: box };
        setCur({ x: e.clientX, y: e.clientY });
        grab(e);
      }
      return;
    }
    if (!inside(p, box)) return;
    if (tool === "pin" || tool === "step" || tool === "text" || tool === "magnify") {
      ix.current = { k: "drop", a: p, tool };
      grab(e);
      return;
    }
    const id = ++ids.current;
    const shape: Shape =
      tool === "arrow" || tool === "rect" || tool === "redact" ? { id, kind: tool, c: color, a: p, b: p } : { id, kind: tool, c: tool === "highlight" ? HL : color, pts: [p] };
    ix.current = { k: "draw", shape };
    setLive(shape);
    setLayer("drawing");
    grab(e);
  }

  function startShift(e: PE<SVGCircleElement>, sh: Shape & { at: Pt }) {
    e.stopPropagation();
    if (e.button !== 0) return;
    ix.current = { k: "shift", id: sh.id, a: toImg(e), o: sh.at, at: sh.at, kind: sh.kind };
    setPicked(sh.id);
    grab(e);
  }

  // A hidden area is grabbed whatever the tool (it sits on top of everything); a rectangle's edge only when the tool
  // would not draw there.
  function startGrab(e: PE<SVGRectElement>, sh: Box2) {
    if (sh.kind === "rect" && DRAWS.has(tool)) return;
    e.stopPropagation();
    if (e.button !== 0) return;
    ix.current = { k: "grab", o: sh, a: toImg(e), cx: e.clientX, cy: e.clientY, moved: false };
    setPicked(sh.id);
    grab(e);
  }

  function startReshape(e: PE<HTMLDivElement>, sh: Box2, h: Handle) {
    e.stopPropagation();
    if (e.button !== 0) return;
    ix.current = { k: "reshape", o: sh, h };
    grab(e);
  }

  function pick(e: PE<SVGRectElement>, id: number) {
    e.stopPropagation();
    if (e.button === 0) setPicked(id);
  }

  // The box's border moves it, whatever the tool; while cropping it moves that edge.
  function startMove(e: PE<HTMLDivElement>, side: (typeof SIDES)[number]) {
    if (crop) return startSize(e, side);
    e.stopPropagation();
    setPal(false);
    if ((e.button !== 0 && e.button !== 1) || !box) return;
    setPicked(null);
    ix.current = { k: "move", a: toImg(e), o: box };
    setCur({ x: e.clientX, y: e.clientY });
    grab(e);
  }

  function startSize(e: PE<HTMLDivElement>, h: Handle) {
    e.stopPropagation();
    setPal(false);
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
      m.d = resize(m.o, m.h, p, 12 * s);
      setBox(m.d);
      setCur({ x: e.clientX, y: e.clientY });
    } else if (m.k === "move") {
      const q = toImg(e);
      m.d = { ...m.o, x: clamp(m.o.x + q.x - m.a.x, 0, natural.w - m.o.w), y: clamp(m.o.y + q.y - m.a.y, 0, natural.h - m.o.h) };
      setBox(m.d);
      setCur({ x: e.clientX, y: e.clientY });
    } else if (m.k === "drop") {
      // Dragged out from a step: a leader from the number to where it is let go.
      if (m.tool !== "step") return;
      const far = Math.hypot(p.x - m.a.x, p.y - m.a.y) >= (M.step + 10) * s;
      m.to = far ? p : undefined;
      setLive(far ? { id: -1, kind: "step", c: color, at: m.a, text: "", to: p } : null);
    } else if (m.k === "shift") {
      const q = toImg(e);
      const b = box ?? full;
      const pad = Math.min(M.mark * s, b.w / 2, b.h / 2);
      m.at = { x: clamp(m.o.x + q.x - m.a.x, b.x + pad, b.x + b.w - pad), y: clamp(m.o.y + q.y - m.a.y, b.y + pad, b.y + b.h - pad) };
      setNudge({ id: m.id, at: m.at });
    } else if (m.k === "grab") {
      if (!m.moved && Math.hypot(e.clientX - m.cx, e.clientY - m.cy) < 3) return;
      m.moved = true;
      const q = toImg(e);
      const r = norm(m.o.a, m.o.b);
      const b = box ?? full;
      const dx = clamp(q.x - m.a.x, b.x - r.x - r.w + 4 * s, b.x + b.w - r.x - 4 * s);
      const dy = clamp(q.y - m.a.y, b.y - r.y - r.h + 4 * s, b.y + b.h - r.y - 4 * s);
      m.d = { ...m.o, a: { x: m.o.a.x + dx, y: m.o.a.y + dy }, b: { x: m.o.b.x + dx, y: m.o.b.y + dy } };
      setDrag(m.d);
    } else if (m.k === "reshape") {
      const r = resize(norm(m.o.a, m.o.b), m.h, p, 4 * s);
      m.d = { ...m.o, a: { x: r.x, y: r.y }, b: { x: r.x + r.w, y: r.y + r.h } };
      setDrag(m.d);
    } else if (m.k === "draw") {
      const sh = m.shape;
      if (sh.kind === "arrow" || sh.kind === "rect" || sh.kind === "redact") m.shape = { ...sh, b: p };
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
    try {
      rootRef.current?.releasePointerCapture(e.pointerId);
    } catch {
      /* not captured */
    }
    if (!m) return;
    if ((m.k === "size" || m.k === "move") && m.d) {
      dispatch({ t: "crop", r: m.d });
    } else if (m.k === "grab" || m.k === "reshape") {
      const d = m.d;
      setDrag(null);
      if (d && (d.a.x !== m.o.a.x || d.a.y !== m.o.a.y || d.b.x !== m.o.b.x || d.b.y !== m.o.b.y)) dispatch({ t: "put", s: d });
    } else if (m.k === "shift") {
      setNudge(null);
      if (Math.hypot(m.at.x - m.o.x, m.at.y - m.o.y) >= 1) {
        dispatch({ t: "move", id: m.id, at: m.at });
        tap.current = null;
      } else if (m.kind === "step") {
        // A second press on the same step within a moment: give it a label.
        const now = performance.now();
        if (tap.current?.id === m.id && now - tap.current.t < 450) {
          tap.current = null;
          setPicked(null);
          startEdit(m.id, true);
        } else tap.current = { id: m.id, t: now };
      }
    } else if (m.k === "sel") {
      if (!m.moved) {
        setBox(full);
        dispatch({ t: "frame", r: full });
        setLayer("annotating");
      } else if (m.r && m.r.w * fit >= 8 && m.r.h * fit >= 8) {
        dispatch({ t: "frame", r: m.r });
        setLayer("annotating");
      } else setBox(null);
    } else if (m.k === "draw") {
      const sh = m.shape;
      setLive(null);
      setLayer("annotating");
      const long =
        sh.kind === "redact"
          ? Math.min(Math.abs(sh.b.x - sh.a.x), Math.abs(sh.b.y - sh.a.y)) >= 4 * s
          : sh.kind === "arrow" || sh.kind === "rect"
            ? Math.hypot(sh.b.x - sh.a.x, sh.b.y - sh.a.y) >= 6 * s
            : true;
      if (long) dispatch({ t: "add", s: sh });
    } else if (m.k === "drop") {
      const id = ++ids.current;
      if (m.tool === "magnify") dispatch({ t: "add", s: { id, kind: "magnify", at: m.a } });
      else if (m.tool === "step") {
        setLive(null);
        dispatch({ t: "add", s: { id, kind: "step", c: color, at: m.a, text: "", ...(m.to ? { to: m.to } : {}) } });
      } else {
        dispatch({ t: "add", s: { id, kind: m.tool, c: color, at: m.a, text: "" } });
        startEdit(id, true);
      }
    }
  }

  function cancel() {
    if (ix.current?.k === "shift") setNudge(null);
    if (ix.current?.k === "drop") setLive(null);
    setDrag(null);
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
    const floor = vp.h - 62;
    let y = sb.y + sb.h + 12;
    if (y + tb.h > floor) {
      y = sb.y - 12 - tb.h;
      if (y < m) y = floor - tb.h;
    }
    bar = { x: clamp(sb.x + sb.w / 2 - tb.w / 2, m, Math.max(m, vp.w - m - tb.w)), y: clamp(y, m, Math.max(m, vp.h - m - tb.h)) };
  }

  const ed = editing != null ? vis.find((x) => x.id === editing) : undefined;
  const edW = ed && "text" in ed ? Math.max(112, textW(ed.text || " ", M.font) + 2 * M.padX + 10) : 0;
  const readout = cur && box ? `${Math.round(box.w)} × ${Math.round(box.h)} px` : "";
  const rw = readout.length * 7 + 20;
  const cursor = layer === "selecting" ? "crosshair" : tool === "move" || space || crop || ix.current?.k === "move" ? "move" : tool === "text" ? "text" : "crosshair";
  // The colour pop-over opens away from the box, or toward it when there is no room above the hint pill.
  const popUp = sb ? (bar.y < sb.y ? bar.y > 64 : bar.y + tb.h + 56 > vp.h - 62) : false;
  const dim = `M0 0H${vp.w}V${vp.h}H0Z${sb ? `M${sb.x} ${sb.y}h${sb.w}v${sb.h}h${-sb.w}Z` : ""}`;
  const canEdit = tool === "pin" || tool === "text";
  const pickedShape = vis.find((x) => x.id === picked);
  const pickedBox = isBox(pickedShape) ? pickedShape : null;

  return (
    <div
      ref={rootRef}
      className={`ov-root${edit ? " ov-edit" : ""}${crop ? " ov-crop" : ""}`}
      style={{ cursor }}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={cancel}
      onContextMenu={(e) => e.preventDefault()}
      // The middle button moves the box; keep the browser's autoscroll out of it.
      onMouseDown={(e) => e.button === 1 && e.preventDefault()}
    >
      {gone ? null : src && natural.w ? (
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
              {[...vis, ...(live ? [live] : [])].sort((p, q) => tier(p) - tier(q)).map((sh) => (
                <Ops key={sh.id} id={sh.id} src={src} nat={natural} list={ops(sh, s, nums.get(sh.id) ?? (sh === live ? base + nums.size + 1 : 0), sh.id === editing, box ?? full)} />
              ))}
            </g>
          </svg>
          {sb ? <div className="ov-sel" style={{ left: sb.x, top: sb.y, width: sb.w, height: sb.h }} /> : null}
          {layer === "annotating" && !crop ? (
            <svg className="ov-grips" style={{ left: ox, top: oy, width: dw, height: dh }} viewBox={`0 0 ${natural.w} ${natural.h}`}>
              {[...vis].sort((p, q) => +(p.kind === "redact") - +(q.kind === "redact")).map((sh) =>
                sh.kind === "element" ? (
                  <g key={sh.id}>
                    {picked === sh.id ? (
                      <rect
                        {...svgRect(ringed(sh.r, s * 2.5))}
                        rx={6 * s}
                        fill="none"
                        stroke="#fff"
                        strokeWidth={1.5 * s}
                        strokeDasharray={`${4 * s} ${3 * s}`}
                      />
                    ) : null}
                    <rect
                      className="ov-grip ov-pick"
                      aria-label={`Outline of ${sh.role} ${sh.name}`.trim()}
                      {...svgRect(ringed(sh.r, s))}
                      rx={4 * s}
                      fill="none"
                      stroke="transparent"
                      strokeWidth={M.grip * s}
                      onPointerDown={(e) => pick(e, sh.id)}
                    />
                  </g>
                ) : isBox(sh) ? (
                  <g key={sh.id}>
                    {picked === sh.id ? (
                      <rect {...svgRect(norm(sh.a, sh.b))} fill="none" stroke="#fff" strokeWidth={1.5 * s} strokeDasharray={`${4 * s} ${3 * s}`} />
                    ) : null}
                    <rect
                      className={sh.kind === "redact" ? "ov-grip ov-fill" : DRAWS.has(tool) ? "ov-grip ov-off" : "ov-grip"}
                      aria-label={sh.kind === "redact" ? "Hidden area" : "Rectangle"}
                      {...svgRect(norm(sh.a, sh.b))}
                      fill="transparent"
                      stroke="transparent"
                      strokeWidth={M.grip * s}
                      onPointerDown={(e) => startGrab(e, sh)}
                    />
                  </g>
                ) : sh.kind === "step" ? (
                  <g key={sh.id}>
                    {picked === sh.id ? (
                      <circle cx={sh.at.x} cy={sh.at.y} r={(M.step + 6) * s} fill="none" stroke="#fff" strokeWidth={1.5 * s} strokeDasharray={`${4 * s} ${3 * s}`} />
                    ) : null}
                    <circle
                      className="ov-grip ov-fill"
                      aria-label={`Step ${nums.get(sh.id) ?? ""}`}
                      cx={sh.at.x}
                      cy={sh.at.y}
                      r={(M.step + 2) * s}
                      fill="transparent"
                      onPointerDown={(e) => startShift(e, sh)}
                    />
                  </g>
                ) : sh.kind === "ring" ? (
                  <g key={sh.id}>
                    {picked === sh.id ? (
                      <circle cx={sh.at.x} cy={sh.at.y} r={(M.mark + M.markW + 4) * s} fill="none" stroke="#fff" strokeWidth={1.5 * s} strokeDasharray={`${4 * s} ${3 * s}`} />
                    ) : null}
                    <circle
                      className="ov-grip"
                      aria-label="Where you pointed"
                      cx={sh.at.x}
                      cy={sh.at.y}
                      r={M.mark * s}
                      fill="none"
                      stroke="transparent"
                      strokeWidth={M.grip * s}
                      onPointerDown={(e) => startShift(e, sh)}
                    />
                  </g>
                ) : null,
              )}
            </svg>
          ) : null}
          {sb && layer !== "selecting" && layer !== "drawing"
            ? SIDES.map((d) => (
                <div
                  key={d}
                  className={`ov-edge ov-edge-${d}`}
                  style={d === "n" || d === "s" ? { left: sb.x, top: d === "n" ? sb.y : sb.y + sb.h, width: sb.w } : { left: d === "w" ? sb.x : sb.x + sb.w, top: sb.y, height: sb.h }}
                  onPointerDown={(e) => startMove(e, d)}
                />
              ))
            : null}
          {sb && layer !== "selecting"
            ? HANDLES.map((h) => (
                <div key={h} className={`ov-h ov-h-${h}`} title="Drag to crop" style={{ left: sb.x + sb.w * HX[h], top: sb.y + sb.h * HY[h] }} onPointerDown={(e) => startSize(e, h)} />
              ))
            : null}
          {pickedBox && layer === "annotating"
            ? HANDLES.map((h) => {
                const r = toScr(norm(pickedBox.a, pickedBox.b));
                return (
                  <div
                    key={`s-${h}`}
                    className={`ov-h ov-h-sm ov-h-${h}`}
                    style={{ left: r.x + r.w * HX[h], top: r.y + r.h * HY[h] }}
                    onPointerDown={(e) => startReshape(e, pickedBox, h)}
                  />
                );
              })
            : null}
          {layer !== "drawing" && !crop
            ? vis.map((sh) => {
                if (sh.kind !== "pin" && sh.kind !== "text" && sh.kind !== "step") return null;
                // A step's circle is its grip (drag, double-click); its label, once it has one, opens the editor.
                if (sh.kind !== "step" && !canEdit) return null;
                const pw = sh.text ? pillW(sh.text, s) : 0;
                if (sh.kind !== "pin" && !pw) return null;
                const lx = pw ? labelX(sh, pw, s, box ?? full) : sh.at.x;
                const l0 = sh.kind === "pin" ? Math.min(lx, sh.at.x - M.pin * s) : lx;
                const l1 = sh.kind === "pin" ? Math.max(pw ? lx + pw : 0, sh.at.x + M.pin * s) : lx + pw;
                return (
                  <div
                    key={sh.id}
                    className="ov-hit"
                    style={{ left: ox + l0 * fit, top: oy + sh.at.y * fit - M.pillH / 2, width: (l1 - l0) * fit, height: M.pillH }}
                    onPointerDown={(e) => {
                      e.stopPropagation();
                      startEdit(sh.id, false);
                    }}
                  />
                );
              })
            : null}
          {ed && (ed.kind === "pin" || ed.kind === "step" || ed.kind === "text") ? (
            <EditInput
              key={ed.id}
              now={editNow.current}
              x={ox + labelX(ed, edW * s, s, box ?? full) * fit}
              y={oy + ed.at.y * fit - M.pillH / 2}
              w={edW}
              text={ed.text}
              hold={ed.kind === "text" ? "Type a label…" : "Label…"}
              bg={ed.kind === "text" ? ed.c : "#fff"}
              fg={ed.kind === "text" ? ink(ed.c) : "#1c1c1e"}
              edge={ed.kind === "text" ? "#fff" : ed.c}
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
      {sb && layer !== "selecting" && !gone ? (
        <div
          ref={barRef}
          className="ov-hud ov-bar"
          style={{ left: bar.x, top: bar.y }}
          onPointerDown={(e) => {
            e.stopPropagation();
            if (!(e.target as HTMLElement).closest(".ov-pal")) setPal(false);
          }}
          onMouseDown={(e) => e.preventDefault()}
        >
          {TOOLS.map((t) => (
            <button key={t.id} type="button" className="ov-btn" aria-pressed={tool === t.id} aria-label={t.label} title={`${t.label} (${t.key})`} onClick={() => pickTool(t.id)}>
              <t.Icon size={18} strokeWidth={1.75} />
            </button>
          ))}
          <span className="ov-pal">
            <button
              type="button"
              className="ov-btn ov-cur"
              aria-label={`Colour: ${COLOR_NAMES[COLORS.indexOf(color)]}`}
              aria-haspopup="true"
              aria-expanded={pal}
              title="Colour (1 to 5)"
              onClick={() => setPal((o) => !o)}
            >
              <i style={{ ["--c" as string]: color }} />
            </button>
            {pal ? (
              <span className={popUp ? "ov-hud ov-pop ov-pop-up" : "ov-hud ov-pop"} role="group" aria-label="Colours">
                {COLORS.map((c, i) => (
                  <button
                    key={c}
                    type="button"
                    className="ov-sw"
                    aria-pressed={color === c}
                    aria-label={COLOR_NAMES[i]}
                    title={`${COLOR_NAMES[i]} (${i + 1})`}
                    onClick={() => {
                      setColor(c);
                      setPal(false);
                    }}
                  >
                    <i style={{ ["--c" as string]: c }} />
                  </button>
                ))}
              </span>
            ) : null}
          </span>
          <span className="ov-sep" />
          <button type="button" className="ov-btn" aria-label="Undo" title="Undo (Ctrl/Cmd+Z)" disabled={!hs.past.length} onClick={() => dispatch({ t: "undo" })}>
            <Undo2 size={18} strokeWidth={1.75} />
          </button>
          <button type="button" className="ov-btn" aria-label="Redo" title="Redo (Shift+Ctrl/Cmd+Z)" disabled={!hs.future.length} onClick={() => dispatch({ t: "redo" })}>
            <Redo2 size={18} strokeWidth={1.75} />
          </button>
          <span className="ov-sep" />
          <button type="button" className="ov-go" title={edit ? "Save (Return)" : "Capture (Enter)"} onClick={() => void commit()}>
            <Check size={16} strokeWidth={2.25} />
            {edit ? "Save" : "Capture"}
          </button>
          <button type="button" className="ov-btn ov-x" aria-label="Cancel" title="Cancel (Esc)" onClick={leave}>
            <X size={18} strokeWidth={1.75} />
          </button>
        </div>
      ) : null}
      {src && !gone ? <div className="ov-hud ov-hint">{hint(layer, tool, vis.find((x) => x.id === picked)?.kind, edit, crop)}</div> : null}
      {out ? (
        <div className="ov-hud ov-out" onPointerDown={(e) => e.stopPropagation()}>
          <img src={out.url} alt="Captured result" />
          <p>
            Preview capture {out.w} × {out.h} px ·{" "}
            {[out.element ? `${out.element.role} “${out.element.name}”` : "", ...captionMarks(out.labels), hiddenNote(out.hidden)].filter(Boolean).join(" · ") ||
              "no labels"}
          </p>
        </div>
      ) : null}
    </div>
  );
}
