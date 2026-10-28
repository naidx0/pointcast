import { useCallback, useEffect, useRef, useState, type CSSProperties } from "react";
import { Icon, type IconName } from "@/components/Icons";
import { onOverlayFrame, regionCancel, regionCommit, type Label } from "@/lib/api";

type Layer = "selecting" | "annotating" | "pinning" | "drawing";
type Tool = "pin" | "arrow" | "text" | "pen";
type Rect = { x: number; y: number; w: number; h: number };
type Pin = { n: number; x: number; y: number; text: string };
type Note = { id: number; x: number; y: number; text: string };
type Stroke = { tool: "pen" | "arrow"; points: { x: number; y: number }[] };

function esc(layer: Layer): Layer | "idle" {
  if (layer === "drawing" || layer === "pinning") return "annotating";
  if (layer === "annotating") return "selecting";
  return "idle";
}

function hint(layer: Layer, tool: Tool): string {
  if (layer === "selecting") return "Drag a box around the issue · Esc cancel";
  if (layer === "annotating" && tool === "pin") return "Click to drop a numbered pin · Enter to capture";
  if (layer === "annotating" && tool === "arrow") return "Drag an arrow · Enter to capture";
  if (layer === "annotating" && tool === "text") return "Click to place a label · Enter to capture";
  if (layer === "annotating") return "Draw · Enter to capture";
  if (layer === "pinning") return "Click to drop pin · Esc cancel";
  return "Release to finish · Esc cancel";
}

export function Overlay() {
  const imgRef = useRef<HTMLImageElement | null>(null);
  const [src, setSrc] = useState<string | null>(null);
  const [natural, setNatural] = useState({ w: 0, h: 0 });
  const [layer, setLayer] = useState<Layer>("selecting");
  const [tool, setTool] = useState<Tool>("pin");
  const [box, setBox] = useState<Rect | null>(null);
  const [drag, setDrag] = useState<{ x: number; y: number } | null>(null);
  const [pins, setPins] = useState<Pin[]>([]);
  const [notes, setNotes] = useState<Note[]>([]);
  const [strokes, setStrokes] = useState<Stroke[]>([]);
  const [liveStroke, setLiveStroke] = useState<Stroke | null>(null);
  const [editingPin, setEditingPin] = useState<number | null>(null);
  const [editingNote, setEditingNote] = useState<number | null>(null);

  useEffect(() => {
    const load = (f: { png_b64: string; width: number; height: number }) => {
      setSrc(`data:image/png;base64,${f.png_b64}`);
      setNatural({ w: f.width, h: f.height });
      setLayer("selecting");
      setBox(null);
      setPins([]);
      setNotes([]);
      setStrokes([]);
    };
    const u = onOverlayFrame(load);
    if (typeof location !== "undefined" && location.search.includes("preview=1")) {
      const img = new Image();
      img.onload = () => {
        setSrc("/freeze.png");
        setNatural({ w: img.naturalWidth || 1280, h: img.naturalHeight || 800 });
        setLayer("annotating");
        setBox({ x: 420, y: 280, w: 360, h: 220 });
        setPins([{ n: 1, x: 560, y: 360, text: "Place order — 28px too far left" }]);
      };
      img.src = "/freeze.png";
    }
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  const toImg = useCallback(
    (clientX: number, clientY: number) => {
      const img = imgRef.current;
      if (!img || !natural.w) return { x: 0, y: 0 };
      const r = img.getBoundingClientRect();
      return {
        x: ((clientX - r.left) / r.width) * natural.w,
        y: ((clientY - r.top) / r.height) * natural.h,
      };
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
          setNotes([]);
          setStrokes([]);
        }
        setLayer(next);
      } else if (e.key === "Enter" && box) void commit();
      else if (e.key === "p" || e.key === "P") setTool("pin");
      else if (e.key === "a" || e.key === "A") setTool("arrow");
      else if (e.key === "t" || e.key === "T") setTool("text");
      else if (e.key === "d" || e.key === "D") setTool("pen");
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
        setEditingPin(n);
        setLayer("annotating");
      } else if (tool === "text") {
        const id = notes.length + 1;
        setNotes((xs) => [...xs, { id, x: p.x, y: p.y, text: "" }]);
        setEditingNote(id);
      } else {
        setLayer("drawing");
        setLiveStroke({ tool: tool === "arrow" ? "arrow" : "pen", points: [p] });
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
      if (liveStroke.tool === "arrow") {
        setLiveStroke({ ...liveStroke, points: [liveStroke.points[0], p] });
      } else {
        setLiveStroke({ ...liveStroke, points: [...liveStroke.points, p] });
      }
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
    const png = await rasterize(src, natural, box, pins, notes, [...strokes, ...(liveStroke ? [liveStroke] : [])]);
    const labels: Label[] = [
      ...pins.filter((p) => inside(p, box)).map((p) => ({ n: p.n, text: p.text || `pin ${p.n}` })),
      ...notes
        .filter((n) => n.x >= box.x && n.y >= box.y && n.x <= box.x + box.w && n.y <= box.y + box.h)
        .map((n, i) => ({ n: pins.length + i + 1, text: n.text || `note ${i + 1}` })),
    ];
    await regionCommit(png, labels);
  }

  const allStrokes = [...strokes, ...(liveStroke ? [liveStroke] : [])];
  const tools: { id: Tool; icon: IconName; title: string }[] = [
    { id: "pin", icon: "pin", title: "Pin (P)" },
    { id: "arrow", icon: "arrow", title: "Arrow (A)" },
    { id: "text", icon: "type", title: "Text (T)" },
    { id: "pen", icon: "pen", title: "Pen (D)" },
  ];

  return (
    <div
      style={{
        position: "relative",
        height: "100vh",
        width: "100vw",
        cursor: "crosshair",
        overflow: "hidden",
        background: "rgba(0,0,0,0.35)",
      }}
      onPointerDown={onDown}
      onPointerMove={onMove}
      onPointerUp={onUp}
    >
      {src ? (
        <img
          ref={imgRef}
          src={src}
          alt=""
          style={{ pointerEvents: "none", height: "100%", width: "100%", objectFit: "contain" }}
          draggable={false}
        />
      ) : (
        <div style={{ display: "grid", placeItems: "center", height: "100%", color: "#fff", font: "500 13px var(--font)" }}>
          Capturing freeze-frame…
        </div>
      )}
      {box ? <BoxOverlay img={imgRef.current} box={box} natural={natural} dim={layer !== "selecting"} /> : null}
      {pins.map((p) => (
        <PinMark
          key={p.n}
          pin={p}
          img={imgRef.current}
          natural={natural}
          editing={editingPin === p.n}
          onText={(text) => setPins((xs) => xs.map((x) => (x.n === p.n ? { ...x, text } : x)))}
        />
      ))}
      {notes.map((n) => (
        <TextMark
          key={n.id}
          note={n}
          img={imgRef.current}
          natural={natural}
          editing={editingNote === n.id}
          onText={(text) => setNotes((xs) => xs.map((x) => (x.id === n.id ? { ...x, text } : x)))}
        />
      ))}
      <SvgStrokes img={imgRef.current} natural={natural} strokes={allStrokes} />
      <div className="hint-bar">{hint(layer, tool)}</div>
      {layer === "annotating" ? (
        <div
          className="capbar glassbar no-drag"
          style={{ position: "absolute", left: "50%", top: 20, transform: "translateX(-50%)", zIndex: 20 }}
          onPointerDown={(e) => e.stopPropagation()}
        >
          {tools.map((t) => (
            <button
              key={t.id}
              type="button"
              className={`ib ${tool === t.id ? "on" : ""}`}
              title={t.title}
              onClick={() => setTool(t.id)}
            >
              <Icon name={t.icon} size={18} />
            </button>
          ))}
          <span className="sep" />
          <button type="button" className="pill pri lg" title="Capture (Enter)" onClick={() => void commit()}>
            <Icon name="check" size={16} />
            Capture
          </button>
        </div>
      ) : null}
    </div>
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
  const style: CSSProperties = {
    position: "absolute",
    left: r.left + box.x * sx,
    top: r.top + box.y * sy,
    width: box.w * sx,
    height: box.h * sy,
    borderRadius: 12,
    outline: "3px solid #fff",
    boxShadow: dim ? "0 0 0 9999px rgba(0,0,0,0.4)" : "0 0 0 9999px rgba(0,0,0,0.25)",
    zIndex: 10,
    pointerEvents: "none",
  };
  return <div style={style} />;
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
    <div style={{ position: "absolute", zIndex: 20, left, top, transform: "translate(-50%, -50%)" }}>
      <div
        style={{
          display: "grid",
          placeItems: "center",
          width: 36,
          height: 36,
          borderRadius: 999,
          background: "#FF453A",
          color: "#fff",
          font: "650 15px var(--font)",
          boxShadow: "0 4px 14px rgba(255,69,58,0.4)",
        }}
      >
        {pin.n}
      </div>
      {editing || pin.text ? (
        <input
          autoFocus={editing}
          value={pin.text}
          onChange={(e) => onText(e.target.value)}
          onPointerDown={(e) => e.stopPropagation()}
          placeholder="Label…"
          className="field"
          style={{ marginTop: 8, width: 180, height: 36, fontSize: 14 }}
        />
      ) : null}
    </div>
  );
}

function TextMark({
  note,
  img,
  natural,
  editing,
  onText,
}: {
  note: Note;
  img: HTMLImageElement | null;
  natural: { w: number; h: number };
  editing: boolean;
  onText: (t: string) => void;
}) {
  if (!img || !natural.w) return null;
  const r = img.getBoundingClientRect();
  const left = r.left + (note.x / natural.w) * r.width;
  const top = r.top + (note.y / natural.h) * r.height;
  return (
    <div style={{ position: "absolute", zIndex: 20, left, top, transform: "translate(-8px, -50%)" }} onPointerDown={(e) => e.stopPropagation()}>
      <input
        autoFocus={editing}
        value={note.text}
        onChange={(e) => onText(e.target.value)}
        placeholder="Type a label…"
        className="field"
        style={{
          width: 200,
          height: 40,
          fontSize: 15,
          fontWeight: 600,
          background: "rgba(255,255,255,0.95)",
          border: "2px solid #FF453A",
          boxShadow: "0 6px 18px rgba(0,0,0,0.18)",
        }}
      />
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
  const to = (p: { x: number; y: number }) => ({
    x: r.left + (p.x / natural.w) * r.width,
    y: r.top + (p.y / natural.h) * r.height,
  });
  return (
    <svg style={{ pointerEvents: "none", position: "absolute", inset: 0, zIndex: 10 }} width={window.innerWidth} height={window.innerHeight}>
      <defs>
        <marker id="pc-arrow" markerWidth="8" markerHeight="8" refX="6" refY="3" orient="auto">
          <path d="M0,0 L6,3 L0,6 Z" fill="#FF453A" />
        </marker>
      </defs>
      {strokes.map((s, i) => {
        if (s.tool === "arrow" && s.points.length >= 2) {
          const a = to(s.points[0]);
          const b = to(s.points[s.points.length - 1]);
          return (
            <line
              key={i}
              x1={a.x}
              y1={a.y}
              x2={b.x}
              y2={b.y}
              stroke="#FF453A"
              strokeWidth={4}
              strokeLinecap="round"
              markerEnd="url(#pc-arrow)"
            />
          );
        }
        const d = s.points
          .map((p, idx) => {
            const q = to(p);
            return `${idx ? "L" : "M"}${q.x} ${q.y}`;
          })
          .join(" ");
        return (
          <path
            key={i}
            d={d}
            fill="none"
            stroke="#FF453A"
            strokeWidth={3.5}
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
  _natural: { w: number; h: number },
  box: Rect,
  pins: Pin[],
  notes: Note[],
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
    if (s.tool === "arrow" && s.points.length >= 2) {
      const a = s.points[0];
      const b = s.points[s.points.length - 1];
      const x1 = a.x - box.x;
      const y1 = a.y - box.y;
      const x2 = b.x - box.x;
      const y2 = b.y - box.y;
      ctx.strokeStyle = "#FF453A";
      ctx.lineWidth = 4;
      ctx.lineCap = "round";
      ctx.beginPath();
      ctx.moveTo(x1, y1);
      ctx.lineTo(x2, y2);
      ctx.stroke();
      const ang = Math.atan2(y2 - y1, x2 - x1);
      ctx.beginPath();
      ctx.moveTo(x2, y2);
      ctx.lineTo(x2 - 12 * Math.cos(ang - 0.4), y2 - 12 * Math.sin(ang - 0.4));
      ctx.lineTo(x2 - 12 * Math.cos(ang + 0.4), y2 - 12 * Math.sin(ang + 0.4));
      ctx.closePath();
      ctx.fillStyle = "#FF453A";
      ctx.fill();
      continue;
    }
    ctx.beginPath();
    ctx.strokeStyle = "#FF453A";
    ctx.lineWidth = 3.5;
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
    ctx.fillStyle = "#FF453A";
    ctx.arc(x, y, 16, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.font = "650 15px Geist, system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(String(p.n), x, y + 0.5);
    if (p.text) {
      ctx.fillStyle = "#111";
      ctx.font = "600 14px Geist, system-ui, sans-serif";
      ctx.textAlign = "left";
      ctx.fillText(p.text, x + 20, y);
    }
  }
  for (const n of notes.filter((n) => n.x >= box.x && n.y >= box.y && n.x <= box.x + box.w && n.y <= box.y + box.h)) {
    if (!n.text) continue;
    const x = n.x - box.x;
    const y = n.y - box.y;
    ctx.font = "600 15px Geist, system-ui, sans-serif";
    const tw = ctx.measureText(n.text).width + 16;
    ctx.fillStyle = "rgba(255,255,255,0.95)";
    ctx.strokeStyle = "#FF453A";
    ctx.lineWidth = 2;
    roundRect(ctx, x, y - 16, tw, 32, 8);
    ctx.fill();
    ctx.stroke();
    ctx.fillStyle = "#111";
    ctx.textAlign = "left";
    ctx.textBaseline = "middle";
    ctx.fillText(n.text, x + 8, y);
  }
  return c.toDataURL("image/png").replace(/^data:image\/png;base64,/, "");
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
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
