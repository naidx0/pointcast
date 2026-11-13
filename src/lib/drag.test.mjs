import { test } from "node:test";
import assert from "node:assert/strict";
import { dragAct } from "./drag.ts";

const h = (tag, classes = [], role = null, editable = false) => ({ tag, classes, role, editable });

test("a press on a drag area moves the window; a double press zooms it, as a title bar does", () => {
  const bar = [h("H1"), h("DIV", ["prefs-bar", "drag"]), h("MAIN"), h("DIV", ["prefs"])];
  assert.equal(dragAct(bar, 0, 1), "move");
  assert.equal(dragAct(bar, 0, 2), "zoom");
  assert.equal(dragAct(bar, 2, 1), "none", "a right press is not a drag");
});

test("controls, fields and .no-drag inside a drag area keep their own press", () => {
  const inside = (first) => [first, h("NAV", ["prefs-side", "drag"]), h("DIV", ["prefs"])];
  for (const c of [h("BUTTON"), h("A"), h("INPUT"), h("TEXTAREA"), h("SPAN", [], "switch"), h("DIV", ["no-drag"]), h("DIV", [], null, true)]) {
    assert.equal(dragAct(inside(c), 0, 1), "none", JSON.stringify(c));
  }
  // An icon inside a button is still the button.
  assert.equal(dragAct([h("svg"), h("BUTTON", ["side-item"]), h("NAV", ["drag"])], 0, 1), "none");
});

test("outside every drag area nothing moves", () => {
  assert.equal(dragAct([h("P"), h("DIV", ["prefs-pane"]), h("MAIN")], 0, 1), "none");
  assert.equal(dragAct([], 0, 1), "none");
});
