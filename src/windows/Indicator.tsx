import { useEffect, useState, type ReactNode } from "react";
import { Mark } from "@/components/brand/Mark";
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
            partial: "The Place order button is sitting on top of the total…",
            duration_ms: 16_000,
            image_count: 1,
            level: 0.45,
          });
        }
      });
    const u = onState(setS);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  const live = s.partial || s.text || "Listening…";
  const rec = s.phase === "recording" || s.phase === "loading";

  return (
    <div
      style={{ height: "100vh", width: "100vw", background: "transparent", padding: 12, boxSizing: "border-box" }}
      data-tauri-drag-region
    >
      <div
        className="panel"
        style={{ height: "100%", display: "flex", flexDirection: "column", padding: 14, borderRadius: 20, boxSizing: "border-box" }}
      >
        <div className="row-between" data-tauri-drag-region>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <Mark />
            <span className={`dot ${rec ? "rec" : ""}`} />
            <span
              className="mono"
              style={{
                fontSize: "var(--fs-xs)",
                letterSpacing: "0.08em",
                textTransform: "uppercase",
                color: rec ? "var(--red-ink)" : "var(--text3)",
              }}
            >
              {s.phase === "transcribing" ? "Transcribing" : rec ? "Recording" : s.phase}
            </span>
            <span className="mono" style={{ fontSize: "var(--fs-xs)", color: "var(--text3)" }}>
              {stamp(s.duration_ms)}
            </span>
          </div>
          <div style={{ display: "flex", gap: 10, fontSize: "var(--fs-xs)", color: "var(--text3)" }}>
            <span style={{ display: "inline-flex", alignItems: "center", gap: 4, color: s.mic ? "var(--red-ink)" : undefined }}>
              <i className={`dot ${s.mic ? "mic" : ""}`} style={{ width: 6, height: 6 }} />
              mic
            </span>
            <span
              style={{ display: "inline-flex", alignItems: "center", gap: 4, color: s.screen ? "var(--accent-ink)" : undefined }}
            >
              <i className={`dot ${s.screen ? "screen" : ""}`} style={{ width: 6, height: 6 }} />
              screen
            </span>
          </div>
        </div>

        <div className="level" style={{ marginTop: 10 }}>
          <i style={{ width: `${Math.round(Math.min(1, s.level) * 100)}%` }} />
        </div>

        <p
          style={{
            margin: "10px 0 0",
            flex: 1,
            minHeight: 0,
            overflow: "hidden",
            font: "400 var(--fs-sm) / 1.45 var(--mono)",
            color: "var(--text)",
          }}
        >
          {live}
          {s.image_count ? (
            <span className="photo-chip" style={{ marginLeft: 8 }}>
              {s.image_count} image{s.image_count === 1 ? "" : "s"}
            </span>
          ) : null}
        </p>

        {s.error ? (
          <p
            style={{
              margin: "4px 0 0",
              fontSize: "var(--fs-xs)",
              color: "var(--orange-ink)",
              overflow: "hidden",
              textOverflow: "ellipsis",
              whiteSpace: "nowrap",
            }}
          >
            {s.error}
          </p>
        ) : null}

        <div className="glassbar no-drag" style={{ marginTop: 10, width: "100%", justifyContent: "space-between" }}>
          <Mini onClick={() => captureFull()}>Full</Mini>
          <Mini onClick={() => regionBegin()}>Region</Mini>
          <Mini onClick={() => captureZoom()}>Zoom</Mini>
          <span style={{ width: 1, height: 18, background: "var(--line)", margin: "0 2px" }} />
          <Mini onClick={() => sessionToggle()} primary>
            Stop
          </Mini>
          <Mini onClick={() => sessionCancel()}>Esc</Mini>
        </div>
      </div>
    </div>
  );
}

function Mini({
  children,
  onClick,
  primary,
}: {
  children: ReactNode;
  onClick: () => void;
  primary?: boolean;
}) {
  return (
    <button type="button" className={`pill sm ${primary ? "pri" : "ghost"}`} onClick={onClick}>
      {children}
    </button>
  );
}
