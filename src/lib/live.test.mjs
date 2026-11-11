// Run: node --test src/lib/live.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CLOSED,
  PREVIEW_ANSWER,
  SLOW_MS,
  STALL_MS,
  errorView,
  initialLive,
  isPinned,
  liveReduce,
  liveView,
  previewChunks,
  previewLevel,
  previewPicture,
  previewSnapshot,
  safePicture,
  sentLine,
} from "./live.ts";

const pic = previewPicture();
const snap = (over = {}) => ({ ...CLOSED, session: { id: "s1", title: "Why divide" }, turn: 1, provider: "ChatGPT", picture: pic, ...over });
const at = (s, now = 0, m = initialLive()) => liveReduce(m, { type: "snapshot", snap: s, now });
const deltas = (m, ...d) => liveReduce(m, { type: "deltas", deltas: d });

test("each engine state is reached from a snapshot, with its one line behaviour and tone", () => {
  const cases = [
    ["closed", "closed", "none", "ink"],
    ["listening", "listening", "voice", "ink"],
    ["finishing", "finishing", "loops", "ink"],
    ["sending", "thinking", "loops", "accent"],
    ["answering", "answering", "margin", "accent"],
    ["shown", "shown", "rest", "quiet"],
    ["error", "error", "rest", "warn"],
  ];
  for (const [state, phase, line, tone] of cases) {
    const v = liveView(at(snap({ state, error: state === "error" ? "Network down" : null })), 0);
    assert.equal(v.phase, phase, state);
    assert.equal(v.line, line, state);
    assert.equal(v.tone, tone, state);
  }
});

test("red means the microphone is live and nothing else; the accent means a request is in flight", () => {
  for (const state of ["closed", "listening", "finishing", "sending", "answering", "shown", "error"]) {
    const v = liveView(at(snap({ state })), 0);
    assert.equal(v.mic, state === "listening", state);
    assert.equal(v.inFlight, state === "sending" || state === "answering", state);
  }
});

test("an unknown state or a missing field is read as closed, never as a crash", () => {
  const m = at({ state: "dancing" });
  assert.equal(m.snap.state, "closed");
  assert.equal(liveView(m, 0).phase, "closed");
  assert.equal(at(null).snap.state, "closed");
});

test("deltas for the current turn append in order; a delta for another turn is dropped", () => {
  let m = at(snap({ state: "answering", turn: 3 }));
  m = deltas(m, { turn: 3, text: "Hello" }, { turn: 2, text: " stale" }, { turn: 3, text: " world" });
  m = deltas(m, { turn: 4, text: " early" });
  assert.equal(m.text, "Hello world");
});

test("a new turn starts an empty answer; the same turn keeps what has streamed", () => {
  let m = at(snap({ state: "answering", turn: 1 }));
  m = deltas(m, { turn: 1, text: "first answer" });
  m = at(snap({ state: "shown", turn: 1 }), 10, m);
  assert.equal(m.text, "first answer");
  m = at(snap({ state: "listening", turn: 2 }), 20, m);
  assert.equal(m.text, "");
  m = deltas(m, { turn: 1, text: "late piece of the old answer" });
  assert.equal(m.text, "");
});

test("an answer sent with the snapshot replaces what was streamed (a window opened mid-answer)", () => {
  const m = at(snap({ state: "answering", answer: "So far" }));
  assert.equal(deltas(m, { turn: 1, text: ", more" }).text, "So far, more");
});

test("thinking says nothing for 8 s, then one quiet line, then at 20 s the error line with Try again", () => {
  const m = at(snap({ state: "sending" }), 1_000);
  assert.equal(liveView(m, 1_000 + SLOW_MS - 1).note, null);
  assert.equal(liveView(m, 1_000 + SLOW_MS - 1).phase, "thinking");
  const slow = liveView(m, 1_000 + SLOW_MS);
  assert.equal(slow.note, "Waiting for ChatGPT");
  assert.equal(slow.phase, "thinking");
  assert.equal(slow.line, "loops");
  const stall = liveView(m, 1_000 + STALL_MS);
  assert.equal(stall.phase, "error");
  assert.equal(stall.error?.action, "retry");
  assert.equal(stall.error?.label, "Try again");
  assert.equal(stall.stop, true, "Esc still stops the request that is in flight");
});

test("the wait clock starts when sending starts, survives repeated sending snapshots, and counts time already waited", () => {
  let m = at(snap({ state: "sending" }), 5_000);
  m = at(snap({ state: "sending" }), 9_000, m);
  assert.equal(m.waitFrom, 5_000);
  const opened = at(snap({ state: "sending", thinking: 9_000 }), 100_000);
  assert.equal(liveView(opened, 100_000).note, "Waiting for ChatGPT");
  assert.equal(at(snap({ state: "answering" }), 6_000, m).waitFrom, null);
});

test("a first delta before the answering snapshot shows the answer, not the loops", () => {
  const m = deltas(at(snap({ state: "sending" })), { turn: 1, text: "The" });
  assert.equal(liveView(m, 0).phase, "answering");
});

test("a dropped picture shows no thumbnail and cannot be dropped again", () => {
  const v = liveView(at(snap({ state: "listening", dropped: true })), 0);
  assert.equal(v.thumb, null);
  assert.equal(v.canDrop, false);
  const kept = liveView(at(snap({ state: "listening" })), 0);
  assert.ok(kept.thumb);
  assert.equal(kept.canDrop, true);
  assert.equal(liveView(at(snap({ state: "sending" })), 0).canDrop, false, "once sent it has left the computer");
});

test("only a picture held on this computer is shown: never a web address", () => {
  assert.equal(safePicture({ src: "https://example.com/a.png", w: 1, h: 1 }), null);
  assert.equal(safePicture({ src: "//example.com/a.png", w: 1, h: 1 }), null);
  assert.equal(safePicture({ src: "javascript:alert(1)", w: 1, h: 1 }), null);
  assert.ok(safePicture({ src: "data:image/png;base64,AAAA", w: 1, h: 1 }));
  assert.ok(safePicture({ src: "blob:tauri://localhost/1", w: 1, h: 1 }));
  assert.equal(liveView(at(snap({ state: "listening", picture: { src: "https://x/y.png", w: 1, h: 1 } })), 0).thumb, null);
});

test("no AI chosen is the Settings error; any other failure offers Try again", () => {
  assert.deepEqual(errorView({ code: "no_provider" }, ""), { text: "Choose an AI in Settings first", action: "settings", label: "Open Settings" });
  assert.equal(errorView(null, "").action, "settings");
  assert.equal(errorView("Choose an AI in Settings first", "").action, "settings");
  const net = errorView({ code: "provider", message: "ChatGPT could not be reached" }, "ChatGPT");
  assert.equal(net.text, "ChatGPT could not be reached");
  assert.equal(net.action, "retry");
  assert.equal(errorView(null, "Claude").text, "Claude could not answer");
});

test("Esc stops while anything is under way and closes otherwise", () => {
  for (const state of ["listening", "finishing", "sending", "answering"]) assert.equal(liveView(at(snap({ state })), 0).stop, true, state);
  for (const state of ["shown", "error"]) assert.equal(liveView(at(snap({ state })), 0).stop, false, state);
});

test("only a complete answer has the foot row, and it says what left the computer", () => {
  assert.equal(liveView(at(snap({ state: "answering" })), 0).complete, false);
  const v = liveView(at(snap({ state: "shown" })), 0);
  assert.equal(v.complete, true);
  assert.equal(sentLine(v), "Sent your words and the picture to ChatGPT");
  assert.equal(sentLine(liveView(at(snap({ state: "shown", dropped: true })), 0)), "Sent your words only to ChatGPT");
});

test("the view follows the newest line only while the reader is at the bottom", () => {
  assert.equal(isPinned(600, 1000, 400), true);
  assert.equal(isPinned(590, 1000, 400), true);
  assert.equal(isPinned(300, 1000, 400), false);
});

test("the preview fixtures are fixed: every state, the stream joins back to the answer, and the mic has pauses", () => {
  for (const k of ["listening", "finishing", "thinking", "answering", "shown", "error"]) assert.ok(previewSnapshot(k).state);
  assert.equal(previewSnapshot("thinking").state, "sending");
  assert.equal(previewChunks(PREVIEW_ANSWER).join(""), PREVIEW_ANSWER);
  assert.deepEqual(previewChunks(PREVIEW_ANSWER), previewChunks(PREVIEW_ANSWER));
  const lv = Array.from({ length: 70 }, (_, i) => previewLevel(i));
  assert.ok(lv.some((v) => v < 0.05) && lv.some((v) => v > 0.6));
  assert.ok(lv.every((v) => v >= 0 && v <= 1));
  assert.ok(previewPicture().src.startsWith("data:image/svg+xml,"));
});
