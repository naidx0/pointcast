// Run: node --experimental-strip-types --test src/lib/markup.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { isMarkKey, markHint, markPurpose, markWording, markedLabel } from "./markup.ts";

test("a marked picture's label says so in words; M alone marks up", () => {
  assert.deepEqual([markedLabel("PowerPoint", true), markedLabel(" ", true), markedLabel("PowerPoint ", false)], ["PowerPoint · Marked up", "Marked up", "PowerPoint"]);
  assert.deepEqual(
    [isMarkKey({ key: "m" }), isMarkKey({ key: "M" }), isMarkKey({ key: "m", ctrlKey: true }), isMarkKey({ key: "m", metaKey: true }), isMarkKey({ key: "m", repeat: true }), isMarkKey({ key: "n" })],
    [true, true, false, false, false, false],
  );
});

test("opened for a question, the overlay's primary action uses the picture for it; otherwise it captures or saves", () => {
  assert.deepEqual(markWording("ask", false), {
    go: "Use for my question",
    goTitle: "Use for my question (Enter)",
    cancelTitle: "Cancel and keep the picture taken (Esc)",
  });
  assert.equal(markWording("ask", true).go, "Use for my question", "a question's picture is never saved to History");
  assert.deepEqual([markWording("capture", false).go, markWording("capture", true).go], ["Capture", "Save"]);
  assert.equal(markWording("capture", false).cancelTitle, "Cancel (Esc)");
});

test("only an explicit ask is a question; an older frame without a purpose keeps today's wording", () => {
  assert.deepEqual([markPurpose("ask"), markPurpose(undefined), markPurpose("capture"), markPurpose("ASK")], ["ask", "capture", "capture", "capture"]);
});

test("the hint line speaks of the same action as the button", () => {
  const h = "Click to drop a numbered pin · C crop · Enter capture · Esc back";
  assert.equal(markHint(h, "capture"), h);
  assert.equal(markHint(h, "ask"), "Click to drop a numbered pin · C crop · Enter use it · Esc back");
  assert.equal(markHint("Edit picture · C crop · Return saves · Esc cancels", "ask"), "Mark up the picture · C crop · Enter uses it · Esc cancels");
});
