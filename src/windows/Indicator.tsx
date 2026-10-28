import { useEffect, useState, type ReactNode } from "react";
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
    sessionState().then(setS).catch(() => undefined);
    const u = onState(setS);
    return () => {
      u.then((fn) => fn());
    };
  }, []);

  const live = s.partial || s.text || "Listening…";
  const rec = s.phase === "recording" || s.phase === "loading";

  return (
    <div className="h-screen w-screen bg-transparent p-2" data-tauri-drag-region>
      <div className="flex h-full flex-col rounded-2xl border border-ember-400/30 bg-ink-950/92 p-3 shadow-tray backdrop-blur-md">
        <div className="flex items-center justify-between gap-2" data-tauri-drag-region>
          <div className="flex items-center gap-2">
            <span className={`h-2.5 w-2.5 rounded-full bg-ember-400 ${rec ? "rec-dot" : ""}`} />
            <span className="font-mono text-xs uppercase tracking-widest text-ember-400">
              {s.phase === "transcribing" ? "Transcribing" : rec ? "Recording" : s.phase}
            </span>
            <span className="font-mono text-xs text-ink-300">{stamp(s.duration_ms)}</span>
          </div>
          <div className="flex items-center gap-2 text-[11px]">
            <span className={`inline-flex items-center gap-1 ${s.mic ? "text-ember-400" : "text-ink-500"}`}>
              <i className="h-1.5 w-1.5 rounded-full bg-current" />
              mic
            </span>
            <span className={`inline-flex items-center gap-1 ${s.screen ? "text-ice-400" : "text-ink-500"}`}>
              <i className="h-1.5 w-1.5 rounded-full bg-current" />
              screen
            </span>
          </div>
        </div>
        <div className="mt-2 h-1 overflow-hidden rounded-full bg-ink-800">
          <div
            className="h-full bg-ember-400 transition-[width]"
            style={{ width: `${Math.round(Math.min(1, s.level) * 100)}%` }}
          />
        </div>
        <p className="mt-2 min-h-0 flex-1 overflow-hidden font-mono text-[12px] leading-5 text-ink-100">
          {live}
          {s.image_count ? (
            <span className="ml-2 text-ice-400">
              {s.image_count} image{s.image_count === 1 ? "" : "s"}
            </span>
          ) : null}
        </p>
        {s.error ? <p className="truncate text-[11px] text-ember-400">{s.error}</p> : null}
        <div className="mt-2 flex gap-1.5">
          <Mini onClick={() => captureFull()}>Full</Mini>
          <Mini onClick={() => regionBegin()}>Region</Mini>
          <Mini onClick={() => captureZoom()}>Zoom</Mini>
          <Mini onClick={() => sessionToggle()}>Stop</Mini>
          <Mini onClick={() => sessionCancel()}>Esc</Mini>
        </div>
      </div>
    </div>
  );
}

function Mini({ children, onClick }: { children: ReactNode; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      className="rounded-md bg-ink-800 px-2 py-1 text-[11px] uppercase tracking-wide text-ink-100 hover:bg-ink-700"
    >
      {children}
    </button>
  );
}
