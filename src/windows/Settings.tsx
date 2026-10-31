import { useEffect, useRef, useState, type ReactNode } from "react";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AudioLines, Camera, Info, Keyboard, Settings2, Share } from "lucide-react";
import { ACTIONS, ShortcutField } from "@/components/Shortcut";
import { PermissionRows, usePermissions } from "@/components/Permissions";
import { ModelPicker } from "@/components/Models";
import {
  HOTKEY_FIELD,
  historyClear,
  historyList,
  inTauri,
  loginGet,
  loginSet,
  mcpCopySetup,
  onSettingsPane,
  onUpdateChecked,
  onUpdateFailed,
  subscribe,
  updateCheck,
  updateInstall,
  windowOpen,
  type Config,
  type Login,
} from "@/lib/api";
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
  const { cfg, setCfg, patch, refresh, error } = useConfig();
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
  // The menu's update item brings Settings forward on the pane that has Updates.
  useEffect(() => subscribe(onSettingsPane((p) => PANES.some((x) => x.id === p) && setPane(p as Pane))), []);

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
          {error ? (
            <div className="t-small" role="alert" style={{ color: "var(--red-ink)", marginBottom: 12 }}>
              {error.replace(/^Error:\s*/, "")}
            </div>
          ) : null}
          {!cfg ? null : pane === "general" ? (
            <General cfg={cfg} patch={patch} refresh={refresh} />
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

function General({ cfg, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
  const mac = platform() === "mac";
  const [theme, setTheme] = useState<ThemePref>(themePref());
  const [login, setLogin] = useState<Login>({ enabled: cfg.start_at_login, machine: false });
  const [loginErr, setLoginErr] = useState<string | null>(null);
  const [perms, setPerms] = usePermissions(cfg.permissions);
  useEffect(() => {
    if (inTauri) loginGet().then(setLogin).catch(() => undefined);
  }, []);
  return (
    <>
      <div className="group">
        <Row
          label="Open at login"
          sub={
            loginErr ??
            (login.machine
              ? "An entry for every user, left by an earlier version run as administrator, also opens Pointcast at sign-in. An administrator can remove it in Task Manager › Startup apps."
              : `Pointcast starts quietly in the ${mac ? "menu bar" : "system tray"} when you sign in.`)
          }
        >
          <Toggle
            label="Open at login"
            on={login.enabled}
            set={async (v) => {
              setLoginErr(null);
              try {
                setLogin(inTauri ? await loginSet(v) : { enabled: v, machine: false });
                patch({ start_at_login: v });
              } catch (e) {
                setLoginErr(String(e));
              }
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
        Updates
      </h2>
      <Updates cfg={cfg} patch={patch} refresh={refresh} />
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

function Updates({ cfg, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
  const mac = platform() === "mac";
  const [found, setFound] = useState<string | null>(cfg.update);
  const [state, setState] = useState<"idle" | "checking" | "latest" | "installing">("idle");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    // The pane may have been away while an update started or failed: ask again.
    refresh();
    return subscribe(
      onUpdateChecked((v) => {
        setFound(v);
        setError(null);
        setState(v ? "idle" : "latest");
      }),
      onUpdateFailed((why) => {
        setError(why);
        setState("idle");
      }),
    );
  }, [refresh]);
  useEffect(() => setFound((f) => cfg.update ?? f), [cfg.update]);
  const installing = state === "installing" || !!cfg.update_installing;
  const check = async () => {
    setState("checking");
    setError(null);
    try {
      const v = inTauri ? await updateCheck() : null;
      setFound(v);
      setState(v ? "idle" : "latest");
    } catch (e) {
      setError(String(e));
      setState("idle");
    }
  };
  const install = async () => {
    setState("installing");
    setError(null);
    try {
      if (inTauri) await updateInstall();
    } catch (e) {
      setError(String(e));
      setState("idle");
    }
  };
  const checked = cfg.update_checked;
  const sub = installing
    ? `Downloading ${found}. Pointcast closes, updates and opens again in about a minute.`
    : (error ??
      cfg.update_failure ??
      (found
        ? `You have ${cfg.version}.${mac ? " After updating, macOS asks again for Screen Recording and Accessibility." : ""}`
        : state === "checking"
          ? "Checking…"
          : state === "latest"
            ? `${cfg.version} is the latest version.`
            : checked?.error
              ? `Could not check for updates: ${checked.error}`
              : checked && !checked.version
                ? `${cfg.version} is the latest version.`
                : `Version ${cfg.version}.`));
  return (
    <div className="group">
      <Row label={found ? `Pointcast ${found} is available` : "Pointcast"} sub={sub}>
        {found ? (
          <button type="button" className="btn primary" disabled={installing} onClick={install}>
            {installing ? "Updating…" : "Update"}
          </button>
        ) : (
          <button type="button" className="btn" disabled={state === "checking"} onClick={check}>
            Check Now
          </button>
        )}
      </Row>
      <Row
        label="Check for updates automatically"
        sub="Once a day, Pointcast asks GitHub whether a newer version is out. Your recordings stay on this computer."
      >
        <Toggle
          label="Check for updates automatically"
          on={cfg.check_updates}
          set={(v) => patch({ check_updates: v })}
        />
      </Row>
    </div>
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
      <ModelPicker cfg={cfg} patch={patch} refresh={refresh} removable />
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
        <Row
          label="Pick up quieter and farther speech"
          sub="Cleans up room noise and evens out your volume before transcribing. Turn it off if it ever makes your voice sound wrong."
        >
          <Toggle
            label="Pick up quieter and farther speech"
            on={cfg.voice_boost !== false}
            set={(v) => patch({ voice_boost: v })}
          />
        </Row>
        <Row
          label="Get the faster GPU version by itself"
          sub="With Whisper Turbo, Pointcast downloads the version for your graphics card in the background (about 590 MB) and uses it once it works here."
        >
          <Toggle
            label="Get the faster GPU version by itself"
            on={cfg.auto_gpu !== false}
            set={(v) => patch({ auto_gpu: v })}
          />
        </Row>
        <Row
          label="Keep my recordings so I can transcribe them again"
          sub="The newest 10 recordings keep their audio on this computer, deleted with the entry. Nothing is uploaded."
        >
          <Toggle
            label="Keep my recordings so I can transcribe them again"
            on={cfg.keep_audio !== false}
            set={(v) => patch({ keep_audio: v })}
          />
        </Row>
      </div>
      <p className="group-note">Models are saved in Pointcast’s data folder.</p>
    </>
  );
}

function Capture({ cfg, patch }: { cfg: Config; patch: Patch }) {
  const [hidden, setHidden] = useState(cfg.hidden.join("\n"));
  const unsaved = useRef<string | null>(null);
  const saveHidden = (text: string) => {
    unsaved.current = null;
    patch({
      hidden: text
        .split("\n")
        .map((s) => s.trim())
        .filter(Boolean),
    });
  };
  // Saved a moment after typing stops, and on leaving the pane, not only when the field loses focus.
  useEffect(() => {
    if (unsaved.current == null) return;
    const t = setTimeout(() => saveHidden(hidden), 600);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hidden]);
  useEffect(
    () => () => {
      if (unsaved.current != null) saveHidden(unsaved.current);
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [],
  );
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
        <Row label="Close-ups center on" sub="Where I paused is the spot the pointer last rested on.">
          <Seg
            label="Close-ups center on"
            value={cfg.zoom_target}
            options={[
              ["pointer", "Pointer"],
              ["dwell", "Where I paused"],
              ["window", "Window"],
            ]}
            set={(v) => patch({ zoom_target: v })}
          />
        </Row>
        <Row label="Sharpen close-ups" sub="Doubles small text so it stays readable.">
          <Toggle label="Sharpen close-ups" on={cfg.zoom_sharpen} set={(v) => patch({ zoom_sharpen: v })} />
        </Row>
        <Row label="Mark where I point" sub="Rings the spot you paused on. In a region you can move or delete it.">
          <Seg
            label="Mark where I point"
            value={cfg.mark_point}
            options={[
              ["off", "Off"],
              ["ring", "Ring"],
              ["trail", "Ring and trail"],
            ]}
            set={(v) => patch({ mark_point: v })}
          />
        </Row>
        <Row label="Mark what I say “this” about" sub="When you say this or here, the spot you pointed at then is ringed on the nearest picture.">
          <Seg
            label="Mark what I say “this” about"
            value={cfg.spoken_anchors}
            options={[
              ["off", "Off"],
              ["ring", "Ring"],
            ]}
            set={(v) => patch({ spoken_anchors: v })}
          />
        </Row>
        <Row label="Outline the control I point at" sub="Names it in the prompt, like button “Place order”. Uses Accessibility on a Mac.">
          <Toggle label="Outline the control I point at" on={cfg.outline_element} set={(v) => patch({ outline_element: v })} />
        </Row>
        <Row
          label="Pick key frames for me"
          sub="While you record, Pointcast keeps the last two minutes of the screen in memory, once a second, and adds the moments you talked about. Nothing is written to disk unless it ends up in the prompt. Password managers are painted out."
        >
          <Toggle label="Pick key frames for me" on={cfg.key_frames} set={(v) => patch({ key_frames: v })} />
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
          onChange={(e) => {
            unsaved.current = e.target.value;
            setHidden(e.target.value);
          }}
          onBlur={() => unsaved.current != null && saveHidden(hidden)}
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
  const [copied, setCopied] = useState<"" | "claude" | "cursor">("");
  const [copyErr, setCopyErr] = useState("");
  const copy = (agent: "claude" | "cursor") => {
    setCopyErr("");
    if (!inTauri) return;
    mcpCopySetup(agent)
      .then(() => {
        setCopied(agent);
        setTimeout(() => setCopied(""), 1600);
      })
      .catch((e) => setCopyErr(String(e)));
  };
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
        {cfg.output === "folder" ? (
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
        <Row
          label="Include picture file paths in the text"
          sub="So terminal agents such as Claude Code can open the screenshots. Chat apps that take pictures ignore it."
        >
          <Toggle
            label="Include picture file paths in the text"
            on={cfg.picture_paths !== false}
            set={(v) => patch({ picture_paths: v })}
          />
        </Row>
      </div>
      <h2 className="group-title" style={{ marginTop: 24 }}>
        Agents
      </h2>
      <div className="group">
        <Row
          label="Share the last prompt with coding agents"
          sub={
            cfg.mcp_enabled && cfg.mcp_error ? (
              <span style={{ color: "var(--red-ink)" }}>
                Sharing did not start: {cfg.mcp_error}. Close whatever uses{" "}
                <span className="mono">{cfg.mcp_bind}</span>, then reopen Pointcast.
              </span>
            ) : (
              <>
                Claude Code, Cursor and Codex can read it over MCP at{" "}
                <span className="mono">http://{cfg.mcp_bind}/mcp</span>. Only this computer can connect. Applies after
                Pointcast reopens.
              </>
            )
          }
        >
          <Toggle label="Share over MCP" on={cfg.mcp_enabled} set={(v) => patch({ mcp_enabled: v })} />
        </Row>
        {cfg.mcp_enabled ? (
          <Row
            label="Agent setup"
            sub={
              copyErr ||
              "For your user settings only: run the Claude Code command in a terminal, or paste the Cursor entry into mcpServers in ~/.cursor/mcp.json. It holds this computer's key, so never put it in a project file."
            }
          >
            <button type="button" className="btn" onClick={() => copy("claude")}>
              {copied === "claude" ? "Copied" : "Claude Code"}
            </button>
            <button type="button" className="btn" onClick={() => copy("cursor")}>
              {copied === "cursor" ? "Copied" : "Cursor"}
            </button>
          </Row>
        ) : null}
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
