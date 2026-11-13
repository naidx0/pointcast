import { invoke } from "@tauri-apps/api/core";
import { emptyUsage, previewUsage, type Usage } from "./usage";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { LogicalSize } from "@tauri-apps/api/dpi";
import { chainOf, dragAct } from "./drag";
import {
  LEVEL_FIELDS,
  PANEL_W,
  PREVIEW_ANSWER,
  currentTurn,
  firstWords,
  levelOf,
  openNumber,
  previewChunks,
  previewLevel,
  previewPicture,
  previewSnapshot,
  providerName,
  type LiveDelta,
  type LiveSnapshot,
  type LiveTurn,
} from "./live";
import { previewSent } from "./conversation";
import { normalizeSessions, previewSearch, previewSessions, type SessionMeta } from "./sessions";
import { countsLine, normalizeSummary, previewSummary, previewSummaryOf, type Summary } from "./summary";
import { CUSTOM, checkingAssist, normalizeAssist, normalizeModels, previewApply, previewAssist, previewModels, type Assist, type AssistModel } from "./assist";

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
export type Permissions = Record<PermissionKind, Access> & {
  gated: Record<PermissionKind, boolean>;
  /** Screen Recording is switched on, but macOS applies it only once Pointcast reopens. */
  screen_pending?: boolean;
};

export type Action = "session" | "full" | "region" | "zoom" | "ask";
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
  /** `ask` is the Ask key; absent from builds before 0.5. */
  hotkeys: Record<Action, HotkeyView> & { ask?: HotkeyView };
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
  /** Ask was introduced: the setup assistant's step, or Home's one-time card. */
  ask_intro_seen?: boolean;
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
  ask: "ask_hotkey",
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
    /** Opened by `live_mark` on a question's picture; new pins number on from `base`. */
    ask?: boolean;
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
  /** command: ask the kept question again after an error */
  retry: "live_retry",
  drop: "live_drop_picture",
  stop: "live_stop",
  close: "live_close",
  /** The microphone level while listening: the capsule's own session stream (UiState), read as it is. */
  level: "session://state",
  /** The level's fields on UiState, first found wins: an engine-smoothed level if the engine sends one, else the raw
   * meter level (0 to 1 over 50 dB). */
  levelFields: LEVEL_FIELDS,
  /** command { text, picture: boolean }: a typed follow-up in the open session. It takes no picture itself: the one
   * waiting above the field (`next_picture`, taken by `take` or marked up) goes with it, else none. An error string
   * while a question is under way ("A question is already under way.") */
  askTyped: "live_ask_typed",
  /** command: the field's picture switch went on: one picture of the screen, taken now, waits above the field as
   * `next_picture` (marked: false) and goes with the next question. Refused while a question is under way. Switched
   * off, `drop` takes it away. */
  take: "live_take_picture",
  /** command { turn } → { text, pictures: [{ n, w, h, bytes, mime, src, turn }], earlier: number[], provider, model,
   * adds: string[], replay?: string, again?: [{ n, w, h, bytes, mime, src, turn }] }. `turn` is the turn's number in
   * the session; `replay` and `again` are the earlier turns sent again when the conversation had to be reopened. */
  sent: "live_sent",
  /** command: open the mark-up overlay on the question's picture (its frame says `purpose: "ask"`); afterwards the
   * snapshot's `picture.marked` is true, or the picture is as it was when the overlay was cancelled */
  mark: "live_mark",
} as const;

/** Ask's kept sessions: the one adapter. Every engine name the session list and Home use is here. */
export const SESSIONS = {
  /** command → [{ id, title, updated_ms, turns, pictures, provider, size_bytes, has_summary }] */
  list: "sessions_list",
  /** command { id }: the panel opens on that session */
  open: "sessions_open",
  /** command: a new, empty session; the panel stays open on nothing (shown, no turns) */
  new: "sessions_new",
  /** command { id, title } */
  rename: "sessions_rename",
  /** command { id } */
  delete: "sessions_delete",
  /** command { q } → the list's shape, each with `hit` */
  search: "sessions_search",
  /** command { id } → the folder it was saved to (pictures may be .jpg) */
  export: "sessions_export",
} as const;

/** A session's summary: the one adapter. Every engine name the summary view uses is here. */
export const SUMMARY = {
  /** command { id }: make (or make again) the summary of a session; how it goes arrives as `state` */
  make: "summary_make",
  /** command { id } */
  stop: "summary_stop",
  /** command { id } → the kept summary ({ title, key_points, open_questions, kept, dropped, made_ms, provider, model })
   * or null */
  get: "summary_get",
  /** event { id, state: "working" | "done" | "error" | "stopped", summary, counts, error } */
  state: "summary://state",
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
  inTauri ? listen<UiState>(LIVE.level, (e) => fn(levelOf(e.payload, LIVE.levelFields))) : pvOn(livePv.level)(fn);
export const liveDropPicture = () => (inTauri ? invoke<void>(LIVE.drop) : pvCommand("drop"));
export const liveStop = () => (inTauri ? invoke<void>(LIVE.stop) : pvCommand("stop"));
export const liveClose = () => (inTauri ? invoke<void>(LIVE.close) : pvCommand("close"));
export const liveRetry = () => (inTauri ? invoke<void>(LIVE.retry) : pvCommand("retry"));
/** A typed follow-up; the picture waiting above the field, if there is one, goes with it. */
export const liveAskTyped = (text: string, picture: boolean) =>
  inTauri ? invoke<void>(LIVE.askTyped, { text, picture }) : pvAskTyped(text, picture);
/** Take a picture of the screen now; it waits above the field, shown, for the next question. */
export const liveTakePicture = () => (inTauri ? invoke<void>(LIVE.take) : pvTake());
/** Exactly what left the computer for a turn (its number in the session), as the engine kept it (read with sentView in
 * conversation.ts). */
export const liveSent = (turn: number): Promise<unknown> => (inTauri ? invoke<unknown>(LIVE.sent, { turn }) : pvLater(160, () => pvSent(turn)));
/** Mark up the question's picture in the overlay. */
export const liveMark = () => (inTauri ? invoke<void>(LIVE.mark) : pvMark());

export const summaryMake = (id: string) => (inTauri ? invoke<void>(SUMMARY.make, { id }) : pvSummaryMake(id));
export const summaryStop = (id: string) => (inTauri ? invoke<void>(SUMMARY.stop, { id }) : pvSummaryStop(id));
export const summaryGet = (id: string): Promise<Summary | null> =>
  (inTauri ? invoke<unknown>(SUMMARY.get, { id }) : pvLater(120, () => summaryPv.kept.get(id) ?? null)).then(normalizeSummary);
/** The raw payload: read with summaryReduce in summary.ts. */
export const onSummaryState = (fn: Sink<unknown>) => (inTauri ? listen<unknown>(SUMMARY.state, (e) => fn(e.payload)) : pvOn(summaryPv.sinks)(fn));

export const sessionsList = (): Promise<SessionMeta[]> => (inTauri ? invoke<unknown>(SESSIONS.list) : Promise.resolve(pvSessions())).then(normalizeSessions);
export const sessionsSearch = (q: string): Promise<SessionMeta[]> =>
  (inTauri ? invoke<unknown>(SESSIONS.search, { q }) : Promise.resolve(previewSearch(pvSessions(), q))).then(normalizeSessions);
export const sessionsOpen = (id: string) => (inTauri ? invoke<void>(SESSIONS.open, { id }) : pvOpen(id));
/** The next question starts a session of its own. `show`: the panel opens on it too, its field ready (Home). */
export const sessionsNew = (show = false) => (inTauri ? invoke<void>(SESSIONS.new, { show }) : pvNew());
export const sessionsRename = (id: string, title: string) => (inTauri ? invoke<void>(SESSIONS.rename, { id, title }) : pvRename(id, title));
export const sessionsDelete = (id: string) => (inTauri ? invoke<void>(SESSIONS.delete, { id }) : pvDelete(id));
export const sessionsExport = (id: string): Promise<string> =>
  inTauri
    ? invoke<string>(SESSIONS.export, { id })
    : Promise.resolve(
        `${location.search.includes("skin=mac") || location.search.includes("os=mac") ? "/Users/you" : "C:/Users/you"}/Documents/Pointcast/Ask/${pvSessions().find((s) => s.id === id)?.title ?? id}`,
      );

// —— Home's "All sessions" opens the panel on its session list. The panel may not be built yet, so the ask goes through
// storage, as Settings' pane does: read once as the panel opens, or as a storage event when it is open. ——

const LIST_KEY = "pointcast.asklist";

/** Open the panel on `newest` with its session list showing. */
export function openSessionList(newest: string): Promise<void> {
  try {
    localStorage.setItem(LIST_KEY, String(Date.now()));
  } catch {
    /* private mode: the panel opens on the session */
  }
  return sessionsOpen(newest);
}

/** The panel, as it opens or when asked: whether Home asked for the list just now, once. */
export function takeListAsk(): boolean {
  try {
    const at = Number(localStorage.getItem(LIST_KEY));
    localStorage.removeItem(LIST_KEY);
    return Number.isFinite(at) && at > 0 && Date.now() - at < PANE_FRESH_MS;
  } catch {
    return false;
  }
}

export function onListAsk(fn: () => void): () => void {
  const f = (e: StorageEvent) => {
    if (e.key === LIST_KEY && e.newValue && takeListAsk()) fn();
  };
  window.addEventListener("storage", f);
  return () => window.removeEventListener("storage", f);
}
/** Every error that asks for Settings is about the AI: Settings opens on that pane. */
export const liveOpenSettings = () => settingsOpenOn("ai");
/** The panel's height follows its content, at the width the person chose; Rust keeps it docked at the top right. */
export const liveFit = (height: number, width: number = PANEL_W) =>
  inTauri ? getCurrentWindow().setSize(new LogicalSize(Math.round(width), Math.round(height))) : Promise.resolve();

/** The Ask panel's size as the person last dragged it; it lives in this computer's page storage. */
const PANEL_SIZE_KEY = "pointcast.panelSize";
export function panelSizeRead(): string | null {
  try {
    return localStorage.getItem(PANEL_SIZE_KEY);
  } catch {
    return null;
  }
}
export function panelSizeSave(size: { w: number; h: number | null } | null) {
  try {
    if (size) localStorage.setItem(PANEL_SIZE_KEY, JSON.stringify(size));
    else localStorage.removeItem(PANEL_SIZE_KEY);
  } catch {
    /* nothing to keep it in: it lasts until the panel closes */
  }
}

function pvCommand(c: "drop" | "stop" | "close" | "retry"): Promise<void> {
  const s = livePv.snap;
  if (!s) return Promise.resolve();
  if (c === "retry") {
    startLivePreview("answering");
    return Promise.resolve();
  }
  // Drop takes off a picture waiting for the next question first, as the engine does.
  if (c === "drop") pvEmit(s.next_picture ? { ...s, next_picture: null } : { ...s, dropped: true });
  else if (c === "close") {
    livePv.timers.forEach(clearTimeout);
    pvEmit({ ...s, state: "closed" });
  }
  // Stop: an answer under way ends as shown; a question still being heard is thrown away, as the engine does.
  else if (s.state === "answering") pvEmit({ ...s, state: "shown" });
  else if (s.state === "listening" || s.state === "finishing") {
    livePv.timers.forEach(clearTimeout);
    pvEmit({ ...s, state: "discarded" });
  } else pvEmit({ ...s, state: "closed" });
  return Promise.resolve();
}

// —— The preview's sessions and follow-ups: the fixtures in sessions.ts and live.ts, played as the engine would. ——

const sessionsPv = { list: null as SessionMeta[] | null };
const pvSessions = () => (sessionsPv.list ??= location.search.includes("sessions=empty") ? [] : previewSessions(Date.now()));

/** A follow-up answered in a few short pieces, so the preview shows a typed question's whole round. */
const FOLLOW_ANSWER = "Yes. Without the bottom, the top is a count of cases, not a share of them: dividing by **P(B)** is what makes the shares add up to 1.";

/** `replay=1`: the conversation was reopened, so the earlier turns went again with each question. */
function pvSent(turn: number) {
  const s = livePv.snap;
  const all = s ? keptTurns(s) : [];
  const t: Pick<LiveTurn, "n" | "question" | "picture"> | null = all.find((x) => x.n === turn) ?? null;
  return previewSent(t, providerName(s?.provider ?? null) || "ChatGPT", { replay: location.search.includes("replay=1") ? all : [] });
}

/** Every turn of the session, the open one kept too: what the engine lists in `turns` once the next question begins. */
function keptTurns(s: LiveSnapshot): LiveTurn[] {
  if (currentTurn(s) || !s.words.trim()) return s.turns;
  const n = openNumber(s);
  if (s.turns.some((t) => t.n === n)) return s.turns;
  return [...s.turns, { n, question: s.words, answer: s.answer ?? PREVIEW_ANSWER, picture: s.dropped ? null : s.picture, stopped: false, error: null, at_ms: Date.now() }];
}

function pvAskTyped(text: string, _picture: boolean): Promise<void> {
  const s = livePv.snap;
  if (!s) return Promise.resolve();
  livePv.timers.forEach(clearTimeout);
  livePv.timers = [];
  // The open turn joins the earlier ones; the new one is kept at once, so the session counts it.
  const turns = keptTurns(s);
  const n = turns.reduce((m, t) => Math.max(m, t.n), 0) + 1;
  const id = s.session.id || "new";
  const title = s.session.title || firstWords(text);
  if (!s.session.id)
    sessionsPv.list = [{ id, title, updated_ms: Date.now(), turns: 1, pictures: s.next_picture ? 1 : 0, provider: "ChatGPT", size_bytes: 0, has_summary: false }, ...pvSessions()];
  const next: LiveSnapshot = {
    ...s,
    state: "sending",
    turn: s.turn + 1,
    words: text,
    answer: undefined,
    thinking: false,
    error: null,
    dropped: false,
    provider: s.provider || "ChatGPT",
    // Only the picture waiting above the field goes, as the engine sends it; none is taken as it is sent.
    picture: s.next_picture ?? null,
    next_picture: null,
    turns,
    session: { id, title, turns: n },
  };
  pvEmit(next);
  pvStream(FOLLOW_ANSWER, 900);
  return Promise.resolve();
}

function pvOpen(id: string): Promise<void> {
  livePv.timers.forEach(clearTimeout);
  livePv.timers = [];
  const meta = pvSessions().find((s) => s.id === id);
  if (!meta) return Promise.reject("That session is gone");
  if (id === "preview") {
    pvEmit({ ...previewSnapshot("opened"), session: { id, title: meta.title, turns: 3 } });
    return Promise.resolve();
  }
  // Opened finished, with nothing in the panel's own fields: the panel shows the newest kept turn. The engine names no
  // AI until the next question.
  const turns: LiveTurn[] = Array.from({ length: Math.max(1, meta.turns) }, (_, i) => ({
    n: i + 1,
    question: i === 0 ? `${meta.title}?` : `And what about part ${i + 1}?`,
    answer: i === 0 ? "Here is what the picture shows, in short." : `Part ${i + 1} works the same way.`,
    picture: i === 0 ? previewPicture("code") : null,
    stopped: false,
    error: null,
    at_ms: meta.updated_ms,
  }));
  pvEmit({ ...previewSnapshot("opened"), session: { id, title: meta.title, turns: turns.length }, turns });
  return Promise.resolve();
}

/** The switch went on: a picture of the screen waits above the field, unmarked. Refused as the engine refuses. */
function pvTake(): Promise<void> {
  const s = livePv.snap;
  if (!s) return Promise.resolve();
  if (s.state === "listening" || s.state === "finishing" || s.state === "sending" || s.state === "answering") return Promise.reject("A question is already under way.");
  if (!s.next_picture) pvEmit({ ...s, next_picture: { ...previewPicture("code"), marked: false } });
  return Promise.resolve();
}

/** The panel stays open on nothing: shown, no session, no turns. */
function pvNew(): Promise<void> {
  livePv.timers.forEach(clearTimeout);
  livePv.timers = [];
  pvEmit(previewSnapshot("empty"));
  return Promise.resolve();
}

/** The overlay is the engine's: the preview plays a mark-up used a moment later, on the question's picture while it
 * is heard, else on a picture of the screen that waits for the next question. Refused as the engine refuses. */
function pvMark(): Promise<void> {
  const s = livePv.snap;
  if (!s) return Promise.resolve();
  if (s.state === "finishing" || s.state === "sending" || s.state === "answering") return Promise.reject("A question is already under way.");
  const later = (f: (x: LiveSnapshot) => LiveSnapshot) => livePv.timers.push(setTimeout(() => livePv.snap && pvEmit(f(livePv.snap)), 500));
  if (s.state === "listening") {
    if (s.picture && !s.dropped) later((x) => (x.picture ? { ...x, picture: { ...x.picture, marked: true } } : x));
  } else later((x) => ({ ...x, next_picture: { ...previewPicture("code"), marked: true } }));
  return Promise.resolve();
}

// —— The preview's summaries: `summary=done` gives the preview session one; the one about the header always has one.
// `summary=working` makes one that never ends (for a look at the work under way). ——

const summaryPv = {
  sinks: new Set<Sink<unknown>>(),
  timer: undefined as ReturnType<typeof setTimeout> | undefined,
  kept: new Map<string, Summary>([
    ["s-header", previewSummaryOf("Why the header wraps on a small screen", "Codex")],
    ...(typeof location !== "undefined" && location.search.includes("summary=done") ? ([["preview", previewSummary()]] as [string, Summary][]) : []),
  ]),
};
const pvSummaryEmit = (e: Record<string, unknown>) => summaryPv.sinks.forEach((f) => f(e));

function pvSummaryMake(id: string): Promise<void> {
  clearTimeout(summaryPv.timer);
  pvSummaryEmit({ id, state: "working", summary: null, counts: "", error: null });
  if (location.search.includes("summary=working")) return Promise.resolve();
  summaryPv.timer = setTimeout(() => {
    const title = pvSessions().find((x) => x.id === id)?.title ?? "This session";
    const s = id === "preview" ? previewSummary() : previewSummaryOf(title);
    summaryPv.kept.set(id, s);
    sessionsPv.list = pvSessions().map((x) => (x.id === id ? { ...x, has_summary: true } : x));
    pvSummaryEmit({ id, state: "done", summary: s, counts: countsLine(s.kept, s.dropped), error: null });
  }, 1600);
  return Promise.resolve();
}

function pvSummaryStop(id: string): Promise<void> {
  clearTimeout(summaryPv.timer);
  pvSummaryEmit({ id, state: "stopped", summary: null, counts: "", error: null });
  return Promise.resolve();
}

function pvRename(id: string, title: string): Promise<void> {
  sessionsPv.list = pvSessions().map((s) => (s.id === id ? { ...s, title } : s));
  const s = livePv.snap;
  if (s && s.session.id === id) pvEmit({ ...s, session: { ...s.session, title } });
  return Promise.resolve();
}

function pvDelete(id: string): Promise<void> {
  sessionsPv.list = pvSessions().filter((s) => s.id !== id);
  const s = livePv.snap;
  if (s && s.session.id === id) {
    livePv.timers.forEach(clearTimeout);
    pvEmit({ ...s, state: "closed" });
  }
  return Promise.resolve();
}

/** The scripted stream for the open turn, ending in shown with the answer kept. */
function pvStream(text: string, delay: number) {
  const q = new URLSearchParams(location.search);
  const chunks = previewChunks(text);
  const turn = livePv.snap?.turn ?? 0;
  let i = 0;
  const tick = () => {
    const s = livePv.snap;
    if (!s || s.turn !== turn || (s.state !== "answering" && s.state !== "sending")) return;
    if (s.state === "sending") pvEmit({ ...s, state: "answering" });
    if (i >= chunks.length) return pvEmit({ ...livePv.snap!, state: "shown", answer: text });
    livePv.delta.forEach((f) => f({ turn, text: chunks[i] }));
    i++;
    livePv.timers.push(setTimeout(tick, Number(q.get("speed")) || 32));
  };
  livePv.timers.push(setTimeout(tick, delay));
}

/** The browser preview: live.html?preview=1&state=listening|finishing|thinking|answering|shown|error|discarded|conversation
 * |opened|empty[&error=provider][&pic=switch][&provider=Codex][&marked=1][&replay=1][&summary=working|done] plays the
 * fixtures in src/lib/live.ts: a synthetic microphone for listening (with pic=switch the person moves to another app
 * after 2 s, so the picture changes), the scripted stream for answering (which ends in shown); thinking shows its 8 s
 * and 20 s lines when left that long. `conversation` is a session of three turns, the third open; a typed follow-up
 * there is answered by the script too. `opened` is a session opened from the list (`open=<id>` picks another one);
 * `empty` a new session. */
export function startLivePreview(kind?: string) {
  // `next=1`: a picture marked with no question under way waits for the next one.
  livePv.timers.forEach(clearTimeout);
  livePv.timers = [];
  const q = new URLSearchParams(location.search);
  const state = kind ?? q.get("state") ?? "thinking";
  const later = (ms: number, f: () => void) => livePv.timers.push(setTimeout(f, ms));
  const other = q.get("open");
  if (state === "opened" && other) {
    void pvOpen(other).catch(() => undefined);
    return;
  }
  const snap = previewSnapshot(state, { error: q.get("error"), provider: q.get("provider") ?? undefined, marked: q.get("marked") === "1", next: q.get("next") === "1", blind: q.get("blind") === "1" });
  pvEmit(snap);
  if (state === "listening" || state === "follow") {
    let i = 0;
    const tick = () => {
      livePv.level.forEach((f) => f(previewLevel(i)));
      if (i === 22 && q.get("pic") === "switch" && livePv.snap?.state === "listening") pvEmit({ ...livePv.snap, picture: previewPicture("code") });
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

// —— The window's own controls, for a window that draws its own title bar (Settings on Windows). In the browser
// preview they do nothing. ——

const win = () => getCurrentWindow();
export const winMinimize = () => (inTauri ? win().minimize() : Promise.resolve());
export const winToggleMaximize = () => (inTauri ? win().toggleMaximize() : Promise.resolve());
export const winClose = () => (inTauri ? win().close() : Promise.resolve());
export const winStartDragging = () => (inTauri ? win().startDragging() : Promise.resolve());
export const winIsMaximized = () => (inTauri ? win().isMaximized() : Promise.resolve(false));
/** Whether the OS draws this window's frame; in the preview, no (so the drawn bar shows). */
/** Every `.drag` area moves its window (src/lib/drag.ts): a press starts the native drag, a double press zooms, as a
 * title bar does. A handler that already moved the window says so with preventDefault. Call once per window. */
export function startWindowDrag() {
  if (!inTauri) return;
  document.addEventListener("mousedown", (e) => {
    if (e.defaultPrevented) return;
    const act = dragAct(chainOf(e.target as Element | null), e.button, e.detail);
    if (act === "move") void win().startDragging().catch(() => undefined);
    else if (act === "zoom") void win().toggleMaximize().catch(() => undefined);
  });
}
export const winIsDecorated = () => (inTauri ? win().isDecorated() : Promise.resolve(false));
export const onWinResized = (fn: () => void): Promise<UnlistenFn> => (inTauri ? win().onResized(() => fn()) : Promise.resolve(() => undefined));

// —— Opening Settings on a pane from another window. The window may not exist yet, so the ask goes through storage,
// which every Pointcast window shares (as the theme does): read once as Settings opens, or as a storage event when it
// is already open. ——

const PANE_KEY = "pointcast.pane";
/** How long an ask for a pane stays good: long enough for Settings to be built, short enough never to surprise. */
const PANE_FRESH_MS = 15_000;

export function settingsOpenOn(pane: string): Promise<void> {
  try {
    localStorage.setItem(PANE_KEY, `${pane}@${Date.now()}`);
  } catch {
    /* private mode: Settings opens where it was */
  }
  return inTauri ? windowOpen("settings") : Promise.resolve();
}

const readPaneAsk = (v: string | null, now: number) => {
  const m = /^([a-z]+)@(\d+)$/.exec(v ?? "");
  return m && now - Number(m[2]) < PANE_FRESH_MS ? m[1] : null;
};

/** Settings, as it opens: the pane another window asked for just now, once. */
export function takePaneAsk(): string | null {
  try {
    const p = readPaneAsk(localStorage.getItem(PANE_KEY), Date.now());
    localStorage.removeItem(PANE_KEY);
    return p;
  } catch {
    return null;
  }
}

/** Settings, while open: each pane another window asks for. Returns the cleanup. */
export function onPaneAsk(fn: (pane: string) => void): () => void {
  const f = (e: StorageEvent) => {
    if (e.key !== PANE_KEY) return;
    const p = takePaneAsk();
    if (p) fn(p);
  };
  window.addEventListener("storage", f);
  return () => window.removeEventListener("storage", f);
}

// —— Choosing the AI (Settings › AI): the one adapter. Every engine name the pane uses is in ASSIST. Outside Tauri the
// fixtures in assist.ts play the engine: settings.html?preview=1&pane=ai&assist=fresh|codex-signed-out|codex-ready|key|
// local|offline. ——

export const ASSIST = {
  /** command → Assist: the routes, the choice and the offline switch (cheap: on open and after every action) */
  providers: "assist_providers",
  /** command { id } → AssistModel[] (may take a few seconds) */
  models: "assist_models",
  /** command { id, model, consent, base? }: ok, or an error string; `base` is the typed address, only for `custom` */
  choose: "assist_choose",
  /** command { id }: the route's key and consent are cleared */
  forget: "assist_forget",
  /** command { id, key }: the engine keeps the key in the system's credential store */
  keySet: "assist_key_set",
  keyDelete: "assist_key_delete",
  /** command { id }: opens the browser and resolves when signed in (up to 5 minutes) */
  signIn: "assist_sign_in",
  /** command { id, job: "install" | "update" | "sign_in" }: install and update resolve when the package manager is done;
   * sign_in opens the vendor's own sign-in in a terminal window and resolves at once */
  setup: "assist_setup",
  /** command { on } */
  offlineOnly: "assist_offline_only",
  /** command { effort }: the thinking effort for the chosen AI, where it offers one */
  effort: "assist_effort",
  /** command { speed }: the speed (Codex's service tier) for the chosen AI, where its model offers one; "standard" sends none */
  speed: "assist_speed",
  /** event → Assist: the whole list again, each time one route's probe answers */
  event: "assist-providers",
} as const;

const assistPv = { a: null as Assist | null, checking: typeof location !== "undefined" && location.search.includes("assist=checking") };
const pvAssist = () => (assistPv.a ??= previewAssist(new URLSearchParams(location.search).get("assist") ?? "fresh"));
/** The preview's first answer: with `assist=checking`, every route a probe finds says it is being checked. */
const pvAssistFirst = () => (assistPv.checking ? checkingAssist(pvAssist()) : pvAssist());
const pvDo = (c: Parameters<typeof previewApply>[1]): Promise<void> => {
  const r = previewApply(pvAssist(), c);
  if (typeof r === "string") return Promise.reject(r);
  assistPv.a = r;
  return Promise.resolve();
};
const pvLater = <T,>(ms: number, f: () => T | Promise<T>) => new Promise<T>((res, rej) => setTimeout(() => Promise.resolve().then(f).then(res, rej), ms));

/** The list from what the engine knows now: it never waits on a probe. `refresh` asks every route again (Check again,
 * after a sign-in); otherwise a round starts only when the last is stale. Each probe's answer arrives through
 * `onAssistProviders`. */
export const assistProviders = (refresh = false): Promise<Assist> =>
  (inTauri
    ? invoke<unknown>(ASSIST.providers, { refresh })
    : new URLSearchParams(location.search).get("assist") === "fail"
      ? Promise.reject("assist_providers failed (preview)")
      : Promise.resolve(pvAssistFirst())
  ).then(normalizeAssist);
/** The whole list again, each time one route's probe answers. In the preview (`assist=checking`) the routes being
 * checked fill in a moment after the first draw. */
export function onAssistProviders(fn: (a: Assist) => void): () => void {
  if (inTauri) return subscribe(listen<unknown>(ASSIST.event, (e) => fn(normalizeAssist(e.payload))));
  const t = setTimeout(() => {
    if (assistPv.checking) {
      assistPv.checking = false;
      fn(normalizeAssist(pvAssist()));
    }
  }, 1200);
  return () => clearTimeout(t);
}
export const assistModels = (id: string): Promise<AssistModel[]> =>
  inTauri ? invoke<unknown>(ASSIST.models, { id }).then(normalizeModels) : pvLater(1200, () => previewModels(id));
/** `base` reaches the engine only for the other service (`custom`); every other route has its own address. */
export const assistChoose = (id: string, model: string | null, consent: boolean, base?: string) => {
  const at = id === CUSTOM && base ? { base } : {};
  return inTauri ? invoke<void>(ASSIST.choose, { id, model, consent, ...at }) : pvDo({ cmd: "choose", id, model, consent, ...at });
};
export const assistForget = (id: string) => (inTauri ? invoke<void>(ASSIST.forget, { id }) : pvDo({ cmd: "forget", id }));
/** The key is handed straight to the engine and never kept, logged or echoed here. */
export const assistKeySet = (id: string, key: string) =>
  inTauri ? invoke<void>(ASSIST.keySet, { id, key }) : pvDo({ cmd: "key_set", id });
export const assistKeyDelete = (id: string) => (inTauri ? invoke<void>(ASSIST.keyDelete, { id }) : pvDo({ cmd: "key_delete", id }));
export const assistSignIn = (id: string) => (inTauri ? invoke<void>(ASSIST.signIn, { id }) : pvLater(1800, () => pvDo({ cmd: "signed_in", id })));
export const assistSetup = (id: string, job: "install" | "update" | "sign_in") =>
  inTauri ? invoke<void>(ASSIST.setup, { id, job }) : pvLater(job === "sign_in" ? 600 : 1800, () => pvDo({ cmd: "signed_in", id }));
export const assistEffort = (effort: string) => (inTauri ? invoke<void>(ASSIST.effort, { effort }) : pvDo({ cmd: "effort", effort }));
export const assistSpeed = (speed: string) => (inTauri ? invoke<void>(ASSIST.speed, { speed }) : pvDo({ cmd: "speed", speed }));
export const assistOfflineOnly = (on: boolean) => (inTauri ? invoke<void>(ASSIST.offlineOnly, { on }) : pvDo({ cmd: "offline", on }));

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
