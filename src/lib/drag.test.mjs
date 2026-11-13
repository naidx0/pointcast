import { test } from "node:test";
import assert from "node:assert/strict";
import { DRAG_SLOP, dragAct, holdClick, holdDown, holdMove, holdUp, settleStep } from "./drag.ts";

const h = (tag, classes = [], role = null, editable = false) => ({ tag, classes, role, editable });

/** A whole press on a drag area: down at (0,0), moves to each point with the left button held, up, click. */
function press(act, points) {
  let { hold, run } = holdDown(act, 0, 0);
  const runs = [run];
  for (const [x, y] of points) {
    const r = holdMove(hold, x, y, 1);
    hold = r.hold;
    runs.push(r.run);
  }
  hold = holdUp(hold);
  const c = holdClick(hold, 1);
  return { moves: runs.filter((r) => r === "move").length, zooms: runs.filter((r) => r === "zoom").length, clicked: !c.swallow };
}

test("a press on a drag area moves the window once the pointer goes 3 px, and that press never clicks", () => {
  const bar = [h("H1"), h("DIV", ["prefs-bar", "drag"]), h("MAIN"), h("DIV", ["prefs"])];
  assert.equal(dragAct(bar, 0, 1), "move");
  assert.equal(DRAG_SLOP, 3);
  assert.deepEqual(press("move", [[1, 0], [2, 1], [3, 0], [9, 4], [20, 8]]), { moves: 1, zooms: 0, clicked: false });
  // Diagonal: 2.2 px each way is past 3 px.
  assert.deepEqual(press("move", [[2.2, 2.2]]), { moves: 1, zooms: 0, clicked: false });
});

test("a press and release under 3 px is a click: the window stays and the click keeps its meaning", () => {
  assert.deepEqual(press("move", []), { moves: 0, zooms: 0, clicked: true });
  assert.deepEqual(press("move", [[1, 1], [2, 0], [0, 2.9]]), { moves: 0, zooms: 0, clicked: true });
  // A keyboard click after a move is never swallowed.
  assert.equal(holdClick({ x: 0, y: 0, moved: true }, 0).swallow, false);
});

test("a move with the button already up ends a press whose release the page never saw", () => {
  const { hold } = holdDown("move", 0, 0);
  assert.deepEqual(holdMove(hold, 50, 50, 0), { hold: null, run: "none" });
});

test("controls, fields and .no-drag inside a drag area keep their own press and never move the window", () => {
  const inside = (first) => [first, h("NAV", ["prefs-side", "drag"]), h("DIV", ["prefs"])];
  for (const c of [h("BUTTON"), h("A"), h("INPUT"), h("TEXTAREA"), h("SPAN", [], "switch"), h("DIV", [], "separator"), h("DIV", [], null, true)]) {
    assert.equal(dragAct(inside(c), 0, 1), "none", JSON.stringify(c));
  }
  // An icon inside a button is still the button: the capsule's tools and Finish.
  assert.equal(dragAct([h("svg"), h("BUTTON", ["hb"]), h("DIV", ["hud"], "toolbar"), h("DIV", ["hud-wrap", "drag"])], 0, 1), "none");
  // A control's press makes no hold, so no amount of travel moves the window.
  assert.deepEqual(press(dragAct(inside(h("BUTTON")), 0, 1), [[30, 30]]), { moves: 0, zooms: 0, clicked: true });
});

test(".no-drag marks out a part of a drag area", () => {
  assert.equal(dragAct([h("SPAN"), h("DIV", ["no-drag"]), h("DIV", ["drag"])], 0, 1), "none");
  assert.equal(dragAct([h("DIV", ["no-drag", "drag"])], 0, 1), "none");
});

test("only the left button moves a window", () => {
  const bar = [h("DIV", ["drag"])];
  for (const b of [1, 2, 3, 4]) assert.equal(dragAct(bar, b, 1), "none", `button ${b}`);
});

test("a double press zooms only a window that zooms today; elsewhere it is one more press", () => {
  const bar = [h("H1"), h("DIV", ["prefs-bar", "drag"])];
  assert.equal(dragAct(bar, 0, 2, true), "zoom", "Settings zooms, as a title bar does");
  assert.deepEqual(holdDown("zoom", 0, 0), { hold: null, run: "zoom" });
  for (const surface of [
    [h("path"), h("svg"), h("DIV", ["line-voice"]), h("SPAN", ["scribe"]), h("DIV", ["hud"]), h("DIV", ["hud-wrap", "drag"])],
    [h("SPAN", ["live-title"]), h("HEADER", ["live-head", "drag"]), h("DIV", ["live"])],
  ]) {
    assert.equal(dragAct(surface, 0, 2, false), "move", "the capsule and the Ask panel never zoom");
    assert.equal(dragAct(surface, 0, 3, true), "move", "a third press is a press");
  }
});

test("the capsule moves from its wave, its time and its gaps, not only from the elements that carried the old attribute", () => {
  const wrap = h("DIV", ["hud-wrap", "drag"]);
  const hud = h("DIV", ["hud"], "toolbar");
  for (const first of [h("path"), h("svg"), h("DIV", ["line-voice"]), h("SPAN", ["sep"]), h("SPAN", ["discard-slot"]), h("SPAN", ["time"]), h("B")]) {
    const chain = first.tag === "path" ? [first, h("svg"), h("DIV", ["line-voice"]), h("SPAN", ["scribe"]), hud, wrap] : [first, hud, wrap];
    assert.equal(dragAct(chain, 0, 1, false), "move", JSON.stringify(first));
  }
});

test(".drag-self moves the window from the box's own empty space, never from what is in it", () => {
  const body = h("DIV", ["live-body", "drag-self"]);
  assert.equal(dragAct([body, h("DIV", ["live", "drag-self"])], 0, 1, false), "move");
  assert.equal(dragAct([h("DIV", ["live", "drag-self"])], 0, 1, false), "move");
  // The answer's words stay selectable.
  assert.equal(dragAct([h("P"), h("DIV", ["live-answer"]), body, h("DIV", ["live", "drag-self"])], 0, 1, false), "none");
});

test("outside every drag area nothing moves", () => {
  assert.equal(dragAct([h("P"), h("DIV", ["prefs-pane"]), h("MAIN")], 0, 1), "none");
  assert.equal(dragAct([], 0, 1), "none");
  assert.deepEqual(holdDown("none", 0, 0), { hold: null, run: "none" });
});

test("where the person put the window: after it moves, two still looks; a later move and stop is put again", () => {
  const looks = ["0,0", "0,0", "5,5", "40,9", "80,20", "80,20", "80,20", "80,20", "90,20", "90,20", "90,20"];
  let s = null;
  const puts = [];
  for (const at of looks) {
    const r = settleStep(s, at);
    s = r.s;
    if (r.put) puts.push(at);
  }
  assert.deepEqual(puts, ["80,20", "90,20"]);
});
