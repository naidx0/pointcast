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
  }[];
};

export type DetectedTool = { id: string; name: string; path: string; kind: string };

export type Access = "granted" | "unknown" | "denied";
export type PermissionKind = "microphone" | "screen" | "accessibility";
export type Permissions = Record<PermissionKind, Access> & { gated: Record<PermissionKind, boolean> };

export type Action = "session" | "full" | "region" | "zoom";
export type HotkeyView = { label: string; keys: string[]; default: string };

export type SpeechModel = { id: string; name: string; installed: boolean; bytes: number };
export type SpeechView = {
  runtime?: { id: string; installed: boolean; lib: string } | null;
  models?: SpeechModel[];
};

export type Output = "clipboard" | "auto_paste" | "folder";
export type TalkMode = "tap" | "hold";

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
  start_at_login: boolean;
  include_app: boolean;
  hidden: string[];
  mcp_bind: string;
  mcp_enabled: boolean;
  onboarded: boolean;
  sounds: boolean;
  speech: SpeechView;
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
export const captureFull = () => invoke<void>("capture_full");
export const captureZoom = () => invoke<void>("capture_zoom");
export const regionBegin = () => invoke<{ png_b64: string; width: number; height: number }>("region_begin");
export const regionCommit = (pngB64: string, labels: Label[]) =>
  invoke<void>("region_commit", { body: { png_b64: pngB64, labels } });
export const regionCancel = () => invoke<void>("region_cancel");
export const lastPrompt = () => invoke<SessionExport | null>("last_prompt");
export const lastFolder = () => invoke<string | null>("last_folder");
export const openFolder = (path: string) => invoke<void>("open_folder", { path });
export const copyAgain = () => invoke<void>("copy_again");
export const pasteAgain = () => invoke<void>("paste_again");
export const downloadModel = (id: string) => invoke<void>("download_model", { id });
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
export const windowOpen = (label: string) => invoke<void>("window_open", { label });
export const platformInfo = () => invoke<{ os: string; materials: boolean }>("platform_info");

const on =
  <T,>(name: string) =>
  (fn: (p: T) => void): Promise<UnlistenFn> =>
    inTauri ? listen<T>(name, (e) => fn(e.payload)) : Promise.resolve(() => undefined);

export const onState = on<UiState>("session://state");
export const onResult = on<SessionExport>("session://result");
export const onPracticeResult = on<SessionExport>("practice://result");
export const onOverlayFrame = on<{ png_b64: string; width: number; height: number }>("overlay://frame");
export const onSpeechProgress = on<{ state?: string; received?: number; total?: number; part?: string }>(
  "speech://progress",
);
export const onSpeechDone = on<{ id: string; ok: boolean; error?: string | null }>("speech://done");
export const onHotkey = on<{ action: Action; pressed: boolean }>("hotkey://fired");
export const onMicLevel = on<number>("mic://level");

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
