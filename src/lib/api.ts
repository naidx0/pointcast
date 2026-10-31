import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export type Label = { n: number; text: string };

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
  /** Clean up room noise and even out the volume before transcribing. Treated as on when absent. */
  voice_boost?: boolean;
  /** Get the faster GPU version of Whisper Turbo by itself. Treated as on when absent. */
  auto_gpu?: boolean;
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
export const captureFull = () => invoke<void>("capture_full");
export const captureZoom = () => invoke<void>("capture_zoom");
export const regionBegin = () => invoke<{ png_b64: string; width: number; height: number }>("region_begin");
/** The control under the pointer, its rect in picture pixels. */
export type ElementMark = { role: string; name: string; rect: { x: number; y: number; w: number; h: number } };
export const regionCommit = (pngB64: string, labels: Label[], element: ElementMark | null = null) =>
  invoke<void>("region_commit", { body: { png_b64: pngB64, labels, element } });
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
/** Copies the MCP setup for one coding agent, with this install's key, straight to the clipboard. */
export const mcpCopySetup = (agent: "claude" | "cursor") => invoke<void>("mcp_copy_setup", { agent });
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
export const micMonitor = (on: boolean) => invoke<void>("mic_monitor", { on });
export const onboardingFinish = () => invoke<void>("onboarding_finish");
/** Open at login for this user; `machine` is an older entry for every user that only an administrator can remove. */
export type Login = { enabled: boolean; machine: boolean };
export const loginGet = () => invoke<Login>("login_get");
export const loginSet = (on: boolean) => invoke<Login>("login_set", { on });
export const updateCheck = () => invoke<string | null>("update_check");
export const updateInstall = () => invoke<void>("update_install");
export const windowOpen = (label: string) => invoke<void>("window_open", { label });
export const platformInfo = () => invoke<{ os: string; materials: boolean }>("platform_info");

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
export const onTranscribeProgress = on<{ id: string; done_ms: number; total_ms: number }>("transcribe://progress");
export const onSpeechProgress = on<SpeechProgress>("speech://progress");
export const onSpeechDone = on<{ id: string; ok: boolean; error?: string | null }>("speech://done");
export const onGpuUpgrade = on<GpuUpgrade>("gpu://upgrade");
export const onHotkey = on<{ action: Action; pressed: boolean }>("hotkey://fired");
export const onMicLevel = on<number>("mic://level");
export const onUpdateChecked = on<string | null>("update://checked");
export const onUpdateFailed = on<string>("update://failed");
export const onSettingsPane = on<string>("settings://pane");

/** Subscribe in an effect; returns the cleanup. */
export function subscribe(...subs: Promise<UnlistenFn>[]) {
  return () => subs.forEach((u) => u.then((fn) => fn()));
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
