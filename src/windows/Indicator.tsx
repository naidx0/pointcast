import { useEffect, useState } from "react";
import { Icon } from "@/components/Icons";
import {
  captureFull,
  captureZoom,
  onState,
  regionBegin,
  sessionCancel,
  sessionState,
  sessionToggle,
  stamp,
  type UiState,
} from "@/lib/api";

const idle: UiState = {
  phase: "idle",
  mic: false,
  screen: false,
  partial: "",
  text: "",
  error: null,
  duration_ms: 0,
  image_count: 0,
  model_ready: false,
  level: 0,
};

export function Indicator() {
  const [s, setS] = useState<UiState>(idle);

  useEffect(() => {
    sessionState()
      .then(setS)
      .catch(() => {
        if (typeof location !== "undefined" && location.search.includes("preview=1")) {
          setS({
            ...idle,
            phase: "recording",
            mic: true,
            screen: true,
            partial: "The Place order button…",
            duration_ms: 16_000,
            image_count: 1,
            level: 0.4,
          });
        }
      });
    const u = onState(setS);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  const rec = s.phase === "recording" || s.phase === "loading";
  const tip = s.partial || s.text || (rec ? "Listening…" : "Idle");

  return (
    <div
      style={{
        height: "100vh",
        width: "100vw",
        background: "transparent",
        display: "grid",
        placeItems: "center",
        padding: 8,
        boxSizing: "border-box",
      }}
      data-tauri-drag-region
      title={tip}
    >
      <div className="capbar glassbar ind-bar" data-tauri-drag-region>
        <span className={`dot ${rec ? "rec" : ""}`} style={{ marginLeft: 6 }} />
        <span className="mono ind-clock">{stamp(s.duration_ms)}</span>
        <span className="sep" />
        <button type="button" className="ib no-drag" title="Full screenshot" onClick={() => captureFull()}>
          <Icon name="monitor" />
        </button>
        <button type="button" className="ib no-drag" title="Region / annotate" onClick={() => regionBegin()}>
          <Icon name="crop" />
        </button>
        <button type="button" className="ib no-drag" title="Zoom crop" onClick={() => captureZoom()}>
          <Icon name="zoom" />
        </button>
        <span className="sep" />
        <button type="button" className="ib no-drag" title={s.mic ? "Mic on" : "Mic off"} aria-pressed={s.mic}>
          <Icon name="mic" />
        </button>
        {s.image_count ? <span className="photo-chip">{s.image_count}</span> : null}
        <span className="vb-sp" style={{ flex: 1 }} />
        <button type="button" className="pill pri no-drag" title="Stop" onClick={() => sessionToggle()}>
          <Icon name="stop" size={12} />
        </button>
        <button type="button" className="ib no-drag" title="Cancel" onClick={() => sessionCancel()}>
          <Icon name="x" />
        </button>
      </div>
    </div>
  );
}
