// Run: node --test src/lib/tips.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { TIP_DELAY_MS, placeTip } from "./tips.ts";

const tip = { width: 80, height: 24 };

test("a tip sits centred under its element, and above it when the window ends below", () => {
  const under = placeTip({ left: 100, top: 20, width: 40, height: 30 }, tip, 700, 500);
  assert.deepEqual(under, { x: 120 - 40, y: 20 + 30 + 6 });
  const above = placeTip({ left: 100, top: 470, width: 40, height: 30 }, tip, 700, 500);
  assert.equal(above.y, 470 - 6 - 24);
});

test("a tip never leaves the window, at either side or when the window is tiny", () => {
  assert.equal(placeTip({ left: 0, top: 0, width: 10, height: 10 }, tip, 700, 500).x, 6);
  assert.equal(placeTip({ left: 690, top: 0, width: 10, height: 10 }, tip, 700, 500).x, 700 - 80 - 6);
  const tiny = placeTip({ left: 5, top: 5, width: 10, height: 10 }, { width: 300, height: 40 }, 200, 30);
  assert.ok(tiny.x >= 6 && tiny.y >= 6);
});

test("the tip shows well before the webview's own second-long delay", () => {
  assert.ok(TIP_DELAY_MS <= 150);
});
