import { useEffect, useRef, useState } from "react";
import { Check, Monitor, SquareDashedMousePointer, TriangleAlert, X, ZoomIn } from "lucide-react";
import {
  captureFull,
  captureZoom,
  historyOpen,
  historyRetranscribe,
  hudHold,
  inTauri,
  onState,
  permissionRequest,
  preview,
  regionBegin,
  sessionCancel,
  sessionFinishNow,
  sessionState,
  sessionToggle,
  settingsGet,
  stamp,
  subscribe,
  type Config,
  type UiState,
} from "@/lib/api";
import { discardClick, finishingLine, micCaption, noticeView } from "@/lib/capsule";
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

/** The state the window was built for, set by the app before the page loads, so a freshly built capsule's first frame
 * is the true one ("Getting ready…" for a recording just started) and never a recording at 0:00. */
function built(): UiState {
  const st = (window as { __POINTCAST_HUD__?: Partial<UiState> }).__POINTCAST_HUD__;
  return { ...idle, phase: "loading", ...st };
}

/** What the capsule says while a recording gets ready: before the microphone is live nothing is recorded (a neutral
 * dot, no clock); once it is live but the speech model still loads, audio is kept and transcribed after, so it says so. */
export function readyNote(s: UiState): { lines: [string, string?]; full: string } | null {
  const live = s.mic;
  const loading = s.phase === "loading";
  if (!(s.phase === "loading" || s.phase === "recording")) return null;
  if (live && !loading) return null;
  if (s.preparing_gpu) {
    // The full sentence is too wide for the capsule's two short lines; it is the tooltip and what a screen reader says.
    const lines: [string, string] = ["Setting up graphics card", live ? "keep talking" : "(first time only)"];
    return { lines, full: `Setting up your graphics card (first time only)${live ? ", keep talking" : ""}` };
  }
  if (live) return { lines: ["Getting ready…", "keep talking"], full: "Getting ready… keep talking" };
  return { lines: ["Getting ready…"], full: "Getting ready…" };
}

/** The recording HUD: a small capsule that never takes focus from the app you are talking about. */
export function Indicator() {
  const [s, setS] = useState<UiState>(inTauri ? built : idle);
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
      // ?phase=transcribing&behind=40000, ?phase=transcribing&progress=42[&stalled=1][&engine=gpu|cpu], ?phase=ready&notice=partial
      // [&blocked=1 for a copy auto-paste could not make] or ?quiet=silent|quiet, ?pics=0 and ?discard=1 (asking) to look at the
      // other states; ?hud=ready (microphone not live yet), ?hud=gpu (and the graphics card being set up), ?hud=talk (live,
      // model still loading), ?hud=gpu-talk for the start of a recording.
      const q = new URLSearchParams(location.search);
      const quiet = q.get("quiet");
      const phase = q.get("phase");
      const pct = q.get("progress");
      const stalled = q.get("stalled") === "1";
      const engine = q.get("engine");
      const hud = q.get("hud");
      const starting = hud === "ready" || hud === "gpu" || hud === "talk" || hud === "gpu-talk";
      const live = !starting || hud === "talk" || hud === "gpu-talk";
      setS({
        ...idle,
        phase: phase === "transcribing" || phase === "ready" ? phase : starting ? "loading" : "recording",
        mic: live,
        preparing_gpu: hud === "gpu" || hud === "gpu-talk",
        duration_ms: live ? (starting ? 2_000 : 16_000) : 0,
        image_count: q.get("pics") == null ? 2 : Number(q.get("pics")),
        level: 0.4,
        notice: (q.get("notice") as UiState["notice"]) ?? null,
        entry: "preview",
        paste_blocked: q.get("blocked") === "1",
        behind_ms: Number(q.get("behind")) || 0,
        quiet_mic: quiet === "silent" || quiet === "quiet" ? quiet : null,
        engine_label: engine === "gpu" ? "your graphics card" : engine === "cpu" ? "your processor" : null,
        progress:
          pct == null
            ? null
            : { done_ms: Number(pct) * 600, total_ms: 60_000, percent: Number(pct), stalled },
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

  if (s.phase === "transcribing") return <Finishing s={s} />;
  if (s.phase === "ready" && s.notice) return <Done s={s} />;

  return <Recording s={s} keys={keys} lv={lv} />;
}

/** The recording row. A quiet microphone keeps the timer and the bars (flat bars are the evidence) and says so under
 * them; Discard asks once before it throws the recording away. */
function Recording({ s, keys, lv }: { s: UiState; keys: Config["hotkeys"] | null; lv: number[] }) {
  const [armed, setArmed] = useState<number | null>(() =>
    preview && new URLSearchParams(location.search).get("discard") === "1" ? Number.MAX_SAFE_INTEGER : null,
  );
  useEffect(() => {
    if (armed == null) return;
    const id = setTimeout(() => setArmed(null), Math.min(armed - Date.now(), 2 ** 31 - 1));
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setArmed(null);
    window.addEventListener("keydown", esc);
    return () => {
      clearTimeout(id);
      window.removeEventListener("keydown", esc);
    };
  }, [armed]);
  const discard = () => {
    const r = discardClick(armed, Date.now());
    setArmed(r.armedUntil);
    if (r.discard) sessionCancel().catch(() => undefined);
  };

  const tip = (name: string, label?: string) => (label ? `${name} (${chordText(label)})` : name);
  const problem = s.error;
  const ready = readyNote(s);
  // Nothing is recorded until the microphone's first samples arrive: no red dot and no clock before that.
  const live = s.mic;
  const quiet = s.phase === "recording" && live && s.quiet_mic && !problem && !ready ? s.quiet_mic : null;
  const mic = quiet ? micNotice(quiet) : null;
  const n = s.image_count;

  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div className={`hud ${mic ? "warned" : ""}`} data-tauri-drag-region role="toolbar" aria-label="Pointcast recording">
        <span className={`rec ${live ? "" : "ready"}`} data-tauri-drag-region />
        {ready || !live ? null : (
          <span className="time" data-tauri-drag-region>
            {stamp(s.duration_ms)}
          </span>
        )}
        {problem ? (
          <span className="note" title={problem} data-tauri-drag-region>
            {problem}
          </span>
        ) : ready ? (
          <span className={`note getting ${ready.lines[1] ? "two" : ""}`} role="status" aria-label={ready.full} title={ready.full} data-tauri-drag-region>
            <b data-tauri-drag-region>{ready.lines[0]}</b>
            {ready.lines[1] ? <span data-tauri-drag-region>{ready.lines[1]}</span> : null}
          </span>
        ) : (
          <span className="wave" aria-hidden="true" data-tauri-drag-region>
            {lv.map((v, i) => (
              <i key={i} style={{ height: `${Math.max(3, v * 22)}px`, opacity: 0.45 + v * 0.55 }} />
            ))}
          </span>
        )}
        {mic && quiet ? (
          <span className="cap warn" role="status" title={mic.full} aria-label={mic.full} data-tauri-drag-region>
            {micCaption(quiet)}
          </span>
        ) : null}
        <span className="sep" />
        <button
          type="button"
          className="hb"
          title={tip("Mark up a region", keys?.region.label)}
          aria-label="Mark up a region"
          onClick={() => regionBegin().catch(() => undefined)}
        >
          <SquareDashedMousePointer size={18} strokeWidth={1.75} />
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
        {/* While recording the slot is always there, so the first picture does not move the buttons. Getting ready has
            no room for it at 392 px: the count shows once recording runs. */}
        {!ready ? (
          <span
            className="count"
            style={{ visibility: n ? "visible" : "hidden" }}
            title={`${n} ${n === 1 ? "picture" : "pictures"} in this prompt`}
            aria-label={`${n} ${n === 1 ? "picture" : "pictures"}`}
            data-tauri-drag-region
          >
            {n || 0}
          </span>
        ) : null}
        <span className="sep" />
        <span className="discard-slot">
          <button
            type="button"
            className={`hb discard ${armed != null ? "armed" : ""}`}
            title={armed != null ? "Click again to discard (Esc keeps it)" : "Discard"}
            aria-label={armed != null ? "Click again to discard this recording" : "Discard this recording"}
            onClick={discard}
          >
            {armed != null ? "Discard?" : <X size={17} strokeWidth={2} />}
          </button>
        </span>
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

/** After stop, while the last words are written down: how far that is on a determinate bar, and Paste what's done to stop
 * waiting, offered from the start and in the accent once nothing has moved for 20 s. */
function Finishing({ s }: { s: UiState }) {
  const p = s.progress ?? null;
  const [cutting, setCutting] = useState(false);
  const pct = p ? Math.min(100, Math.max(0, Math.round(p.percent))) : null;
  const stalled = !!p?.stalled;
  const text = finishingLine(p, cutting);
  const cancel = () => {
    setCutting(true);
    if (inTauri) sessionFinishNow().catch(() => setCutting(false));
  };
  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div className="hud finishing" data-tauri-drag-region>
        <div className="fin-main" data-tauri-drag-region>
          <span className="fin-text" role="status" data-tauri-drag-region>
            {text}
          </span>
          <div
            className={`progress on-hud ${pct == null ? "indet" : ""}`}
            role="progressbar"
            aria-label="Transcribing"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={pct ?? undefined}
            data-tauri-drag-region
          >
            <i style={pct == null ? undefined : { width: `${pct}%` }} />
          </div>
        </div>
        <button
          type="button"
          className={`fin-cancel ${stalled ? "accent" : ""}`}
          disabled={cutting}
          title="Stop waiting: paste what is written so far and keep the audio in History"
          onClick={cancel}
        >
          Paste what's done
        </button>
      </div>
    </div>
  );
}

/** After stop: a short pill that says where the prompt went, then the window hides itself. The pill is a button: it opens
 * History on the prompt just saved (or does what its second line says), and stays while the pointer is on it. */
function Done({ s }: { s: UiState }) {
  const entry = s.entry ?? null;
  const v = noticeView(s.notice ?? "empty", { mac: platform() === "mac", pasteBlocked: !!s.paste_blocked, entry, error: s.error });
  const held = useRef({ pointer: false, focus: false });
  const hold = (k: "pointer" | "focus", on: boolean) => {
    held.current[k] = on;
    if (inTauri) hudHold(held.current.pointer || held.current.focus).catch(() => undefined);
  };
  const act = () => {
    if (!inTauri) return;
    if (v.action === "permission") permissionRequest("accessibility").catch(() => undefined);
    else if (v.action === "transcribe" && entry)
      historyOpen(entry)
        .then(() => historyRetranscribe(entry))
        .catch(() => undefined);
    else historyOpen(entry).catch(() => undefined);
  };
  const [first, second] = v.lines;
  const mark = (
    <span className={`done-mark ${v.mark}`} aria-hidden="true">
      {v.mark === "warn" ? <TriangleAlert size={18} strokeWidth={2.25} /> : <Check size={13} strokeWidth={3} />}
    </span>
  );
  const text = second ? (
    <span className="lines">
      <b>{first}</b>
      <span className="act">{second}</span>
    </span>
  ) : (
    <span>{first}</span>
  );
  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div role="status" aria-live="polite">
        {v.action ? (
          <button
            type="button"
            className={`hud done ${second ? "two" : ""}`}
            title={second ? undefined : "Open in History"}
            onClick={act}
            onPointerEnter={() => hold("pointer", true)}
            onPointerLeave={() => hold("pointer", false)}
            onFocus={() => hold("focus", true)}
            onBlur={() => hold("focus", false)}
          >
            {mark}
            {text}
          </button>
        ) : (
          <div className="hud done" data-tauri-drag-region>
            {mark}
            {text}
          </div>
        )}
      </div>
    </div>
  );
}
