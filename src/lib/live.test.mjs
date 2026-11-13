// Run: node --test src/lib/live.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CLOSED,
  actsOf,
  normalizeSnapshot,
  PREVIEW_ANSWER,
  SLOW_MS,
  STALL_MS,
  errorView,
  initialLive,
  isPinned,
  levelOf,
  LEVEL_FIELDS,
  liveReduce,
  liveView,
  noPictureView,
  previewChunks,
  previewLevel,
  previewPicture,
  previewSnapshot,
  pictureCaption,
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
    ["discarded", "discarded", "none", "ink"],
  ];
  for (const [state, phase, line, tone] of cases) {
    const v = liveView(at(snap({ state, error: state === "error" ? "Network down" : null, answer: state === "shown" ? "Yes." : undefined })), 0);
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
  assert.equal(stall.error?.action, "stop");
  assert.equal(stall.error?.label, "Stop");
  assert.equal(stall.stop, true, "Esc still stops the request that is in flight");
});

test("the wait clock starts when sending starts and survives repeated sending snapshots", () => {
  let m = at(snap({ state: "sending" }), 5_000);
  m = at(snap({ state: "sending", thinking: true }), 9_000, m);
  assert.equal(m.waitFrom, 5_000);
  assert.equal(at(snap({ state: "answering" }), 6_000, m).waitFrom, null);
});

test("reasoning arriving (thinking: true) is the AI at work: loops, no 20 s error, and the long-wait line says thinking", () => {
  const m = at(snap({ state: "sending", thinking: true }), 0);
  assert.equal(liveView(m, 1_000).note, null);
  const late = liveView(m, STALL_MS + 5_000);
  assert.equal(late.phase, "thinking");
  assert.equal(late.error, null);
  assert.equal(late.note, "ChatGPT is thinking");
  const ans = liveView(at(snap({ state: "answering", thinking: true })), 0);
  assert.equal(ans.phase, "thinking", "answering with only reasoning so far still loops");
  assert.equal(ans.line, "loops");
  assert.equal(liveView(at(snap({ state: "sending", thinking: "yes" })), STALL_MS).phase, "error", "only true means reasoning");
});

test("turn 0 with an empty session before the first question; the id arriving does not reset the answer", () => {
  let m = at({ ...CLOSED, state: "listening", provider: "ChatGPT" });
  assert.equal(m.snap.turn, 0);
  m = at(snap({ state: "answering", session: { id: "", title: "" } }), 0, m);
  m = deltas(m, { turn: 1, text: "Hi" });
  m = at(snap({ state: "answering", session: { id: "s9", title: "" } }), 1, m);
  assert.equal(m.text, "Hi");
});

test("the header shows the session title, or the first words of the question until there is one", () => {
  const untitled = { id: "", title: "" };
  // While the question is heard the title is not the transcript: empty, whatever the engine has so far.
  for (const state of ["listening", "finishing"]) {
    const v = liveView(at(snap({ state, words: "Why do we divide by P(B) here and not" })), 0);
    assert.equal(v.title, "", state);
    assert.equal(v.question, "", state);
    assert.equal(v.compact, true, state);
  }
  assert.equal(liveView(at(snap({ state: "sending", session: untitled, words: "Why do we divide by P(B) here and not" })), 0).title, "Why do we divide by P(B)…");
  assert.equal(liveView(at(snap({ state: "listening", session: untitled, words: "" })), 0).title, "");
  assert.equal(liveView(at(snap({ state: "sending", words: "x y" })), 0).title, "Why divide");
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

test("the caption under the listening row says what the picture is of, or the screen when the engine gives no name", () => {
  assert.equal(pictureCaption("Snipping Tool"), "Picture of Snipping Tool");
  assert.equal(pictureCaption("  Visual Studio Code "), "Picture of Visual Studio Code");
  assert.equal(pictureCaption(""), "Picture of your screen");
  assert.equal(pictureCaption("   "), "Picture of your screen");
});

test("no AI chosen is the Settings error; any other failure offers Try again", () => {
  assert.deepEqual(errorView({ code: "no_provider" }, ""), { text: "Choose an AI in Settings first", action: "settings", label: "Open Settings" });
  assert.equal(errorView(null, "").action, "settings");
  assert.equal(errorView("Choose an AI in Settings first", "").action, "settings");
  assert.equal(errorView("Choose an AI in Settings first", "").label, "Open Settings");
  const net = errorView({ code: "provider", message: "ChatGPT could not be reached" }, "ChatGPT");
  assert.equal(net.text, "ChatGPT could not be reached");
  assert.equal(net.action, "close", "nothing kept: nothing to ask again");
  assert.equal(errorView(null, "Claude").text, "Claude could not answer");
  const again = errorView({ code: "provider", message: "ChatGPT could not be reached" }, "ChatGPT", true);
  assert.deepEqual(again, { text: "ChatGPT could not be reached", action: "retry", label: "Try again" });
  assert.equal(errorView({ code: "no_provider" }, "", true).action, "settings", "Settings wins over Try again");
});

test("an AI that is not allowed yet sends the person to Settings, whatever was kept", () => {
  const allow = { text: "Allow ChatGPT in Settings first", action: "settings", label: "Open Settings" };
  assert.deepEqual(errorView({ code: "not_allowed", message: "Allow ChatGPT in Settings first" }, "ChatGPT", true), allow);
  assert.deepEqual(errorView("Allow ChatGPT in Settings first", "ChatGPT", true), allow);
  assert.deepEqual(errorView({ code: "not_allowed" }, "ChatGPT"), allow);
});

test("the panel's error offers Try again only when the question was kept", () => {
  const err = { code: "provider", message: "ChatGPT could not be reached" };
  assert.equal(liveView(at(snap({ state: "error", error: err, words: "why is this red" })), 0).error?.action, "retry");
  assert.equal(liveView(at(snap({ state: "error", error: err, words: "  " })), 0).error?.action, "close");
  for (const k of ["provider", "allow", "none"]) {
    const v = liveView(at(previewSnapshot("error", { error: k })), 0);
    assert.equal(v.error?.action, k === "provider" ? "retry" : "settings", k);
  }
});

test("Esc stops while anything is under way and closes otherwise", () => {
  for (const state of ["listening", "finishing", "sending", "answering"]) assert.equal(liveView(at(snap({ state })), 0).stop, true, state);
  for (const state of ["shown", "error"]) assert.equal(liveView(at(snap({ state })), 0).stop, false, state);
});

test("only a complete answer has the foot row, and it says what left the computer", () => {
  assert.equal(liveView(at(snap({ state: "answering" })), 0).complete, false);
  const v = liveView(at(snap({ state: "shown", answer: "Yes." })), 0);
  assert.equal(v.complete, true);
  assert.equal(sentLine(v), "Sent your words and the picture to ChatGPT");
  assert.equal(sentLine(liveView(at(snap({ state: "shown", dropped: true, answer: "Yes." })), 0)), "Sent your words to ChatGPT");
});

test("a question stopped before it was sent never says it was sent, nor that an answer is ready", () => {
  // The owner's bug: Stop while listening, the engine ends the turn as shown with no answer.
  let m = at(snap({ state: "listening", words: "why is" }));
  m = at(snap({ state: "shown", words: "why is" }), 1, m);
  let v = liveView(m, 0);
  assert.equal(v.phase, "discarded");
  assert.equal(v.note, "Not sent");
  assert.equal(v.complete, false, "no foot row");
  assert.equal(v.sent, false);
  assert.equal(sentLine(v), "");
  assert.notEqual(v.announce, "Answer ready");
  assert.equal(v.hide, true, "it says so for a moment, then the panel hides");
  // The same after an earlier answer in the same turn: listening is a new question, the old answer is not its own.
  m = at(snap({ state: "shown", answer: "Earlier answer" }));
  m = at(snap({ state: "listening" }), 1, m);
  m = at(snap({ state: "shown" }), 2, m);
  assert.equal(liveView(m, 0).phase, "discarded");
  assert.equal(liveView(m, 0).note, "Not sent");
  // The engine's own discarded state reads the same.
  v = liveView(at(snap({ state: "discarded" })), 0);
  assert.deepEqual([v.phase, v.note, v.complete, v.sent, v.thumb, v.hide], ["discarded", "Not sent", false, false, null, true]);
});

test("a question stopped after it was sent says Stopped, not Answer ready; one with an answer keeps it", () => {
  let m = at(snap({ state: "listening" }));
  m = at(snap({ state: "sending" }), 1, m);
  m = at(snap({ state: "shown" }), 2, m);
  const v = liveView(m, 0);
  assert.deepEqual([v.phase, v.note, v.complete, v.sent], ["discarded", "Stopped", false, true]);
  m = deltas(at(snap({ state: "answering" })), { turn: 1, text: "Part" });
  m = at(snap({ state: "shown" }), 3, m);
  assert.equal(liveView(m, 0).phase, "shown");
  assert.equal(liveView(m, 0).complete, true);
  assert.equal(sentLine(liveView(m, 0)), "Sent your words and the picture to ChatGPT");
});

test("the picture carries the app's name when the engine gives one, and nothing else is required", () => {
  assert.equal(liveView(at(snap({ state: "listening", picture: { ...pic, label: "  Chrome " } })), 0).label, "Chrome");
  assert.equal(liveView(at(snap({ state: "listening", picture: { src: pic.src, w: 4, h: 3 } })), 0).label, "");
  assert.equal(liveView(at(snap({ state: "listening", picture: { src: pic.src, w: 4, h: 3, label: 7 } })), 0).label, "");
  assert.equal(liveView(at(snap({ state: "listening", dropped: true, picture: { ...pic, label: "Chrome" } })), 0).label, "");
  // Unknown fields from a newer engine are ignored.
  assert.equal(liveView(at(snap({ state: "listening", future: { x: 1 }, picture: { ...pic, extra: true } })), 0).phase, "listening");
});

test("the view follows the newest line only while the reader is at the bottom", () => {
  assert.equal(isPinned(600, 1000, 400), true);
  assert.equal(isPinned(590, 1000, 400), true);
  assert.equal(isPinned(300, 1000, 400), false);
});

test("the preview fixtures are fixed: every state, the stream joins back to the answer, and the mic has pauses", () => {
  for (const k of ["listening", "finishing", "thinking", "answering", "shown", "error", "discarded"]) assert.ok(previewSnapshot(k).state);
  assert.equal(previewSnapshot("listening").session.title, "", "no title while listening");
  assert.ok(previewPicture().label && previewPicture("code").label && previewPicture("code").src !== previewPicture().src);
  assert.equal(previewSnapshot("thinking").state, "sending");
  assert.equal(previewChunks(PREVIEW_ANSWER).join(""), PREVIEW_ANSWER);
  assert.deepEqual(previewChunks(PREVIEW_ANSWER), previewChunks(PREVIEW_ANSWER));
  const lv = Array.from({ length: 70 }, (_, i) => previewLevel(i));
  assert.ok(lv.some((v) => v < 0.05) && lv.some((v) => v > 0.6));
  assert.ok(lv.every((v) => v >= 0 && v <= 1));
  assert.ok(previewPicture().src.startsWith("data:image/svg+xml,"));
});

test("the level prefers an engine-smoothed field, falls back to the raw level, and is 0 with the mic off", () => {
  const fields = ["level_smooth", "voice", "level"];
  assert.equal(levelOf({ mic: true, level: 0.4 }, fields), 0.4);
  assert.equal(levelOf({ mic: true, level: 0.4, level_smooth: 0.3 }, fields), 0.3);
  assert.equal(levelOf({ mic: true, level: 0.4, voice: 0.2 }, fields), 0.2);
  assert.equal(levelOf({ mic: false, level: 0.9 }, fields), 0);
  assert.equal(levelOf({ level: 7 }, fields), 1);
  assert.equal(levelOf({ level: "loud" }, fields), 0);
  assert.equal(levelOf(null, fields), 0);
  // The engine names its smoothed Ask level `loudness`; the panel reads it before the raw level.
  assert.equal(levelOf({ mic: true, level: 0.4, loudness: 0.7 }, LEVEL_FIELDS), 0.7);
  assert.equal(levelOf({ mic: true, level: 0.4 }, LEVEL_FIELDS), 0.4);
});

// —— The answer's reveal: even, never ahead of what arrived, and exactly all of it at the end ——
import { REVEAL_CATCH_MS, REVEAL_MAX_LAG_MS, revealSlice, revealStep, settledSplit } from "./live.ts";
import { parseMarkdown, settleTail } from "./markdown.ts";

/** Plays chunks arriving at the given times through the reveal at 60 frames a second; returns each frame's state. */
function play(arrivals, until, o = {}) {
  let total = 0;
  let shown = 0;
  const at = [];
  const frames = [];
  let k = 0;
  for (let t = 0; t <= until; t += 16) {
    while (k < arrivals.length && arrivals[k][0] <= t) {
      for (let i = 0; i < arrivals[k][1]; i++) at.push(arrivals[k][0]);
      total += arrivals[k][1];
      k++;
    }
    const lag = Math.floor(shown) < total ? t - at[Math.floor(shown)] : 0;
    const next = revealStep({ shown, total, dt: 16, lag, done: o.doneAt != null && t >= o.doneAt });
    frames.push({ t, total, prev: shown, shown: next, lag });
    shown = next;
  }
  return frames;
}

test("the reveal is even: bursts become a steady flow, never ahead of what arrived, never going back", () => {
  // 60 characters at once every 400 ms, as a provider streams.
  const bursts = Array.from({ length: 12 }, (_, i) => [i * 400, 60]);
  const f = play(bursts, 6000);
  for (const x of f) {
    assert.ok(x.shown <= x.total, `ahead at ${x.t}`);
    assert.ok(x.shown >= x.prev, `back at ${x.t}`);
  }
  const steps = f.slice(1).map((x) => Math.floor(x.shown) - Math.floor(x.prev));
  assert.ok(Math.max(...steps) <= 6, `at most a few characters a frame, not a burst of 60: ${Math.max(...steps)}`);
  // Moving most of the time while text waits: frames with a backlog where nothing appears are rare.
  const waiting = f.filter((x) => x.total - x.prev >= 1);
  const still = waiting.filter((x) => Math.floor(x.shown) === Math.floor(x.prev)).length;
  assert.ok(still / waiting.length < 0.5, `${still} still of ${waiting.length}`);
  assert.equal(f[f.length - 1].shown, 720, "it all shows in the end");
});

test("the reveal catches up in a bounded time and stays only a little behind", () => {
  const f = play([[0, 2000]], 3000);
  const done = f.find((x) => x.shown === 2000);
  assert.ok(done && done.t <= REVEAL_MAX_LAG_MS + 16, `2000 characters at once are shown by ${done?.t} ms`);
  const steady = play(Array.from({ length: 60 }, (_, i) => [i * 100, 12]), 7000);
  assert.ok(Math.max(...steady.map((x) => x.lag)) <= REVEAL_CATCH_MS * 2.5, "a steady stream is followed closely");
});

test("done, or less motion, shows everything at once and lands exactly on the full text", () => {
  assert.equal(revealStep({ shown: 3.4, total: 500, dt: 16, done: true }), 500);
  assert.equal(revealStep({ shown: 10, total: 500, dt: 16, lag: REVEAL_MAX_LAG_MS }), 500, "an old backlog");
  const f = play([[0, 300]], 200, { doneAt: 100 });
  assert.equal(f[f.length - 1].shown, 300);
  assert.equal(revealStep({ shown: 499.9, total: 500, dt: 16 }), 500, "the last fraction lands exactly");
  assert.equal(revealStep({ shown: 900, total: 500, dt: 16 }), 500, "a shorter text (a new turn) is never overshot");
  assert.equal(revealStep({ shown: 10, total: 500, dt: 0 }), 10);
  assert.equal(revealSlice("ab😀c", 3), "ab😀", "never half a character");
  assert.equal(revealSlice("abc", 9.7), "abc");
});

test("only the tail is re-read: the settled blocks parse the same alone as in the whole, at every length", () => {
  const src = PREVIEW_ANSWER;
  let moved = 0;
  let last = 0;
  for (let n = 0; n <= src.length; n++) {
    const p = src.slice(0, n);
    const k = settledSplit(p);
    assert.ok(k >= 0 && k <= p.length);
    if (k !== last) moved++;
    last = k;
    const parts = [...parseMarkdown(p.slice(0, k)), ...parseMarkdown(settleTail(p.slice(k)))];
    assert.deepEqual(parts, parseMarkdown(settleTail(p)), `at ${n}: split ${k}`);
  }
  assert.ok(moved >= 4, "it settles block by block");
  const fenced = ["```", "a", "", "b"].join(String.fromCharCode(10));
  assert.equal(settledSplit(fenced), 0, "never inside a code block");
  const list = ["1. one", "", "2. two"].join(String.fromCharCode(10));
  assert.equal(settledSplit(list), 0, "never between items of one list");
});

import { ANSWER_MIN_H, PANEL_MAX_SHARE, PANEL_W, panelHeight } from "./live.ts";

test("the panel opens bigger: 500 wide, a reading surface once an answer shows, never past 60 % of the screen", () => {
  assert.equal(PANEL_W, 640, "as platform.rs PANEL_WIDTH docks it");
  assert.equal(panelHeight(96, 1040, false), 96, "listening: one row, nothing empty below");
  assert.equal(panelHeight(180, 1040, true), ANSWER_MIN_H, "a short answer still opens tall");
  assert.ok(ANSWER_MIN_H >= 300);
  assert.equal(panelHeight(500, 1040, true), 500, "a longer one grows with its content");
  assert.equal(panelHeight(2000, 1040, true), Math.round(1040 * PANEL_MAX_SHARE), "up to 60 % of the screen");
  assert.equal(panelHeight(180, 400, true), 240, "a short screen wins over the minimum");
  assert.equal(panelHeight(NaN, 1040, false), 0);
});

test("a spoken follow-up is heard inside the conversation; only a brand-new question is the one quiet row", () => {
  const at = (kind) => liveView(liveReduce(initialLive(), { type: "snapshot", snap: previewSnapshot(kind), now: 0 }), 0);
  const fresh = at("listening");
  assert.deepEqual([fresh.phase, fresh.compact, fresh.title, fresh.mic, fresh.line], ["listening", true, "", true, "voice"]);
  const follow = at("follow");
  assert.deepEqual([follow.phase, follow.compact, follow.title, follow.mic, follow.line, follow.question], ["listening", false, "Bayes' rule on the slide", true, "voice", ""]);
  assert.equal(follow.stop, true, "Esc and Stop still stop the follow-up");
  // Finishing the follow-up's words: still the conversation.
  const snap = { ...previewSnapshot("follow"), state: "finishing" };
  const fin = liveView(liveReduce(initialLive(), { type: "snapshot", snap, now: 0 }), 0);
  assert.deepEqual([fin.phase, fin.compact, fin.line], ["finishing", false, "loops"]);
});

test("a question with no picture of the screen says so, with Allow only when a permission is missing", () => {
  const blind = (state, why, over = {}) => liveView(at(snap({ state, picture: null, no_picture: why, ...over })), 0);
  for (const state of ["listening", "finishing", "sending", "answering", "shown", "error"]) {
    assert.deepEqual(blind(state, "not_allowed").noPicture, { text: "No picture: Pointcast isn't allowed to see your screen.", allow: true }, state);
  }
  assert.deepEqual(blind("shown", "failed").noPicture, { text: "No picture: the screen could not be read.", allow: false });
  // The receipt still says only the words went.
  assert.equal(sentLine(liveView(at(snap({ state: "shown", picture: null, no_picture: "not_allowed", answer: "No." })), 0)), "Sent your words to ChatGPT");
  assert.equal(blind("listening", null).noPicture, null);
  assert.equal(blind("listening", "nonsense").noPicture, null, "an unknown reason is not shown");
  assert.equal(blind("listening", "not_allowed", { picture: pic }).noPicture, null, "a picture came after all");
  assert.equal(blind("listening", "not_allowed", { dropped: true }).noPicture, null, "the person took it out");
  assert.equal(blind("closed", "not_allowed").noPicture, null);
  assert.equal(blind("discarded", "not_allowed").noPicture, null);
  assert.equal(noPictureView(undefined), null);
});

import { PANEL_MAX_W, PANEL_MIN_H, PANEL_MIN_W, dragSize, panelWidth, parsePanelSize } from "./live.ts";

test("the Ask panel can be resized: its width stays readable and on screen", () => {
  assert.deepEqual([PANEL_MIN_W, PANEL_MAX_W], [380, 900], "as platform.rs PANEL_MIN_WIDTH and PANEL_MAX_WIDTH");
  assert.equal(panelWidth(undefined, 1512), PANEL_W, "no choice yet: the default width");
  assert.equal(panelWidth(NaN, 1512), PANEL_W);
  assert.equal(panelWidth(640.4, 1512), 640);
  assert.equal(panelWidth(100, 1512), PANEL_MIN_W, "never too narrow to read");
  assert.equal(panelWidth(5000, 3000), PANEL_MAX_W, "never wider than a reading column");
  assert.equal(panelWidth(900, 600), 576, "never past the screen's edge margins");
});

test("a chosen height is the answer's height; the listening row still fits its one row", () => {
  assert.equal(panelHeight(96, 1040, false, 700), 96, "listening: still one row");
  assert.equal(panelHeight(180, 1040, true, 700), 700, "a short answer opens at the chosen height");
  assert.equal(panelHeight(2000, 1040, true, 700), 700, "a long one scrolls inside the chosen height");
  assert.equal(panelHeight(2000, 1040, true, 900), 900, "a chosen height may pass 60 % of the screen");
  assert.equal(panelHeight(500, 1040, true, 50), PANEL_MIN_H, "never too short to read");
  assert.equal(panelHeight(500, 600, true, 5000), 576, "never past the screen");
  assert.equal(panelHeight(500, 1040, true, null), 500, "no choice: as before");
});

test("dragging the corner grip: left widens (the panel stays docked right), down makes it taller", () => {
  const screen = { w: 1512, h: 945 };
  assert.deepEqual(dragSize({ w: 500, h: 400 }, -120, 80, screen), { w: 620, h: 480 });
  assert.deepEqual(dragSize({ w: 500, h: 400 }, 400, -400, screen), { w: PANEL_MIN_W, h: PANEL_MIN_H });
  assert.deepEqual(dragSize({ w: 500, h: 400 }, -2000, 2000, screen), { w: PANEL_MAX_W, h: Math.round(945 - 24) });
});

test("the saved size is read back only when it is a real size", () => {
  assert.deepEqual(parsePanelSize('{"w":640,"h":520}'), { w: 640, h: 520 });
  assert.deepEqual(parsePanelSize('{"w":640}'), { w: 640, h: null });
  for (const bad of [null, "", "nope", "[]", '{"w":"wide"}', '{"w":-3,"h":0}']) assert.equal(parsePanelSize(bad), null, String(bad));
});

test("what the AI did on the computer rides on a turn and on the snapshot only when there is some", () => {
  assert.deepEqual(actsOf(["Clicked button \"Send\" in Mail", 3, "", null, "Opened TextEdit"]), ["Clicked button \"Send\" in Mail", "Opened TextEdit"]);
  assert.deepEqual(actsOf("Opened TextEdit"), []);
  const s = normalizeSnapshot({ state: "shown", acts: ["Opened TextEdit"], turns: [{ n: 1, question: "q", answer: "a", acts: ["Typed into text area in TextEdit"] }, { n: 2, question: "q2", answer: "a2" }] });
  assert.deepEqual(s.acts, ["Opened TextEdit"]);
  assert.deepEqual(s.turns[0].acts, ["Typed into text area in TextEdit"]);
  assert.equal("acts" in s.turns[1], false, "a turn that did nothing reads as it always did");
  assert.equal("acts" in normalizeSnapshot({ state: "shown" }), false);
});
