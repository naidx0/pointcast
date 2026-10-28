import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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

export async function settingsGet(): Promise<Record<string, unknown>> {
  return invoke("settings_get");
}

export async function settingsSet(change: unknown): Promise<Record<string, unknown>> {
  return invoke("settings_set", { change });
}

export async function sessionToggle(): Promise<UiState> {
  return invoke("session_toggle");
}

export async function sessionState(): Promise<UiState> {
  return invoke("session_state");
}

export async function sessionCancel(): Promise<void> {
  return invoke("session_cancel");
}

export async function captureFull(): Promise<void> {
  await invoke("capture_full");
}

export async function captureZoom(): Promise<void> {
  await invoke("capture_zoom");
}

export async function regionBegin(): Promise<{ png_b64: string; width: number; height: number }> {
  return invoke("region_begin");
}

export async function regionCommit(pngB64: string, labels: Label[]): Promise<void> {
  await invoke("region_commit", { body: { png_b64: pngB64, labels } });
}

export async function regionCancel(): Promise<void> {
  await invoke("region_cancel");
}

export async function lastPrompt(): Promise<SessionExport | null> {
  return invoke("last_prompt");
}

export async function copyAgain(): Promise<void> {
  await invoke("copy_again");
}

export async function downloadModel(id: string): Promise<void> {
  await invoke("download_model", { id });
}

export async function requestPermissions(): Promise<unknown> {
  return invoke("request_permissions");
}

export function onState(fn: (s: UiState) => void): Promise<UnlistenFn> {
  return listen<UiState>("session://state", (e) => fn(e.payload));
}

export function onResult(fn: (s: SessionExport) => void): Promise<UnlistenFn> {
  return listen<SessionExport>("session://result", (e) => fn(e.payload));
}

export function onOverlayFrame(
  fn: (f: { png_b64: string; width: number; height: number }) => void,
): Promise<UnlistenFn> {
  return listen("overlay://frame", (e) => fn(e.payload as { png_b64: string; width: number; height: number }));
}

export function onSpeechProgress(fn: (p: unknown) => void): Promise<UnlistenFn> {
  return listen("speech://progress", (e) => fn(e.payload));
}

export function onSpeechDone(fn: (p: unknown) => void): Promise<UnlistenFn> {
  return listen("speech://done", (e) => fn(e.payload));
}

export function stamp(ms: number): string {
  const s = Math.floor(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}
