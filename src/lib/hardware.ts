// What Settings › Speech and History say about this computer and its microphone, as pure functions of what the backend
// measured. No imports with a runtime: node --test runs this file directly.
import type { GpuUpgrade, HistoryMeta, Machine, MicRecent, SpeechModel } from "./api";

/** Below this a recording's loudest sample counts as no sound at all (dBFS). */
export const SILENT_DB = -60;
/** The microphone check hears nothing louder than this over a few seconds: nothing is coming (a working microphone
 * hears its room above it, even with nobody talking). */
export const SILENT_LIVE_DB = -70;
/** A recording whose loudest sample stayed below this was very quiet: speech peaks well above it. */
export const QUIET_DB = -30;
/** Another microphone whose loudest sample passes this picked up a voice or a room. */
export const OTHER_HEARS_DB = -50;
/** A microphone that takes longer than this to deliver its first sample loses the first words. */
export const SLOW_MS = 700;

/** "1.4 s": one decimal. */
const secs = (ms: number) => `${Math.round(ms / 100) / 10} s`;

export function median(xs: number[]) {
  if (!xs.length) return 0;
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

export type MicStatus = { tone: "ok" | "warn" | "bad"; text: string; offer?: string };

/** The one line under the microphone. `live` is the loudest sample the check heard over the last few seconds (dBFS),
 * null until it has listened long enough; `recent` the newest recordings' microphones, newest first; `others` the other
 * microphones the scan heard. Null when there is nothing to judge by yet. */
export function micStatus(i: {
  device: string;
  live: number | null;
  recent: MicRecent[];
  others?: { name: string; peak_db: number }[];
  mac: boolean;
}): MicStatus | null {
  const mine = i.recent.filter((r) => r.mic === i.device);
  const last = mine[0];
  const silent = i.live !== null ? i.live < SILENT_LIVE_DB : !!last && last.peak_db < SILENT_DB;
  if (silent) {
    const other = (i.others ?? []).filter((o) => o.name !== i.device && o.peak_db >= OTHER_HEARS_DB).sort((a, b) => b.peak_db - a.peak_db)[0];
    return { tone: "bad", text: "No sound from this microphone. Check it is not muted, or pick another one.", ...(other ? { offer: other.name } : {}) };
  }
  const slow = median(mine.slice(0, 5).map((r) => r.open_ms));
  if (mine.length && slow > SLOW_MS)
    return {
      tone: "warn",
      text: `This microphone takes about ${secs(slow)} to start (Bluetooth headsets do). Wait for the red dot before you talk, or pick a wired one.`,
    };
  if (last && last.peak_db < QUIET_DB && (i.live === null || i.live < QUIET_DB))
    return {
      tone: "warn",
      text: `Very quiet: move closer or raise its input level in ${i.mac ? "System Settings › Sound" : "Windows Sound settings"}`,
    };
  if (i.live === null && !last) return null;
  return { tone: "ok", text: "Hearing you" };
}

/** History's detail line about the microphone: empty for an entry from before this was kept. */
export function micText(m: Pick<HistoryMeta, "mic" | "mic_open_ms" | "mic_peak_db">) {
  if (!m.mic) return "";
  const notes = [
    (m.mic_open_ms ?? 0) > SLOW_MS ? `took ${secs(m.mic_open_ms!)} to start: words before that were not recorded` : "",
    (m.mic_peak_db ?? 0) < SILENT_DB ? "no sound came from it" : "",
  ].filter(Boolean);
  return `Microphone: ${m.mic}${notes.length ? `, ${notes.join(", ")}` : ""}`;
}

/** A graphics card's short name: "NVIDIA GeForce RTX 2060 SUPER" is "RTX 2060 SUPER". */
export function shortCard(d: string) {
  return d
    .replace(/ via .*$/, "")
    .replace(/[(](R|TM)[)]/g, "")
    .replace(/^(NVIDIA |AMD |Intel )/, "")
    .replace(/^GeForce /, "")
    .replace(/\s+/g, " ")
    .trim();
}

/** Whether this computer's graphics can run the fast engine (`machine::gpu_fits`). */
export function gpuFits(m: Machine) {
  if (m.mac) return m.apple_silicon && m.ram_gb >= 7.5;
  if (!m.gpu) return false;
  return m.gpu_integrated ? m.ram_gb >= 7.5 : m.gpu[1] >= 2.9;
}

/** "about 14x faster than you talk", in plain words. */
export function pace(x: number) {
  if (x >= 1.25) return `about ${x >= 10 ? Math.round(x) : Math.round(x * 10) / 10}x faster than you talk`;
  if (x >= 0.8) return "about as fast as you talk";
  return "slower than you talk: long recordings wait at the end";
}

export type ComputerAction = "setup" | "retry";
export type ComputerView = { card: string; engine: string | null; why: string | null; action: ComputerAction | null };

/** The "This computer" block: its graphics, the engine in use and how fast it is here, and, when speech runs on the
 * processor although the graphics could do it, why and the one thing that fixes it. */
export function computerView(i: {
  machine: Machine;
  /** The model recordings use, as Settings lists it. */
  model: SpeechModel | null;
  upgrade: GpuUpgrade | null;
  declined: boolean;
}): ComputerView {
  const { machine: m, model } = i;
  const card = m.mac
    ? m.apple_silicon
      ? `Apple silicon: its graphics share the ${fmtGb(m.ram_gb)} of memory`
      : "Intel Mac: speech runs on the processor"
    : !m.gpu
      ? "No graphics card found: speech runs on the processor"
      : m.gpu_integrated
        ? `${shortCard(m.gpu[0])}, built into the processor`
        : `${shortCard(m.gpu[0])} graphics card with ${fmtGb(m.gpu[1])} of its own memory`;
  if (!model) return { card, engine: null, why: null, action: null };
  const name = model.name;
  // Where it runs: what the engine said while in memory, else what is set up.
  const onGpu = model.backend ? model.backend === "gpu" : !!model.gpu?.installed && !model.gpu.unusable;
  const where = onGpu ? "on your graphics card" : "on your processor";
  const engine = `${name} ${where}${model.speed_x ? `: ${pace(model.speed_x)}` : ""}`;
  if (onGpu || !gpuFits(m)) return { card, engine, why: null, action: null };
  const turbo = model.id === "whisper-turbo";
  const up = i.upgrade?.state;
  if (!m.helper) return { card, engine, why: "There is no graphics-card version of the speech engine for this computer yet.", action: null };
  if (turbo && (up === "downloading" || up === "checking")) return { card, engine, why: null, action: null };
  if (model.gpu?.unusable) return { card, engine, why: "Your graphics card could not run the speech engine when it was tried.", action: "retry" };
  if (turbo && up === "failed") return { card, engine, why: "Setting up the graphics-card version did not finish.", action: "retry" };
  if (turbo && i.declined) return { card, engine, why: "You stopped the graphics-card version from downloading.", action: "setup" };
  if (!turbo) return { card, engine, why: "Whisper Turbo can use your graphics card and is much faster.", action: "setup" };
  return { card, engine, why: "The graphics-card version is not set up yet.", action: "setup" };
}

/** "16 GB", "0.5 GB". */
function fmtGb(gb: number) {
  return `${gb >= 2 ? Math.round(gb) : Math.round(gb * 10) / 10} GB`;
}
