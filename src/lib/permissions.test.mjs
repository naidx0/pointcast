// Run: node --experimental-strip-types --test src/lib/permissions.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { rowAction, untilAnswered } from "./permissions.ts";

const row = (over) => rowAction({ kind: "screen", access: "unknown", asked: false, pending: false, mac: true, ...over });

test("Screen Recording switched on but not applied yet offers Reopen, not System Settings", () => {
  assert.equal(row({ access: "denied", pending: true }), "reopen");
  assert.equal(row({ access: "unknown", asked: true, pending: true }), "reopen");
  assert.equal(row({ access: "granted", pending: true }), "allowed");
  assert.equal(row({ kind: "microphone", access: "denied", pending: true }), "open", "only Screen Recording waits for a reopen");
});

test("the rows keep what they offered before", () => {
  assert.equal(row({}), "allow");
  assert.equal(row({ asked: true }), "open");
  assert.equal(row({ access: "denied" }), "open", "macOS asks once: the switch is the only way");
  assert.equal(row({ access: "denied", mac: false }), "allow-open");
  assert.equal(row({ access: "granted" }), "allowed");
});

const perms = (microphone) => ({ microphone, screen: "granted", accessibility: "granted", gated: { microphone: true, screen: true, accessibility: true } });
const noWait = () => Promise.resolve();

test("the microphone row changes as soon as the prompt is answered", async () => {
  const reads = ["unknown", "unknown", "granted", "unknown"];
  const seen = [];
  const got = await untilAnswered(() => Promise.resolve(perms(reads.shift())), (p) => seen.push(p.microphone), "microphone", { sleep: noWait });
  assert.equal(got.microphone, "granted");
  assert.deepEqual(seen, ["unknown", "unknown", "granted"], "each read reaches the row, and reading stops at the answer");
});

test("a prompt never answered stops being read, and a failed read is skipped", async () => {
  let n = 0;
  const get = () => (++n === 2 ? Promise.reject("busy") : Promise.resolve(perms("unknown")));
  assert.equal(await untilAnswered(get, () => undefined, "microphone", { every: 10, max: 50, sleep: noWait }), null);
  assert.equal(n, 5);
});
