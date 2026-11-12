import { useEffect, useRef, useState, type ReactNode } from "react";
import { AudioLines, Check, ChevronRight, ClipboardPaste, MessageCircleQuestion, Mic, MousePointer2, Play, ShieldCheck } from "lucide-react";
import { ACTIONS, ASK_ACTION, Keycaps, ShortcutField } from "@/components/Shortcut";
import { PermissionRows, usePermissions } from "@/components/Permissions";
import { ModelPicker, modelSize, speechInstalled, useSpeechJob, type SpeechJob } from "@/components/Models";
import { STEPS, downloadOnce, nextButton, percent, speechLine, type SpeechLine, type Step } from "@/lib/setup";
import { askReady, chosenRoute, shortName } from "@/lib/assist";
import {
  assistProviders,
  inTauri,
  loginSet,
  onboardingFinish,
  settingsOpenOn,
  onHotkey,
  onPracticeResult,
  onState,
  practice,
  preview,
  subscribe,
  type Action,
  type Config,
  type SessionExport,
  type UiState,
} from "@/lib/api";
import { useConfig } from "@/lib/useConfig";
import { chordText } from "@/lib/keys";
import { micNotice } from "@/lib/mic";
import { platform } from "@/lib/theme";
import { PromptDoc } from "./Result";

export function Onboarding() {
  const { cfg, setCfg, patch, refresh, error } = useConfig();
  // The recommended model downloads in the background from Get Started on; no step waits for it.
  const speech = useSpeechJob(refresh);
  const askDownload = useRef(downloadOnce()).current;
  const [step, setStep] = useState<Step>(() => {
    const q = new URLSearchParams(location.search).get("step");
    return (STEPS as readonly string[]).includes(q ?? "") ? (q as Step) : "welcome";
  });
  // "Change model…" shows the model cards in place of the step, until Done.
  const [models, setModels] = useState(() => preview && new URLSearchParams(location.search).get("models") === "1");
  const [practiced, setPracticed] = useState(false);
  const i = STEPS.indexOf(step);
  const go = (d: number) => setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, i + d))]);

  // The practice run is armed only on Try it.
  useEffect(() => {
    if (!inTauri) return;
    practice(step === "try").catch(() => undefined);
  }, [step]);

  // Reopened for an existing user (a macOS update dropped a permission): go straight to access.
  useEffect(() => {
    if (cfg?.onboarded) setStep((s) => (s === "welcome" ? "access" : s));
  }, [cfg?.onboarded]);

  // Browser preview: `?speech=ready` shows the recommended model already in place.
  const seeded = !!cfg;
  useEffect(() => {
    if (!preview || !cfg || new URLSearchParams(location.search).get("speech") !== "ready") return;
    patch({
      speech: {
        runtime: { id: "runtime", installed: true, lib: "" },
        models: cfg.speech.models?.map((m) => (m.id === cfg.recommended ? { ...m, installed: true } : m)),
      },
      speech_check: { ok: true, error: null },
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [seeded]);

  if (!cfg) return <div className="onb" />;

  const n = nextButton(step, practiced);
  const line = speechLine({ ready: speechInstalled(cfg), busy: !!speech.job, error: speech.error?.text ?? null });
  const getSpeech = () => speech.start(cfg.recommended, cfg, patch);

  return (
    <div className="onb">
      <div className="onb-body drag">
        {models ? (
          <ModelsStep cfg={cfg} patch={patch} refresh={refresh} speech={speech} />
        ) : (
          <>
            {step === "welcome" && <Welcome />}
            {step === "access" && <AccessStep cfg={cfg} />}
            {step === "try" && (
              <TryStep
                cfg={cfg}
                patch={patch}
                setCfg={setCfg}
                speech={line}
                onModels={() => setModels(true)}
                onResult={() => setPracticed(true)}
              />
            )}
            {step === "ask" && <AskStep cfg={cfg} patch={patch} />}
            {step === "done" && <DoneStep cfg={cfg} patch={patch} />}
          </>
        )}
        {error ? (
          <div className="t-small" role="alert" style={{ color: "var(--red-ink)", marginTop: 12, maxWidth: 440 }}>
            {error.replace(/^Error:\s*/, "")}
          </div>
        ) : null}
      </div>
      <footer className="onb-foot">
        <div className="dots" aria-label={`Step ${i + 1} of ${STEPS.length}`}>
          {STEPS.map((s) => (
            <i key={s} className={s === step ? "on" : ""} />
          ))}
        </div>
        {step !== "welcome" ? (
          <SpeechStatus
            cfg={cfg}
            line={line}
            speech={speech}
            onRetry={getSpeech}
            onModels={models ? undefined : () => setModels(true)}
          />
        ) : null}
        {models ? (
          <button type="button" className="btn primary lg" onClick={() => setModels(false)}>
            Done
          </button>
        ) : (
          <>
            {i > 0 && step !== "done" ? (
              <button type="button" className="btn plain" onClick={() => go(-1)}>
                Back
              </button>
            ) : null}
            {n.skip && n.enabled === false ? (
              <button type="button" className="btn" onClick={() => go(1)}>
                {n.skip}
              </button>
            ) : null}
            <button
              type="button"
              className="btn primary lg"
              disabled={n.enabled === false}
              onClick={async () => {
                if (step === "welcome")
                  askDownload({ installed: speechInstalled(cfg), busy: !!speech.job }, () => void getSpeech());
                if (step !== "done") return go(1);
                if (inTauri) await onboardingFinish().catch(() => undefined);
              }}
            >
              {n.label}
            </button>
          </>
        )}
      </footer>
    </div>
  );
}

/** The footer's one quiet line about the speech model, on every step after the welcome. */
function SpeechStatus({
  cfg,
  line,
  speech,
  onRetry,
  onModels,
}: {
  cfg: Config;
  line: SpeechLine;
  speech: SpeechJob;
  onRetry: () => void;
  onModels?: () => void;
}) {
  const job = speech.job;
  const size = modelSize(cfg, job?.id ?? cfg.recommended);
  const pct = job?.phase === "download" ? percent(job.received, job.total) : null;
  const offline = typeof navigator !== "undefined" && navigator.onLine === false;
  return (
    <div className="onb-speech t-small" role="status" title={line === "failed" ? speech.error?.text : undefined}>
      {line === "downloading" ? (
        <>
          <span className="spinner" aria-hidden />
          <span>
            Getting speech ready
            {job?.phase !== "download" ? (
              " · Checking it works"
            ) : pct != null ? (
              <span aria-hidden>{` · ${pct}%${size ? ` of ${size}` : ""}`}</span>
            ) : size ? (
              ` · ${size}`
            ) : null}
          </span>
        </>
      ) : line === "ready" ? (
        <>
          <span className="onb-speech-ok" aria-hidden>
            <Check size={10} strokeWidth={3.4} />
          </span>
          <span>Speech is ready</span>
        </>
      ) : line === "failed" ? (
        <span className="onb-speech-warn">
          {offline ? "No internet, so speech didn’t download." : "Speech didn’t download."}{" "}
          <button type="button" className="link" onClick={onRetry}>
            Retry
          </button>
        </span>
      ) : (
        <span>No speech model yet</span>
      )}
      {onModels ? (
        <>
          <span aria-hidden>·</span>
          <button type="button" className="link" onClick={onModels}>
            Change model…
          </button>
        </>
      ) : null}
    </div>
  );
}

function AppIcon() {
  return <img className="onb-icon" src={platform() === "mac" ? "/icon-mac.png" : "/icon-win.png"} alt="" draggable={false} />;
}

/** A step's mark: a line icon in the text colour, never a tinted tile. */
function Glyph({ children }: { children: ReactNode }) {
  return (
    <span className="onb-glyph" aria-hidden="true">
      {children}
    </span>
  );
}

function Welcome() {
  const points = [
    { icon: Mic, t: "Talk", s: "Say what you see. Speech is turned into text on this computer." },
    { icon: MousePointer2, t: "Point", s: "Box, pin, arrow or zoom anything on screen, in any app." },
    { icon: ClipboardPaste, t: "Paste", s: "Get one prompt with your words and pictures for ChatGPT, Claude, Cursor or any chat." },
  ];
  return (
    <div className="onb-step">
      <AppIcon />
      <h1 className="t-hero">Welcome to Pointcast</h1>
      <p className="t-body">Show an AI what you mean instead of describing it. Setup takes about a minute.</p>
      <ul className="onb-points">
        {points.map((p) => (
          <li key={p.t}>
            <p.icon size={20} strokeWidth={1.5} aria-hidden="true" />
            <div>
              <div className="onb-point-t">{p.t}</div>
              <div className="t-caption">{p.s}</div>
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

/** One calm step for Ask: what it does, its key, and where its AI is chosen. Nothing here is required. */
function AskStep({ cfg, patch }: { cfg: Config; patch: (p: Record<string, unknown>) => Promise<string | null> }) {
  // Seen here, Ask needs no card on Home later.
  useEffect(() => {
    if (!cfg.ask_intro_seen) patch({ ask_intro_seen: true }).catch(() => undefined);
  }, []);
  // The AI that answers, by its short name; "" when none is chosen, null until the engine says. Read again when the
  // window comes back from Settings.
  const [ai, setAi] = useState<string | null>(null);
  useEffect(() => {
    const load = () =>
      assistProviders()
        .then((a) => {
          const r = askReady(a) ? chosenRoute(a) : null;
          setAi(r ? shortName(r) : "");
        })
        .catch(() => setAi(""));
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, []);
  const key = cfg.hotkeys.ask?.label;
  return (
    <div className="onb-step">
      <Glyph>
        <MessageCircleQuestion size={32} strokeWidth={1.5} />
      </Glyph>
      <h1 className="t-title">Ask about your screen</h1>
      <p className="t-body">Press the key and ask out loud about what you see. The answer opens beside your work.</p>
      <div className="onb-rows">
        <div className="onb-row">
          <span className="lbl">Ask</span>
          {key ? <Keycaps label={key} /> : <span className="t-caption">Set a key in Settings › Shortcuts</span>}
        </div>
        <div className="onb-row">
          <span className="lbl">AI</span>
          {ai == null ? null : (
            <button
              type="button"
              className={`link onb-ai${ai ? "" : " strong"}`}
              aria-label={ai ? `Answers come from ${ai}. Change it in Settings` : "Choose an AI in Settings"}
              onClick={() => void settingsOpenOn("ai").catch(() => undefined)}
            >
              {ai || "Choose an AI"}
              <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
          )}
        </div>
      </div>
      {key && ai ? (
        <p className="t-body onb-try">
          Try it: press <Keycaps label={key} /> and ask what is on your screen.
        </p>
      ) : null}
      <p className="t-caption onb-later">{ai ? "You can change it any time in Settings." : "You can skip this and choose one later in Settings."}</p>
    </div>
  );
}

function AccessStep({ cfg }: { cfg: Config }) {
  const [perms, setPerms] = usePermissions(cfg.permissions);
  const mac = platform() === "mac";
  return (
    <div className="onb-step">
      <Glyph>
        <ShieldCheck size={32} strokeWidth={1.5} />
      </Glyph>
      <h1 className="t-title">Allow access</h1>
      <p className="t-body">
        Pointcast needs to hear you and see what you point at.{" "}
        {mac ? "You can change these later in System Settings → Privacy & Security." : "You can change this later in Settings → Privacy."}
      </p>
      {perms ? <PermissionRows perms={perms} setPerms={setPerms} meter /> : null}
    </div>
  );
}

function ModelsStep({
  cfg,
  patch,
  refresh,
  speech,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  refresh: () => void;
  speech: SpeechJob;
}) {
  return (
    <div className="onb-step">
      <Glyph>
        <AudioLines size={32} strokeWidth={1.5} />
      </Glyph>
      <h1 className="t-title">Speech model</h1>
      <p className="t-body">
        Speech turns into text right on this computer: no account, no upload. The recommended one suits this computer.
      </p>
      <ModelPicker cfg={cfg} patch={patch} refresh={refresh} speech={speech} />
    </div>
  );
}

/** What was said, without the [image n] markers captures add to the running text. */
const words = (t: string) => t.replace(/\[image[^\]]*\]/g, " ").replace(/\s+/g, " ").trim();

/** One practice run with the shortcuts on cards: each card ticks off when its shortcut is used in the run, and its
 * Change opens the shortcut recorder. */
function TryStep({
  cfg,
  patch,
  setCfg,
  speech,
  onModels,
  onResult,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  setCfg: (c: Config) => void;
  speech: SpeechLine;
  onModels: () => void;
  onResult: () => void;
}) {
  const [st, setSt] = useState<UiState | null>(null);
  const [result, setResult] = useState<SessionExport | null>(null);
  const [fired, setFired] = useState<Partial<Record<Action, boolean>>>({});
  const [down, setDown] = useState<Action | null>(null);
  // `quiet`: what the microphone gave during this run, the worse of what was reported.
  const [seen, setSeen] = useState<{ started: boolean; spoke: boolean; quiet: UiState["quiet_mic"] }>({
    started: false,
    spoke: false,
    quiet: null,
  });
  useEffect(() => {
    // ?quiet=silent|quiet, ?result=empty and ?tried=session,region show those states in a browser.
    if (preview) {
      const q = new URLSearchParams(location.search);
      const quiet = q.get("quiet") === "silent" || q.get("quiet") === "quiet" ? (q.get("quiet") as "silent" | "quiet") : null;
      const tried = (q.get("tried") ?? "").split(",").filter((a): a is Action => ACTIONS.some((x) => x.id === a));
      if (tried.length) setFired(Object.fromEntries(tried.map((a) => [a, true])));
      if (q.get("result") === "empty") {
        setSeen({ started: true, spoke: false, quiet });
        setResult({ plain: "", markdown: "", html: "", images: [] });
        onResult();
      } else if (quiet) {
        setSt({ ...PREVIEW_STATE, quiet_mic: quiet });
        setSeen({ started: true, spoke: false, quiet });
      }
    }
    return subscribe(
      onState((s) => {
        setSt(s);
        setSeen((x) => ({
          started: x.started || s.phase === "recording",
          spoke: x.spoke || !!words(s.partial || s.text),
          quiet: x.quiet === "silent" ? "silent" : (s.quiet_mic ?? x.quiet),
        }));
      }),
      onPracticeResult((r) => {
        setResult(r);
        onResult();
      }),
      onHotkey((h) => {
        if (h.pressed) {
          setFired((f) => ({ ...f, [h.action]: true }));
          setDown(h.action);
        } else setDown(null);
      }),
    );
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  const session = chordText(cfg.hotkeys.session.label);
  const region = chordText(cfg.hotkeys.region.label);
  const rec = st?.phase === "recording" || st?.phase === "loading";
  // Discarded (or gone without a result): offer another run instead of "Finishing…" for ever.
  const dropped = seen.started && !rec && !result && (st?.phase === "idle" || st?.phase === "error");
  const empty = !!result && !words(result.plain) && !result.images.length;
  // A couple of words and no pictures: too little to be what was said.
  const tiny = !!result && !empty && !result.images.length && words(result.plain).split(" ").length <= 2;
  const quiet = seen.quiet ? micNotice(seen.quiet).full : "";
  // A practice run is one recording; another needs asking for again.
  const again = () => {
    setResult(null);
    setSeen({ started: false, spoke: false, quiet: null });
    if (inTauri) practice(true).catch(() => undefined);
  };
  const tryAgain = (
    <button type="button" className="link" onClick={again}>
      Try again
    </button>
  );
  const enter = platform() === "mac" ? "Return" : "Enter";
  const subs: Record<Action, string> = {
    session: "Press to start, talk, then press again to finish.",
    region: `Freeze the screen, drag a box, then press ${enter}.`,
    full: ACTIONS.find((a) => a.id === "full")!.sub,
    zoom: ACTIONS.find((a) => a.id === "zoom")!.sub,
    ask: ASK_ACTION.sub,
  };
  const used = (a: Action) => !!fired[a] || (a === "session" && seen.started);
  const caption = rec
    ? words(st?.partial || st?.text || "")
      ? `“${words(st?.partial || st?.text || "").slice(-90)}”`
      : st?.quiet_mic
        ? micNotice(st.quiet_mic).full
        : "Listening…"
    : dropped
      ? <>That run was discarded. {tryAgain}</>
      : seen.started
        ? "Finishing…"
        : inTauri
          ? `Try it now: ${session} to start, ${region} to mark.`
          : "Preview: shortcuts are not live in a browser.";
  const warn = rec && !!st?.quiet_mic && !words(st.partial || st.text);
  return (
    <div className="onb-step">
      <Glyph>
        <Play size={32} strokeWidth={1.5} />
      </Glyph>
      <h1 className="t-title">Try it</h1>
      <p className="t-body">
        Start, say what you see, like “this button should be blue”, and mark something. Nothing is pasted anywhere; the
        prompt shows up here.
      </p>
      {speech === "downloading" ? (
        <p className="t-caption onb-note">
          Speech is still downloading, so your words may not be written down yet. Captures work now.
        </p>
      ) : speech !== "ready" ? (
        <p className="t-caption onb-note">
          {speech === "failed" ? "Speech didn’t download" : "No speech model yet"}, so your words won’t be written down.{" "}
          <button type="button" className="link" onClick={onModels}>
            Choose one
          </button>
        </p>
      ) : null}
      {empty ? (
        <p className="t-caption" role="alert">
          {quiet ? (
            <>{quiet} That is why nothing was written down.</>
          ) : (
            <>No speech was heard. Check that the microphone is not muted and is the one picked in Settings › Speech.</>
          )}{" "}
          {tryAgain}
        </p>
      ) : result ? (
        <>
          <div className="stage" style={{ maxHeight: 260, overflowY: "auto" }}>
            <PromptDoc exp={result} />
          </div>
          {tiny && quiet ? (
            <p className="t-caption" role="alert" style={{ marginTop: 8 }}>
              {quiet} That is why so little was written down.
            </p>
          ) : null}
          <p className="t-caption" style={{ marginTop: 8 }}>{tryAgain}</p>
        </>
      ) : (
        <div className="cards">
          {ACTIONS.map((a) => (
            <div key={a.id} className={`card try ${down === a.id ? "down" : ""}`}>
              <span className={`check ${used(a.id) ? "on" : ""}`} aria-label={used(a.id) ? "Used" : "Not used yet"}>
                <Check size={13} strokeWidth={3} />
              </span>
              <div className="grow">
                <div className="ttl">{a.name}</div>
                <div className="sub">{subs[a.id]}</div>
              </div>
              <ShortcutField action={a.id} cfg={cfg} patch={patch} setCfg={setCfg} />
            </div>
          ))}
          <p
            className="t-caption"
            role={warn ? "alert" : undefined}
            style={{ minHeight: 18, textAlign: "center", marginTop: 4, color: warn ? "var(--orange-ink)" : undefined }}
          >
            {caption}
          </p>
        </div>
      )}
    </div>
  );
}

const PREVIEW_STATE: UiState = {
  phase: "recording",
  mic: true,
  screen: false,
  partial: "",
  text: "",
  error: null,
  duration_ms: 6_000,
  image_count: 0,
  model_ready: true,
  level: 0,
  notice: null,
};

function DoneStep({ cfg, patch }: { cfg: Config; patch: (p: Record<string, unknown>) => Promise<string | null> }) {
  const mac = platform() === "mac";
  const canPaste = !mac || cfg.permissions.accessibility === "granted";
  const [login, setLogin] = useState(cfg.start_at_login);
  const [loginErr, setLoginErr] = useState<string | null>(null);
  return (
    <div className="onb-step">
      <AppIcon />
      <h1 className="t-hero">You’re all set</h1>
      <p className="t-body">
        Press <b style={{ color: "var(--label)" }}>{chordText(cfg.hotkeys.session.label)}</b> in any app to start.
        With no pictures it is plain dictation: your words are typed where your cursor is.
      </p>
      <p className="t-small" style={{ maxWidth: 440 }}>
        {mac
          ? "Pointcast lives in the menu bar at the top right. Click its icon, or open Pointcast from Spotlight, for History and Settings."
          : "Pointcast lives in the system tray by the clock. Windows tucks new icons under the ^ arrow: drag Pointcast onto the taskbar to keep it in sight. Search Start for Pointcast to open Settings."}
      </p>
      <div className="group" style={{ width: "100%", textAlign: "left" }}>
        <div className="row">
          <div className="grow">
            <div className="lbl">Paste into the app I was using</div>
            <div className="sub">
              {canPaste
                ? "When you finish, the prompt is pasted where your cursor was. It stays on the clipboard too."
                : "Needs Accessibility. Until then the prompt is copied and you paste it yourself."}
            </div>
          </div>
          <button
            type="button"
            role="switch"
            className="switch"
            aria-checked={cfg.output === "auto_paste"}
            disabled={!canPaste}
            onClick={() => patch({ output: cfg.output === "auto_paste" ? "clipboard" : "auto_paste" })}
          />
        </div>
        <div className="row">
          <div className="grow">
            <div className="lbl">Open at login</div>
            <div className="sub">
              {loginErr ?? (
                <>
                  Pointcast starts quietly in the {mac ? "menu bar" : "system tray"} whenever you sign in, so you never
                  run the install command again. Updates come from Settings.
                </>
              )}
            </div>
          </div>
          <button
            type="button"
            role="switch"
            className="switch"
            aria-checked={login}
            onClick={async () => {
              const v = !login;
              setLoginErr(null);
              try {
                if (inTauri) await loginSet(v);
                setLogin(v);
                patch({ start_at_login: v });
              } catch (e) {
                setLoginErr(String(e));
              }
            }}
          />
        </div>
      </div>
    </div>
  );
}
