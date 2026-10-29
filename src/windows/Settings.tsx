import { useEffect, useRef, useState, type ReactNode } from "react";
import { enable as autostartOn, disable as autostartOff, isEnabled as autostartIs } from "@tauri-apps/plugin-autostart";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AudioLines, Camera, Info, Keyboard, Settings2, Share } from "lucide-react";
import { ACTIONS, ShortcutField } from "@/components/Shortcut";
import { PermissionRows, usePermissions } from "@/components/Permissions";
import { ModelPicker } from "@/components/Models";
import { HOTKEY_FIELD, historyClear, historyList, inTauri, windowOpen, type Config } from "@/lib/api";
import { useConfig } from "@/lib/useConfig";
import { platform, setThemePref, themePref, type ThemePref } from "@/lib/theme";

const PANES = [
  { id: "general", name: "General", icon: Settings2, tile: "tile-gray" },
  { id: "shortcuts", name: "Shortcuts", icon: Keyboard, tile: "tile-indigo" },
  { id: "speech", name: "Speech", icon: AudioLines, tile: "tile-orange" },
  { id: "capture", name: "Capture", icon: Camera, tile: "tile-purple" },
  { id: "output", name: "Output", icon: Share, tile: "tile-green" },
  { id: "about", name: "About", icon: Info, tile: "tile-blue" },
] as const;
type Pane = (typeof PANES)[number]["id"];

type Patch = (p: Record<string, unknown>) => Promise<string | null>;

export function Settings() {
  const { cfg, setCfg, patch, refresh } = useConfig();
  const [pane, setPane] = useState<Pane>(() => {
    const q = new URLSearchParams(location.search).get("pane");
    return PANES.some((p) => p.id === q) ? (q as Pane) : "general";
  });
  const [scrolled, setScrolled] = useState(false);
  const main = useRef<HTMLElement | null>(null);
  const mac = platform() === "mac";
  const cur = PANES.find((p) => p.id === pane)!;

  useEffect(() => {
    main.current?.scrollTo({ top: 0 });
    setScrolled(false);
  }, [pane]);

  return (
    <div className="prefs">
      <nav className="prefs-side drag" aria-label="Settings">
        {!mac ? (
          <div className="side-head">
            <img src="/icon-win.png" alt="" />
            <div>
              <div style={{ fontWeight: 600 }}>Pointcast</div>
              <div className="t-small">Settings</div>
            </div>
          </div>
        ) : null}
        {PANES.map((p) => (
          <button
            key={p.id}
            type="button"
            className="side-item"
            aria-current={pane === p.id ? "page" : undefined}
            onClick={() => setPane(p.id)}
          >
            <span className={`ico ${p.tile}`}>
              <p.icon size={mac ? 13 : 16} strokeWidth={mac ? 2.2 : 1.6} />
            </span>
            {p.name}
          </button>
        ))}
      </nav>
      <main ref={main} className="prefs-main" onScroll={(e) => setScrolled(e.currentTarget.scrollTop > 4)}>
        <div className={`prefs-bar drag ${scrolled ? "scrolled" : ""}`}>
          <h1 className={mac ? "t-head" : "t-title"} style={mac ? { fontSize: 15 } : undefined}>
            {cur.name}
          </h1>
        </div>
        <div className="prefs-pane">
          {!cfg ? null : pane === "general" ? (
            <General cfg={cfg} patch={patch} />
          ) : pane === "shortcuts" ? (
            <Shortcuts cfg={cfg} patch={patch} setCfg={setCfg} />
          ) : pane === "speech" ? (
            <Speech cfg={cfg} patch={patch} refresh={refresh} />
          ) : pane === "capture" ? (
            <Capture cfg={cfg} patch={patch} />
          ) : pane === "output" ? (
            <Output cfg={cfg} patch={patch} />
          ) : (
            <About cfg={cfg} />
          )}
        </div>
      </main>
    </div>
  );
}

function Row({ label, sub, children }: { label: ReactNode; sub?: ReactNode; children?: ReactNode }) {
  return (
    <div className="row">
      <div className="grow">
        <div className="lbl">{label}</div>
        {sub ? <div className="sub">{sub}</div> : null}
      </div>
      {children}
    </div>
  );
}

function Toggle({ on, set, label }: { on: boolean; set: (v: boolean) => void; label: string }) {
  return <button type="button" role="switch" className="switch" aria-checked={on} aria-label={label} onClick={() => set(!on)} />;
}

function Seg<T extends string | number>({
  value,
  options,
  set,
  label,
}: {
  value: T;
  options: readonly (readonly [T, string])[];
  set: (v: T) => void;
  label: string;
}) {
  // Windows 11 Settings picks from a dropdown; System Settings on a Mac uses a segmented control.
  if (platform() === "win")
    return (
      <select
        className="field"
        aria-label={label}
        style={{ minWidth: 150 }}
        value={String(value)}
        onChange={(e) => {
          const hit = options.find(([id]) => String(id) === e.target.value);
          if (hit) set(hit[0]);
        }}
      >
        {options.map(([id, name]) => (
          <option key={String(id)} value={String(id)}>
            {name}
          </option>
        ))}
      </select>
    );
  return (
    <div className="seg" role="group" aria-label={label}>
      {options.map(([id, name]) => (
        <button key={String(id)} type="button" aria-pressed={value === id} onClick={() => set(id)}>
          {name}
        </button>
      ))}
    </div>
  );
}

function General({ cfg, patch }: { cfg: Config; patch: Patch }) {
  const [theme, setTheme] = useState<ThemePref>(themePref());
  const [login, setLogin] = useState(cfg.start_at_login);
  const [perms, setPerms] = usePermissions(cfg.permissions);
  useEffect(() => {
    if (inTauri) autostartIs().then(setLogin).catch(() => undefined);
  }, []);
  return (
    <>
      <div className="group">
        <Row label="Open at login" sub="Keep Pointcast ready whenever you sign in.">
          <Toggle
            label="Open at login"
            on={login}
            set={async (v) => {
              setLogin(v);
              if (inTauri) await (v ? autostartOn() : autostartOff()).catch(() => undefined);
              patch({ start_at_login: v });
            }}
          />
        </Row>
        <Row label="Play sounds" sub="A soft cue when a recording starts and stops.">
          <Toggle label="Play sounds" on={cfg.sounds} set={(v) => patch({ sounds: v })} />
        </Row>
        <Row label="Appearance">
          <Seg
            label="Appearance"
            value={theme}
            options={[
              ["system", "Auto"],
              ["light", "Light"],
              ["dark", "Dark"],
            ]}
            set={(t) => {
              setTheme(t);
              setThemePref(t);
            }}
          />
        </Row>
      </div>
      <h2 className="group-title" style={{ marginTop: 24 }}>
        Permissions
      </h2>
      {perms ? <PermissionRows perms={perms} setPerms={setPerms} /> : null}
      <History />
      <div style={{ marginTop: 20, display: "flex", gap: 8 }}>
        <button type="button" className="btn" onClick={() => windowOpen("onboarding").catch(() => undefined)}>
          Open Setup Assistant…
        </button>
      </div>
    </>
  );
}

function History() {
  const [count, setCount] = useState<number | null>(null);
  const [sure, setSure] = useState(false);
  const load = () => {
    if (inTauri)
      historyList()
        .then((h) => setCount(h.entries.length))
        .catch(() => setCount(0));
    else setCount(3);
  };
  useEffect(load, []);
  return (
    <>
      <h2 className="group-title" style={{ marginTop: 24 }}>
        History
      </h2>
      <div className="group">
        <Row
          label={count == null ? "History" : `${count} ${count === 1 ? "prompt" : "prompts"} kept`}
          sub="Your newest 100 prompts stay on this computer, so a closed window or a crash loses nothing."
        >
          <button type="button" className="btn" onClick={() => windowOpen("result").catch(() => undefined)}>
            Show History
          </button>
          <button
            type="button"
            className={`btn ${sure ? "danger" : ""}`}
            disabled={!count}
            onBlur={() => setSure(false)}
            onClick={async () => {
              if (!sure) return setSure(true);
              setSure(false);
              if (inTauri) await historyClear().catch(() => undefined);
              load();
            }}
          >
            {sure ? "Click Again to Clear" : "Clear…"}
          </button>
        </Row>
      </div>
    </>
  );
}

function Shortcuts({ cfg, patch, setCfg }: { cfg: Config; patch: Patch; setCfg: (c: Config) => void }) {
  const custom = ACTIONS.some((a) => cfg.hotkeys[a.id].label !== cfg.hotkeys[a.id].default);
  return (
    <>
      <div className="group">
        {ACTIONS.map((a) => (
          <Row key={a.id} label={a.name} sub={a.sub}>
            <ShortcutField action={a.id} cfg={cfg} patch={patch} setCfg={setCfg} />
          </Row>
        ))}
      </div>
      <p className="group-note">Click a shortcut, then press the new keys. Esc cancels. Shortcuts work in every app.</p>
      <div className="group" style={{ marginTop: 20 }}>
        <Row
          label="Start or stop by"
          sub={
            cfg.talk_mode === "hold"
              ? "Hold the shortcut while you talk; release to finish. A quick tap keeps recording."
              : "Press once to start, press again to finish."
          }
        >
          <Seg
            label="Start or stop by"
            value={cfg.talk_mode}
            options={[
              ["tap", "Pressing"],
              ["hold", "Holding"],
            ]}
            set={(v) => patch({ talk_mode: v })}
          />
        </Row>
      </div>
      <div style={{ marginTop: 20 }}>
        <button
          type="button"
          className="btn"
          disabled={!custom}
          onClick={() => patch(Object.fromEntries(ACTIONS.map((a) => [HOTKEY_FIELD[a.id], null])))}
        >
          Restore Defaults
        </button>
      </div>
    </>
  );
}

function Speech({ cfg, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
  return (
    <>
      <h2 className="group-title">Model</h2>
      <ModelPicker cfg={cfg} patch={patch} refresh={refresh} />
      <div className="group" style={{ marginTop: 12 }}>
        <Row label="Microphone" sub="Used only while you record.">
          <select
            className="field"
            style={{ maxWidth: 240 }}
            value={cfg.device ?? ""}
            onChange={(e) => patch({ device: e.target.value || null })}
          >
            <option value="">System default</option>
            {cfg.devices.map((d) => (
              <option key={d} value={d}>
                {d}
              </option>
            ))}
          </select>
        </Row>
      </div>
      <p className="group-note">
        Audio stays in memory and is dropped once it is transcribed. Models are saved in Pointcast’s data folder.
      </p>
    </>
  );
}

function Capture({ cfg, patch }: { cfg: Config; patch: Patch }) {
  const [hidden, setHidden] = useState(cfg.hidden.join("\n"));
  return (
    <>
      <div className="group">
        <Row label="Close-up size" sub="How much of the screen around the pointer a close-up keeps.">
          <Seg
            label="Close-up size"
            value={[256, 384, 512, 768].includes(cfg.zoom_px) ? cfg.zoom_px : 512}
            options={[
              [256, platform() === "win" ? "Small (256 px)" : "S"],
              [384, platform() === "win" ? "Medium (384 px)" : "M"],
              [512, platform() === "win" ? "Large (512 px)" : "L"],
              [768, platform() === "win" ? "Extra large (768 px)" : "XL"],
            ]}
            set={(v) => patch({ zoom_px: v })}
          />
        </Row>
        <Row label="Show Pointcast in captures" sub="Off keeps the recording bar and the annotate overlay out of every picture.">
          <Toggle label="Show Pointcast in captures" on={cfg.include_app} set={(v) => patch({ include_app: v })} />
        </Row>
      </div>
      <h2 className="group-title" style={{ marginTop: 24 }}>
        Never capture
      </h2>
      <div className="group" style={{ padding: 10 }}>
        <textarea
          className="field"
          style={{ width: "100%", minHeight: 120 }}
          value={hidden}
          spellCheck={false}
          onChange={(e) => setHidden(e.target.value)}
          onBlur={() =>
            patch({
              hidden: hidden
                .split("\n")
                .map((s) => s.trim())
                .filter(Boolean),
            })
          }
        />
      </div>
      <p className="group-note">
        One app or window title per line. Matching windows are painted over before a picture is kept. Password managers are
        listed by default.
      </p>
    </>
  );
}

function Output({ cfg, patch }: { cfg: Config; patch: Patch }) {
  const mac = platform() === "mac";
  const needsAx = mac && cfg.permissions.accessibility !== "granted";
  return (
    <>
      <div className="group">
        <Row
          label="When you finish"
          sub={
            cfg.output === "auto_paste"
              ? needsAx
                ? "Needs Accessibility in Permissions. Until then the prompt is only copied."
                : "Pasted where your cursor was, and kept on the clipboard."
              : cfg.output === "clipboard"
                ? "Copied with pictures. Paste it wherever you like."
                : "Saved as prompt.md with the pictures in a folder."
          }
        >
          <Seg
            label="When you finish"
            value={cfg.output}
            options={[
              ["auto_paste", "Paste"],
              ["clipboard", "Copy"],
              ["folder", "Save"],
            ]}
            set={(v) => patch({ output: v })}
          />
        </Row>
        {cfg.output !== "clipboard" ? (
          <Row label="Also copy to the clipboard">
            <Toggle label="Also copy to the clipboard" on={cfg.also_clipboard} set={(v) => patch({ also_clipboard: v })} />
          </Row>
        ) : null}
        <Row label="Save folder" sub={cfg.export_dir || "Documents › Pointcast, one folder per recording"}>
          {cfg.export_dir ? (
            <button type="button" className="btn plain" onClick={() => patch({ export_dir: null })}>
              Reset
            </button>
          ) : null}
          <button
            type="button"
            className="btn"
            onClick={async () => {
              if (!inTauri) return;
              const dir = await pickFolder({ directory: true, title: "Save recordings in" }).catch(() => null);
              if (typeof dir === "string") patch({ export_dir: dir });
            }}
          >
            Choose…
          </button>
        </Row>
      </div>
      <h2 className="group-title" style={{ marginTop: 24 }}>
        Agents
      </h2>
      <div className="group">
        <Row
          label="Share the last prompt with coding agents"
          sub={
            <>
              Claude Code, Cursor and Codex can read it over MCP at{" "}
              <span className="mono">http://{cfg.mcp_bind}/mcp</span>. Only this computer can connect. Applies after
              Pointcast reopens.
            </>
          }
        >
          <Toggle label="Share over MCP" on={cfg.mcp_enabled} set={(v) => patch({ mcp_enabled: v })} />
        </Row>
      </div>
    </>
  );
}

function About({ cfg }: { cfg: Config }) {
  const mac = platform() === "mac";
  const link = (url: string) => () => (inTauri ? openUrl(url).catch(() => undefined) : window.open(url));
  return (
    <div style={{ display: "flex", flexDirection: "column", alignItems: "center", textAlign: "center", paddingTop: 24 }}>
      <img src={mac ? "/icon-mac.png" : "/icon-win.png"} alt="" style={{ width: 96, height: 96 }} draggable={false} />
      <h2 className="t-title" style={{ marginTop: 12 }}>
        Pointcast
      </h2>
      <p className="t-caption">Version {cfg.version}</p>
      <p className="t-body" style={{ maxWidth: 380, marginTop: 12 }}>
        Talk, point and paste one prompt into any AI tool. Open source under the MIT license.
      </p>
      <div style={{ display: "flex", gap: 8, marginTop: 18 }}>
        <button type="button" className="btn" onClick={link("https://github.com/naidx0/pointcast")}>
          Source Code
        </button>
        <button type="button" className="btn" onClick={link("https://github.com/naidx0/pointcast/issues")}>
          Report a Problem
        </button>
      </div>
      <p className="t-small" style={{ marginTop: 28, maxWidth: 380 }}>
        Speech recognition by Moonshine (MIT) running on sherpa-onnx (Apache 2.0). Both are downloaded, not bundled.
      </p>
    </div>
  );
}
