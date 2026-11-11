// Run: node --test src/lib/hardware.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { computerView, median, micStatus, micText } from "./hardware.ts";

const DEV = "Microphone (Realtek(R) Audio)";
const rec = (open_ms, peak_db = -12, mic = DEV) => ({ mic, open_ms, peak_db });
const st = (o) => micStatus({ device: DEV, live: null, recent: [], mac: false, ...o });

test("hearing: sound now, nothing wrong with the last recordings", () => {
  assert.deepEqual(st({ live: -24 }), { tone: "ok", text: "Hearing you" });
  assert.deepEqual(st({ live: -55, recent: [rec(120)] }), { tone: "ok", text: "Hearing you" }, "a room with nobody talking still delivers");
  assert.equal(st({}), null, "nothing to judge by yet");
});

test("silent: the check hears nothing, or the last recording on it had no sound while the check has not listened yet", () => {
  const silent = { tone: "bad", text: "No sound from this microphone. Check it is not muted, or pick another one." };
  assert.deepEqual(st({ live: -100 }), silent);
  assert.deepEqual(st({ live: -74 }), silent);
  assert.deepEqual(st({ recent: [rec(100, -100)] }), silent);
  assert.deepEqual(st({ live: -20, recent: [rec(100, -100)] }), { tone: "ok", text: "Hearing you" }, "sound now wins over an old silent recording");
  assert.deepEqual(st({ recent: [rec(100, -100, "Other")] }), null, "another microphone's recording says nothing about this one");
});

test("another microphone has sound while the chosen one is silent: offer it by name, the loudest", () => {
  const others = [
    { name: "Laptop mic", peak_db: -100 },
    { name: "USB Microphone", peak_db: -31 },
    { name: "Webcam", peak_db: -44 },
  ];
  const s = st({ live: -100, others });
  assert.equal(s.offer, "USB Microphone");
  assert.equal(s.tone, "bad");
  assert.equal(st({ live: -100, others: [{ name: "Laptop mic", peak_db: -65 }] }).offer, undefined, "nothing that hears");
  assert.equal(st({ live: -20, others }).offer, undefined, "never while this one hears");
});

test("very quiet: the last recording on it peaked low and the check has not heard it louder", () => {
  const win = "Very quiet: move closer or raise its input level in Windows Sound settings";
  assert.deepEqual(st({ recent: [rec(100, -41)] }), { tone: "warn", text: win });
  assert.deepEqual(st({ live: -48, recent: [rec(100, -41)] }), { tone: "warn", text: win });
  assert.equal(st({ live: -48, recent: [rec(100, -41)], mac: true }).text, "Very quiet: move closer or raise its input level in System Settings › Sound");
  assert.equal(st({ live: -12, recent: [rec(100, -41)] }).text, "Hearing you", "louder now");
});

test("slow start: the median of the last five recordings on this microphone, over 700 ms", () => {
  const slow = (s) => `This microphone takes about ${s} to start (Bluetooth headsets do). Wait for the red dot before you talk, or pick a wired one.`;
  assert.deepEqual(st({ live: -20, recent: [rec(1400), rec(1300), rec(1500)] }), { tone: "warn", text: slow("1.4 s") });
  // One slow open among fast ones is not a slow microphone.
  assert.equal(st({ live: -20, recent: [rec(2300), rec(120), rec(150), rec(90), rec(200)] }).text, "Hearing you");
  // Only the newest five count, and only this microphone's.
  const old = [rec(150), rec(150), rec(150), rec(150), rec(150)];
  assert.equal(st({ live: -20, recent: [rec(1200), rec(1100), rec(1300), rec(900, -12, "Other"), rec(1000), rec(800), ...old] }).text, slow("1.1 s"));
  assert.equal(st({ live: -20, recent: [rec(700), rec(700)] }).text, "Hearing you", "700 ms is not over");
  assert.equal(median([3, 1, 2]), 2);
  assert.equal(median([4, 1, 2, 3]), 2.5);
});

test("History's microphone line", () => {
  assert.equal(micText({}), "", "an entry from before this was kept");
  assert.equal(micText({ mic: "USB", mic_open_ms: 140, mic_peak_db: -9 }), "Microphone: USB");
  assert.equal(
    micText({ mic: "Headset (WH-1000XM4)", mic_open_ms: 1_420, mic_peak_db: -9 }),
    "Microphone: Headset (WH-1000XM4), took 1.4 s to start: words before that were not recorded",
  );
  assert.equal(micText({ mic: "USB", mic_open_ms: 90, mic_peak_db: -100 }), "Microphone: USB, no sound came from it");
});

const PC = { cores: 8, ram_gb: 16, gpu: ["NVIDIA GeForce RTX 2060 SUPER", 8], gpu_integrated: false, mac: false, apple_silicon: false, helper: true };
const turbo = (o = {}) => ({ id: "whisper-turbo", name: "Whisper Turbo", installed: true, bytes: 1, gpu: { available: true, installed: true }, ...o });
const view = (o) => computerView({ machine: PC, model: turbo(), upgrade: null, declined: false, ...o });

test("GPU in use: the card, and the engine on it with its speed in plain words", () => {
  const v = view({ model: turbo({ backend: "gpu", speed_x: 14.2 }) });
  assert.equal(v.card, "RTX 2060 SUPER graphics card with 8 GB of its own memory");
  assert.equal(v.engine, "Whisper Turbo on your graphics card: about 14x faster than you talk");
  assert.deepEqual([v.why, v.action], [null, null]);
  assert.equal(view({ model: turbo() }).engine, "Whisper Turbo on your graphics card", "set up, not loaded yet, no speed measured");
});

test("GPU present but unused: each stored reason with the one action that fixes it", () => {
  const cpu = (gpu, o = {}) => view({ model: turbo({ backend: "cpu", speed_x: 3.4, gpu: { available: true, ...gpu } }), ...o });
  const unusable = cpu({ installed: false, unusable: "no device" });
  assert.equal(unusable.engine, "Whisper Turbo on your processor: about 3.4x faster than you talk");
  assert.deepEqual([unusable.why, unusable.action], ["Your graphics card could not run the speech engine when it was tried.", "retry"]);
  const declined = cpu({ installed: false, cpu_installed: true }, { declined: true });
  assert.deepEqual([declined.why, declined.action], ["You stopped the graphics-card version from downloading.", "setup"]);
  const notYet = cpu({ installed: false, cpu_installed: true });
  assert.deepEqual([notYet.why, notYet.action], ["The graphics-card version is not set up yet.", "setup"]);
  const failed = cpu({ installed: false }, { upgrade: { state: "failed", percent: 0 } });
  assert.deepEqual([failed.why, failed.action], ["Setting up the graphics-card version did not finish.", "retry"]);
  const busy = cpu({ installed: false }, { upgrade: { state: "downloading", percent: 38 } });
  assert.deepEqual([busy.why, busy.action], [null, null], "the model card shows the download");
  const other = view({ model: { id: "moonshine-base-en", name: "Moonshine Base", installed: true, bytes: 1, backend: "cpu", speed_x: 0.6 } });
  assert.equal(other.engine, "Moonshine Base on your processor: slower than you talk: long recordings wait at the end");
  assert.deepEqual([other.why, other.action], ["Whisper Turbo can use your graphics card and is much faster.", "setup"]);
});

test("no graphics card: said plainly, nothing to fix", () => {
  const v = view({ machine: { ...PC, gpu: null }, model: turbo({ backend: "cpu", speed_x: 1, gpu: { available: true, installed: false } }) });
  assert.equal(v.card, "No graphics card found: speech runs on the processor");
  assert.equal(v.engine, "Whisper Turbo on your processor: about as fast as you talk");
  assert.deepEqual([v.why, v.action], [null, null]);
  const small = view({ machine: { ...PC, gpu: ["NVIDIA GeForce GT 1030", 2] }, model: turbo({ backend: "cpu" }) });
  assert.equal(small.card, "GT 1030 graphics card with 2 GB of its own memory");
  assert.equal(small.why, null, "a card too small for the engine is no reason to offer it");
  const built = view({ machine: { ...PC, gpu: ["Intel(R) Iris(R) Xe Graphics", 0.1], gpu_integrated: true }, model: turbo({ backend: "gpu" }) });
  assert.equal(built.card, "Iris Xe Graphics, built into the processor");
});

test("Apple silicon and an Intel Mac", () => {
  const mac = { ...PC, gpu: null, mac: true, apple_silicon: true, ram_gb: 16 };
  const v = view({ machine: mac, model: turbo({ backend: "gpu", speed_x: 22 }) });
  assert.equal(v.card, "Apple silicon: its graphics share the 16 GB of memory");
  assert.equal(v.engine, "Whisper Turbo on your graphics card: about 22x faster than you talk");
  const intel = view({ machine: { ...mac, apple_silicon: false }, model: turbo({ backend: "cpu" }) });
  assert.equal(intel.card, "Intel Mac: speech runs on the processor");
  assert.equal(intel.why, null);
});

test("CPU only because no graphics-card helper is published for this computer", () => {
  const v = view({ machine: { ...PC, helper: false }, model: turbo({ backend: "cpu", gpu: undefined }) });
  assert.deepEqual([v.why, v.action], ["There is no graphics-card version of the speech engine for this computer yet.", null]);
  assert.equal(view({ model: null }).engine, null, "no model yet: the card alone");
});
