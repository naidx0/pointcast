import { useEffect, useRef, useState } from "react";
import { PressButton } from "@/components/PressButton";
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
import { NO_PEEK, READY_WAIT_MS, capsuleLine, discardClick, finishingLine, micCaption, noticeView, peekStep, readyNote, timeShown, type PeekEvent } from "@/lib/capsule";
import { Line, type LevelSource } from "@/components/Line";
import { chordText } from "@/lib/keys";
import { LEVEL_FIELDS, levelOf, previewLevel } from "@/lib/live";
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

/** The capsule's line hears the session's state stream as Ask's line does: `loudness` first (0 while the microphone is
 * not live), so one voice moves both lines alike. */
const sinks = new Set<(raw: number) => void>();
const hudLevels: LevelSource = (set) => {
  sinks.add(set);
  return () => void sinks.delete(set);
};
/** The line's height in the 48 px capsule, and its width per stroke: narrower than Ask's, for a shorter line. */
const LINE_H = 28;
const LINE_DX = 11;

/** The state the window was built for, set by the app before the page loads, so a freshly built capsule's first frame
 * is the true one ("Getting ready…" for a recording just started) and never a recording at 0:00. */
function built(): UiState {
  const st = (window as { __POINTCAST_HUD__?: Partial<UiState> }).__POINTCAST_HUD__;
  return { ...idle, phase: "loading", ...st };
}

/** The recording HUD: a small capsule that never takes focus from the app you are talking about. */
export function Indicator() {
  const [s, setS] = useState<UiState>(inTauri ? built : idle);
  const [keys, setKeys] = useState<Config["hotkeys"] | null>(null);

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
      // A made-up voice for the line, at the engine's rate.
      let i = 0;
      const id = setInterval(() => sinks.forEach((f) => f(previewLevel(i++))), 33);
      return () => clearInterval(id);
    }
    return subscribe(
      onState((st) => {
        setS(st);
        const raw = levelOf(st, LEVEL_FIELDS);
        sinks.forEach((f) => f(raw));
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

  return <Recording s={s} keys={keys} />;
}

/** The recording row: the red dot, the line the voice draws (the time over its faded end on hover, and from a minute
 * on), the picture tools, Discard and Finish. A quiet microphone keeps the time and the line (a flat line is the
 * evidence) and says so under them; Discard asks once before it throws the recording away. */
function Recording({ s, keys }: { s: UiState; keys: Config["hotkeys"] | null }) {
  // How long this recording has waited for its microphone: a normal start is never said, a slow one is.
  const [waited, setWaited] = useState(0);
  const waiting = (s.phase === "loading" || s.phase === "recording") && !s.mic;
  useEffect(() => {
    setWaited(0);
    if (!waiting) return;
    const t = setTimeout(() => setWaited(READY_WAIT_MS), READY_WAIT_MS);
    return () => clearTimeout(t);
  }, [waiting]);
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
  const ready = readyNote(s, waited);
  // Nothing is recorded until the microphone's first samples arrive: no red dot and no clock before that.
  const live = s.mic;
  const quiet = s.phase === "recording" && live && s.quiet_mic && !problem && !ready ? s.quiet_mic : null;
  const mic = quiet ? micNotice(quiet) : null;
  const n = s.image_count;
  // Whether the person looks at the time (peekStep): forgotten when the window hides and when a recording starts.
  const [peek, setPeek] = useState(NO_PEEK);
  const look = (e: PeekEvent) => setPeek((p) => peekStep(p, e));
  useEffect(() => {
    const vis = () => document.hidden && look("hidden");
    document.addEventListener("visibilitychange", vis);
    return () => document.removeEventListener("visibilitychange", vis);
  }, []);
  useEffect(() => {
    if (live) look("start");
  }, [live]);
  const timed = timeShown({ live, durationMs: s.duration_ms, warned: !!mic, peek });

  return (
    <div className="hud-wrap" data-tauri-drag-region>
      <div
        className={`hud ${mic ? "warned" : ""}`}
        data-tauri-drag-region
        role="toolbar"
        aria-label="Pointcast recording"
        onPointerEnter={() => look("enter")}
        onPointerLeave={() => look("leave")}
        onFocus={(e) => e.target instanceof Element && e.target.matches(":focus-visible") && look("key-focus")}
        onBlur={(e) => !(e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget)) && look("blur")}
      >
        <span className={`rec ${live ? "" : "ready"}`} data-tauri-drag-region />
        {problem ? (
          <>
            {live ? (
              <span className="time" data-tauri-drag-region>
                {stamp(s.duration_ms)}
              </span>
            ) : null}
            <span className="note" title={problem} data-tauri-drag-region>
              {problem}
            </span>
          </>
        ) : ready ? (
          <span className={`note getting ${ready.lines[1] ? "two" : ""}`} role="status" aria-label={ready.full} title={ready.full} data-tauri-drag-region>
            <b data-tauri-drag-region>{ready.lines[0]}</b>
            {ready.lines[1] ? <span data-tauri-drag-region>{ready.lines[1]}</span> : null}
          </span>
        ) : (
          <span className={`scribe${timed ? " timed" : ""}`} data-tauri-drag-region>
            <Line mode={capsuleLine(s)} height={LINE_H} dx={LINE_DX} tip="ink" levels={hudLevels} />
            {live ? (
              <span className="time" data-tauri-drag-region>
                {stamp(s.duration_ms)}
              </span>
            ) : null}
          </span>
        )}
        {mic && quiet ? (
          <span className="cap warn" role="status" title={mic.full} aria-label={mic.full} data-tauri-drag-region>
            {micCaption(quiet)}
          </span>
        ) : null}
        <span className="sep" />
        <PressButton
          className="hb"
          title={tip("Mark up a region", keys?.region.label)}
          aria-label="Mark up a region"
          act={() => regionBegin().catch(() => undefined)}
        >
          <SquareDashedMousePointer size={18} strokeWidth={1.75} />
        </PressButton>
        <PressButton
          className="hb"
          title={tip("Capture the screen", keys?.full.label)}
          aria-label="Capture the screen"
          act={() => captureFull().catch(() => undefined)}
        >
          <Monitor size={18} strokeWidth={1.75} />
        </PressButton>
        <PressButton
          className="hb"
          title={tip("Capture around the pointer", keys?.zoom.label)}
          aria-label="Capture around the pointer"
          act={() => captureZoom().catch(() => undefined)}
        >
          <ZoomIn size={18} strokeWidth={1.75} />
        </PressButton>
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
          <PressButton
            className={`hb discard ${armed != null ? "armed" : ""}`}
            title={armed != null ? "Click again to discard (Esc keeps it)" : "Discard"}
            aria-label={armed != null ? "Click again to discard this recording" : "Discard this recording"}
            act={discard}
          >
            {armed != null ? "Discard?" : <X size={17} strokeWidth={2} />}
          </PressButton>
        </span>
        <PressButton
          className="stop"
          title={tip("Finish", keys?.session.label)}
          aria-label="Finish recording"
          act={() => sessionToggle().catch(() => undefined)}
        >
          <i />
        </PressButton>
      </div>
    </div>
  );
}

/** After stop, while the last words are written down: the line loops (in the warning colour once nothing has moved for
 * 20 s), the words say how far it is, and Paste what's done stops waiting, offered from the start and in the accent
 * once it stalls. */
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
        <span
          className="fin-line"
          role="progressbar"
          aria-label="Transcribing"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={pct ?? undefined}
          data-tauri-drag-region
        >
          <Line mode={capsuleLine(s)} height={LINE_H - 4} tone={stalled ? "warn" : "ink"} loops={3} pitch={13} lead={4} />
        </span>
        <span className="fin-text" role="status" data-tauri-drag-region>
          {text}
        </span>
        <PressButton
          className={`fin-cancel ${stalled ? "accent" : ""}`}
          disabled={cutting}
          title="Stop waiting: paste what is written so far and keep the audio in History"
          act={cancel}
        >
          Paste what's done
        </PressButton>
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
          <PressButton
            className={`hud done ${second ? "two" : ""}`}
            title={second ? undefined : "Open in History"}
            act={act}
            onPointerEnter={() => hold("pointer", true)}
            onPointerLeave={() => hold("pointer", false)}
            onFocus={() => hold("focus", true)}
            onBlur={() => hold("focus", false)}
          >
            {mark}
            {text}
          </PressButton>
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
