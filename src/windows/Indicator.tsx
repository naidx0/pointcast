import { useEffect, useRef, useState } from "react";
import { Check, Monitor, SquareDashedMousePointer, X, ZoomIn } from "lucide-react";
import {
  captureFull,
  captureZoom,
  inTauri,
  onState,
  preview,
  regionBegin,
  sessionCancel,
  sessionState,
  sessionToggle,
  settingsGet,
  stamp,
  subscribe,
  type Config,
  type UiState,
} from "@/lib/api";
import { chordText } from "@/lib/keys";
import { micNotice } from "@/lib/mic";
import { platform } from "@/lib/theme";

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
  notice: null,
};

const BARS = 9;

/** The recording HUD: a small capsule that never takes focus from the app you are talking about. */
export function Indicator() {
  const [s, setS] = useState<UiState>(idle);
  const [keys, setKeys] = useState<Config["hotkeys"] | null>(null);
  const [lv, setLv] = useState<number[]>(Array(BARS).fill(0));
  const last = useRef(0);

  useEffect(() => {
    if (inTauri) {
      sessionState().then(setS).catch(() => undefined);
      settingsGet()
        .then((c) => setKeys(c.hotkeys))
        .catch(() => undefined);
    } else if (preview) {
      // ?phase=transcribing&behind=40000 or ?quiet=silent|quiet to look at the other states.
      const q = new URLSearchParams(location.search);
      const quiet = q.get("quiet");
      setS({
        ...idle,
        phase: q.get("phase") === "transcribing" ? "transcribing" : "recording",
        mic: true,
        duration_ms: 16_000,
        image_count: 2,
        level: 0.4,
        behind_ms: Number(q.get("behind")) || 0,
        quiet_mic: quiet === "silent" || quiet === "quiet" ? quiet : null,
      });
      const id = setInterval(() => setLv((x) => [...x.slice(1), Math.random() * 0.9]), 110);
      return () => clearInterval(id);
    }
    return subscribe(
      onState((st) => {
        setS(st);
        const now = performance.now();
        if (now - last.current > 80) {
          last.current = now;
          setLv((x) => [...x.slice(1), st.mic ? Math.min(1, st.level) : 0]);
        }
      }),
    );
  }, []);

  useEffect(() => {
    if (s.phase === "recording" && inTauri)
      settingsGet()
        .then((c) => setKeys(c.hotkeys))
        .catch(() => undefined);
  }, [s.phase === "recording"]);

  if (s.phase === "transcribing" || (s.phase === "ready" && s.notice)) return <Done s={s} />;

  const waiting = s.phase === "loading";
  const tip = (name: string, label?: string) => (label ? `${name} (${chordText(label)})` : name);
  const problem = s.error;
  const mic = s.phase === "recording" && s.quiet_mic ? micNotice(s.quiet_mic) : null;

  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div className="hud" data-tauri-drag-region role="toolbar" aria-label="Pointcast recording">
        <span className={`rec ${waiting ? "wait" : ""}`} data-tauri-drag-region />
        {mic ? null : (
          <span className="time" data-tauri-drag-region>
            {stamp(s.duration_ms)}
          </span>
        )}
        {problem ? (
          <span className="note" title={problem} data-tauri-drag-region>
            {problem}
          </span>
        ) : mic ? (
          <MicNote notice={mic} />
        ) : (
          <span className="wave" aria-hidden="true" data-tauri-drag-region>
            {lv.map((v, i) => (
              <i key={i} style={{ height: `${Math.max(3, v * 22)}px`, opacity: 0.45 + v * 0.55 }} />
            ))}
          </span>
        )}
        <span className="sep" />
        <button
          type="button"
          className="hb"
          title={tip("Mark up a region", keys?.region.label)}
          aria-label="Mark up a region"
          onClick={() => regionBegin().catch(() => undefined)}
        >
          <SquareDashedMousePointer size={18} strokeWidth={1.75} />
          {s.image_count ? <span className="n">{s.image_count}</span> : null}
        </button>
        <button
          type="button"
          className="hb"
          title={tip("Capture the screen", keys?.full.label)}
          aria-label="Capture the screen"
          onClick={() => captureFull().catch(() => undefined)}
        >
          <Monitor size={18} strokeWidth={1.75} />
        </button>
        <button
          type="button"
          className="hb"
          title={tip("Capture around the pointer", keys?.zoom.label)}
          aria-label="Capture around the pointer"
          onClick={() => captureZoom().catch(() => undefined)}
        >
          <ZoomIn size={18} strokeWidth={1.75} />
        </button>
        <span className="sep" />
        <button
          type="button"
          className="hb"
          title="Discard"
          aria-label="Discard this recording"
          onClick={() => sessionCancel().catch(() => undefined)}
        >
          <X size={17} strokeWidth={2} />
        </button>
        <button
          type="button"
          className="stop"
          title={tip("Finish", keys?.session.label)}
          aria-label="Finish recording"
          onClick={() => sessionToggle().catch(() => undefined)}
        >
          <i />
        </button>
      </div>
    </div>
  );
}

/** The microphone warning takes the place of the time and the wave, so the buttons stay where they are. */
function MicNote({ notice }: { notice: ReturnType<typeof micNotice> }) {
  return (
    <span className="note warn" role="status" title={notice.full} aria-label={notice.full} data-tauri-drag-region>
      <b data-tauri-drag-region>{notice.lines[0]}</b>
      <span data-tauri-drag-region>{notice.lines[1]}</span>
    </span>
  );
}

/** Seconds left on the part of the recording still to be written down, counted down between updates. */
function useLeft(behind: number) {
  const [left, setLeft] = useState(behind);
  useEffect(() => {
    const from = Date.now();
    setLeft(behind);
    if (behind <= 0) return;
    const id = setInterval(() => setLeft(Math.max(0, behind - (Date.now() - from))), 500);
    return () => clearInterval(id);
  }, [behind]);
  return left;
}

const leftText = (ms: number) => (ms <= 90_000 ? `${Math.max(1, Math.ceil(ms / 1000))} s left` : `${Math.ceil(ms / 60_000)} min left`);

/** After stop: a short pill that says where the prompt went, then the window hides itself. */
function Done({ s }: { s: UiState }) {
  const paste = platform() === "mac" ? "⌘V" : "Ctrl+V";
  const behind = s.phase === "transcribing" ? (s.behind_ms ?? 0) : 0;
  const left = useLeft(behind);
  const text =
    s.phase === "transcribing"
      ? behind > 0
        ? left > 0
          ? `Finishing the transcript · ${leftText(left)}`
          : "Finishing the transcript"
        : "Finishing…"
      : s.notice === "pasted"
        ? "Pasted"
        : s.notice === "copied"
          ? `Copied · ${paste} to paste`
          : s.notice === "saved"
            ? "Saved to your folder"
            : s.error || "Nothing was recorded";
  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div className="hud done" role="status" aria-live="polite" data-tauri-drag-region>
        {s.phase === "transcribing" ? (
          <span className="spinner on-hud" />
        ) : (
          <span className={`done-mark ${s.notice === "empty" ? "none" : ""}`}>
            <Check size={13} strokeWidth={3} />
          </span>
        )}
        <span style={{ fontVariantNumeric: "tabular-nums" }}>{text}</span>
      </div>
    </div>
  );
}
