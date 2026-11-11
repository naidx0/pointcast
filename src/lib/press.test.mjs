// Run: node --experimental-strip-types --test src/lib/press.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { pressOnce } from "./press.ts";

test("a press acts once: on the press, not again on its click; the keyboard and a lone click still act", () => {
  let t = 1000;
  let n = 0;
  const p = pressOnce(() => n++, () => t);
  p.onPointerDown({ button: 0 });
  assert.equal(n, 1, "the first press acts, though no click follows");
  p.onClick({ detail: 1 });
  assert.equal(n, 1, "its own click does not act again");
  p.onPointerDown({ button: 2 });
  assert.equal(n, 1, "a right press does nothing");
  p.onClick({ detail: 0 });
  assert.equal(n, 2, "Enter or Space acts");
  t += 5000;
  p.onClick({ detail: 1 });
  assert.equal(n, 3, "a click with no press of its own acts");
});
