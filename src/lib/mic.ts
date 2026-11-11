import { useEffect, useRef, useState } from "react";
import { inTauri, micMonitor, micScan, onMicLevel, onMicPeak, subscribe, type Config, type Machine, type SpeechModel, type UiState } from "./api";
import { SILENT_LIVE_DB } from "./hardware";
import { platform } from "./theme";

/** What to tell the user when the microphone gives nothing, or very little. `lines` are the same message cut short
 * for the recording capsule, which has room for two short lines. */
export function micNotice(quiet: NonNullable<UiState["quiet_mic"]>): { full: string; lines: [string, string] } {
  const mac = platform() === "mac";
  if (quiet === "silent")
    return {
      full: `No sound from the microphone. Check the input in ${mac ? "System Settings › Sound" : "Windows Sound settings"}.`,
      lines: ["No sound from the mic", mac ? "Check Settings › Sound" : "Check Sound settings"],
    };
  return {
    full: "Your microphone is very quiet. Raise its input volume or move closer.",
    lines: ["Your mic is very quiet", "Raise volume, move closer"],
  };
}

/** How long the microphone check stays open without being asked again, and how often Settings asks. */
const LEASE_MS = 4_000;
const RENEW_MS = 1_500;
/** The status line judges the loudest sample of this last stretch, once the check has listened this long. */
const WINDOW_MS = 3_000;
const SETTLE_MS = 1_500;

export type MicCheck = {
  /** The meter, 0 to 1. */
  level: number;
  /** The loudest sample of the last few seconds (dBFS); null until the check has listened long enough. */
  live: number | null;
  /** Other microphones the scan heard, once the chosen one was silent. */
  others: { name: string; peak_db: number }[];
};

/** The live microphone check while Settings › Speech shows (`on`): it opens the microphone on a lease it renews, lets go
 * of it while the window is hidden, and closes it when the pane goes. When the chosen microphone stays silent it listens
 * once to the others. In the browser preview `?mic=hearing|silent|quiet|slow|other` plays a state. */
export function useMicCheck(on: boolean, device: string | null): MicCheck {
  const [level, setLevel] = useState(0);
  const [live, setLive] = useState<number | null>(null);
  const [others, setOthers] = useState<MicCheck["others"]>([]);
  const scanned = useRef(false);

  useEffect(() => {
    setLive(null);
    setOthers([]);
    scanned.current = false;
    if (!on) return;
    if (!inTauri) return previewMic(setLevel, setLive, setOthers);
    let peaks: [number, number][] = [];
    let since = 0;
    let timers: number[] = [];
    const start = () => {
      since = Date.now();
      peaks = [];
      micMonitor(true, LEASE_MS).catch(() => undefined);
      timers = [
        window.setInterval(() => micMonitor(true, LEASE_MS).catch(() => undefined), RENEW_MS),
        window.setInterval(() => {
          const now = Date.now();
          peaks = peaks.filter(([t]) => now - t <= WINDOW_MS);
          setLive(now - since < SETTLE_MS || !peaks.length ? null : Math.max(...peaks.map(([, db]) => db)));
        }, 250),
      ];
    };
    const stop = () => {
      timers.forEach((t) => clearInterval(t));
      timers = [];
      micMonitor(false).catch(() => undefined);
      setLevel(0);
      setLive(null);
    };
    const onVis = () => (document.hidden ? stop() : start());
    const off = subscribe(
      onMicLevel((l) => setLevel(Math.min(1, l))),
      onMicPeak((db) => peaks.push([Date.now(), db])),
    );
    if (!document.hidden) start();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      document.removeEventListener("visibilitychange", onVis);
      off();
      stop();
    };
  }, [on, device]);

  // Silent for a while: listen once to the other microphones, in case one of them hears the person.
  useEffect(() => {
    if (!inTauri || !on || live === null || live >= SILENT_LIVE_DB || scanned.current) return;
    scanned.current = true;
    micScan().then(setOthers).catch(() => undefined);
  }, [on, live]);

  return { level, live, others };
}

/** Browser preview: the meter and the level of each microphone state. */
function previewMic(
  setLevel: (n: number) => void,
  setLive: (n: number | null) => void,
  setOthers: (o: MicCheck["others"]) => void,
) {
  const mic = new URLSearchParams(location.search).get("mic") ?? "hearing";
  const db = mic === "silent" || mic === "other" ? -100 : mic === "quiet" ? -46 : -16;
  const id = window.setInterval(() => setLevel(db < -90 ? 0 : db < -30 ? 0.05 + Math.random() * 0.08 : 0.2 + Math.random() * 0.6), 90);
  const t = window.setTimeout(() => {
    setLive(db);
    if (mic === "other") setOthers([{ name: "USB Microphone (Blue Yeti)", peak_db: -28 }]);
  }, 300);
  return () => {
    clearInterval(id);
    clearTimeout(t);
  };
}

/** Browser preview: what the backend would say about this computer and the last recordings, by `?pc=` (gpu, unusable,
 * declined, setup, failed, nogpu, apple, intelmac, nohelper) and `?mic=` (slow and quiet need recordings). */
export function previewHardware(cfg: Config): Config {
  if (inTauri) return cfg;
  const q = new URLSearchParams(location.search);
  const pc = q.get("pc") ?? "gpu";
  const mic = q.get("mic") ?? "hearing";
  const def = "Microphone (Realtek(R) Audio)";
  const devices = [def, "Headset (WH-1000XM4)", "USB Microphone (Blue Yeti)"];
  const rec = (open_ms: number, peak_db: number) => ({ mic: cfg.device ?? def, open_ms, peak_db });
  const mic_recent =
    mic === "slow" ? [rec(1400, -14), rec(1350, -14), rec(1520, -14), rec(1380, -14)] : mic === "quiet" ? [rec(120, -41)] : [rec(110, -9)];
  const machine: Machine = {
    cores: 8,
    ram_gb: 16,
    gpu: pc === "nogpu" || pc === "apple" || pc === "intelmac" ? null : ["NVIDIA GeForce RTX 2060 SUPER", 8],
    gpu_integrated: false,
    mac: pc === "apple" || pc === "intelmac",
    apple_silicon: pc === "apple",
    helper: pc !== "nohelper",
  };
  const onGpu = pc === "gpu" || pc === "apple";
  const turbo: SpeechModel = {
    id: "whisper-turbo",
    name: "Whisper Turbo",
    installed: true,
    bytes: 563_790_207,
    gpu: {
      available: pc !== "nohelper" && pc !== "nogpu" && pc !== "intelmac",
      installed: onGpu,
      bytes: 589_282_638,
      cpu_installed: !onGpu,
      ...(pc === "unusable" ? { unusable: "The GPU engine found no graphics card it can use on this computer." } : {}),
    } as SpeechModel["gpu"],
    backend: onGpu ? "gpu" : "cpu",
    speed_x: onGpu ? 14.2 : 2.6,
  };
  return {
    ...cfg,
    devices,
    default_device: def,
    mic_recent,
    machine,
    gpu_declined: pc === "declined",
    gpu_upgrade: pc === "failed" ? { state: "failed", percent: 0 } : cfg.gpu_upgrade,
    speech_in_use: "whisper-turbo",
    speech: { ...cfg.speech, models: [turbo, ...(cfg.speech.models ?? []).filter((m) => m.id !== "whisper-turbo")] },
  };
}
