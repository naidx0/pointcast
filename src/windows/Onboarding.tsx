import { useEffect, useState, type ReactNode } from "react";
import { enable as autostartOn, disable as autostartOff } from "@tauri-apps/plugin-autostart";
import { AudioLines, Check, ClipboardPaste, Keyboard, Mic, MousePointer2, Play, ShieldCheck } from "lucide-react";
import { ACTIONS, Keycaps, ShortcutField } from "@/components/Shortcut";
import { PermissionRows, usePermissions } from "@/components/Permissions";
import { ModelPicker, speechReady, useSpeechJob } from "@/components/Models";
import {
  hotkeyTest,
  inTauri,
  onboardingFinish,
  onHotkey,
  onPracticeResult,
  onState,
  practice,
  subscribe,
  type Action,
  type Config,
  type SessionExport,
  type UiState,
} from "@/lib/api";
import { useConfig } from "@/lib/useConfig";
import { chordText } from "@/lib/keys";
import { platform } from "@/lib/theme";
import { PromptDoc } from "./Result";

const STEPS = ["welcome", "access", "speech", "keys", "practice", "done"] as const;
type Step = (typeof STEPS)[number];

export function Onboarding() {
  const { cfg, setCfg, patch, refresh } = useConfig();
  // The download keeps going in the background, so the next steps need not wait for it.
  const speech = useSpeechJob(refresh);
  const [step, setStep] = useState<Step>(() => {
    const q = new URLSearchParams(location.search).get("step");
    return (STEPS as readonly string[]).includes(q ?? "") ? (q as Step) : "welcome";
  });
  const i = STEPS.indexOf(step);
  const go = (d: number) => setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, i + d))]);

  // Each step turns on only the machinery it needs.
  useEffect(() => {
    if (!inTauri) return;
    hotkeyTest(step === "keys").catch(() => undefined);
    practice(step === "practice").catch(() => undefined);
  }, [step]);

  // Reopened for an existing user (a macOS update dropped a permission): go straight to access.
  useEffect(() => {
    if (cfg?.onboarded) setStep((s) => (s === "welcome" ? "access" : s));
  }, [cfg?.onboarded]);

  if (!cfg) return <div className="onb" />;

  const next: Record<Step, { label: string; enabled?: boolean; skip?: string }> = {
    welcome: { label: "Get Started" },
    access: { label: "Continue" },
    speech: { label: "Continue", enabled: speechReady(cfg), skip: "Skip for Now" },
    keys: { label: "Continue" },
    practice: { label: "Continue", skip: "Skip" },
    done: { label: "Start Using Pointcast" },
  };
  const n = next[step];

  return (
    <div className="onb">
      <div className="onb-body drag">
        {step === "welcome" && <Welcome />}
        {step === "access" && <AccessStep cfg={cfg} />}
        {step === "speech" && <SpeechStep cfg={cfg} patch={patch} refresh={refresh} />}
        {step === "keys" && <KeysStep cfg={cfg} patch={patch} setCfg={setCfg} />}
        {step === "practice" && (
          <PracticeStep cfg={cfg} downloading={!!speech.job} onBack={() => setStep("speech")} />
        )}
        {step === "done" && <DoneStep cfg={cfg} patch={patch} />}
      </div>
      <footer className="onb-foot">
        <div className="dots" aria-label={`Step ${i + 1} of ${STEPS.length}`}>
          {STEPS.map((s) => (
            <i key={s} className={s === step ? "on" : ""} />
          ))}
        </div>
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
            if (step !== "done") return go(1);
            if (inTauri) await onboardingFinish().catch(() => undefined);
          }}
        >
          {n.label}
        </button>
      </footer>
    </div>
  );
}

function AppIcon() {
  return <img className="onb-icon" src={platform() === "mac" ? "/icon-mac.png" : "/icon-win.png"} alt="" draggable={false} />;
}

function Glyph({ tile, children }: { tile: string; children: ReactNode }) {
  return <span className={`onb-glyph ${tile}`}>{children}</span>;
}

function Welcome() {
  const points = [
    { icon: Mic, tile: "tile-orange", t: "Talk", s: "Say what you see. Speech is turned into text on this computer." },
    {
      icon: MousePointer2,
      tile: "tile-purple",
      t: "Point",
      s: "Box, pin, arrow or zoom anything on screen, in any app.",
    },
    {
      icon: ClipboardPaste,
      tile: "tile-blue",
      t: "Paste",
      s: "Get one prompt with your words and pictures for ChatGPT, Claude, Cursor or any chat.",
    },
  ];
  return (
    <div className="onb-step">
      <AppIcon />
      <h1 className="t-hero">Welcome to Pointcast</h1>
      <p className="t-body">Show an AI what you mean instead of describing it. Setup takes about a minute.</p>
      <div className="cards" style={{ gap: 14, maxWidth: 420 }}>
        {points.map((p) => (
          <div key={p.t} style={{ display: "flex", gap: 14, alignItems: "flex-start" }}>
            <span className={`onb-glyph ${p.tile}`} style={{ width: 36, height: 36, borderRadius: 9, margin: 0, flex: "none" }}>
              <p.icon size={19} strokeWidth={2} />
            </span>
            <div>
              <div style={{ fontWeight: 600 }}>{p.t}</div>
              <div className="t-caption">{p.s}</div>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}

function AccessStep({ cfg }: { cfg: Config }) {
  const [perms, setPerms] = usePermissions(cfg.permissions);
  const mac = platform() === "mac";
  return (
    <div className="onb-step">
      <Glyph tile="tile-blue">
        <ShieldCheck size={28} strokeWidth={1.8} />
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

function SpeechStep({
  cfg,
  patch,
  refresh,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  refresh: () => void;
}) {
  return (
    <div className="onb-step">
      <Glyph tile="tile-orange">
        <AudioLines size={28} strokeWidth={1.8} />
      </Glyph>
      <h1 className="t-title">Choose a speech model</h1>
      <p className="t-body">
        Speech turns into text right on this computer: no account, no upload. Pick a model; you can switch later.
      </p>
      <ModelPicker cfg={cfg} patch={patch} refresh={refresh} />
    </div>
  );
}

function KeysStep({
  cfg,
  patch,
  setCfg,
}: {
  cfg: Config;
  patch: (p: Record<string, unknown>) => Promise<string | null>;
  setCfg: (c: Config) => void;
}) {
  const [fired, setFired] = useState<Partial<Record<Action, boolean>>>({});
  const [down, setDown] = useState<Action | null>(null);
  useEffect(
    () =>
      subscribe(
        onHotkey((h) => {
          if (h.pressed) {
            setFired((f) => ({ ...f, [h.action]: true }));
            setDown(h.action);
          } else setDown(null);
        }),
      ),
    [],
  );
  const all = ACTIONS.every((a) => fired[a.id]);
  return (
    <div className="onb-step">
      <Glyph tile="tile-indigo">
        <Keyboard size={28} strokeWidth={1.8} />
      </Glyph>
      <h1 className="t-title">Try your shortcuts</h1>
      <p className="t-body">
        Press each one now. They work in every app. To change one, click it and press the new keys.
      </p>
      <div className="cards">
        {ACTIONS.map((a) => (
          <div
            key={a.id}
            className="card"
            style={down === a.id ? { boxShadow: "0 0 0 2px var(--accent)", borderColor: "transparent" } : undefined}
          >
            <span className={`check ${fired[a.id] ? "on" : ""}`} aria-label={fired[a.id] ? "Works" : "Not pressed yet"}>
              <Check size={13} strokeWidth={3} />
            </span>
            <div className="grow">
              <div className="ttl">{a.name}</div>
              <div className="sub">{a.sub}</div>
            </div>
            <ShortcutField action={a.id} cfg={cfg} patch={patch} setCfg={setCfg} />
          </div>
        ))}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 16, alignSelf: "stretch" }}>
        <div style={{ flex: 1, textAlign: "left" }}>
          <div style={{ fontWeight: 600 }}>Start or stop by</div>
          <div className="t-caption">
            {cfg.talk_mode === "hold"
              ? "Hold to talk, release to finish. A quick tap keeps recording."
              : "Press once to start, again to finish."}
          </div>
        </div>
        <div className="seg" role="group" aria-label="Start or stop by">
          {(
            [
              ["tap", "Pressing"],
              ["hold", "Holding"],
            ] as const
          ).map(([id, name]) => (
            <button key={id} type="button" aria-pressed={cfg.talk_mode === id} onClick={() => patch({ talk_mode: id })}>
              {name}
            </button>
          ))}
        </div>
      </div>
      <p className="t-small" style={{ marginTop: 14, minHeight: 16 }}>
        {all ? "All four work." : inTauri ? "Nothing records while you test." : "Preview: shortcuts are not live in a browser."}
      </p>
    </div>
  );
}

/** What was said, without the [image n] markers captures add to the running text. */
const words = (t: string) => t.replace(/\[image[^\]]*\]/g, " ").replace(/\s+/g, " ").trim();

function PracticeStep({ cfg, downloading, onBack }: { cfg: Config; downloading: boolean; onBack: () => void }) {
  const [st, setSt] = useState<UiState | null>(null);
  const [result, setResult] = useState<SessionExport | null>(null);
  const [seen, setSeen] = useState({ started: false, spoke: false, marked: false });
  useEffect(
    () =>
      subscribe(
        onState((s) => {
          setSt(s);
          setSeen((x) => ({
            started: x.started || s.phase === "recording",
            spoke: x.spoke || !!words(s.partial || s.text),
            marked: x.marked || s.image_count > 0,
          }));
        }),
        onPracticeResult((r) => setResult(r)),
      ),
    [],
  );
  const session = chordText(cfg.hotkeys.session.label);
  const region = chordText(cfg.hotkeys.region.label);
  const rec = st?.phase === "recording" || st?.phase === "loading";
  const steps = [
    { done: seen.started, text: <>Press <Keycaps label={cfg.hotkeys.session.label} /> to start</> },
    { done: seen.spoke, text: <>Say what you see, like “this button should be blue”</> },
    {
      done: seen.marked,
      text: (
        <>
          Press <Keycaps label={cfg.hotkeys.region.label} />, drag a box, then press {platform() === "mac" ? "Return" : "Enter"}
        </>
      ),
    },
    { done: !!result, text: <>Press <Keycaps label={cfg.hotkeys.session.label} /> again to finish</> },
  ];
  return (
    <div className="onb-step">
      <Glyph tile="tile-green">
        <Play size={26} strokeWidth={2} />
      </Glyph>
      <h1 className="t-title">Do one practice run</h1>
      <p className="t-body">
        A real run, start to finish. Nothing is pasted anywhere; the prompt shows up here.
      </p>
      {downloading ? (
        <p className="t-caption" style={{ marginTop: -12, marginBottom: 16, display: "flex", gap: 8, alignItems: "center" }}>
          <span className="spinner" /> The speech model is still downloading. Wait for it, or try captures now.
        </p>
      ) : !speechReady(cfg) ? (
        <p className="t-caption" style={{ marginTop: -12, marginBottom: 16 }}>
          No speech model yet, so your words won’t be written down.{" "}
          <button type="button" className="link" onClick={onBack}>
            Download one
          </button>
        </p>
      ) : null}
      {result ? (
        <div className="stage" style={{ maxHeight: 260, overflowY: "auto" }}>
          <PromptDoc exp={result} />
        </div>
      ) : (
        <div className="cards">
          {steps.map((s, k) => (
            <div key={k} className="card" style={{ minHeight: 48 }}>
              <span className={`check ${s.done ? "on" : ""}`}>
                <Check size={13} strokeWidth={3} />
              </span>
              <div className="grow" style={{ display: "flex", alignItems: "center", gap: 6, flexWrap: "wrap" }}>
                {s.text}
              </div>
            </div>
          ))}
          <p className="t-caption" style={{ minHeight: 18, textAlign: "center", marginTop: 4 }}>
            {rec
              ? words(st?.partial || st?.text || "")
                ? `“${words(st?.partial || st?.text || "").slice(-90)}”`
                : "Listening…"
              : seen.started
                ? "Finishing…"
                : `Try it now: ${session} to start, ${region} to mark.`}
          </p>
        </div>
      )}
    </div>
  );
}

function DoneStep({ cfg, patch }: { cfg: Config; patch: (p: Record<string, unknown>) => Promise<string | null> }) {
  const mac = platform() === "mac";
  const canPaste = !mac || cfg.permissions.accessibility === "granted";
  const [login, setLogin] = useState(cfg.start_at_login);
  useEffect(() => {
    // Without Accessibility, macOS drops the synthetic paste: copy instead so nothing is lost.
    if (!canPaste && cfg.output === "auto_paste") patch({ output: "clipboard" });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  return (
    <div className="onb-step">
      <AppIcon />
      <h1 className="t-hero">You’re all set</h1>
      <p className="t-body">
        Pointcast lives in the {mac ? "menu bar" : "system tray"}. Press{" "}
        <b style={{ color: "var(--label)" }}>{chordText(cfg.hotkeys.session.label)}</b> in any app to start. With no
        pictures it is plain dictation: your words are typed where your cursor is.
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
            <div className="sub">Keep Pointcast ready whenever you sign in.</div>
          </div>
          <button
            type="button"
            role="switch"
            className="switch"
            aria-checked={login}
            onClick={async () => {
              const v = !login;
              setLogin(v);
              if (inTauri) await (v ? autostartOn() : autostartOff()).catch(() => undefined);
              patch({ start_at_login: v });
            }}
          />
        </div>
      </div>
    </div>
  );
}
