import { useEffect, useState, type ReactNode } from "react";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Mark, PRODUCT } from "@/components/brand/Mark";
import { Icon } from "@/components/Icons";
import {
  detectTools,
  onSetupDone,
  onSetupProgress,
  onSpeechDone,
  onSpeechProgress,
  openTool,
  sessionToggle,
  settingsGet,
  settingsSet,
  setupRun,
  type DetectedTool,
} from "@/lib/api";
import { setThemePref, themePref, type ThemePref } from "@/lib/theme";

type SpeechView = {
  runtime?: { id: string; installed: boolean; lib: string };
  models?: { id: string; name: string; installed: boolean; bytes: number }[];
};

export function Settings() {
  const [cfg, setCfg] = useState<Record<string, unknown> | null>(null);
  const [busy, setBusy] = useState(false);
  const [label, setLabel] = useState("");
  const [pct, setPct] = useState<number | null>(null);
  const [done, setDone] = useState(false);
  const [failed, setFailed] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [tools, setTools] = useState<DetectedTool[]>([]);
  const [theme, setTheme] = useState<ThemePref>(() => themePref());

  async function refresh() {
    try {
      const s = await settingsGet();
      try {
        s.start_at_login = await isEnabled();
      } catch {
        /* preview */
      }
      setCfg({ ...s });
      const speech = s.speech as SpeechView | undefined;
      const ready =
        !!speech?.runtime?.installed && (speech.models ?? []).some((m) => m.installed);
      if (ready) setDone(true);
    } catch {
      setCfg(BROWSER_PREVIEW);
    }
    try {
      setTools(await detectTools());
    } catch {
      setTools(
        typeof location !== "undefined" && location.search.includes("preview=1")
          ? [
              {
                id: "superwhisper",
                name: "SuperWhisper",
                path: "/Applications/Superwhisper.app",
                kind: "stt",
              },
            ]
          : [],
      );
    }
  }

  useEffect(() => {
    refresh();
    const offs = [
      onSetupProgress((p) => {
        if (p.label) setLabel(p.label);
      }),
      onSpeechProgress((p) => {
        const x = p as { state?: string; received?: number; total?: number; part?: string };
        if (x.state === "downloading" && x.total) {
          setPct(Math.min(99, Math.round(((x.received ?? 0) / x.total) * 100)));
          setLabel(x.part === "runtime" ? "Downloading runtime…" : "Downloading model…");
        } else if (x.state === "unpacking") {
          setPct(null);
          setLabel("Unpacking…");
        } else if (x.state === "checking") {
          setPct(null);
          setLabel("Checking…");
        }
      }),
      onSetupDone((p) => {
        setBusy(false);
        if (p.ok) {
          setDone(true);
          setLabel("Ready — press the hotkey and talk.");
          setPct(100);
          refresh();
        } else {
          setFailed(p.error || "Setup failed");
        }
      }),
      onSpeechDone(() => refresh()),
    ];
    return () => {
      offs.forEach((u) => u.then((fn) => fn()));
    };
  }, []);

  async function oneClick() {
    setFailed("");
    setBusy(true);
    setLabel("Checking microphone and screen…");
    setPct(null);
    try {
      await setupRun();
    } catch (e) {
      // Browser preview: simulate success.
      setBusy(false);
      setDone(true);
      setLabel("Ready (preview).");
      setFailed(String(e).includes("invoke") ? "" : String(e));
    }
  }

  if (!cfg) {
    return (
      <div className="page" style={{ display: "grid", placeItems: "center" }}>
        <span style={{ color: "var(--text3)" }}>Opening…</span>
      </div>
    );
  }

  const speech = (cfg.speech as SpeechView) ?? {};
  const perms = cfg.permissions as { microphone: boolean; screen: boolean; note: string };
  const superW = tools.find((t) => t.id === "superwhisper");
  const preview = Boolean(cfg.__preview);

  return (
    <div className="page">
      <header className="title-drag" style={{ borderBottom: "1px solid var(--line)", padding: "14px 24px 16px" }}>
        <div className="row-between">
          <div style={{ display: "flex", gap: 12, alignItems: "center" }}>
            <div className="traffic" aria-hidden="true">
              <i />
              <i />
              <i />
            </div>
            <Mark size="lg" />
            <div>
              <p className="kicker">Tray app</p>
              <h1 className="title">{PRODUCT}</h1>
            </div>
          </div>
          <div className="no-drag seg" aria-label="Theme">
            {(["system", "light", "dark"] as ThemePref[]).map((t) => (
              <button
                key={t}
                type="button"
                className={theme === t ? "on" : ""}
                onClick={() => {
                  setTheme(t);
                  setThemePref(t);
                }}
              >
                {t === "system" ? "Auto" : t[0].toUpperCase() + t.slice(1)}
              </button>
            ))}
          </div>
        </div>
      </header>

      <main style={{ maxWidth: 520, margin: "0 auto", padding: "28px 24px 48px" }}>
        <p className="onb-eyebrow">Set up once</p>
        <h2 className="onb-title">Talk, mark the screen, paste anywhere.</h2>
        <p className="onb-sub">
          One click installs local speech (Moonshine) and asks for mic + screen. Hotkeys stay on their stock chords.
          Speech never leaves this machine.
        </p>

        <div className="mic-install card" style={{ width: "auto", position: "relative", marginBottom: 16 }} data-state={done ? "done" : busy ? "busy" : "ask"}>
          {done ? (
            <div className="mic-install-done" role="status">
              <Icon name="check" />
              <span style={{ flex: 1 }}>{label || "Installed. Press Ctrl/Cmd+Shift+Space and talk."}</span>
              <Button onClick={() => sessionToggle().catch(() => undefined)}>Start</Button>
            </div>
          ) : (
            <>
              <div className="mic-install-t">Voice + capture on this computer</div>
              {busy ? (
                <>
                  <div className="mic-install-s" aria-live="polite">
                    <span>{label || "Working…"}</span>
                    {pct != null ? <span className="mic-install-n">{pct}%</span> : null}
                  </div>
                  <div className={`mic-install-p ${pct == null ? "indet" : ""}`} role="progressbar">
                    <i style={pct == null ? undefined : { width: `${pct}%` }} />
                  </div>
                </>
              ) : (
                <>
                  <div className="mic-install-d">
                    Downloads the speech runtime and the recommended model, then requests microphone and screen
                    recording. Defaults are fine for almost everyone.
                  </div>
                  <div className="mic-install-a">
                    <Button onClick={() => void oneClick()}>
                      <Icon name="spark" size={14} />
                      Set up Pointcast
                    </Button>
                  </div>
                </>
              )}
              {failed ? (
                <div className="mic-install-f" role="alert">
                  {failed}
                </div>
              ) : null}
            </>
          )}
        </div>

        {superW ? (
          <button
            type="button"
            className="perm-row"
            style={{ width: "100%", cursor: "pointer", marginBottom: 12, textAlign: "left" }}
            onClick={() => openTool("superwhisper").catch(() => undefined)}
          >
            <Icon name="mic" />
            <div style={{ flex: 1 }}>
              <div style={{ fontWeight: 600 }}>Use existing SuperWhisper</div>
              <div style={{ fontSize: "var(--fs-sm)", color: "var(--text3)" }}>
                Detected at {superW.path}. Opens SuperWhisper for dictation; Pointcast still handles captures.
              </div>
            </div>
            <span className="badge ok">Found</span>
          </button>
        ) : null}

        <div style={{ display: "grid", gap: 8, marginBottom: 16 }}>
          <div className={`perm-row ${perms?.microphone ? "ok" : ""}`}>
            <span className={`dot ${perms?.microphone ? "on" : ""}`} />
            <div style={{ flex: 1 }}>
              <b>Microphone</b>
              <div style={{ fontSize: "var(--fs-sm)", color: "var(--text3)" }}>Local speech-to-text only</div>
            </div>
            <span className={`badge ${perms?.microphone ? "ok" : "warn"}`}>{perms?.microphone ? "Ready" : "Needed"}</span>
          </div>
          <div className={`perm-row ${perms?.screen ? "ok" : ""}`}>
            <span className={`dot ${perms?.screen ? "on" : ""}`} />
            <div style={{ flex: 1 }}>
              <b>Screen Recording</b>
              <div style={{ fontSize: "var(--fs-sm)", color: "var(--text3)" }}>Annotated screenshots for the prompt</div>
            </div>
            <span className={`badge ${perms?.screen ? "ok" : "warn"}`}>{perms?.screen ? "Ready" : "Needed"}</span>
          </div>
        </div>

        {preview ? (
          <p style={{ fontSize: "var(--fs-sm)", color: "var(--orange-ink)", marginBottom: 16 }}>
            Browser preview — run `npm run tauri dev` for live capture.
          </p>
        ) : null}

        <button type="button" className="adv-toggle" onClick={() => setAdvanced((v) => !v)} aria-expanded={advanced}>
          <Icon name="chev" size={14} />
          Advanced
          <span style={{ marginLeft: "auto", color: "var(--text3)", fontWeight: 400 }}>
            Hotkeys, model, export, privacy
          </span>
        </button>

        {advanced ? (
          <div className="adv-body">
            <AdvCard title="Hotkeys">
              <Hotkey row="Session" value={String(cfg.session_hotkey)} field="session_hotkey" onSave={(p) => patch(cfg, p, setCfg)} />
              <Hotkey row="Full" value={String(cfg.full_hotkey)} field="full_hotkey" onSave={(p) => patch(cfg, p, setCfg)} />
              <Hotkey row="Region" value={String(cfg.region_hotkey)} field="region_hotkey" onSave={(p) => patch(cfg, p, setCfg)} />
              <Hotkey row="Zoom" value={String(cfg.zoom_hotkey)} field="zoom_hotkey" onSave={(p) => patch(cfg, p, setCfg)} />
              <label className="label-stack" style={{ marginTop: 8 }}>
                Zoom size (px)
                <Input type="number" value={Number(cfg.zoom_px)} onChange={(e) => patch(cfg, { zoom_px: Number(e.target.value) }, setCfg)} />
              </label>
            </AdvCard>
            <AdvCard title="Speech model">
              <select
                className="field"
                value={String(cfg.model)}
                onChange={(e) => patch(cfg, { model: e.target.value }, setCfg)}
              >
                <option value="auto">Auto ({String(cfg.recommended)})</option>
                {(speech.models ?? []).map((m) => (
                  <option key={m.id} value={m.id}>
                    {m.name}
                    {m.installed ? " — installed" : ""}
                  </option>
                ))}
              </select>
            </AdvCard>
            <AdvCard title="Output">
              <div className="seg">
                {(
                  [
                    ["clipboard", "Clipboard"],
                    ["auto_paste", "Auto-paste"],
                    ["folder", "Folder"],
                  ] as const
                ).map(([id, name]) => (
                  <button key={id} type="button" className={cfg.output === id ? "on" : ""} onClick={() => patch(cfg, { output: id }, setCfg)}>
                    {name}
                  </button>
                ))}
              </div>
              <div style={{ marginTop: 10 }}>
                <Switch checked={Boolean(cfg.also_clipboard)} onChange={(v) => patch(cfg, { also_clipboard: v }, setCfg)} label="Also copy to clipboard" />
              </div>
              <label className="label-stack" style={{ marginTop: 10 }}>
                Export folder
                <Input value={String(cfg.export_dir ?? "")} onChange={(e) => patch(cfg, { export_dir: e.target.value || null }, setCfg)} />
              </label>
            </AdvCard>
            <AdvCard title="Privacy & startup">
              <Switch
                checked={Boolean(cfg.start_at_login)}
                onChange={async (v) => {
                  try {
                    if (v) await enable();
                    else await disable();
                  } catch {
                    /* preview */
                  }
                  patch(cfg, { start_at_login: v }, setCfg);
                }}
                label="Start at login"
              />
              <div style={{ marginTop: 10 }}>
                <Switch checked={Boolean(cfg.include_app)} onChange={(v) => patch(cfg, { include_app: v }, setCfg)} label="Include Pointcast windows" />
              </div>
              <label className="label-stack" style={{ marginTop: 10 }}>
                Never capture (one per line)
                <textarea
                  className="field"
                  value={((cfg.hidden as string[]) ?? []).join("\n")}
                  onChange={(e) =>
                    patch(
                      cfg,
                      {
                        hidden: e.target.value
                          .split("\n")
                          .map((s) => s.trim())
                          .filter(Boolean),
                      },
                      setCfg,
                    )
                  }
                />
              </label>
            </AdvCard>
          </div>
        ) : null}
      </main>
    </div>
  );
}

function AdvCard({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="card" style={{ marginBottom: 12, padding: 16 }}>
      <h3 className="section-label">{title}</h3>
      {children}
    </section>
  );
}

function Hotkey({
  row,
  value,
  field,
  onSave,
}: {
  row: string;
  value: string;
  field: string;
  onSave: (p: Record<string, unknown>) => void;
}) {
  const [v, setV] = useState(value);
  useEffect(() => setV(value), [value]);
  return (
    <label className="row-between" style={{ marginBottom: 6, fontSize: "var(--fs-sm)" }}>
      <span style={{ color: "var(--text2)" }}>{row}</span>
      <Input className="mono" style={{ maxWidth: 200 }} value={v} onChange={(e) => setV(e.target.value)} onBlur={() => onSave({ [field]: v })} />
    </label>
  );
}

async function patch(
  cfg: Record<string, unknown>,
  partial: Record<string, unknown>,
  setCfg: (c: Record<string, unknown>) => void,
) {
  const next = { ...flatten(cfg), ...partial };
  try {
    const s = await settingsSet(next);
    setCfg({ ...s });
  } catch {
    setCfg({ ...cfg, ...partial });
  }
}

const BROWSER_PREVIEW: Record<string, unknown> = {
  __preview: true,
  session_hotkey: "Ctrl+Shift+Space",
  full_hotkey: "Ctrl+Shift+1",
  region_hotkey: "Ctrl+Shift+2",
  zoom_hotkey: "Ctrl+Shift+Z",
  talk_mode: "tap",
  model: "auto",
  device: null,
  output: "clipboard",
  also_clipboard: true,
  export_dir: null,
  zoom_px: 400,
  start_at_login: false,
  include_app: false,
  hidden: ["1Password", "Bitwarden", "KeePass", "Dashlane", "LastPass", "Keychain Access"],
  mcp_bind: "127.0.0.1:47821",
  mcp_enabled: true,
  recommended: "moonshine-tiny-en",
  devices: [],
  permissions: {
    microphone: false,
    screen: false,
    note: "Open the Tauri app for permission prompts.",
  },
  speech: {
    runtime: { id: "linux", installed: false, lib: "libsherpa-onnx-c-api.so" },
    models: [
      { id: "moonshine-tiny-en", name: "Moonshine Tiny v2", installed: false, bytes: 0 },
      { id: "moonshine-base-en", name: "Moonshine Base v2", installed: false, bytes: 0 },
    ],
  },
};

function flatten(cfg: Record<string, unknown>) {
  return {
    session_hotkey: cfg.session_hotkey,
    full_hotkey: cfg.full_hotkey,
    region_hotkey: cfg.region_hotkey,
    zoom_hotkey: cfg.zoom_hotkey,
    talk_mode: cfg.talk_mode ?? "tap",
    model: cfg.model,
    device: cfg.device,
    output: cfg.output,
    also_clipboard: cfg.also_clipboard,
    export_dir: cfg.export_dir,
    zoom_px: cfg.zoom_px,
    start_at_login: cfg.start_at_login,
    include_app: cfg.include_app,
    hidden: cfg.hidden,
    mcp_bind: cfg.mcp_bind,
    mcp_enabled: cfg.mcp_enabled ?? true,
  };
}
