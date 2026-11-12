// Run: node --test src/lib/capsule.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { DISCARD_ARM_MS, READY_WAIT_MS, discardClick, discardLive, finishingLine, micCaption, noticeView, readyNote } from "./capsule.ts";

const NOTICES = ["pasted", "copied", "saved", "kept", "partial", "cut", "empty"];
const win = { mac: false, pasteBlocked: false, entry: "e1", error: null };
const mac = { ...win, mac: true };

test("every end notice with a saved entry is a button; only empty has nothing to open", () => {
  for (const n of NOTICES) {
    const v = noticeView(n, win);
    if (n === "empty") assert.equal(v.action, null, n);
    else assert.notEqual(v.action, null, n);
  }
});

test("kept: no words yet, audio kept, a warning mark, and the click opens History", () => {
  const v = noticeView("kept", win);
  assert.deepEqual(v.lines, ["No words yet · audio kept", "Open History"]);
  assert.equal(v.mark, "warn");
  assert.equal(v.action, "history");
});

test("partial: pasted what is done, and the click transcribes the rest", () => {
  const v = noticeView("partial", win);
  assert.deepEqual(v.lines, ["Pasted what's done · rest kept in History", "Transcribe the rest"]);
  assert.equal(v.action, "transcribe");
  assert.equal(v.mark, "part");
});

test("copied names the paste keys per platform", () => {
  assert.deepEqual(noticeView("copied", win).lines, ["Copied · Ctrl+V to paste"]);
  assert.deepEqual(noticeView("copied", mac).lines, ["Copied · ⌘V to paste"]);
  assert.equal(noticeView("copied", mac).action, "history");
});

test("an auto-paste that did not happen says the prompt is on the clipboard, and on macOS offers Allow", () => {
  const v = noticeView("copied", { ...mac, pasteBlocked: true });
  assert.deepEqual(v.lines, ["Not pasted · on the clipboard, press ⌘V", "Allow auto-paste"]);
  assert.equal(v.action, "permission");
  assert.equal(v.mark, "warn");
  // Windows refuses a paste into an app run as administrator: no permission to give, so no Allow.
  const w = noticeView("copied", { ...win, pasteBlocked: true });
  assert.deepEqual(w.lines, ["Not pasted · on the clipboard, press Ctrl+V"]);
  assert.equal(w.action, "history");
});

test("pasted and saved keep their short form", () => {
  assert.deepEqual(noticeView("pasted", win).lines, ["Pasted"]);
  assert.deepEqual(noticeView("saved", win).lines, ["Saved to your folder"]);
});

test("empty says the error when there is one", () => {
  assert.deepEqual(noticeView("empty", { ...win, entry: null, error: "Busy" }).lines, ["Busy"]);
  assert.deepEqual(noticeView("empty", { ...win, entry: null }).lines, ["Nothing was recorded"]);
  assert.equal(noticeView("empty", win).mark, "none");
});

test("no notice line carries an em dash", () => {
  for (const n of NOTICES)
    for (const o of [win, mac, { ...mac, pasteBlocked: true }])
      for (const l of noticeView(n, o).lines) assert.ok(!l.includes("—"), l);
});

test("transcribing: writing it down with a percent, no hardware name", () => {
  assert.equal(finishingLine({ percent: 42, stalled: false }, false), "Writing it down… 42%");
  assert.equal(finishingLine({ percent: 42.6, stalled: true }, false), "Still working… 43%");
  assert.equal(finishingLine(null, false), "Finishing…");
  assert.equal(finishingLine({ percent: 42, stalled: false }, true), "Saving what's done…");
  assert.ok(!finishingLine({ percent: 42, stalled: false }, false).includes("graphics"));
});

test("discard asks once: the first click arms, the second within 3 s discards", () => {
  const a = discardClick(null, 1000);
  assert.deepEqual(a, { discard: false, armedUntil: 1000 + DISCARD_ARM_MS });
  assert.equal(DISCARD_ARM_MS, 3000);
  assert.deepEqual(discardClick(a.armedUntil, 2500), { discard: true, armedUntil: null });
});

test("discard: after 3 s the arm is gone and a click arms again", () => {
  const until = discardClick(null, 0).armedUntil;
  assert.equal(discardLive(until, 2999), true);
  assert.equal(discardLive(until, 3000), false);
  assert.deepEqual(discardClick(until, 3000), { discard: false, armedUntil: 6000 });
  assert.equal(discardLive(null, 0), false);
});

test("the mic caption fits the capsule: one short line per kind", () => {
  assert.equal(micCaption("silent"), "No sound from the mic");
  assert.equal(micCaption("quiet"), "Mic is very quiet");
});

test("a live microphone looks like recording at once, whatever the speech model is doing", () => {
  for (const phase of ["loading", "recording"]) {
    assert.equal(readyNote({ phase, mic: true }, 0), null);
    assert.equal(readyNote({ phase, mic: true, preparing_gpu: true }, 5_000), null);
  }
});

test("only a microphone slow to start is said, and never in the first moment", () => {
  assert.equal(readyNote({ phase: "loading", mic: false }, 0), null, "no flash of text on a normal start");
  assert.equal(readyNote({ phase: "loading", mic: false }, READY_WAIT_MS - 1), null);
  assert.deepEqual(readyNote({ phase: "loading", mic: false }, READY_WAIT_MS)?.lines, ["Getting ready…"]);
  assert.equal(readyNote({ phase: "loading", mic: false, preparing_gpu: true }, READY_WAIT_MS)?.full, "Setting up your graphics card (first time only)");
  assert.equal(readyNote({ phase: "transcribing", mic: false }, 9_000), null);
  assert.equal(readyNote({ phase: "idle", mic: false }, 9_000), null);
});

test("the capsule's line: the voice draws it once the microphone is live, flat before, loops while finishing", async () => {
  const { capsuleLine } = await import("./capsule.ts");
  assert.equal(capsuleLine({ phase: "recording", mic: true }), "listen");
  assert.equal(capsuleLine({ phase: "loading", mic: true }), "listen");
  assert.equal(capsuleLine({ phase: "recording", mic: false }), "rest");
  assert.equal(capsuleLine({ phase: "loading", mic: false }), "rest");
  assert.equal(capsuleLine({ phase: "transcribing", mic: false }), "think");
  assert.equal(capsuleLine({ phase: "transcribing", mic: true }), "think");
  assert.equal(capsuleLine({ phase: "ready", mic: true }), "rest");
});

test("the time shows on hover until a minute, then stays; a quiet-mic warning keeps it", async () => {
  const { TIME_STAYS_MS, timeStays } = await import("./capsule.ts");
  assert.equal(timeStays(0, false), false);
  assert.equal(timeStays(TIME_STAYS_MS - 1, false), false);
  assert.equal(timeStays(TIME_STAYS_MS, false), true);
  assert.equal(timeStays(5_000, true), true);
});

test("the capsule's line reads the same level as Ask's: loudness before the raw meter, nothing while the mic is not live", async () => {
  const { LEVEL_FIELDS, levelOf } = await import("./live.ts");
  // A session state carries both: level (0 to 1 over -60..-10 dBFS) and loudness (over -50..-10 dBFS), as Ask reads it.
  assert.equal(levelOf({ phase: "recording", mic: true, level: 0.62, loudness: 0.5 }, LEVEL_FIELDS), 0.5);
  assert.equal(levelOf({ phase: "recording", mic: false, level: 0.62, loudness: 0.5 }, LEVEL_FIELDS), 0);
  // An engine without loudness still moves the line.
  assert.equal(levelOf({ phase: "recording", mic: true, level: 0.62 }, LEVEL_FIELDS), 0.62);
});

import { NO_PEEK, TIME_STAYS_MS, peekStep, timeShown } from "./capsule.ts";

test("the time shows only while someone looks, and never stays after a click, a hide or the end of a recording", () => {
  const at = (peek, o = {}) => timeShown({ live: true, durationMs: 16_000, warned: false, peek, ...o });
  assert.equal(at(NO_PEEK), false, "a recording under a minute shows no time by itself");
  const over = peekStep(NO_PEEK, "enter");
  assert.equal(at(over), true, "the pointer over the capsule shows it");
  assert.equal(at(peekStep(over, "leave")), false, "and it goes when the pointer leaves");
  // A click on a picture button keeps that button's focus: that is not a look, so the time does not stay.
  assert.equal(at(peekStep(peekStep(over, "leave"), "blur")), false);
  // The keyboard on a button shows it until the focus leaves the capsule.
  const key = peekStep(NO_PEEK, "key-focus");
  assert.equal(at(key), true);
  assert.equal(at(peekStep(key, "blur")), false);
  // Hidden with the pointer over it, no pointer-leave ever arrives: hiding, and a new recording, forget the look.
  for (const e of ["hidden", "start"]) assert.deepEqual(peekStep({ pointer: true, keyboard: true }, e), NO_PEEK, e);
  assert.equal(at(peekStep(over, "hidden")), false);
  // Ended (the microphone is no longer live): no time, whatever the look or the length.
  assert.equal(timeShown({ live: false, durationMs: TIME_STAYS_MS * 3, warned: true, peek: over }), false);
  // From a minute on, and with the microphone warning, it stays without a look.
  assert.equal(at(NO_PEEK, { durationMs: TIME_STAYS_MS }), true);
  assert.equal(at(NO_PEEK, { warned: true }), true);
});
