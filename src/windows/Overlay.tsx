import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { onOverlayFrame, regionCancel, regionCommit, type Label } from "@/lib/api";

type Layer = "selecting" | "annotating" | "pinning" | "drawing";
type Tool = "pin" | "pen" | "highlight";
type Rect = { x: number; y: number; w: number; h: number };
type Pin = { n: number; x: number; y: number; text: string };
type Stroke = { tool: "pen" | "highlight"; points: { x: number; y: number }[] };

function esc(layer: Layer): Layer | "idle" {
  if (layer === "drawing" || layer === "pinning") return "annotating";
  if (layer === "annotating") return "selecting";
  return "idle";
}

function hint(layer: Layer, tool: Tool): string {
  if (layer === "selecting") return "Drag a box · Esc cancel";
  if (layer === "annotating" && tool === "pin")
    return "Click to pin · P pen · H highlight · Enter capture · Esc back";
  if (layer === "annotating" && tool === "pen")
    return "Draw · P pin · H highlight · Enter capture · Esc back";
  if (layer === "annotating") return "Highlight · P pin · D pen · Enter capture · Esc back";
  if (layer === "pinning") return "Click to drop pin · Esc cancel pin";
  return "Release to finish stroke · Esc cancel stroke";
}

export function Overlay() {
  const imgRef = useRef<HTMLImageElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [natural, setNatural] = useState({ w: 0, h: 0 });
  const [layer, setLayer] = useState<Layer>("selecting");
  const [tool, setTool] = useState<Tool>("pin");
  const [box, setBox] = useState<Rect | null>(null);
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null);
  const [pins, setPins] = useState<Pin[]>([]);
  const [strokes, setStrokes] = useState<Stroke[]>([]);
  const [liveStroke, setLiveStroke] = useState<Stroke | null>(null);
  const [editing, setEditing] = useState<number | null>(null);

  useEffect(() => {
    const load = (f: { png_b64: string; width: number; height: number }) => {
      setSrc(`data:image/png;base64,${f.png_b64}`);
      setNatural({ w: f.width, h: f.height });
      setLayer("selecting");
      setBox(null);
      setPins([]);
      setStrokes([]);
    };
    const u = onOverlayFrame(load);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  const toImg = useCallback(
    (clientX: number, clientY: number) => {
      const img = imgRef.current;
      if (!img || !natural.w) return { x: 0, y: 0 };
      const r = img.getBoundingClientRect();
      const x = ((clientX - r.left) / r.width) * natural.w;
      const y = ((clientY - r.top) / r.height) * natural.h;
      return { x, y };
    },
    [natural],
  );

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        const next = esc(layer);
        if (next === "idle") {
          regionCancel();
          return;
        }
        if (layer === "drawing") setLiveStroke(null);
        if (next === "selecting") {
          setBox(null);
          setPins([]);
          setStrokes([]);
        }
        setLayer(next);
      } else if (e.key === "Enter" && box) {
        void commit();
      } else if (e.key === "p" || e.key === "P") setTool("pin");
      else if (e.key === "d" || e.key === "D") setTool("pen");
      else if (e.key === "h" || e.key === "H") setTool("highlight");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  function onDown(e: { clientX: number; clientY: number }) {
    const p = toImg(e.clientX, e.clientY);
    if (layer === "selecting") {
      setDrag(p);
      setBox({ x: p.x, y: p.y, w: 0, h: 0 });
    } else if (layer === "annotating" || layer === "pinning") {
      if (tool === "pin" || layer === "pinning") {
        const n = pins.length + 1;
        setPins((xs) => [...xs, { n, x: p.x, y: p.y, text: "" }]);
        setEditing(n);
        setLayer("annotating");
      } else {
        setLayer("drawing");
        setLiveStroke({ tool, points: [p] });
      }
    }
  }

  function onMove(e: { clientX: number; clientY: number }) {
    const p = toImg(e.clientX, e.clientY);
    if (layer === "selecting" && drag) {
      setBox({
        x: Math.min(drag.x, p.x),
        y: Math.min(drag.y, p.y),
        w: Math.abs(p.x - drag.x),
        h: Math.abs(p.y - drag.y),
      });
    } else if (layer === "drawing" && liveStroke) {
      setLiveStroke({ ...liveStroke, points: [...liveStroke.points, p] });
    }
  }

  function onUp() {
    if (layer === "selecting" && box && box.w > 8 && box.h > 8) {
      setLayer("annotating");
      setDrag(null);
    } else if (layer === "drawing" && liveStroke) {
      setStrokes((s) => [...s, liveStroke]);
      setLiveStroke(null);
      setLayer("annotating");
    }
    setDrag(null);
  }

  async function commit() {
    if (!src || !box) return;
    const png = await rasterize(src, natural, box, pins, [...strokes, ...(liveStroke ? [liveStroke] : [])]);
    const labels: Label[] = pins
      .filter((p) => inside(p, box))
      .map((p) => ({ n: p.n, text: p.text || `pin ${p.n}` }));
    await regionCommit(png, labels);
  }

  const allStrokes = [...strokes, ...(liveStroke ? [liveStroke] : [])];

  return (
    <div
      className="relative h-screen w-screen cursor-crosshair overflow-hidden bg-black/40"
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
    >
      {src ? (
        <img
          ref={imgRef}
          src={src}
          alt=""
          className="pointer-events-none h-full w-full object-contain"
          draggable={false}
        />
      ) : (
        <div className="flex h-full items-center justify-center text-ink-300">Capturing freeze-frame…</div>
      )}
      <canvas ref={canvasRef} className="hidden" />
      {box ? <BoxOverlay img={imgRef.current} box={box} natural={natural} dim={layer !== "selecting"} /> : null}
      {pins.map((p) => (
        <PinMark
          key={p.n}
          pin={p}
          img={imgRef.current}
          natural={natural}
          editing={editing === p.n}
          onText={(text) => setPins((xs) => xs.map((x) => (x.n === p.n ? { ...x, text } : x)))}
        />
      ))}
      <SvgStrokes img={imgRef.current} natural={natural} strokes={allStrokes} />
      <div className="hint-bar pointer-events-none absolute bottom-6 left-1/2 z-20 -translate-x-1/2 rounded-full border border-ink-500/30 bg-ink-950/80 px-4 py-2 font-mono text-xs text-ink-100">
        {hint(layer, tool)}
      </div>
      {layer === "annotating" ? (
        <div className="absolute left-1/2 top-5 z-20 flex -translate-x-1/2 gap-2">
          <ToolBtn active={tool === "pin"} onClick={() => setTool("pin")}>
            Pin
          </ToolBtn>
          <ToolBtn active={tool === "pen"} onClick={() => setTool("pen")}>
            Pen
          </ToolBtn>
          <ToolBtn active={tool === "highlight"} onClick={() => setTool("highlight")}>
            Highlight
          </ToolBtn>
          <ToolBtn active={false} onClick={() => void commit()}>
            Capture ↵
          </ToolBtn>
        </div>
      ) : null}
    </div>
  );
}

function ToolBtn({
  active,
  onClick,
  children,
}: {
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
      className={`rounded-full px-3 py-1 text-xs ${active ? "bg-ember-400 text-ink-950" : "bg-ink-900/90 text-ink-100"}`}
    >
      {children}
    </button>
  );
}

function BoxOverlay({
  img,
  box,
  natural,
  dim,
}: {
  img: HTMLImageElement | null;
  box: Rect;
  natural: { w: number; h: number };
  dim: boolean;
}) {
  if (!img || !natural.w) return null;
  const r = img.getBoundingClientRect();
  const sx = r.width / natural.w;
  const sy = r.height / natural.h;
  const style = {
    left: r.left + box.x * sx,
    top: r.top + box.y * sy,
    width: box.w * sx,
    height: box.h * sy,
  };
  return (
    <>
      {dim ? <div className="pointer-events-none absolute inset-0 bg-black/35" /> : null}
      <div
        className="pointer-events-none absolute z-10 box-border border-2 border-ember-400"
        style={style}
      />
    </>
  );
}

function PinMark({
  pin,
  img,
  natural,
  editing,
  onText,
}: {
  pin: Pin;
  img: HTMLImageElement | null;
  natural: { w: number; h: number };
  editing: boolean;
  onText: (t: string) => void;
}) {
  if (!img || !natural.w) return null;
  const r = img.getBoundingClientRect();
  const left = r.left + (pin.x / natural.w) * r.width;
  const top = r.top + (pin.y / natural.h) * r.height;
  return (
    <div className="absolute z-20 -translate-x-1/2 -translate-y-1/2" style={{ left, top }}>
      <div className="flex h-6 w-6 items-center justify-center rounded-full bg-ember-400 text-xs font-semibold text-ink-950">
        {pin.n}
      </div>
      {editing || pin.text ? (
        <input
          autoFocus={editing}
          value={pin.text}
          onChange={(e) => onText(e.target.value)}
          onPointerDown={(e) => e.stopPropagation()}
          placeholder="label"
          className="mt-1 w-28 rounded bg-ink-950/90 px-1 py-0.5 font-mono text-[11px]"
        />
      ) : null}
    </div>
  );
}

function SvgStrokes({
  img,
  natural,
  strokes,
}: {
  img: HTMLImageElement | null;
  natural: { w: number; h: number };
  strokes: Stroke[];
}) {
  if (!img || !natural.w) return null;
  const r = img.getBoundingClientRect();
  return (
    <svg className="pointer-events-none absolute inset-0 z-10" width={window.innerWidth} height={window.innerHeight}>
      {strokes.map((s, i) => {
        const d = s.points
          .map((p, idx) => {
            const x = r.left + (p.x / natural.w) * r.width;
            const y = r.top + (p.y / natural.h) * r.height;
            return `${idx ? "L" : "M"}${x} ${y}`;
          })
          .join(" ");
        return (
          <path
            key={i}
            d={d}
            fill="none"
            stroke={s.tool === "highlight" ? "rgba(245,165,36,0.35)" : "#f5a524"}
            strokeWidth={s.tool === "highlight" ? 18 : 3}
            strokeLinecap="round"
            strokeLinejoin="round"
          />
        );
      })}
    </svg>
  );
}

function inside(p: Pin, box: Rect) {
  return p.x >= box.x && p.y >= box.y && p.x <= box.x + box.w && p.y <= box.y + box.h;
}

async function rasterize(
  src: string,
  natural: { w: number; h: number },
  box: Rect,
  pins: Pin[],
  strokes: Stroke[],
): Promise<string> {
  const img = await loadImg(src);
  const sx = Math.max(1, Math.floor(box.x));
  const sy = Math.max(1, Math.floor(box.y));
  const sw = Math.max(1, Math.floor(box.w));
  const sh = Math.max(1, Math.floor(box.h));
  const c = document.createElement("canvas");
  c.width = sw;
  c.height = sh;
  const ctx = c.getContext("2d")!;
  ctx.drawImage(img, sx, sy, sw, sh, 0, 0, sw, sh);
  for (const s of strokes) {
    ctx.beginPath();
    ctx.strokeStyle = s.tool === "highlight" ? "rgba(245,165,36,0.35)" : "#f5a524";
    ctx.lineWidth = s.tool === "highlight" ? 18 : 3;
    ctx.lineCap = "round";
    s.points.forEach((p, i) => {
      const x = p.x - box.x;
      const y = p.y - box.y;
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    });
    ctx.stroke();
  }
  for (const p of pins.filter((p) => inside(p, box))) {
    const x = p.x - box.x;
    const y = p.y - box.y;
    ctx.beginPath();
    ctx.fillStyle = "#f5a524";
    ctx.arc(x, y, 10, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#0e0d0b";
    ctx.font = "bold 12px sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(String(p.n), x, y);
    if (p.text) {
      ctx.fillStyle = "#f4efe6";
      ctx.font = "11px sans-serif";
      ctx.textAlign = "left";
      ctx.fillText(p.text, x + 14, y);
    }
  }
  const data = c.toDataURL("image/png");
  return data.replace(/^data:image\/png;base64,/, "");
}

function loadImg(src: string) {
  return new Promise<HTMLImageElement>((res, rej) => {
    const i = new Image();
    i.onload = () => res(i);
    i.onerror = rej;
    i.src = src;
  });
}
