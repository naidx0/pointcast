// Run: node --test src/lib/capsule.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { DISCARD_ARM_MS, discardClick, discardLive, finishingLine, micCaption, noticeView } from "./capsule.ts";

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

test("copied because auto-paste had no permission offers to turn it on", () => {
  const v = noticeView("copied", { ...mac, pasteBlocked: true });
  assert.deepEqual(v.lines, ["Copied · ⌘V to paste", "Turn on auto-paste"]);
  assert.equal(v.action, "permission");
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
