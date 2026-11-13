// Run: node --experimental-strip-types --test src/lib/call.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { NO_CALL, callAnnounce, callLine, callLineMode, callTitle, escAction, previewCall, sideWord } from "./call.ts";

const call = (side) => ({ active: true, side, ended: null });

test("the pill says it is a call and which side is talking", () => {
  assert.equal(callLine(call("listening")), "In a call · Listening");
  assert.equal(callLine(call("thinking")), "In a call · Thinking");
  assert.equal(callLine(call("speaking")), "In a call · Speaking");
  assert.equal(callLine(NO_CALL), "");
  assert.equal(sideWord(null), "Listening");
});

test("the switch says what a press does", () => {
  assert.match(callTitle(false), /^Call/);
  assert.equal(callTitle(true), "End the call");
});

test("a screen reader hears the call change and why it ended", () => {
  assert.equal(callAnnounce(call("speaking")), "In a call · Speaking");
  assert.equal(callAnnounce({ active: false, side: null, ended: "quiet" }), "Call ended: nobody spoke for 45 seconds");
  assert.equal(callAnnounce({ active: false, side: null, ended: "toggle" }), "Call ended");
  assert.equal(callAnnounce({ active: false, side: null, ended: "failed" }), "The call could not listen");
  assert.equal(callAnnounce(NO_CALL), "");
});

test("the capsule's line follows a voice while listening or speaking and loops while thinking", () => {
  assert.equal(callLineMode("listening"), "listen");
  assert.equal(callLineMode("speaking"), "listen");
  assert.equal(callLineMode("thinking"), "think");
});

test("Esc closes what is open first, then belongs to the call, and only without a call stops speech or closes", () => {
  const none = { big: false, sheet: false, summary: false, sent: false, call: false, speaking: false, stop: false };
  assert.equal(escAction({ ...none, big: true, call: true }), "picture");
  assert.equal(escAction({ ...none, sheet: true, call: true }), "sheet");
  assert.equal(escAction({ ...none, summary: true, call: true }), "summary");
  assert.equal(escAction({ ...none, sent: true, call: true }), "sent");
  assert.equal(escAction({ ...none, call: true, speaking: true, stop: true }), "call", "in a call the call decides: once silences, twice ends");
  assert.equal(escAction({ ...none, speaking: true, stop: true }), "speech");
  assert.equal(escAction({ ...none, stop: true }), "stop");
  assert.equal(escAction(none), "close");
});

test("the preview shows a call only when asked for one", () => {
  assert.deepEqual(previewCall(new URLSearchParams("call=speaking")), call("speaking"));
  assert.deepEqual(previewCall(new URLSearchParams("call=nope")), NO_CALL);
  assert.deepEqual(previewCall(new URLSearchParams("")), NO_CALL);
});
