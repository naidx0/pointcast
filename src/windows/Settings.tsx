import { Fragment, useEffect, useRef, useState, type KeyboardEvent as ReactKeyboardEvent, type MouseEvent as ReactMouseEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { open as pickFolder } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { AudioLines, Camera, Check, ChevronRight, House, Image, Info, Keyboard, MessageCircleQuestion, ScrollText, Settings2, Share } from "lucide-react";
import { ACTIONS, ASK_ACTION, Keycaps, ShortcutField, askPress } from "@/components/Shortcut";
import { PermissionRows, usePermissions } from "@/components/Permissions";
import { ModelPicker, useSpeechJob } from "@/components/Models";
import { BrandTile } from "@/components/Brands";
import {
  HOTKEY_FIELD,
  agentConnect,
  agentFolder,
  agentRemove,
  agentsStatus,
  assistChoose,
  assistEffort,
  assistForget,
  assistKeySet,
  assistModels,
  assistOfflineOnly,
  assistProviders,
  assistSignIn,
  assistSetup,
  onAssistProviders,
  gpuRetry,
  historyClear,
  historyList,
  historyOpen,
  historyRecent,
  inTauri,
  loginGet,
  loginSet,
  mcpCopySetup,
  onPaneAsk,
  openSessionList,
  sessionsList,
  sessionsNew,
  sessionsOpen,
  onSettingsPane,
  onUpdateChecked,
  onUpdateFailed,
  relaunch,
  statsGet,
  statsReset,
  onLiveState,
  subscribe,
  takePaneAsk,
  updateCheck,
  updateInstall,
  winClose,
  winIsDecorated,
  winIsMaximized,
  winMinimize,
  winStartDragging,
  winToggleMaximize,
  onWinResized,
  windowOpen,
  type AgentId,
  type Agents,
  type AgentView,
  type Config,
  type Login,
} from "@/lib/api";
import { useConfig } from "@/lib/useConfig";
import { computerView, micStatus } from "@/lib/hardware";
import { previewHardware, useMicCheck } from "@/lib/mic";
import { platform, setThemePref, themePref, type ThemePref } from "@/lib/theme";
import { SAVED_NOTE, readUsage, usageView, weekdayNames, type Range, type Usage as Counts } from "@/lib/usage";
import { askNews, recentRows, type RecentRow } from "@/lib/home";
import { RECENT_SESSIONS, sessionRows, type SessionRow } from "@/lib/sessions";
import {
  CODEX_SIGN_OUT,
  CUSTOM,
  askReady,
  chosenRoute,
  consentText,
  customCheck,
  defaultModel,
  effortView,
  groups,
  hostOf,
  isHere,
  modelLabel,
  modelName,
  needsConsent,
  needsConsentAt,
  nowView,
  offlineSaid,
  shortName,
  withOffline,
  type Assist,
  type AssistModel,
  type Route,
  type RouteView,
} from "@/lib/assist";

const PANES = [
  { id: "home", name: "Home", icon: House, tile: "tile-teal" },
  { id: "ai", name: "AI", icon: MessageCircleQuestion, tile: "tile-blue" },
  { id: "general", name: "General", icon: Settings2, tile: "tile-gray" },
  { id: "shortcuts", name: "Shortcuts", icon: Keyboard, tile: "tile-indigo" },
  { id: "speech", name: "Speech", icon: AudioLines, tile: "tile-orange" },
  { id: "capture", name: "Capture", icon: Camera, tile: "tile-purple" },
  { id: "output", name: "Output", icon: Share, tile: "tile-green" },
  { id: "about", name: "About", icon: Info, tile: "tile-blue" },
] as const;
type Pane = (typeof PANES)[number]["id"];

/** A press on the bar itself (not on a button in it) moves the window; a double press maximizes or restores it, as a
 * Windows title bar does. */
function dragWindow(e: ReactMouseEvent) {
  if (e.button !== 0 || e.target !== e.currentTarget) return;
  (e.detail === 2 ? winToggleMaximize() : winStartDragging()).catch(() => undefined);
}

/** Windows: the slim bar across the top of the content, with Minimize, Maximize or Restore, and Close at Windows 11's
 * sizes and colours. No title and no icon: the sidebar already shows the app. */
function TitleBar() {
  const [max, setMax] = useState(false);
  const [active, setActive] = useState(true);
  useEffect(() => {
    const sync = () => {
      winIsMaximized()
        .then(setMax)
        .catch(() => undefined);
    };
    sync();
    const off = subscribe(onWinResized(sync));
    const focus = () => setActive(document.hasFocus());
    focus();
    window.addEventListener("focus", focus);
    window.addEventListener("blur", focus);
    return () => {
      off();
      window.removeEventListener("focus", focus);
      window.removeEventListener("blur", focus);
    };
  }, []);
  const run = (f: () => Promise<void>) => () => {
    f().catch(() => undefined);
  };
  return (
    <div className={`win-bar${active ? "" : " inactive"}`} onMouseDown={dragWindow}>
      <button type="button" className="win-cap" aria-label="Minimize" title="Minimize" onClick={run(winMinimize)}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5H10" />
        </svg>
      </button>
      <button type="button" className="win-cap" aria-label={max ? "Restore" : "Maximize"} title={max ? "Restore" : "Maximize"} onClick={run(winToggleMaximize)}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          {max ? (
            <>
              <rect x="0.5" y="2.5" width="7" height="7" rx="1" />
              <path d="M2.5 2.5V1.5C2.5 0.95 2.95 0.5 3.5 0.5H8.5C9.05 0.5 9.5 0.95 9.5 1.5V6.5C9.5 7.05 9.05 7.5 8.5 7.5H7.5" />
            </>
          ) : (
            <rect x="0.5" y="0.5" width="9" height="9" rx="1" />
          )}
        </svg>
      </button>
      <button type="button" className="win-cap close" aria-label="Close" title="Close" onClick={run(winClose)}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0.5 0.5L9.5 9.5M9.5 0.5L0.5 9.5" />
        </svg>
      </button>
    </div>
  );
}

/** A pane by its id; "usage" names the pane Home took over, so an old link still lands. */
const paneOf = (q: string | null): Pane | null => (q === "usage" ? "home" : PANES.some((p) => p.id === q) ? (q as Pane) : null);

type Patch = (p: Record<string, unknown>) => Promise<string | null>;

/** The pane another window asked for as this one opened (the Ask panel's Open Settings), read once per window. */
const askedPane = typeof window !== "undefined" ? paneOf(takePaneAsk()) : null;

export function Settings() {
  const { cfg, setCfg, patch, refresh, error } = useConfig();
  const [pane, setPane] = useState<Pane>(() => paneOf(new URLSearchParams(location.search).get("pane")) ?? askedPane ?? "home");
  const [scrolled, setScrolled] = useState(false);
  const main = useRef<HTMLElement | null>(null);
  const mac = platform() === "mac";
  const cur = PANES.find((p) => p.id === pane)!;
  // On Windows the window has no frame of its own (the engine makes it so): the page draws a slim bar with the window's
  // buttons. If the OS still draws the frame, nothing is drawn twice.
  const [frameless, setFrameless] = useState(false);
  useEffect(() => {
    if (mac) return;
    winIsDecorated()
      .then((d) => setFrameless(!d))
      .catch(() => setFrameless(true));
  }, [mac]);

  useEffect(() => {
    main.current?.scrollTo({ top: 0 });
    setScrolled(false);
  }, [pane]);
  // The menu's update item brings Settings forward on the pane that has Updates.
  useEffect(
    () =>
      subscribe(
        onSettingsPane((p) => {
          const to = paneOf(p);
          if (to) setPane(to);
        }),
      ),
    [],
  );
  useEffect(
    () =>
      onPaneAsk((p) => {
        const to = paneOf(p);
        if (to) setPane(to);
      }),
    [],
  );

  return (
    <div className={`prefs${frameless ? " frameless" : ""}`}>
      <nav className="prefs-side drag" aria-label="Settings" onMouseDown={frameless ? dragWindow : undefined}>
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
              {/* Even sizes only: an odd glyph in the 20px tile leaves half a pixel, which WebKit rounds to one side. */}
              <p.icon size={mac ? 14 : 16} strokeWidth={mac ? 2 : 1.6} />
            </span>
            {p.name}
          </button>
        ))}
      </nav>
      {frameless ? <TitleBar /> : null}
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
          {!cfg ? null : pane === "home" ? (
            <Home cfg={cfg} go={setPane} patch={patch} />
          ) : pane === "ai" ? (
            <Ai />
          ) : pane === "general" ? (
            <General cfg={cfg} patch={patch} refresh={refresh} />
          ) : pane === "shortcuts" ? (
            <Shortcuts cfg={cfg} patch={patch} setCfg={setCfg} />
          ) : pane === "speech" ? (
            <Speech cfg={cfg} patch={patch} refresh={refresh} />
          ) : pane === "capture" ? (
            <Capture cfg={cfg} patch={patch} />
          ) : pane === "output" ? (
            <Output cfg={cfg} patch={patch} refresh={refresh} />
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
        ? `You have ${cfg.version}. Pointcast closes, updates and opens again by itself.`
        : state === "checking"
          ? "Checking…"
          : state === "latest"
            ? `${cfg.version} is the latest version.`
            : checked?.error
              ? checked.error
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

const RANGES = [
  ["week", "This week"],
  ["all", "All time"],
] as const;

/** The first pane, the hub: the two features side by side, each with its keys and where it is set up (dictation with
 * mark-up; Ask with the AI that answers), then what was dictated (four numbers for the week or for all time, then when
 * in the week) and the last few recordings. Read once, as the pane opens. */
function Home({ cfg, go, patch }: { cfg: Config; go: (p: Pane) => void; patch: Patch }) {
  // "New: Prompt with your screen", once, for someone who finished setup before Ask existed; Got it remembers it.
  const [news, setNews] = useState(() => askNews(cfg));
  const [counts, setCounts] = useState<Counts | null>(null);
  const [recent, setRecent] = useState<RecentRow[]>([]);
  const [range, setRange] = useState<Range>("week");
  const [sure, setSure] = useState(false);
  // The AI that answers Ask, by its short name; "" when none is chosen, null until the engine says.
  const [ai, setAi] = useState<string | null>(null);
  // Ask's newest sessions, which open the panel on themselves.
  const [asks, setAsks] = useState<SessionRow[]>([]);
  // Ask's sessions change while Home is open: read again when an answer settles or the panel closes, and on focus.
  useEffect(() => {
    const load = () =>
      sessionsList()
        .then((l) => setAsks(sessionRows(l, new Date(), { n: RECENT_SESSIONS })))
        .catch(() => undefined);
    load();
    let was = "";
    const off = subscribe(
      onLiveState((s) => {
        const now = String((s as { state?: unknown })?.state ?? "");
        if (now !== was && (now === "shown" || now === "closed")) load();
        was = now;
      }),
    );
    window.addEventListener("focus", load);
    return () => {
      off();
      window.removeEventListener("focus", load);
    };
  }, []);
  useEffect(() => {
    assistProviders()
      .then((a) => {
        const r = askReady(a) ? chosenRoute(a) : null;
        setAi(r ? shortName(r) : "");
      })
      .catch(() => setAi(""));
    statsGet()
      .catch(() => null)
      .then((u) => setCounts(readUsage(u, Date.now())));
    historyRecent()
      .then((e) => setRecent(recentRows(e, new Date())))
      .catch(() => undefined);
  }, []);
  if (!counts) return null;
  const v = usageView(counts, new Date(), range);
  const days = weekdayNames("short");
  const saved = v.figures.some((f) => f.label === "Saved");
  const show = (id: string | null) => {
    if (inTauri) historyOpen(id).catch(() => undefined);
  };
  return (
    <div className="home">
      {news && cfg.hotkeys.ask ? (
        <section className="home-news" aria-labelledby="hub-news">
          <MessageCircleQuestion size={20} strokeWidth={1.5} aria-hidden="true" />
          <div className="grow">
            <div className="t" id="hub-news">
              New: Prompt with your screen
            </div>
            <div className="s">
              Press <Keycaps label={cfg.hotkeys.ask.label} /> and say what you want about what is on your screen.
            </div>
          </div>
          {ai === "" ? (
            <button type="button" className="btn" onClick={() => go("ai")}>
              Choose an AI
            </button>
          ) : null}
          <button
            type="button"
            className="btn plain"
            onClick={() => {
              setNews(false);
              patch({ ask_intro_seen: true }).catch(() => undefined);
            }}
          >
            Got it
          </button>
        </section>
      ) : null}
      <div className="home-hub">
        <section className="feature" aria-labelledby="hub-dictate">
          <h2 id="hub-dictate">Dictate and mark up</h2>
          <p>Talk and mark up the screen. The prompt is pasted where you type.</p>
          <div className="feature-keys">
            <span>Dictate</span>
            <Keycaps label={cfg.hotkeys.session.label} />
            <span>Mark up</span>
            <Keycaps label={cfg.hotkeys.region.label} />
          </div>
          <div className="feature-foot">
            <button type="button" className="link" onClick={() => go("speech")}>
              Speech
              <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
            <button type="button" className="link" onClick={() => go("capture")}>
              Capture
              <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
          </div>
        </section>
        <section className="feature" aria-labelledby="hub-ask">
          <h2 id="hub-ask">Prompt with your screen</h2>
          <p>Say what you want about what is on your screen. Your AI answers beside your work.</p>
          <div className="feature-keys">
            {cfg.hotkeys.ask ? (
              <>
                <span>Ask</span>
                <Keycaps label={cfg.hotkeys.ask.label} />
              </>
            ) : (
              <button type="button" className="link" onClick={() => go("shortcuts")}>
                Set a shortcut
              </button>
            )}
          </div>
          {asks.length ? (
            <div className="feature-recent">
              <div className="feature-recent-head">
                <h3 id="hub-asked">Recent questions</h3>
                <button type="button" className="link" onClick={() => openSessionList(asks[0].id).catch(() => undefined)}>
                  All sessions
                </button>
              </div>
              <ul aria-labelledby="hub-asked">
                {asks.map((r) => (
                  <li key={r.id}>
                    <button type="button" aria-label={`${r.label}. Open it`} onClick={() => sessionsOpen(r.id).catch(() => undefined)}>
                      <span className="t">{r.title}</span>
                      <span className="m">
                        {r.summary ? <ScrollText className="hasum" size={12} strokeWidth={1.5} aria-hidden="true" /> : null}
                        {r.turns} · {r.day}
                      </span>
                    </button>
                  </li>
                ))}
              </ul>
            </div>
          ) : null}
          <div className="feature-foot">
            {/* Asking without the microphone: the panel opens on a new question, its field ready. */}
            <button type="button" className="link" onClick={() => sessionsNew(true).catch(() => undefined)}>
              Type a question
              <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </button>
            {ai == null ? null : (
              <button type="button" className={`link${ai ? "" : " strong"}`} onClick={() => go("ai")} aria-label={ai ? `AI: ${ai}. Change it` : undefined}>
                {ai || "Choose an AI"}
                <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
              </button>
            )}
          </div>
        </section>
      </div>
      {v.empty ? (
        <p className="home-quiet">{v.quiet}</p>
      ) : (
        <section aria-label="Usage">
          <div className="home-head">
            <Seg value={range} options={RANGES} set={setRange} label="Range" />
            {v.streak || v.longest ? <span className="home-streak">{[v.streak, v.longest].filter(Boolean).join(" · ")}</span> : null}
          </div>
          {v.quiet ? (
            <p className="home-quiet">{v.quiet}</p>
          ) : (
            <div className="home-figures">
              {v.figures.map((f) => (
                <div key={f.label} title={f.title}>
                  <div className="n">
                    {f.parts.map((p) => (
                      <span key={p.u ?? ""}>
                        {p.n}
                        {p.u ? <small>{p.u}</small> : null}
                      </span>
                    ))}
                  </div>
                  <div className="l">{f.label}</div>
                </div>
              ))}
            </div>
          )}
          <p className="home-note">
            {v.rate}
            {v.rate && saved ? " " : null}
            {saved ? SAVED_NOTE : null}
          </p>
        </section>
      )}
      {v.empty ? null : (
        <section>
          <h2 className="home-title">When you dictate</h2>
          <div className="home-grid" role="img" aria-label="Recordings by weekday and hour">
            {v.cells.map((row, d) => (
              <div key={d} className="home-row">
                <span className="d">{days[d]}</span>
                {row.map((c, h) => (
                  <i key={h} data-l={c.level} title={c.title} />
                ))}
              </div>
            ))}
            <div className="home-row hours" aria-hidden="true">
              <span className="d" />
              {["00", "06", "12", "18"].map((h) => (
                <span key={h}>{h}</span>
              ))}
            </div>
          </div>
        </section>
      )}
      {recent.length ? (
        <section>
          <div className="home-sec">
            <h2 className="home-title">Recent</h2>
            <button type="button" className="link" onClick={() => show(null)}>
              All history
            </button>
          </div>
          <ul className="home-recent">
            {recent.map((r) => (
              <li key={r.id}>
                <button type="button" aria-label={r.label} onClick={() => show(r.id)}>
                  <span className={r.empty ? "t empty" : "t"}>
                    <span>{r.text}</span>
                    {r.pictures ? <Image size={13} strokeWidth={1.75} aria-hidden="true" /> : null}
                  </span>
                  <span className="when">{r.when}</span>
                  <span className="len">{r.length}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : null}
      <div className="home-foot">
        <span>{v.since}</span>
        <button
          type="button"
          className={`btn plain ${sure ? "danger" : ""}`}
          disabled={v.empty}
          onBlur={() => setSure(false)}
          onClick={async () => {
            if (!sure) return setSure(true);
            setSure(false);
            await statsReset()
              .then((u) => setCounts(readUsage(u, Date.now())))
              .catch(() => undefined);
          }}
        >
          {sure ? "Click again to reset" : "Reset…"}
        </button>
      </div>
    </div>
  );
}

/** An engine error as one plain line. */
const plain = (e: unknown) => String(e).replace(/^Error: */, "");

/** Settings › AI: which AI answers Ask and where the words go, then every way to one, grouped, each with the one action
 * its state allows. Asked again on open, on focus and after every action; the engine is the record. */
function Ai() {
  const [a, setA] = useState<Assist | null>(null);
  const [busy, setBusy] = useState("");
  const [errs, setErrs] = useState<Record<string, string>>({});
  const [more, setMore] = useState<{ id: string; what: "key" | "help" } | null>(null);
  // The route the sheet asks about; for the other service, also the address and model typed (never the key).
  const [consent, setConsent] = useState<{ route: Route; model?: string; base?: string } | null>(null);
  const [found, setFound] = useState<{ id: string; list: AssistModel[] } | null>(null);
  const [finding, setFinding] = useState(false);
  // The routes did not come: one quiet line with Try again, never an empty pane.
  const [failed, setFailed] = useState(false);
  // What the last flip of "Only on this computer" did, in one quiet line.
  const [said, setSaid] = useState("");
  // The agent whose sign-in window is open, watched until it says it is signed in.
  const [waiting, setWaiting] = useState("");
  const watchTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(watchTimer.current), []);
  // The engine answers at once from what it knows; each route's probe fills in through onAssistProviders. `refresh`
  // asks every route again (Check again, after a sign-in).
  const load = (refresh = false) =>
    assistProviders(refresh)
      .then((x) => {
        setA(x);
        setFailed(false);
      })
      .catch(() => setFailed(true));
  useEffect(() => {
    const again = () => load();
    load();
    const off = onAssistProviders(setA);
    window.addEventListener("focus", again);
    return () => {
      off();
      window.removeEventListener("focus", again);
    };
  }, []);
  const chosen = a ? chosenRoute(a) : null;
  // The chosen route's full model list can take a few seconds: the quick list stands in until it comes. An agent picks
  // its own model, so there is nothing to ask.
  useEffect(() => {
    if (!chosen || chosen.kind === "agent") return;
    let live = true;
    setFinding(true);
    assistModels(chosen.id)
      .then((list) => live && setFound({ id: chosen.id, list }))
      .catch(() => undefined)
      .finally(() => live && setFinding(false));
    return () => {
      live = false;
    };
  }, [chosen?.id]);
  if (!a)
    return failed ? (
      <p className="ai-fail" role="status">
        <span>The list of AIs didn’t load.</span>
        <button type="button" className="btn" onClick={() => load()}>
          Try again
        </button>
      </p>
    ) : null;

  const run = async (id: string, job: () => Promise<unknown>, refresh = false) => {
    setErrs((e) => ({ ...e, [id]: "" }));
    setBusy(id);
    try {
      await job();
      return true;
    } catch (e) {
      setErrs((x) => ({ ...x, [id]: plain(e) }));
      return false;
    } finally {
      setBusy("");
      await load(refresh);
    }
  };
  // The switch flips at once and says what it did; the engine's list then confirms it (or puts it back, with why).
  const offline = (on: boolean) => {
    setA((x) => (x ? { ...x, offline_only: on } : x));
    setSaid(on ? "On. Only AIs on this computer are shown and asked." : "Off. Every AI is shown again.");
    run("offline", () => assistOfflineOnly(on)).then((ok) => ok || setSaid(""));
  };
  // A key's or a server's models are not in the quick list: they are asked for before the choice, which needs one.
  const choose = (p: Route, consent: boolean, model?: string, base?: string) =>
    run(p.id, async () => {
      let m = model ?? defaultModel(p);
      if (!m && (p.kind === "key" || p.kind === "local")) m = defaultModel({ models: await assistModels(p.id) });
      return assistChoose(p.id, m, consent, base);
    });
  // The other service: the key goes straight to the engine; an address not agreed to opens the sheet, naming its host.
  const setUp = async (p: Route, f: { base: string; model: string; key: string }) => {
    if (f.key && !(await run(p.id, () => assistKeySet(p.id, f.key)))) return;
    setMore(null);
    if (needsConsentAt(p, f.base)) return setConsent({ route: { ...p, goes_to: hostOf(f.base), offline: isHere(f.base) }, model: f.model, base: f.base });
    return choose(p, false, f.model, f.base);
  };
  // An agent's sign-in runs in a terminal window of its own: the list is asked again every few seconds until the app
  // says it is signed in, for up to five minutes, and Pointcast is told on focus too.
  const watch = (id: string) => {
    setWaiting(id);
    const started = Date.now();
    const tick = () =>
      assistProviders(true)
        .then((x) => {
          setA(x);
          const r = x.providers.find((q) => q.id === id);
          if (r && r.state !== "signed_out" && r.state !== "checking") return setWaiting((w) => (w === id ? "" : w));
          if (Date.now() - started < 300_000) watchTimer.current = window.setTimeout(tick, 3000);
          else setWaiting((w) => (w === id ? "" : w));
        })
        .catch(() => setWaiting((w) => (w === id ? "" : w)));
    window.clearTimeout(watchTimer.current);
    watchTimer.current = window.setTimeout(tick, 3000);
  };
  // Install and update are done here; when one fails, its error shows and the command to run by hand opens under it.
  const setup = (p: Route, job: "install" | "update") =>
    run(p.id, () => assistSetup(p.id, job), true).then((ok) => ok || setMore({ id: p.id, what: "help" }));
  const signIn = (p: Route) =>
    p.id === "codex" ? run(p.id, () => assistSignIn(p.id), true) : run(p.id, () => assistSetup(p.id, "sign_in")).then((ok) => ok && watch(p.id));
  const act = (v: RouteView, p: Route) => {
    if (v.action === "use") return needsConsent(p) ? setConsent({ route: p }) : choose(p, false);
    if (v.action === "sign_in") return signIn(p);
    if (v.action === "install" || v.action === "update") return setup(p, v.action);
    if (v.action === "add_key") return setMore({ id: p.id, what: "key" });
    if (v.action === "get") return setMore((m) => (m?.id === p.id && m.what === "help" ? null : { id: p.id, what: "help" }));
    if (v.action === "check") return run(p.id, () => Promise.resolve(), true);
  };

  const now = nowView(a);
  const quick = chosen?.models ?? [];
  const list = found && found.id === chosen?.id && found.list.length ? found.list : quick;
  const model = chosen ? (a.model ?? defaultModel(chosen)) : null;
  const named = chosen ? modelName({ model }, { models: list }) : null;
  const models = model && !list.some((m) => m.id === model) ? [{ id: model, name: named ?? model, images: true }, ...list] : list;
  const effort = effortView(a, chosen, models.find((m) => m.id === model));
  return (
    <>
      <div className="group">
        <div className="row ai-now">
          {chosen ? <BrandTile id={chosen.id} name={chosen.name} size={24} /> : null}
          <div className="grow">
            <div className="lbl">
              {now.title}
              {named ? <span className="ai-now-model">{named}</span> : null}
            </div>
            <div className="sub">{now.line}</div>
          </div>
        </div>
        {chosen && chosen.kind !== "agent" ? (
          <Row label="Model" sub="The models your plan or key offers. Words only means it cannot see your pictures.">
            <select
              className="field ai-model"
              aria-label="Model"
              value={model ?? ""}
              disabled={!models.length || busy === chosen.id}
              onChange={(e) => run(chosen.id, () => assistChoose(chosen.id, e.target.value, false))}
            >
              {models.length ? (
                models.map((m) => (
                  <option key={m.id} value={m.id}>
                    {modelLabel(m)}
                  </option>
                ))
              ) : (
                <option value="">{finding ? "Finding models…" : "No models found"}</option>
              )}
            </select>
          </Row>
        ) : null}
        {effort ? (
          <Row label="Thinking" sub="Low answers soonest. Higher levels think longer before they answer.">
            <Seg label="Thinking" value={effort.value} options={effort.options} set={(v) => run("effort", () => assistEffort(v))} />
          </Row>
        ) : null}
        <Row
          label="Only on this computer"
          sub={
            errs.offline ? (
              <span style={{ color: "var(--red-ink)" }}>{errs.offline}</span>
            ) : (
              <span role="status">{said || "Hides every AI that sends your words off this computer."}</span>
            )
          }
        >
          <Toggle label="Only on this computer" on={a.offline_only} set={offline} />
        </Row>
      </div>
      {groups(a).map((g, i) => (
        <Fragment key={g.kind}>
          <div className="group-title ai-group-title">
            <h2>{g.title}</h2>
            {i === 0 ? (
              <button type="button" className="btn plain" disabled={busy === "refresh"} onClick={() => run("refresh", () => Promise.resolve(), true)} title="Look again for the AIs on this computer">
                {busy === "refresh" ? "Looking…" : "Refresh"}
              </button>
            ) : null}
          </div>
          <div className="group routes">
            {g.routes.map((v) => {
              const p = a.providers.find((x) => x.id === v.id)!;
              const open = more?.id === v.id ? more.what : null;
              const close = () => setMore(null);
              const form =
                p.id === CUSTOM ? (
                  <CustomForm
                    route={p}
                    model={a.chosen === p.id ? (a.model ?? "") : ""}
                    busy={busy === p.id}
                    save={(f) => setUp(p, f)}
                    fail={(why) => setErrs((e) => ({ ...e, [p.id]: why }))}
                    cancel={() => {
                      close();
                      setErrs((e) => ({ ...e, [p.id]: "" }));
                    }}
                  />
                ) : (
                  <KeyForm
                    name={v.name}
                    busy={busy === p.id}
                    save={(key) => run(p.id, () => assistKeySet(p.id, key)).then((ok) => ok && setMore(null))}
                    cancel={close}
                  />
                );
              return (
                <RouteRow
                  key={v.id}
                  v={v}
                  busy={busy === v.id}
                  waiting={waiting === v.id}
                  signIn={() => signIn(p)}
                  err={errs[v.id] ?? ""}
                  more={open}
                  form={form}
                  act={() => act(v, p)}
                  switchAccount={() => run(p.id, () => assistSignIn(p.id), true)}
                  check={() => run(p.id, () => Promise.resolve(), true)}
                  forget={() => run(p.id, () => assistForget(p.id))}
                />
              );
            })}
          </div>
        </Fragment>
      ))}
      {consent ? (
        <ConsentSheet
          route={consent.route}
          allow={() => {
            setConsent(null);
            choose(consent.route, true, consent.model, consent.base);
          }}
          cancel={() => setConsent(null)}
        />
      ) : null}
    </>
  );
}

const BUSY: Partial<Record<NonNullable<RouteView["action"]>, string>> = { sign_in: "Signing in…", check: "Checking…", install: "Installing…", update: "Updating…" };

/** One way to an AI: its name, its state in plain words, and the one action that fits; its key form (`form`) or the
 * install help opens under it. */
function RouteRow(props: {
  v: RouteView;
  busy: boolean;
  /** Its sign-in window is open and Pointcast is watching for it to finish. */
  waiting: boolean;
  signIn: () => void;
  err: string;
  more: "key" | "help" | null;
  form: ReactNode;
  act: () => void;
  check: () => void;
  forget: () => void;
  switchAccount: () => void;
}) {
  const { v, busy, err, more } = props;
  const [out, setOut] = useState(false);
  const [sure, setSure] = useState(false);
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const t = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(t);
  }, [copied]);
  const line = err ? (
    <span style={{ color: "var(--red-ink)" }}>{err}</span>
  ) : props.waiting ? (
    "Finish signing in in the Terminal window that opened. This updates by itself."
  ) : busy && v.action === "sign_in" ? (
    "Finish signing in in your browser"
  ) : busy && (v.action === "install" || v.action === "update") ? (
    `${v.action === "install" ? "Installing" : "Updating"} ${v.name}. This can take a minute.`
  ) : (
    v.line
  );
  const cmd = v.help?.command;
  return (
    <div className={`row ai-route${v.checking ? " checking" : ""}`}>
      <BrandTile id={v.id} name={v.name} />
      <div className="grow">
        <div className="lbl">
          {v.name}
          {v.account?.plan ? <span className="ai-plan">{v.account.plan}</span> : null}
        </div>
        <div className="sub">{line}</div>
        {v.signIn && !err && !props.waiting ? (
          <div className="ai-account">
            <button type="button" className="link" disabled={busy} onClick={props.signIn}>
              Sign in
            </button>
          </div>
        ) : null}
        {v.account && !err ? (
          <div className="ai-account">
            <button type="button" className="link" disabled={busy} onClick={props.switchAccount}>
              {busy ? "Finish in your browser" : "Use another account"}
            </button>
            <button type="button" className="link" aria-expanded={out} onClick={() => setOut((o) => !o)}>
              Sign out
            </button>
          </div>
        ) : null}
      </div>
      {v.chosen ? (
        <span className="ai-inuse">
          <Check size={12} strokeWidth={2.5} aria-hidden />
          In use
        </span>
      ) : null}
      {v.forget ? (
        <button
          type="button"
          className={`btn plain ${sure ? "danger" : ""}`}
          disabled={busy}
          onBlur={() => setSure(false)}
          onClick={() => {
            if (!sure) return setSure(true);
            setSure(false);
            props.forget();
          }}
        >
          {sure ? "Click again to forget" : "Forget"}
        </button>
      ) : null}
      {v.action && more !== "key" ? (
        <button
          type="button"
          className="btn"
          disabled={busy || (props.waiting && v.action === "sign_in")}
          aria-expanded={v.action === "get" ? more === "help" : undefined}
          onClick={props.act}
        >
          {props.waiting && v.action === "sign_in" ? "Waiting…" : (busy && BUSY[v.action]) || v.label}
        </button>
      ) : null}
      {more === "key" ? props.form : null}
      {out && v.account ? (
        <div className="ai-more">
          <p className="ai-help">Sign out in Codex itself: run this in a terminal, then Refresh. Pointcast never touches your sign-in.</p>
          <div className="ai-help-row">
            <div className="ai-cmd">
              <code>{CODEX_SIGN_OUT}</code>
            </div>
          </div>
        </div>
      ) : null}
      {more === "help" && v.help ? (
        <div className="ai-more">
          <p className="ai-help">{v.help.text}</p>
          {v.help.account ? <p className="ai-help">{v.help.account}</p> : null}
          <div className="ai-help-row">
            {cmd ? (
              <div className="ai-cmd">
                <code>{cmd}</code>
                <button
                  type="button"
                  className="btn plain"
                  aria-label={copied ? "Copied" : "Copy the command"}
                  onClick={() =>
                    navigator.clipboard
                      ?.writeText(cmd)
                      .then(() => setCopied(true))
                      .catch(() => undefined)
                  }
                >
                  {copied ? "Copied" : "Copy"}
                </button>
              </div>
            ) : null}
            {v.help.page ? (
              <button type="button" className="btn plain" onClick={() => (inTauri ? openUrl(v.help!.page!) : window.open(v.help!.page!, "_blank")) && undefined}>
                Official page
              </button>
            ) : null}
            <button type="button" className="btn" disabled={busy} onClick={props.check}>
              {busy ? "Checking…" : "Check again"}
            </button>
          </div>
        </div>
      ) : null}
    </div>
  );
}

/** The key goes from the field straight to the engine: never in state, never echoed, never prefilled, and the field is
 * emptied the moment it is sent. Only whether the field has anything is kept, to enable Save. */
function KeyForm({ name, busy, save, cancel }: { name: string; busy: boolean; save: (key: string) => void; cancel: () => void }) {
  const field = useRef<HTMLInputElement>(null);
  const [filled, setFilled] = useState(false);
  useEffect(() => field.current?.focus(), []);
  return (
    <form
      className="ai-more ai-key"
      onSubmit={(e) => {
        e.preventDefault();
        const el = field.current;
        const key = el?.value.trim() ?? "";
        if (el) el.value = "";
        setFilled(false);
        if (key) save(key);
      }}
    >
      <input
        ref={field}
        type="password"
        className="field"
        aria-label={`${name} key`}
        placeholder="Paste your key"
        autoComplete="off"
        spellCheck={false}
        onChange={(e) => setFilled(!!e.currentTarget.value.trim())}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.preventDefault();
            cancel();
          }
        }}
      />
      <button type="button" className="btn plain" onClick={cancel}>
        Cancel
      </button>
      <button type="submit" className="btn primary" disabled={!filled || busy}>
        {busy ? "Saving…" : "Save"}
      </button>
    </form>
  );
}

/** The other service: its address and a model's name beside the key. The address and model are not secret and stay as
 * typed; the key is read once on Save, the field emptied at once, and handed on, never kept. Esc is Cancel. */
function CustomForm(props: {
  route: Route;
  model: string;
  busy: boolean;
  save: (f: { base: string; model: string; key: string }) => void;
  fail: (why: string) => void;
  cancel: () => void;
}) {
  const { route, busy, cancel } = props;
  const first = useRef<HTMLInputElement>(null);
  const field = useRef<HTMLInputElement>(null);
  const [address, setAddress] = useState(route.base);
  const [model, setModel] = useState(props.model);
  useEffect(() => first.current?.focus(), []);
  const esc = (e: ReactKeyboardEvent) => {
    if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    }
  };
  return (
    <form
      className="ai-more ai-custom"
      noValidate
      onKeyDown={esc}
      onSubmit={(e) => {
        e.preventDefault();
        const el = field.current;
        const c = customCheck(route, { address, model, key: !!el?.value.trim() });
        if ("why" in c) return props.fail(c.why);
        const key = el?.value.trim() ?? "";
        if (el) el.value = "";
        props.save({ ...c, key });
      }}
    >
      <input
        ref={first}
        className="field"
        aria-label="Address"
        placeholder="https://…/v1"
        inputMode="url"
        autoComplete="off"
        spellCheck={false}
        value={address}
        onChange={(e) => setAddress(e.currentTarget.value)}
      />
      <input
        className="field"
        aria-label="Model"
        placeholder="Model"
        autoComplete="off"
        spellCheck={false}
        value={model}
        onChange={(e) => setModel(e.currentTarget.value)}
      />
      <input ref={field} type="password" className="field" aria-label={`${shortName(route)} key`} placeholder="Paste your key" autoComplete="off" spellCheck={false} />
      <div className="ai-custom-actions">
        <button type="button" className="btn plain" onClick={cancel}>
          Cancel
        </button>
        <button type="submit" className="btn primary" disabled={busy || !address.trim()}>
          {busy ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}

/** Consent, once per AI and destination: a sheet on a Mac, a dialog on Windows. The rest of the window is inert while it
 * is open, Tab stays inside it, and Esc is Not now. */
function ConsentSheet({ route, allow, cancel }: { route: Route; allow: () => void; cancel: () => void }) {
  const t = consentText(route);
  const box = useRef<HTMLDivElement>(null);
  const off = useRef(cancel);
  off.current = cancel;
  const win = platform() === "win";
  useEffect(() => {
    const back = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const root = document.getElementById("root");
    root?.setAttribute("inert", "");
    box.current?.querySelector<HTMLButtonElement>(".btn.primary")?.focus();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        off.current();
      } else if (e.key === "Tab" && box.current) {
        const all = [...box.current.querySelectorAll<HTMLElement>("button")];
        const i = all.indexOf(document.activeElement as HTMLElement);
        e.preventDefault();
        all[(i + (e.shiftKey ? -1 : 1) + all.length) % all.length]?.focus();
      }
    };
    document.addEventListener("keydown", key, true);
    return () => {
      document.removeEventListener("keydown", key, true);
      root?.removeAttribute("inert");
      if (back?.isConnected) back.focus();
    };
  }, []);
  const yes = (
    <button type="button" className="btn primary" onClick={allow}>
      Allow
    </button>
  );
  const no = (
    <button type="button" className="btn" onClick={cancel}>
      Not now
    </button>
  );
  return createPortal(
    <div className="sheet-scrim">
      <div className="sheet" role="dialog" aria-modal="true" aria-labelledby="consent-title" aria-describedby="consent-body" ref={box}>
        <h2 id="consent-title">{t.title}</h2>
        <p id="consent-body">{t.body}</p>
        <div className="sheet-actions">
          {win ? yes : no}
          {win ? no : yes}
        </div>
      </div>
    </div>,
    document.body,
  );
}

function Shortcuts({ cfg, patch, setCfg }: { cfg: Config; patch: Patch; setCfg: (c: Config) => void }) {
  const all = [...ACTIONS, ASK_ACTION].filter((a) => cfg.hotkeys[a.id]);
  const custom = all.some((a) => cfg.hotkeys[a.id].label !== cfg.hotkeys[a.id].default);
  return (
    <>
      <h2 className="group-title">Dictate and mark up</h2>
      <div className="group">
        {ACTIONS.map((a) => (
          <Row key={a.id} label={a.name} sub={a.sub}>
            <ShortcutField action={a.id} cfg={cfg} patch={patch} setCfg={setCfg} />
          </Row>
        ))}
      </div>
      {cfg.hotkeys.ask ? (
        <>
          <h2 className="group-title" style={{ marginTop: 20 }}>
            {ASK_ACTION.name}
          </h2>
          <div className="group">
            <Row label="Ask" sub={`${ASK_ACTION.sub} ${askPress(cfg.talk_mode)}`}>
              <ShortcutField action={ASK_ACTION.id} cfg={cfg} patch={patch} setCfg={setCfg} />
            </Row>
          </div>
        </>
      ) : null}
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
          onClick={() => patch(Object.fromEntries(all.map((a) => [HOTKEY_FIELD[a.id], null])))}
        >
          Restore Defaults
        </button>
      </div>
    </>
  );
}

function Speech({ cfg: real, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
  const cfg = previewHardware(real);
  const speech = useSpeechJob(refresh);
  const device = cfg.device ?? cfg.default_device ?? "";
  const check = useMicCheck(true, cfg.device ?? null);
  const status = micStatus({ device, live: check.live, recent: cfg.mic_recent ?? [], others: check.others, mac: platform() === "mac" });
  const model = (cfg.speech.models ?? []).find((m) => m.id === (cfg.speech_in_use ?? cfg.model)) ?? null;
  const pc = cfg.machine ? computerView({ machine: cfg.machine, model, upgrade: cfg.gpu_upgrade ?? null, declined: !!cfg.gpu_declined }) : null;
  const fix = async () => {
    if (pc?.action === "retry" && inTauri) await gpuRetry().catch(() => undefined);
    speech.start("whisper-turbo", cfg, patch);
  };
  return (
    <>
      <h2 className="group-title">Microphone</h2>
      <div className="group">
        <div className="row mic-row">
          <div className="grow">
            <select
              className="field mic-pick"
              aria-label="Microphone"
              value={cfg.device ?? ""}
              onChange={(e) => patch({ device: e.target.value || null })}
            >
              <option value="">{cfg.default_device ? `System default (${cfg.default_device})` : "System default"}</option>
              {cfg.devices.map((d) => (
                <option key={d} value={d}>
                  {d}
                </option>
              ))}
            </select>
            <div className={`sub mic-status ${status?.tone ?? ""}`} role="status">
              {status ? status.text : "Used only while you record."}
              {status?.offer ? (
                <>
                  <span className="mic-offer">
                    {status.offer} is picking up sound.{" "}
                    <button type="button" className="link" onClick={() => patch({ device: status.offer })}>
                      Use it
                    </button>
                  </span>
                </>
              ) : null}
            </div>
          </div>
          <LevelMeter level={check.level} />
        </div>
      </div>
      {pc ? (
        <>
          <h2 className="group-title">This computer</h2>
          <div className="group">
            <div className="row">
              <div className="grow pc-lines">
                <div className="lbl">{pc.card}</div>
                {pc.engine ? <div className="sub">{pc.engine}</div> : null}
                {pc.why ? (
                  <div className="sub pc-why">
                    {pc.why}{" "}
                    {pc.action ? (
                      <button type="button" className="link" disabled={!!speech.job} onClick={fix}>
                        {pc.action === "retry" ? "Try again" : "Set it up"}
                      </button>
                    ) : null}
                  </div>
                ) : null}
              </div>
            </div>
          </div>
        </>
      ) : null}
      <h2 className="group-title">Model</h2>
      <ModelPicker cfg={cfg} patch={patch} refresh={refresh} removable speech={speech} computer={!!pc} />
      <div className="group" style={{ marginTop: 12 }}>
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
          label="Free the speech model from memory when idle"
          sub="After a recording, the speech engine keeps about 0.9 GB of graphics memory and 1.5 GB of RAM so the next one starts at once. Loading it again takes about 3 seconds, while you talk."
        >
          <select
            className="field"
            style={{ maxWidth: 160 }}
            value={cfg.free_memory_minutes ?? 1}
            onChange={(e) => patch({ free_memory_minutes: Number(e.target.value) })}
          >
            {[1, 5, 10, 30].map((m) => (
              <option key={m} value={m}>
                {m === 1 ? "After 1 minute" : `After ${m} minutes`}
              </option>
            ))}
            <option value={0}>Never</option>
          </select>
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

/** The live input level as one bar of fixed size: it moves without moving anything around it. */
function LevelMeter({ level }: { level: number }) {
  return (
    <div className="level" aria-label="Microphone level" role="meter" aria-valuemin={0} aria-valuemax={1} aria-valuenow={Math.round(level * 100) / 100}>
      <i style={{ transform: `scaleX(${Math.max(0.02, Math.min(1, level * 1.4))})` }} />
    </div>
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

function Output({ cfg, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
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
        {cfg.output === "folder" ? (
          <Row label="Also copy to the clipboard">
            <Toggle label="Also copy to the clipboard" on={cfg.also_clipboard} set={(v) => patch({ also_clipboard: v })} />
          </Row>
        ) : null}
        <Row label="Tidy the text" sub="Paragraphs at your pauses, no ums and no repeated words. Nothing is summarized or reworded.">
          <Toggle label="Tidy the text" on={cfg.tidy !== false} set={(v) => patch({ tidy: v })} />
        </Row>
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
      <AgentsPart cfg={cfg} patch={patch} refresh={refresh} />
    </>
  );
}

/** Settings › Output › Agents: each coding agent set up in one click, in its own user settings. */
function AgentsPart({ cfg, patch, refresh }: { cfg: Config; patch: Patch; refresh: () => void }) {
  const [st, setSt] = useState<Agents | null>(null);
  const [busy, setBusy] = useState<AgentId | "">("");
  const [errs, setErrs] = useState<Partial<Record<AgentId, string>>>({});
  const [copied, setCopied] = useState<AgentId | "">("");
  // Claude Code's pictures folder, chosen before its first Set up.
  const [folder, setFolder] = useState(true);
  const load = () => {
    agentsStatus()
      .then(setSt)
      .catch(() => undefined);
  };
  useEffect(() => {
    load();
    window.addEventListener("focus", load);
    return () => window.removeEventListener("focus", load);
  }, [cfg.mcp_enabled, cfg.mcp_bind]);

  const run = (id: AgentId, job: () => Promise<Agents>, preview: (s: Agents) => Agents) => {
    setErrs((e) => ({ ...e, [id]: "" }));
    if (!inTauri) {
      setSt((s) => (s ? preview(s) : s));
      return;
    }
    setBusy(id);
    job()
      .then(setSt)
      .catch((e) => {
        setErrs((x) => ({ ...x, [id]: String(e) }));
        load();
      })
      .finally(() => {
        setBusy("");
        refresh();
      });
  };
  const mark = (s: Agents, id: AgentId, entry: AgentView["entry"]): Agents => ({
    ...s,
    server_ok: true,
    server_error: null,
    agents: s.agents.map((a) => (a.id === id ? { ...a, entry, error: null } : a)),
  });
  const connect = (id: AgentId) =>
    run(
      id,
      () => agentConnect(id, folder),
      (s) => ({ ...mark(s, id, "current"), folder_allowed: s.folder_allowed || (id === "claude" && folder) }),
    );
  const remove = (id: AgentId) => run(id, () => agentRemove(id), (s) => mark(s, id, "missing"));
  const copy = (id: AgentId) => {
    setErrs((e) => ({ ...e, [id]: "" }));
    if (!inTauri) return;
    mcpCopySetup(id)
      .then(() => {
        setCopied(id);
        setTimeout(() => setCopied(""), 1600);
      })
      .catch((e) => setErrs((x) => ({ ...x, [id]: String(e) })));
  };
  const claude = st?.agents.find((a) => a.id === "claude");
  const claudeOn = claude?.entry === "current";
  const setFolderAllowed = (on: boolean) => {
    if (!claudeOn) return setFolder(on);
    run("claude", () => agentFolder(on), (s) => ({ ...s, folder_allowed: on }));
  };

  return (
    <>
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
                Coding agents read it over MCP at <span className="mono">http://{cfg.mcp_bind}/mcp</span>. Only this
                computer can connect.
              </>
            )
          }
        >
          {cfg.mcp_enabled && cfg.mcp_error ? (
            <button type="button" className="btn" onClick={() => inTauri && relaunch()}>
              Reopen Pointcast
            </button>
          ) : null}
          <Toggle label="Share over MCP" on={cfg.mcp_enabled} set={(v) => patch({ mcp_enabled: v })} />
        </Row>
        {(st?.agents ?? []).map((a) => {
          const err = errs[a.id] || a.error;
          const set = a.entry === "current";
          const connected = set && cfg.mcp_enabled && st?.server_ok;
          const down = set && cfg.mcp_enabled && !st?.server_ok;
          const sub = err ? (
            <span style={{ color: "var(--red-ink)" }}>{err}</span>
          ) : !a.found && !set ? (
            `${a.name} was not found on this computer. Copy the ${a.id === "claude" ? "command" : "entry"} to set it up by hand.`
          ) : connected ? (
            "New sessions can read your last prompt."
          ) : down ? (
            <span style={{ color: "var(--red-ink)" }}>
              Pointcast's agent server is not answering{st?.server_error ? `: ${st.server_error}` : ""}.
            </span>
          ) : set ? (
            "Set up. Turn on sharing above so it can read your prompts."
          ) : a.entry === "other" ? (
            <>
              <span title={a.file}>{a.path}</span> has an older Pointcast entry.
            </>
          ) : (
            <>
              Adds Pointcast to <span title={a.file}>{a.path}</span>, nothing else.
            </>
          );
          return (
            <Row key={a.id} label={<span title={a.file}>{a.name}</span>} sub={sub}>
              {connected ? (
                <span className="badge ok">
                  <Check size={12} strokeWidth={3} aria-hidden />
                  Connected
                </span>
              ) : null}
              {down ? (
                <button type="button" className="btn primary" onClick={() => inTauri && relaunch()}>
                  Reopen Pointcast
                </button>
              ) : null}
              {a.found && !set ? (
                <button type="button" className="btn primary" disabled={busy === a.id} onClick={() => connect(a.id)}>
                  {busy === a.id ? "Setting up…" : a.entry === "other" ? "Set up again" : "Set up"}
                </button>
              ) : null}
              {set ? (
                <button type="button" className="btn plain" disabled={busy === a.id} onClick={() => remove(a.id)}>
                  Remove
                </button>
              ) : null}
              <button type="button" className="btn plain" onClick={() => copy(a.id)}>
                {copied === a.id ? "Copied" : a.id === "claude" ? "Copy command" : "Copy entry"}
              </button>
            </Row>
          );
        })}
        {claude?.found ? (
          <Row
            label="Let Claude Code open the pictures folder"
            sub={
              <>
                Each recording's pictures are in <span className="mono">{st?.folder}</span>. Adds it to
                additionalDirectories in ~/.claude/settings.json{claudeOn ? "" : " when you set up Claude Code"}.
              </>
            }
          >
            <Toggle
              label="Let Claude Code open the pictures folder"
              on={claudeOn ? !!st?.folder_allowed : folder}
              set={setFolderAllowed}
            />
          </Row>
        ) : null}
      </div>
      <p className="group-note">
        Set up writes to each agent's own user settings, never a project folder: the entry holds this computer's key.
      </p>
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
      <div className="t-small about-credits">
        <div className="about-credits-head">Speech recognition</div>
        <ul>
          <li>Whisper by OpenAI (MIT)</li>
          <li>whisper.cpp (MIT)</li>
          <li>Moonshine by Useful Sensors (MIT)</li>
          <li>Parakeet TDT by NVIDIA, used under CC-BY-4.0</li>
          <li>sherpa-onnx (Apache-2.0)</li>
          <li>Noise reduction by nnnoiseless, a port of RNNoise (BSD-3-Clause)</li>
        </ul>
        <div>Speech models are downloaded at setup, not bundled.</div>
      </div>
    </div>
  );
}
