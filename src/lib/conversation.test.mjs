// Run: node --experimental-strip-types --test src/lib/conversation.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  FOLLOW_MAX,
  canSend,
  earlierLabel,
  earlierRows,
  fieldPlaceholder,
  followKey,
  followUpShown,
  isFocusKey,
  kindLabel,
  normalizeSent,
  sentView,
  sizeLabel,
  turnRow,
} from "./conversation.ts";
import { initialLive, liveReduce, liveView, normalizeSnapshot, openNumber, previewPicture, previewSnapshot } from "./live.ts";

const pic = previewPicture();
const turn = (n, over = {}) => ({ n, question: `Question ${n}?`, answer: `Answer ${n}.`, picture: null, stopped: false, error: null, at_ms: n, ...over });
const snap = (over = {}) => normalizeSnapshot({ state: "shown", session: { id: "s", title: "T" }, turn: 3, words: "Question 3?", ...over });

test("the earlier rows are every kept turn: the engine never lists the open one, and the panel's own count is no turn's number", () => {
  // The panel's first question (its `turn` 1) is the session's third: turns 1 and 2 are both earlier rows.
  for (const own of [1, 2]) {
    const s = snap({ turn: own, session: { id: "s", title: "T", turns: 3 }, turns: [turn(1), turn(2)] });
    assert.deepEqual(earlierRows(s).map((r) => r.n), [1, 2], `panel turn ${own}`);
    assert.equal(openNumber(s), 3, "the open turn is the session's third, as `session.turns` counts it once kept");
  }
  const m = liveReduce(initialLive(), { type: "snapshot", snap: snap({ turn: 1, session: { id: "s", title: "T", turns: 3 }, turns: [turn(1), turn(2)] }), now: 0 });
  assert.equal(liveView(m, 0).question, "Question 3?", "an older turn with the panel's number never stands in for the open one");
});

test("the open turn's number: heard and not kept yet, the next one; kept, the session's count; opened, the newest kept", () => {
  const at = (over) => openNumber(normalizeSnapshot({ session: { id: "s", title: "T", turns: 2 }, turn: 0, words: "Q", ...over }));
  assert.equal(at({ state: "listening", turns: [] }), 3, "joining a closed session of two, before its turns arrive");
  assert.equal(at({ state: "listening", turns: [turn(1), turn(2)] }), 3);
  assert.equal(at({ state: "sending", session: { id: "s", title: "T", turns: 3 }, turns: [] }), 3);
  assert.equal(at({ state: "shown", session: { id: "s", title: "T", turns: 3 }, turns: [turn(1), turn(2)] }), 3);
  assert.equal(at({ state: "sending", session: { id: "s", title: "T", turns: 2 }, turns: [turn(1), turn(2)] }), 3, "sent and not kept yet");
  assert.equal(at({ state: "shown", words: "", turns: [turn(1), turn(2)] }), 2, "opened: the newest kept turn");
  assert.equal(openNumber(previewSnapshot("empty")), 0);
});

test("joining a closed session, its turns arriving a moment later keep the open turn, its answer and its number", () => {
  const first = { state: "answering", session: { id: "s", title: "T", turns: 3 }, turn: 1, words: "Question 3?", turns: [] };
  let m = liveReduce(initialLive(), { type: "snapshot", snap: first, now: 0 });
  m = liveReduce(m, { type: "deltas", deltas: [{ turn: 1, text: "Part of it" }] });
  assert.equal(openNumber(m.snap), 3);
  m = liveReduce(m, { type: "snapshot", snap: { ...first, turns: [turn(1), turn(2)] }, now: 1 });
  assert.deepEqual([m.text, openNumber(m.snap), liveView(m, 1).question], ["Part of it", 3, "Question 3?"]);
  assert.deepEqual(earlierRows(m.snap).map((r) => r.n), [1, 2]);
});

test("a session opened from the list opens on its newest turn, whether the fields come empty or absent", () => {
  const empty = normalizeSnapshot({ state: "shown", session: { id: "s", title: "T", turns: 2 }, turn: 0, words: "", answer: "", picture: null, error: null, turns: [turn(1), turn(2)] });
  const v = liveView(liveReduce(initialLive(), { type: "snapshot", snap: empty, now: 0 }), 0);
  assert.deepEqual([v.phase, v.question, v.complete, v.hide], ["shown", "Question 2?", true, false]);
  const opened = liveView(liveReduce(initialLive(), { type: "snapshot", snap: previewSnapshot("opened"), now: 0 }), 0);
  assert.deepEqual([opened.phase, opened.complete, opened.thumb?.src.slice(0, 10)], ["shown", true, "data:image"]);
  // Its newest turn was stopped before any answer: it says so and the panel stays.
  const stopped = normalizeSnapshot({ state: "shown", session: { id: "s", title: "T", turns: 1 }, turn: 0, words: "", turns: [turn(1, { answer: "", stopped: true })] });
  const sv = liveView(liveReduce(initialLive(), { type: "snapshot", snap: stopped, now: 0 }), 0);
  assert.deepEqual([sv.phase, sv.note, sv.hide], ["shown", "Stopped", false]);
});

test("a new session is open on nothing: never 'Not sent', never hidden, the field ready for its first question", () => {
  const v = liveView(liveReduce(initialLive(), { type: "snapshot", snap: previewSnapshot("empty"), now: 0 }), 0);
  assert.deepEqual([v.phase, v.empty, v.hide, v.note, v.complete, v.thumb], ["shown", true, false, null, false, null]);
  assert.equal(followUpShown(v), true);
  assert.deepEqual([fieldPlaceholder(true), fieldPlaceholder(false)], ["Type a question", "Ask a follow-up"]);
  assert.equal(earlierRows(previewSnapshot("empty")).length, 0);
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

test("Sent shows the earlier turns sent again when the conversation was reopened: their words on request, their pictures small", () => {
  const p = (turn, n) => ({ n, turn, w: 1280, h: 720, bytes: 188_400, mime: "image/jpeg", src: pic.src });
  const v = sentView({
    text: "Question 3?",
    pictures: [{ ...p(3, 1) }],
    earlier: [1, 2],
    provider: "Codex",
    adds: ["Codex's own instructions for answering", "MCP server started by Codex: github"],
    replay: "  Question 1: What does the line mean?  ",
    again: [p(2, 1), p(1, 2), p(1, 1)],
  });
  assert.equal(v.replay.text, "Question 1: What does the line mean?");
  assert.deepEqual(v.replay.pictures.map((x) => x.label), ["Question 1, picture 1", "Question 1, picture 2", "Question 2"]);
  assert.equal(v.replay.pictures[0].size, "1280 × 720 · 188 KB · JPEG");
  assert.equal(v.replay.meta, "Their words and 3 pictures");
  assert.deepEqual([v.addsBy, v.adds[1]], ["Codex added", "MCP server started by Codex: github"]);
  assert.equal(sentView({ text: "x", replay: "", again: [] }).replay, null, "nothing sent again: no row");
  assert.equal(sentView({ text: "x", replay: "Q1" }).replay.meta, "Their words");
});

test("sizes, kinds and the earlier questions read as a person says them", () => {
  assert.deepEqual([sizeLabel(870), sizeLabel(1), sizeLabel(214_300), sizeLabel(1_240_000), sizeLabel(18_400_000)], ["870 bytes", "1 byte", "214 KB", "1.2 MB", "18 MB"]);
  assert.deepEqual([kindLabel("image/png"), kindLabel("image/jpeg"), kindLabel("image/webp"), kindLabel("image/svg+xml"), kindLabel("")], ["PNG", "JPEG", "WebP", "SVG", ""]);
  assert.deepEqual(
    [earlierLabel([]), earlierLabel([3]), earlierLabel([1, 2]), earlierLabel([1, 2, 3, 4]), earlierLabel([1, 3, 4])],
    ["", "Question 3", "Questions 1 and 2", "Questions 1 to 4", "Questions 1, 3 and 4"],
  );
});
