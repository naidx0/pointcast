import { useEffect, useState, type ReactNode } from "react";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import {
  downloadModel,
  onSpeechDone,
  onSpeechProgress,
  requestPermissions,
  sessionToggle,
  settingsGet,
  settingsSet,
} from "@/lib/api";
import { Mic, Monitor, Keyboard, FolderOpen, Shield } from "lucide-react";

type SpeechView = {
  runtime?: { id: string; installed: boolean; lib: string };
  models?: { id: string; name: string; installed: boolean; bytes: number }[];
};

export function Settings() {
  const [cfg, setCfg] = useState<Record<string, unknown> | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [progress, setProgress] = useState<string>("");
  const [err, setErr] = useState<string | null>(null);

  async function refresh() {
    try {
      const s = await settingsGet();
      try {
        s.start_at_login = await isEnabled();
      } catch {
        /* plugin may be unavailable in browser preview */
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
      <div className="flex min-h-screen items-center justify-center bg-ink-950 text-ink-300">
        Opening settings…
      </div>
    );
  }

  const preview = Boolean(cfg.__preview);

  const speech = (cfg.speech as SpeechView) ?? {};
  const devices = (cfg.devices as string[]) ?? [];
  const perms = cfg.permissions as { microphone: boolean; screen: boolean; note: string };

  return (
    <div className="min-h-screen bg-ink-950 text-ink-100">
      <header className="border-b border-ink-700 px-6 py-5">
        <div className="flex items-end justify-between gap-4">
          <div>
            <p className="text-[11px] uppercase tracking-[0.22em] text-ember-400">Tray app</p>
            <h1 className="font-display text-3xl font-medium tracking-tight">Pointcast</h1>
            <p className="mt-1 max-w-xl text-sm text-ink-300">
              Speak, mark the screen, paste one prompt into any AI tool. Speech never leaves this machine.
            </p>
            {preview ? (
              <p className="mt-2 text-xs text-ember-400">
                Browser preview of Settings — live capture needs the desktop app (`npm run tauri
                dev`).{err ? ` (${err})` : ""}
              </p>
            ) : null}
          </div>
          <Button onClick={() => sessionToggle().catch(() => undefined)}>Start recording</Button>
        </div>
      </header>

      <main className="grid gap-8 px-6 py-6 lg:grid-cols-2">
        <Section icon={<Keyboard size={16} />} title="Hotkeys">
          <Hotkey row="Session start / stop" value={String(cfg.session_hotkey)} field="session_hotkey" onSave={patch} />
          <Hotkey row="Full screenshot" value={String(cfg.full_hotkey)} field="full_hotkey" onSave={patch} />
          <Hotkey row="Region / annotate" value={String(cfg.region_hotkey)} field="region_hotkey" onSave={patch} />
          <Hotkey row="Zoom crop" value={String(cfg.zoom_hotkey)} field="zoom_hotkey" onSave={patch} />
          <label className="mt-3 block text-sm text-ink-300">
            Zoom size (px)
            <Input
              type="number"
              className="mt-1"
              value={Number(cfg.zoom_px)}
              onChange={(e) => patch({ zoom_px: Number(e.target.value) })}
            />
          </label>
        </Section>

        <Section icon={<Mic size={16} />} title="Microphone & speech">
          <label className="block text-sm text-ink-300">
            Microphone
            <select
              className="mt-1 h-9 w-full rounded-md border border-ink-500/30 bg-ink-950 px-2 text-sm"
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
          <label className="mt-3 block text-sm text-ink-300">
            STT model
            <select
              className="mt-1 h-9 w-full rounded-md border border-ink-500/30 bg-ink-950 px-2 text-sm"
              value={String(cfg.model)}
              onChange={(e) => patch({ model: e.target.value })}
            >
              <option value="auto">Auto (recommended: {String(cfg.recommended)})</option>
              {(speech.models ?? []).map((m) => (
                <option key={m.id} value={m.id}>
                  {m.name}
                  {m.installed ? " — installed" : ""}
                </option>
              ))}
            </select>
          </label>
          <div className="mt-3 flex flex-wrap gap-2">
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
          {progress ? <p className="mt-2 font-mono text-xs text-ember-400">{progress}</p> : null}
          <p className="mt-2 text-xs text-ink-500">
            Moonshine v2 via sherpa-onnx, loaded locally. MIT. Nothing is uploaded.
          </p>
        </Section>

        <Section icon={<FolderOpen size={16} />} title="Output">
          <div className="flex flex-wrap gap-2">
            {(
              [
                ["clipboard", "Clipboard (HTML + plain)"],
                ["auto_paste", "Auto-paste into focused app"],
                ["folder", "Folder export (prompt.md + PNGs)"],
              ] as const
            ).map(([id, label]) => (
              <Button
                key={id}
                size="sm"
                variant={cfg.output === id ? "default" : "outline"}
                onClick={() => patch({ output: id })}
              >
                {label}
              </Button>
            ))}
          </div>
          <div className="mt-3">
            <Switch
              checked={Boolean(cfg.also_clipboard)}
              onChange={(v) => patch({ also_clipboard: v })}
              label="Also copy to clipboard"
            />
          </div>
          <label className="mt-3 block text-sm text-ink-300">
            Export folder (blank = Documents/Pointcast/…)
            <Input
              className="mt-1"
              value={String(cfg.export_dir ?? "")}
              onChange={(e) => patch({ export_dir: e.target.value || null })}
            />
          </label>
        </Section>

        <Section icon={<Shield size={16} />} title="Privacy & startup">
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
          <div className="mt-3">
            <Switch
              checked={Boolean(cfg.include_app)}
              onChange={(v) => patch({ include_app: v })}
              label="Include Pointcast windows in captures"
            />
          </div>
          <label className="mt-3 block text-sm text-ink-300">
            Never capture windows matching (one per line)
            <textarea
              className="mt-1 h-28 w-full rounded-md border border-ink-500/30 bg-ink-950 p-2 font-mono text-xs"
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
          <div className="mt-3 flex items-center gap-3 text-sm">
            <Dot ok={perms?.microphone} label="Microphone" />
            <Dot ok={perms?.screen} label="Screen" />
            <Button size="sm" variant="outline" onClick={() => requestPermissions().then(refresh)}>
              Request permissions
            </Button>
          </div>
          <p className="mt-2 text-xs text-ink-500">{perms?.note}</p>
          <p className="mt-3 flex items-center gap-2 text-xs text-ink-500">
            <Monitor size={12} />
            Local MCP: {String(cfg.mcp_bind)} — GET /v1/session and POST /mcp
          </p>
        </Section>
      </main>
    </div>
  );
}

function Section({
  icon,
  title,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-xl border border-ink-700 bg-ink-900 p-5">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-medium uppercase tracking-wider text-ink-300">
        {icon}
        {title}
      </h2>
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
    <label className="mb-2 flex items-center justify-between gap-3 text-sm">
      <span className="text-ink-300">{row}</span>
      <Input
        className="max-w-[220px] font-mono"
        value={v}
        onChange={(e) => setV(e.target.value)}
        onBlur={() => onSave({ [field]: v })}
      />
    </label>
  );
}

function Dot({ ok, label }: { ok: boolean; label: string }) {
  return (
    <span className="inline-flex items-center gap-1.5">
      <span className={`h-2 w-2 rounded-full ${ok ? "bg-emerald-400" : "bg-ink-500"}`} />
      {label}
    </span>
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
