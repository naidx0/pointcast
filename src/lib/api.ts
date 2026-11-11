import { invoke } from "@tauri-apps/api/core";
import { emptyUsage, previewUsage, type Usage } from "./usage";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { PREVIEW_ANSWER, PREVIEW_WORDS, previewChunks, previewLevel, previewSnapshot, type LiveDelta, type LiveSnapshot } from "./live";

export const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
export const preview = !inTauri && typeof location !== "undefined" && location.search.includes("preview=1");

export type Phase = "idle" | "loading" | "recording" | "transcribing" | "ready" | "error";

export type UiState = {
  phase: Phase;
  mic: boolean;
  screen: boolean;
  partial: string;
  text: string;
  error: string | null;
  duration_ms: number;
  image_count: number;
  model_ready: boolean;
  level: number;
  /** After a stop: what happened to the prompt. "partial": Cancel saved what was done and kept the audio; "cut": the
   * same with Keep audio off, so the rest is lost. */
  notice: "pasted" | "copied" | "saved" | "kept" | "empty" | "partial" | "cut" | null;
  /** Audio recorded but not yet turned into text, in milliseconds of speech. */
  behind_ms?: number;
  /** The microphone is giving nothing ("silent") or very little ("quiet"). */
  quiet_mic?: "silent" | "quiet" | null;
  /** The speech model is slower than real time on this computer. */
  slow_engine?: boolean;
  /** While the last words are transcribed after a stop: how far that is. */
  progress?: Progress | null;
  /** The GPU engine's first start on this computer: the graphics card is compiling its shaders. */
  preparing_gpu?: boolean;
  /** Where the engine transcribes: "your graphics card" or "your processor"; null before a model is loaded. */
  engine_label?: string | null;
  /** After a stop: the History entry the prompt was saved as; null when nothing was saved. */
  entry?: string | null;
  /** Auto-paste was chosen but macOS would not take it (no Accessibility), so the prompt was copied instead. */
  paste_blocked?: boolean;
};

export type Progress = {
  done_ms: number;
  total_ms: number;
  /** 0 to 100, never back, and 100 only once every word is in. */
  percent: number;
  /** Nothing moved for 20 s: offer Cancel. */
  stalled: boolean;
  /** No longer sent: the time left was too often wrong, so only the bar and the percent are shown. */
  left_ms?: number | null;
};

/** A numbered pin, step or note on a picture; pins and steps share one number sequence. 0 is a note on the whole picture. */
/** `kind` absent: a pin. "note": a note on the whole picture (numbered 0), such as "1 area hidden". */
export type Label = { n: number; text: string; kind?: "step" | "note" };

export type SessionExport = {
  plain: string;
  markdown: string;
  html: string;
  images: {
    n: number;
    kind: string;
    width: number;
    height: number;
    labels: Label[];
    data_url: string;
    /** A key frame Pointcast picked from the recording. */
    auto: boolean;
  }[];
};

export type DetectedTool = { id: string; name: string; path: string; kind: string };

export type Access = "granted" | "unknown" | "denied";
export type PermissionKind = "microphone" | "screen" | "accessibility";
export type Permissions = Record<PermissionKind, Access> & { gated: Record<PermissionKind, boolean> };

export type Action = "session" | "full" | "region" | "zoom";
export type HotkeyView = { label: string; keys: string[]; default: string };

/** A model with a faster version for the graphics card (Whisper Turbo): whether it can be set up here, whether it is,
 * its download, and whether the processor version is on disk too. */
export type GpuVersion = {
  available: boolean;
  installed: boolean;
  bytes: number;
  cpu_installed: boolean;
  /** `bytes` in two: the GPU model, and the helper while it is still to download. */
  model_bytes?: number;
  helper_bytes?: number;
  /** Why the helper cannot run on this computer, once it was tried and found no graphics card. */
  unusable?: string;
};
/** The faster GPU version setting itself up in the background. */
export type GpuUpgrade = {
  state: "idle" | "downloading" | "checking" | "done" | "unusable" | "low_disk" | "metered" | "failed";
  percent: number;
  note?: string;
};
export type SpeechModel = {
  id: string;
  name: string;
  installed: boolean;
  bytes: number;
  gpu?: GpuVersion;
  /** Where the model in memory runs: on the graphics card, on the processor, or not said yet. */
  backend?: "gpu" | "cpu" | "unknown";
  /** The graphics card it runs on, as "NVIDIA GeForce RTX 2060 SUPER via Vulkan". */
  device?: string;
  /** How many times faster than real time it transcribed here, once measured. */
  speed_x?: number;
};
export type SpeechView = {
  runtime?: { id: string; installed: boolean; lib: string } | null;
  models?: SpeechModel[];
};

export type Output = "clipboard" | "auto_paste" | "folder";
export type TalkMode = "tap" | "hold";
export type MarkPoint = "off" | "ring" | "trail";
export type ZoomTarget = "pointer" | "dwell" | "window";
export type SpokenAnchors = "off" | "ring";

export type Config = {
  platform: string;
  materials: boolean;
  version: string;
  hotkeys: Record<Action, HotkeyView>;
  hotkey_errors: { action: Action; error: string }[];
  talk_mode: TalkMode;
  model: string;
  device: string | null;
  output: Output;
  also_clipboard: boolean;
  export_dir: string | null;
  zoom_px: number;
  mark_point: MarkPoint;
  zoom_target: ZoomTarget;
  zoom_sharpen: boolean;
  outline_element: boolean;
  spoken_anchors: SpokenAnchors;
  key_frames: boolean;
  /** Keep the audio of the newest recordings so they can be transcribed again. Treated as on when absent. */
  keep_audio?: boolean;
  /** Put the file path of each picture in the text, for terminal agents. Treated as on when absent. */
  picture_paths?: boolean;
  /** Paragraphs at the pauses, no ums and no repeated words when the prompt is composed. Treated as on when absent. */
  tidy?: boolean;
  /** Clean up room noise and even out the volume before transcribing. Treated as on when absent. */
  voice_boost?: boolean;
  /** Get the faster GPU version of Whisper Turbo by itself. Treated as on when absent. */
  auto_gpu?: boolean;
  /** Minutes after the speech model's last use before it is freed from memory; 0 keeps it loaded. Treated as 1 when absent. */
  free_memory_minutes?: number;
  gpu_upgrade?: GpuUpgrade;
  start_at_login: boolean;
  check_updates: boolean;
  /** A newer version the last check found. */
  update: string | null;
  /** An update is downloading or installing now. */
  update_installing?: boolean;
  /** Why the last update did not finish, if it failed. */
  update_failure?: string | null;
  /** What the last update check found, if one has run: a version, or why it could not tell. */
  update_checked?: { version: string | null; error: string | null } | null;
  include_app: boolean;
  hidden: string[];
  mcp_bind: string;
  mcp_enabled: boolean;
  /** Why sharing with agents did not start at launch. */
  mcp_error?: string | null;
  onboarded: boolean;
  sounds: boolean;
  speech: SpeechView;
  /** How the last load-and-run of the speech engine went; null before the first. */
  speech_check: { ok: boolean; error: string | null } | null;
  /** The model a recording would load now (the picked one, or the installed one it falls back to). */
  speech_in_use?: string | null;
  devices: string[];
  /** The system's default microphone, by name. */
  default_device?: string | null;
  /** The microphone of the newest recordings, newest first. */
  mic_recent?: MicRecent[];
  /** This computer, as measured at launch. */
  machine?: Machine;
  /** Cancel was pressed on the graphics-card download: not tried again until asked. */
  gpu_declined?: boolean;
  permissions: Permissions;
  recommended: string;
};

export const HOTKEY_FIELD: Record<Action, string> = {
  session: "session_hotkey",
  full: "full_hotkey",
  region: "region_hotkey",
  zoom: "zoom_hotkey",
};

export const settingsGet = () => invoke<Config>("settings_get");
export const settingsPatch = (patch: Record<string, unknown>) => invoke<Config>("settings_patch", { patch });
export const sessionToggle = () => invoke<UiState>("session_toggle");
export const sessionState = () => invoke<UiState>("session_state");
export const sessionCancel = () => invoke<void>("session_cancel");
/** Stop waiting for the last words: save what is transcribed now and keep the audio for Transcribe again. */
export const sessionFinishNow = () => invoke<void>("session_finish_now");
/** The capsule's end notice has the pointer or focus, or lost it: it stays while held and a moment after. */
export const hudHold = (over: boolean) => invoke<void>("hud_hold", { over });
/** Open History, on entry `id` when given. */
export const historyOpen = (id: string | null) => invoke<void>("history_open", { id });
export const captureFull = () => invoke<void>("capture_full");
export const captureZoom = () => invoke<void>("capture_zoom");
export const regionBegin = () => invoke<{ png_b64: string; width: number; height: number }>("region_begin");
/** The control under the pointer, its rect in picture pixels. */
export type ElementMark = { role: string; name: string; rect: { x: number; y: number; w: number; h: number } };
/** `hidden`: how many areas the person blacked out; the prompt says so ("1 area hidden"). */
export const regionCommit = (pngB64: string, labels: Label[], element: ElementMark | null = null, hidden = 0) =>
  invoke<void>("region_commit", { body: { png_b64: pngB64, labels, element, hidden } });
export const regionCancel = () => invoke<void>("region_cancel");
export const regionFrame = () =>
  invoke<{
    png_b64: string;
    width: number;
    height: number;
    pointer?: { x: number; y: number } | null;
    element?: ElementMark | null;
    /** A picture from History: its labels already drawn in, new pins numbered on from `base`. */
    edit?: boolean;
    base?: number;
  } | null>(
    "region_frame",
  );
export const lastPrompt = () => invoke<SessionExport | null>("last_prompt");
export const lastFolder = () => invoke<string | null>("last_folder");
export const openFolder = (path: string) => invoke<void>("open_folder", { path });
export const copyAgain = () => invoke<void>("copy_again");
export type AgentId = "claude" | "cursor" | "codex";
/** One coding agent as Settings shows it. Never carries the key. */
export type AgentView = {
  id: AgentId;
  name: string;
  /** The user-scope file Pointcast writes, as people know it ("~/.claude.json"); the full path when POINTCAST_HOME is set. */
  path: string;
  /** The full path of that file, always: the row's tooltip. */
  file: string;
  /** The agent has run on this computer (its folder is there). */
  found: boolean;
  /** "current": this address and key; "other": a pointcast entry with an old address or key. */
  entry: "missing" | "current" | "other";
  /** The file could not be read, in plain words. */
  error: string | null;
};
export type Agents = {
  agents: AgentView[];
  /** The agent server answered an MCP client just now. */
  server_ok: boolean;
  server_error: string | null;
  /** Where each recording's pictures go, and whether Claude Code may open it without asking. */
  folder: string;
  folder_allowed: boolean;
};
/** Copies the MCP setup for one coding agent, with this install's key, straight to the clipboard. */
export const mcpCopySetup = (agent: AgentId) => invoke<void>("mcp_copy_setup", { agent });
/** What was dictated, as running counts, for the Usage pane. Outside Tauri: ten weeks of a stand-in, or with
 * `?usage=empty` an install that has recorded nothing. */
export const statsGet = () => (inTauri ? invoke<Usage>("stats_get") : Promise.resolve(previewStats()));
/** Start counting again from now. History is left as it is. */
export const statsReset = () => (inTauri ? invoke<Usage>("stats_reset") : Promise.resolve(emptyUsage(Date.now())));
const previewStats = () => (location.search.includes("usage=empty") ? emptyUsage(Date.now()) : previewUsage(new Date()));
/** Outside Tauri: a stand-in, `?agents=none|connected|notfound|error` picks which. */
export const agentsStatus = () => (inTauri ? invoke<Agents>("agents_status") : Promise.resolve(previewAgents()));
/** Writes Pointcast into the agent's user settings (turning sharing on first), then checks it answers. */
export const agentConnect = (agent: AgentId, folder: boolean) => invoke<Agents>("agent_connect", { agent, folder });
/** Takes only Pointcast's entry out of the agent's settings. */
export const agentRemove = (agent: AgentId) => invoke<Agents>("agent_remove", { agent });
/** Lets Claude Code open the pictures folder without asking, or stops it. */
export const agentFolder = (on: boolean) => invoke<Agents>("agent_folder", { on });

export function previewAgents(): Agents {
  const state = typeof location !== "undefined" ? new URLSearchParams(location.search).get("agents") ?? "none" : "none";
  const row = (id: AgentId, name: string, path: string): AgentView => ({
    id,
    name,
    path,
    file: path.replace("~", "C:/Users/you"),
    found: true,
    entry: "missing",
    error: null,
  });
  const agents = [
    row("claude", "Claude Code", "~/.claude.json"),
    row("cursor", "Cursor", "~/.cursor/mcp.json"),
    row("codex", "Codex", "~/.codex/config.toml"),
  ];
  if (state === "connected") agents.forEach((a) => (a.entry = "current"));
  if (state === "notfound") {
    agents[0].entry = "current";
    agents[1].found = false;
    agents[2].found = false;
  }
  if (state === "error") {
    agents[0].entry = "other";
    agents[1].error = "~/.cursor/mcp.json is not plain JSON (it may have comments), so Pointcast left it unchanged.";
    agents[2].entry = "current";
  }
  return {
    agents,
    server_ok: state !== "error",
    server_error: state === "error" ? "127.0.0.1:47821 is already in use" : null,
    folder: "~/Pointcast",
    folder_allowed: state === "connected" || state === "notfound",
  };
}
export const pasteAgain = () => invoke<void>("paste_again");
export type SpeechProgress = { phase: "download" | "unpack" | "check"; part?: string; received?: number; total?: number };
export const speechSetup = (id: string) => invoke<void>("speech_setup", { id });
export const speechCancel = () => invoke<void>("speech_cancel");
export const speechStatus = () => invoke<SpeechProgress | null>("speech_status");
export const speechRemove = (id: string) => invoke<Config>("speech_remove", { id });
/** Stop the faster GPU version downloading by itself; it is not tried again until Whisper Turbo is picked again. */
export const gpuUpgradeCancel = () => invoke<void>("gpu_upgrade_cancel");
export const permissionsGet = () => invoke<Permissions>("permissions_get");
export const permissionRequest = (kind: PermissionKind) => invoke<Permissions>("permission_request", { kind });
export const permissionOpen = (kind: PermissionKind) => invoke<void>("permission_open", { kind });
export const relaunch = () => invoke<void>("relaunch");
export const detectTools = () => invoke<DetectedTool[]>("detect_tools");
export const openTool = (id: string) => invoke<void>("open_tool", { id });
export const sendToTool = (id: string) => invoke<void>("send_to_tool", { id });
export const hotkeysSuspend = (on: boolean) => invoke<Config>("hotkeys_suspend", { on });
export const hotkeyTest = (on: boolean) => invoke<void>("hotkey_test", { on });
export const practice = (on: boolean) => invoke<void>("practice", { on });
/** Open or close the microphone check. With `leaseMs` it closes by itself unless asked again within that time. */
export const micMonitor = (on: boolean, leaseMs?: number) => invoke<void>("mic_monitor", { on, leaseMs });
/** Listen briefly to the other microphones: the ones that picked up sound, loudest first (dBFS). */
export const micScan = () => invoke<{ name: string; peak_db: number }[]>("mic_scan");
/** Forget that the graphics card could not run the engine, or that its download was stopped, so it is tried again. */
export const gpuRetry = () => invoke<void>("gpu_retry");
export const onboardingFinish = () => invoke<void>("onboarding_finish");
/** Open at login for this user; `machine` is an older entry for every user that only an administrator can remove. */
export type Login = { enabled: boolean; machine: boolean };
export const loginGet = () => invoke<Login>("login_get");
export const loginSet = (on: boolean) => invoke<Login>("login_set", { on });
export const updateCheck = () => invoke<string | null>("update_check");
export const updateInstall = () => invoke<void>("update_install");
export const windowOpen = (label: string) => invoke<void>("window_open", { label });
export const platformInfo = () => invoke<{ os: string; materials: boolean }>("platform_info");

export type MicRecent = { mic: string; open_ms: number; peak_db: number };
export type Machine = {
  cpu?: string;
  cores: number;
  ram_gb: number;
  /** The graphics card and its own memory in GB. */
  gpu?: [string, number] | null;
  gpu_integrated: boolean;
  mac: boolean;
  apple_silicon: boolean;
  /** A graphics-card version of the speech engine is published for this computer. */
  helper: boolean;
};

export type HistoryMeta = {
  id: string;
  created_ms: number;
  duration_ms: number;
  preview: string;
  images: number;
  recovered: boolean;
  /** The recording's audio is still on this computer, so it can be transcribed again. */
  has_audio?: boolean;
  /** Saved before every word was transcribed (Cancel at the end of a recording). */
  partial?: boolean;
  /** How it was last transcribed: "gpu", "cpu-whisper", "cpu-sherpa", "moonshine" or "parakeet"; empty when not known. */
  engine?: string;
  /** The graphics card it ran on. */
  device?: string;
  /** The engine's time on it and the length of its audio, in ms. */
  transcribe_ms?: number;
  audio_ms?: number;
  /** The microphone it was recorded on; empty for an entry from before this was kept. */
  mic?: string;
  /** From the press to the microphone's first sample, in ms. */
  mic_open_ms?: number;
  /** The loudest sample of the recording, in dBFS (-100 for nothing but zeros). */
  mic_peak_db?: number;
  /** How many words the prompt kept; absent from builds before 0.5. */
  words?: number;
};

/** A graphics card's short name: "NVIDIA GeForce RTX 2060 SUPER via Vulkan" is "RTX 2060 SUPER". */
export function shortDevice(d: string) {
  return d
    .replace(/ via .*$/, "")
    .replace(/^(NVIDIA |AMD |Intel[(]R[)] |Intel )/, "")
    .replace(/^GeForce /, "")
    .trim();
}

/** "20x real time" (whole numbers from 10 up, one decimal below). */
export function realTime(audioMs: number, spentMs: number) {
  const x = audioMs / Math.max(1, spentMs);
  return `${x >= 10 ? Math.round(x) : Math.round(x * 10) / 10}x real time`;
}

/** How a History entry was transcribed, as a quiet line under it; empty for an entry from before this was kept. */
export function ranText(m: Pick<HistoryMeta, "engine" | "device" | "transcribe_ms" | "audio_ms">) {
  if (!m.engine || !m.transcribe_ms || !m.audio_ms) return "";
  const s = m.transcribe_ms < 10_000 ? Math.max(1, Math.round(m.transcribe_ms / 100) / 10) : Math.round(m.transcribe_ms / 1000);
  const speed = `in ${s} s · ${realTime(m.audio_ms, m.transcribe_ms)}`;
  if (m.engine === "gpu") return `Transcribed on your graphics card${m.device ? ` (${shortDevice(m.device)})` : ""} ${speed}`;
  const hint = m.engine === "cpu-sherpa" || m.engine === "cpu-whisper" ? ". A graphics card is much faster: Settings > Speech." : "";
  return `Transcribed on your processor ${speed}${hint}`;
}
export const historyList = () => invoke<{ entries: HistoryMeta[]; last: string | null }>("history_list");
/** History's entries for Home; in the browser preview, a few made-up recordings (none with `usage=empty`). */
export const historyRecent = (): Promise<HistoryMeta[]> =>
  inTauri ? historyList().then((h) => h.entries) : Promise.resolve(location.search.includes("usage=empty") ? [] : previewHistory(Date.now()));

/** Browser preview: six recordings over the last ten days, the newest a few minutes ago. */
export function previewHistory(now: number): HistoryMeta[] {
  const min = 60_000;
  const said: [number, number, number, string][] = [
    // minutes ago, seconds long, pictures, first words
    [7, 18, 2, "Move the save button to the right of the title and give it the primary style, same height as the search field"],
    [134, 42, 1, "The chart labels overlap on small screens, wrap them under the axis instead of rotating them"],
    [26 * 60 + 25, 9, 0, "Reply to Sam: the build is ready for review tomorrow morning"],
    [50 * 60 + 10, 96, 3, "Walk through the onboarding again and note every step where the copy runs longer than one line"],
    [4 * 24 * 60 + 300, 27, 0, "Rename the export setting to Save to folder and keep the old key working"],
    [9 * 24 * 60 + 90, 64, 1, "This error appears when the network drops during upload, retry once before showing it"],
  ];
  return said.map(([ago, s, images, preview], i) => ({
    id: `preview-${i}`,
    created_ms: now - ago * min,
    duration_ms: s * 1000,
    preview: preview.split(" ").slice(0, 24).join(" "),
    images,
    recovered: false,
    // About 150 words a minute; `?words=none` plays a build from before the count.
    words: location.search.includes("words=none") ? undefined : Math.round(s * 2.5),
  }));
}
export const historyGet = (id: string) => invoke<SessionExport>("history_get", { id });
/** Reopen picture n (as `[image n]` numbers it) of a past prompt in the overlay to mark it up again. */
export const historyEdit = (id: string, n: number) => invoke<void>("history_edit", { id, n });
/** Take key frame n out of a past prompt; it becomes the last prompt and is copied again. */
export const historyRemove = (id: string, n: number) => invoke<SessionExport>("history_remove", { id, n });
export const historyCopy = (id: string) => invoke<void>("history_copy", { id });
export const historyPaste = (id: string) => invoke<void>("history_paste", { id });
/** Transcribe a kept recording again; `history://changed` fires when the new text is saved. */
export const historyRetranscribe = (id: string) => invoke<void>("history_retranscribe", { id });
/** Stop transcribing again after the piece under way; the prompt stays as it was. */
export const historyRetranscribeCancel = () => invoke<boolean>("history_retranscribe_cancel");
export const historyDelete = (id: string) => invoke<void>("history_delete", { id });
export const historyClear = () => invoke<void>("history_clear");

const on =
  <T,>(name: string) =>
  (fn: (p: T) => void): Promise<UnlistenFn> =>
    inTauri ? listen<T>(name, (e) => fn(e.payload)) : Promise.resolve(() => undefined);

export const onState = on<UiState>("session://state");
export const onResult = on<SessionExport>("session://result");
export const onPracticeResult = on<SessionExport>("practice://result");
/** The session shortcut while the overlay is open: capture what is marked, then finish the recording. */
export const onOverlayFinish = on<null>("overlay://finish");
export const onOverlayFrame = on<{ png_b64: string; width: number; height: number }>("overlay://frame");
export const onHistoryChanged = on<{ id: string }>("history://changed");
/** Open History on this entry (the capsule's notice was clicked). */
export const onHistorySelect = on<{ id: string }>("history://select");
export const onTranscribeProgress = on<{ id: string; done_ms: number; total_ms: number }>("transcribe://progress");
export const onSpeechProgress = on<SpeechProgress>("speech://progress");
export const onSpeechDone = on<{ id: string; ok: boolean; error?: string | null }>("speech://done");
export const onGpuUpgrade = on<GpuUpgrade>("gpu://upgrade");
export const onHotkey = on<{ action: Action; pressed: boolean }>("hotkey://fired");
export const onMicLevel = on<number>("mic://level");
/** The loudest sample of the microphone check's last 50 ms, in dBFS. */
export const onMicPeak = on<number>("mic://peak");
export const onUpdateChecked = on<string | null>("update://checked");
export const onUpdateFailed = on<string>("update://failed");
export const onSettingsPane = on<string>("settings://pane");

/** Subscribe in an effect; returns the cleanup. */
export function subscribe(...subs: Promise<UnlistenFn>[]) {
  return () => subs.forEach((u) => u.then((fn) => fn()));
}

// —— The Ask panel (live.html): the one adapter. Every engine name the panel uses is in LIVE, so a rename is one edit. ——

export const LIVE = {
  /** event: a full snapshot (LiveSnapshot) on every change of state */
  state: "live://state",
  /** event: { turn, text } to append to the answer */
  delta: "live://delta",
  /** command: the current snapshot */
  get: "live_state",
  drop: "live_drop_picture",
  stop: "live_stop",
  close: "live_close",
  /** The microphone level while listening: the capsule's own session stream (UiState.level), read as it is. */
  level: "session://state",
} as const;

type Sink<T> = (p: T) => void;
const livePv = {
  snap: null as LiveSnapshot | null,
  state: new Set<Sink<LiveSnapshot>>(),
  delta: new Set<Sink<LiveDelta>>(),
  level: new Set<Sink<number>>(),
  timers: [] as ReturnType<typeof setTimeout>[],
};
const pvOn =
  <T,>(set: Set<Sink<T>>) =>
  (fn: Sink<T>): Promise<UnlistenFn> => {
    set.add(fn);
    return Promise.resolve(() => void set.delete(fn));
  };
const pvEmit = (s: LiveSnapshot) => {
  livePv.snap = s;
  livePv.state.forEach((f) => f(s));
};

export const liveState = (): Promise<LiveSnapshot | null> => (inTauri ? invoke<LiveSnapshot>(LIVE.get) : Promise.resolve(livePv.snap));
export const onLiveState = (fn: Sink<LiveSnapshot>) => (inTauri ? listen<LiveSnapshot>(LIVE.state, (e) => fn(e.payload)) : pvOn(livePv.state)(fn));
export const onLiveDelta = (fn: Sink<LiveDelta>) => (inTauri ? listen<LiveDelta>(LIVE.delta, (e) => fn(e.payload)) : pvOn(livePv.delta)(fn));
export const onLiveLevel = (fn: Sink<number>) =>
  inTauri ? listen<UiState>(LIVE.level, (e) => fn(e.payload.mic ? Math.min(1, Math.max(0, e.payload.level)) : 0)) : pvOn(livePv.level)(fn);
export const liveDropPicture = () => (inTauri ? invoke<void>(LIVE.drop) : pvCommand("drop"));
export const liveStop = () => (inTauri ? invoke<void>(LIVE.stop) : pvCommand("stop"));
export const liveClose = () => (inTauri ? invoke<void>(LIVE.close) : pvCommand("close"));
export const liveOpenSettings = () => (inTauri ? windowOpen("settings") : Promise.resolve());
/** The panel's height follows its content; the window keeps its width and place (Rust docks it). */
export const liveFit = (height: number) =>
  inTauri ? getCurrentWindow().setSize(new LogicalSize(380, Math.round(height))) : Promise.resolve();

function pvCommand(c: "drop" | "stop" | "close"): Promise<void> {
  const s = livePv.snap;
  if (!s) return Promise.resolve();
  if (c === "drop") pvEmit({ ...s, dropped: true });
  else if (c === "close") pvEmit({ ...s, state: "closed" });
  else pvEmit(s.state === "answering" ? { ...s, state: "shown" } : { ...s, state: "closed" });
  return Promise.resolve();
}

/** The browser preview: live.html?preview=1&state=listening|finishing|thinking|answering|shown|error[&error=provider]
 * plays the fixtures in src/lib/live.ts: a synthetic microphone for listening, the scripted stream for answering (which
 * ends in shown); thinking shows its 8 s and 20 s lines when left that long. */
export function startLivePreview(kind?: string) {
  livePv.timers.forEach(clearTimeout);
  livePv.timers = [];
  const q = new URLSearchParams(location.search);
  const state = kind ?? q.get("state") ?? "thinking";
  const later = (ms: number, f: () => void) => livePv.timers.push(setTimeout(f, ms));
  const snap = previewSnapshot(state, { error: q.get("error") });
  pvEmit(snap);
  if (state === "listening") {
    const words = PREVIEW_WORDS.split(" ");
    let i = 0;
    const tick = () => {
      livePv.level.forEach((f) => f(previewLevel(i)));
      if (i % 4 === 3 && livePv.snap?.state === "listening") {
        const n = Math.min(words.length, Math.floor(i / 4));
        const said = words.slice(0, n).join(" ");
        if (said !== livePv.snap.words) pvEmit({ ...livePv.snap, words: said, session: { ...livePv.snap.session, title: n >= 6 ? words.slice(0, 6).join(" ") : "" } });
      }
      i++;
      later(90, tick);
    };
    tick();
  } else if (state === "answering") {
    const chunks = previewChunks(PREVIEW_ANSWER);
    let i = 0;
    const tick = () => {
      if (livePv.snap?.state !== "answering") return;
      if (i >= chunks.length) return pvEmit({ ...livePv.snap, state: "shown" });
      livePv.delta.forEach((f) => f({ turn: snap.turn, text: chunks[i] }));
      i++;
      later(Number(q.get("speed")) || 32, tick);
    };
    later(400, tick);
  }
}

export function stamp(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function mb(bytes: number) {
  return `${Math.max(1, Math.round(bytes / 1_000_000))} MB`;
}

/** Sizes we know even when the catalog has no byte count yet. */
export const MODEL_INFO: Record<string, { size: string; blurb: string; short: string }> = {
  "moonshine-base-en": { size: "111 MB", short: "Accurate", blurb: "Best accuracy. Fast on any recent computer." },
  "moonshine-tiny-en": { size: "30 MB", short: "Fast", blurb: "Smallest and quickest. Good for short notes." },
};
