import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import { Icon } from "@/components/Icons";
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
      <canvas ref={canvasRef} style={{ display: "none" }} />
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
      <div className="hint-bar">{hint(layer, tool)}</div>
      {layer === "annotating" ? (
        <div
          className="capbar glassbar no-drag"
          style={{ position: "absolute", left: "50%", top: 20, transform: "translateX(-50%)", zIndex: 20 }}
          onPointerDown={(e) => e.stopPropagation()}
        >
          <button
            type="button"
            className={`ib ${tool === "pin" ? "on" : ""}`}
            title="Pin (P)"
            onClick={() => setTool("pin")}
          >
            <Icon name="pin" />
          </button>
          <button
            type="button"
            className={`ib ${tool === "pen" ? "on" : ""}`}
            title="Pen (D)"
            onClick={() => setTool("pen")}
          >
            <Icon name="pen" />
          </button>
          <button
            type="button"
            className={`ib ${tool === "highlight" ? "on" : ""}`}
            title="Highlight (H)"
            onClick={() => setTool("highlight")}
          >
            <Icon name="hi" />
          </button>
          <span className="sep" />
          <button type="button" className="pill pri" title="Capture (Enter)" onClick={() => void commit()}>
            <Icon name="check" size={14} />
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
    borderRadius: 10,
    outline: "2px solid #fff",
    boxShadow: dim ? "0 0 0 9999px rgba(0,0,0,0.35)" : undefined,
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
          width: 24,
          height: 24,
          borderRadius: 999,
          background: "#FF453A",
          color: "#fff",
          font: "600 12px var(--font)",
          boxShadow: "0 4px 12px rgba(255,69,58,0.35)",
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
          placeholder="label"
          className="field mono"
          style={{ marginTop: 4, width: 120, height: 26, fontSize: 11 }}
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
    <svg
      style={{ pointerEvents: "none", position: "absolute", inset: 0, zIndex: 10 }}
      width={window.innerWidth}
      height={window.innerHeight}
    >
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
            stroke={s.tool === "highlight" ? "rgba(255,159,10,0.35)" : "#FF453A"}
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
    ctx.strokeStyle = s.tool === "highlight" ? "rgba(255,159,10,0.35)" : "#FF453A";
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
    ctx.fillStyle = "#FF453A";
    ctx.arc(x, y, 10, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.font = "bold 12px -apple-system, system-ui, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";
    ctx.fillText(String(p.n), x, y);
    if (p.text) {
      ctx.fillStyle = "#111";
      ctx.font = "11px -apple-system, system-ui, sans-serif";
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
