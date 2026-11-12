// Run: node --experimental-strip-types --test src/lib/conversation.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  FOLLOW_MAX,
  canSend,
  earlierLabel,
  earlierRows,
  followKey,
  followUpShown,
  isFocusKey,
  kindLabel,
  normalizeSent,
  sentView,
  sizeLabel,
  turnRow,
} from "./conversation.ts";
import { initialLive, liveReduce, liveView, normalizeSnapshot, previewPicture, previewSnapshot } from "./live.ts";

const pic = previewPicture();
const turn = (n, over = {}) => ({ n, question: `Question ${n}?`, answer: `Answer ${n}.`, picture: null, stopped: false, error: null, at_ms: n, ...over });
const snap = (over = {}) => normalizeSnapshot({ state: "shown", session: { id: "s", title: "T" }, turn: 3, words: "Question 3?", ...over });

test("the earlier rows are the kept turns before the open one, whether or not the engine keeps the open one too", () => {
  const without = snap({ turns: [turn(1), turn(2)] });
  const withOpen = snap({ turns: [turn(1), turn(2), turn(3)] });
  assert.deepEqual(earlierRows(without).map((r) => r.n), [1, 2]);
  assert.deepEqual(earlierRows(withOpen).map((r) => r.n), [1, 2]);
});

test("a session opened finished, with nothing at the top, opens on its newest turn", () => {
  const s = normalizeSnapshot({ state: "shown", session: { id: "s", title: "T" }, turn: 0, words: "", turns: [turn(1), turn(2)] });
  const m = liveReduce(initialLive(), { type: "snapshot", snap: s, now: 0 });
  const v = liveView(m, 0);
  assert.equal(m.text, "Answer 2.");
  assert.equal(v.question, "Question 2?");
  assert.equal(v.phase, "shown");
  assert.deepEqual(earlierRows(s).map((r) => r.n), [1]);
});

test("a follow-up stopped before any answer keeps the conversation open and says so; a first question still hides", () => {
  const follow = liveView(liveReduce(initialLive(), { type: "snapshot", snap: snap({ turns: [turn(1), turn(2)] }), now: 0 }), 0);
  assert.deepEqual([follow.phase, follow.hide, follow.complete], ["shown", false, true]);
  const first = liveView(liveReduce(initialLive(), { type: "snapshot", snap: snap({ turn: 1, turns: [] }), now: 0 }), 0);
  assert.equal(first.hide, true);
});

test("a row is its question; a row without words says what stands in, and an unanswered one says why", () => {
  assert.equal(turnRow(turn(1, { question: "  Why\n  this?  " })).line, "Why this?");
  assert.equal(turnRow(turn(1, { question: "", picture: pic })).line, "A picture, without words");
  assert.equal(turnRow(turn(1, { question: "" })).line, "No words");
  assert.equal(turnRow(turn(1, { stopped: true })).note, "Stopped");
  assert.equal(turnRow(turn(1, { error: "ChatGPT could not be reached", answer: "" })).note, "Not answered");
  assert.equal(turnRow(turn(1)).note, "");
  assert.equal(turnRow(turn(1, { picture: { src: "https://example.com/a.png", w: 1, h: 1 } })).picture, null, "never a web address");
});

test("the follow-up field shows only once an answer is shown in full", () => {
  const at = (k) => liveView(liveReduce(initialLive(), { type: "snapshot", snap: previewSnapshot(k), now: 0 }), 0);
  assert.equal(followUpShown(at("shown")), true);
  assert.equal(followUpShown(at("conversation")), true);
  for (const k of ["listening", "finishing", "thinking", "answering", "error", "discarded"]) assert.equal(followUpShown(at(k)), false, k);
});

test("Enter sends words, holds an empty field, Shift+Enter is a new line, Esc lets go, composing is left alone", () => {
  assert.equal(followKey({ key: "Enter" }, "why?", false), "send");
  assert.equal(followKey({ key: "Enter" }, "   ", false), "hold");
  assert.equal(followKey({ key: "Enter" }, "why?", true), "hold", "not while a question is under way");
  assert.equal(followKey({ key: "Enter", shiftKey: true }, "why?", false), "newline");
  assert.equal(followKey({ key: "Escape" }, "why?", false), "leave");
  assert.equal(followKey({ key: "Enter", isComposing: true }, "why?", false), "pass");
  assert.equal(followKey({ key: "a" }, "", false), "pass");
  assert.equal(canSend("x".repeat(FOLLOW_MAX + 1), false), false);
});

test("Ctrl+K on Windows and Cmd+K on a Mac reach the field, and nothing else does", () => {
  assert.equal(isFocusKey({ key: "k", ctrlKey: true }, false), true);
  assert.equal(isFocusKey({ key: "K", metaKey: true }, true), true);
  assert.equal(isFocusKey({ key: "k", metaKey: true }, false), false);
  assert.equal(isFocusKey({ key: "k", ctrlKey: true }, true), false);
  assert.equal(isFocusKey({ key: "k", ctrlKey: true, shiftKey: true }, false), false);
  assert.equal(isFocusKey({ key: "j", ctrlKey: true }, false), false);
});

test("Sent says the words, each picture with its size, the earlier questions, the AI and what its tool added", () => {
  const v = sentView({
    text: "Why do we divide by P(B)?",
    pictures: [{ n: 1, w: 1280, h: 800, bytes: 214_300, mime: "image/png", src: pic.src }],
    earlier: [2, 1, 2],
    provider: "Codex",
    model: "gpt-5.1-codex",
    adds: ["  The instructions in AGENTS.md ", ""],
  });
  assert.equal(v.words, "Why do we divide by P(B)?");
  assert.deepEqual(v.pictures, [{ n: 1, src: pic.src, label: "Picture", size: "1280 × 800 · 214 KB · PNG" }]);
  assert.deepEqual(v.rows, [
    { label: "Earlier", value: "Questions 1 and 2, with their answers" },
    { label: "To", value: "Codex · gpt-5.1-codex" },
  ]);
  assert.deepEqual([v.addsBy, v.adds], ["Codex added", ["The instructions in AGENTS.md"]]);
});

test("Sent with nothing but words: no pictures, no earlier questions, nothing added; junk is no view", () => {
  const v = sentView({ text: "", pictures: "x", earlier: null, provider: { name: "ChatGPT" } });
  assert.deepEqual([v.noWords, v.pictures, v.rows, v.addsBy], [true, [], [{ label: "Earlier", value: "None" }, { label: "To", value: "ChatGPT" }], ""]);
  assert.equal(sentView(null), null);
  assert.equal(sentView("text"), null);
  assert.equal(normalizeSent({ pictures: [{ src: "https://example.com/x.png" }] }).pictures[0].n, 1);
  assert.equal(sentView({ pictures: [{ src: "https://example.com/x.png" }] }).pictures[0].src, null, "a web address is never shown");
});

test("sizes, kinds and the earlier questions read as a person says them", () => {
  assert.deepEqual([sizeLabel(870), sizeLabel(1), sizeLabel(214_300), sizeLabel(1_240_000), sizeLabel(18_400_000)], ["870 bytes", "1 byte", "214 KB", "1.2 MB", "18 MB"]);
  assert.deepEqual([kindLabel("image/png"), kindLabel("image/jpeg"), kindLabel("image/webp"), kindLabel("image/svg+xml"), kindLabel("")], ["PNG", "JPEG", "WebP", "SVG", ""]);
  assert.deepEqual(
    [earlierLabel([]), earlierLabel([3]), earlierLabel([1, 2]), earlierLabel([1, 2, 3, 4]), earlierLabel([1, 3, 4])],
    ["", "Question 3", "Questions 1 and 2", "Questions 1 to 4", "Questions 1, 3 and 4"],
  );
});
