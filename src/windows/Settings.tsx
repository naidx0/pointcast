import { useEffect, useState, type ReactNode } from "react";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Mark, PRODUCT } from "@/components/brand/Mark";
import {
  downloadModel,
  onSpeechDone,
  onSpeechProgress,
  requestPermissions,
  sessionToggle,
  settingsGet,
  settingsSet,
} from "@/lib/api";
import { setThemePref, themePref, type ThemePref } from "@/lib/theme";

type SpeechView = {
  runtime?: { id: string; installed: boolean; lib: string };
  models?: { id: string; name: string; installed: boolean; bytes: number }[];
};

export function Settings() {
  const [cfg, setCfg] = useState<Record<string, unknown> | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);
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
      setErr(null);
    } catch (e) {
      setCfg(BROWSER_PREVIEW);
      setErr(String(e));
    }
  }

  useEffect(() => {
    refresh().catch((e) => setErr(String(e)));
    const u1 = onSpeechProgress((p) => {
      const x = p as { state?: string; received?: number; total?: number };
      if (x.state === "downloading") {
        const t = x.total ? ` / ${Math.round((x.total as number) / 1e6)} MB` : "";
        setProgress(`Downloading ${Math.round((x.received ?? 0) / 1e6)} MB${t}`);
      } else if (x.state) setProgress(x.state);
    });
    const u2 = onSpeechDone(() => {
      setBusy(null);
      setProgress("");
      refresh();
    });
    return () => {
      u1.then((u) => u());
      u2.then((u) => u());
    };
  }, []);

  async function patch(partial: Record<string, unknown>) {
    if (!cfg) return;
    const next = { ...flatten(cfg), ...partial };
    try {
      await settingsSet(next);
      await refresh();
    } catch {
      setCfg({ ...cfg, ...partial });
    }
  }

  if (!cfg) {
    return (
      <div className="page" style={{ display: "grid", placeItems: "center" }}>
        <span style={{ color: "var(--text3)" }}>Opening settings…</span>
      </div>
    );
  }

  const preview = Boolean(cfg.__preview);
  const speech = (cfg.speech as SpeechView) ?? {};
  const devices = (cfg.devices as string[]) ?? [];
  const perms = cfg.permissions as { microphone: boolean; screen: boolean; note: string };

  return (
    <div className="page">
      <header className="title-drag" style={{ borderBottom: "1px solid var(--line)", padding: "14px 24px 18px" }}>
        <div className="row-between" style={{ alignItems: "flex-start" }}>
          <div style={{ display: "flex", gap: 14, alignItems: "flex-start" }}>
            <div className="traffic" aria-hidden="true">
              <i />
              <i />
              <i />
            </div>
            <div>
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <Mark size="lg" />
                <div>
                  <p className="kicker">Tray app</p>
                  <h1 className="title">{PRODUCT}</h1>
                </div>
              </div>
              <p className="sub">
                Speak, mark the screen, paste one prompt into any AI tool. Speech never leaves this machine.
              </p>
            {preview ? (
              <p style={{ marginTop: 8, fontSize: "var(--fs-sm)", color: "var(--orange-ink)" }}>
                Browser preview — live capture needs the desktop app (`npm run tauri dev`).
              </p>
            ) : null}
            </div>
          </div>
          <div className="no-drag" style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <div className="seg" aria-label="Theme">
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
            <Button onClick={() => sessionToggle().catch(() => undefined)}>Start recording</Button>
          </div>
        </div>
      </header>

      <main style={{ padding: "20px 24px 40px" }} className="grid-2">
        <Card title="Permissions">
          <p className="sub" style={{ marginTop: 0, marginBottom: 14, fontSize: "var(--fs-sm)" }}>
            {perms?.note || "Grant microphone and screen recording so Pointcast can listen and capture."}
          </p>
          <div style={{ display: "grid", gap: 8 }}>
            <Perm ok={!!perms?.microphone} name="Microphone" detail="Local speech-to-text only" />
            <Perm ok={!!perms?.screen} name="Screen Recording" detail="Annotated screenshots for the prompt" />
          </div>
          <div style={{ marginTop: 12 }}>
            <Button variant="secondary" size="sm" onClick={() => requestPermissions().then(refresh)}>
              Request permissions
            </Button>
          </div>
        </Card>

        <Card title="Hotkeys">
          <Hotkey row="Session start / stop" value={String(cfg.session_hotkey)} field="session_hotkey" onSave={patch} />
          <Hotkey row="Full screenshot" value={String(cfg.full_hotkey)} field="full_hotkey" onSave={patch} />
          <Hotkey row="Region / annotate" value={String(cfg.region_hotkey)} field="region_hotkey" onSave={patch} />
          <Hotkey row="Zoom crop" value={String(cfg.zoom_hotkey)} field="zoom_hotkey" onSave={patch} />
          <label className="label-stack" style={{ marginTop: 10 }}>
            Zoom size (px)
            <Input type="number" value={Number(cfg.zoom_px)} onChange={(e) => patch({ zoom_px: Number(e.target.value) })} />
          </label>
        </Card>

        <Card title="Microphone & speech">
          <label className="label-stack">
            Microphone
            <select
              className="field"
              value={String(cfg.device ?? "")}
              onChange={(e) => patch({ device: e.target.value || null })}
            >
              <option value="">System default</option>
              {devices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
          </label>
          <label className="label-stack" style={{ marginTop: 10 }}>
            STT model
            <select className="field" value={String(cfg.model)} onChange={(e) => patch({ model: e.target.value })}>
              <option value="auto">Auto (recommended: {String(cfg.recommended)})</option>
              {(speech.models ?? []).map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                  {m.installed ? " — installed" : ""}
                </option>
              ))}
            </select>
          </label>
          <div style={{ marginTop: 12, display: "flex", flexWrap: "wrap", gap: 8 }}>
            <Button
              size="sm"
              variant="secondary"
              disabled={!!busy}
              onClick={() => {
                setBusy("runtime");
                downloadModel("runtime");
              }}
            >
              {speech.runtime?.installed ? "Runtime ready" : "Download runtime"}
            </Button>
            {(speech.models ?? []).map((m) => (
              <Button
                key={m.id}
                size="sm"
                variant={m.installed ? "outline" : "secondary"}
                disabled={!!busy}
                onClick={() => {
                  setBusy(m.id);
                  downloadModel(m.id);
                }}
              >
                {m.installed ? m.name : `Get ${m.name}`}
              </Button>
            ))}
          </div>
          {progress ? (
            <p className="mono" style={{ marginTop: 8, color: "var(--accent-ink)", fontSize: "var(--fs-xs)" }}>
              {progress}
            </p>
          ) : null}
          <p style={{ marginTop: 8, fontSize: "var(--fs-xs)", color: "var(--text3)" }}>
            Moonshine v2 via sherpa-onnx, loaded locally. MIT. Nothing is uploaded.
          </p>
        </Card>

        <Card title="Output">
          <div className="seg" role="group" aria-label="Output mode">
            {(
              [
                ["clipboard", "Clipboard"],
                ["auto_paste", "Auto-paste"],
                ["folder", "Folder"],
              ] as const
            ).map(([id, label]) => (
              <button key={id} type="button" className={cfg.output === id ? "on" : ""} onClick={() => patch({ output: id })}>
                {label}
              </button>
            ))}
          </div>
          <div style={{ marginTop: 12 }}>
            <Switch checked={Boolean(cfg.also_clipboard)} onChange={(v) => patch({ also_clipboard: v })} label="Also copy to clipboard" />
          </div>
          <label className="label-stack" style={{ marginTop: 12 }}>
            Export folder (blank = Documents/Pointcast/…)
            <Input value={String(cfg.export_dir ?? "")} onChange={(e) => patch({ export_dir: e.target.value || null })} />
          </label>
        </Card>

        <Card title="Privacy & startup">
          <Switch
            checked={Boolean(cfg.start_at_login)}
            onChange={async (v) => {
              try {
                if (v) await enable();
                else await disable();
              } catch {
                /* preview */
              }
              patch({ start_at_login: v });
            }}
            label="Start at login"
          />
          <div style={{ marginTop: 12 }}>
            <Switch
              checked={Boolean(cfg.include_app)}
              onChange={(v) => patch({ include_app: v })}
              label="Include Pointcast windows in captures"
            />
          </div>
          <label className="label-stack" style={{ marginTop: 12 }}>
            Never capture windows matching (one per line)
            <textarea
              className="field"
              value={((cfg.hidden as string[]) ?? []).join("\n")}
              onChange={(e) =>
                patch({
                  hidden: e.target.value
                    .split("\n")
                    .map((s) => s.trim())
                    .filter(Boolean),
                })
              }
            />
          </label>
          <p style={{ marginTop: 12, fontSize: "var(--fs-xs)", color: "var(--text3)", display: "flex", gap: 8, alignItems: "center" }}>
            Local MCP: {String(cfg.mcp_bind)} — GET /v1/session and POST /mcp
          </p>
        </Card>
      </main>
    </div>
  );
}

function Card({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="card">
      <h2 className="section-label">{title}</h2>
      {children}
    </section>
  );
}

function Perm({ ok, name, detail }: { ok: boolean; name: string; detail: string }) {
  return (
    <div className={`perm-row ${ok ? "ok" : ""}`}>
      <span className={`dot ${ok ? "on" : ""}`} />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ fontWeight: 600, fontSize: "var(--fs-ui)" }}>{name}</div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--text3)" }}>{detail}</div>
      </div>
      <span className={`badge ${ok ? "ok" : "warn"}`}>{ok ? "Ready" : "Needed"}</span>
    </div>
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
    <label className="row-between" style={{ marginBottom: 8, fontSize: "var(--fs-sm)" }}>
      <span style={{ color: "var(--text2)" }}>{row}</span>
      <Input
        className="mono"
        style={{ maxWidth: 220 }}
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => onSave({ [field]: v })}
      />
    </label>
  );
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
  hidden: [
    "1Password",
    "Bitwarden",
    "KeePass",
    "Dashlane",
    "LastPass",
    "Keeper",
    "NordPass",
    "Enpass",
    "RoboForm",
    "Proton Pass",
    "Credential Manager",
    "Windows Security",
    "Keychain Access",
  ],
  mcp_bind: "127.0.0.1:47821",
  mcp_enabled: true,
  recommended: "moonshine-tiny-en",
  devices: [],
  permissions: {
    microphone: false,
    screen: false,
    note: "Open the Tauri app for microphone and screen-recording permission prompts.",
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
