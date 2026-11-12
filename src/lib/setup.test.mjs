// Run: node --test src/lib/setup.test.mjs (Node 22.6+ reads the TypeScript module directly).
import { test } from "node:test";
import assert from "node:assert/strict";
import { STEPS, downloadOnce, nextButton, percent, primaryDownload, speechLine } from "./setup.ts";

test("setup has five steps: welcome, access, try it, Ask, done", () => {
  assert.deepEqual([...STEPS], ["welcome", "access", "try", "ask", "done"]);
});

test("the Ask step never holds Continue back: choosing an AI can wait", () => {
  for (const practiced of [true, false]) assert.deepEqual(nextButton("ask", practiced), { label: "Continue", skip: "Skip" }, "choosing an AI can always be skipped");
});

test("only the recommended model's Download is primary, and only while no model can run", () => {
  const models = [
    { id: "base", ready: false },
    { id: "tiny", ready: false },
  ];
  assert.equal(primaryDownload(models, "base"), "base");
  assert.equal(primaryDownload([{ id: "base", ready: false }, { id: "tiny", ready: true }], "base"), null);
  assert.equal(primaryDownload(models, "gone"), null);
  assert.equal(primaryDownload(models, null), null);
});

test("Continue is enabled on every step while the speech model downloads", () => {
  assert.equal(speechLine({ ready: false, busy: true, error: null }), "downloading");
  for (const step of STEPS) {
    const n = nextButton(step, true);
    assert.notEqual(n.enabled, false, `${step} with the practice run done`);
  }
  for (const step of ["welcome", "access", "ask", "done"]) assert.notEqual(nextButton(step, false).enabled, false, step);
});

test("before the practice run, Try it offers Skip, which the footer shows only beside a held-back Continue", () => {
  const n = nextButton("try", false);
  assert.equal(n.enabled, false);
  assert.equal(n.skip, "Skip");
});

test("the download starts once", () => {
  const ask = downloadOnce();
  let started = 0;
  const start = () => started++;
  assert.equal(ask({ installed: false, busy: false }, start), true);
  assert.equal(ask({ installed: false, busy: false }, start), false);
  assert.equal(started, 1);
});

test("the download does not start when a model is installed or one is already downloading", () => {
  let started = 0;
  downloadOnce()({ installed: true, busy: false }, () => started++);
  downloadOnce()({ installed: false, busy: true }, () => started++);
  assert.equal(started, 0);
});

test("the footer line: downloading, ready, failed, and a cancel is not a failure", () => {
  assert.equal(speechLine({ ready: true, busy: false, error: null }), "ready");
  assert.equal(speechLine({ ready: false, busy: false, error: "Pointcast can't reach GitHub." }), "failed");
  assert.equal(speechLine({ ready: false, busy: false, error: "Cancelled." }), "none");
  assert.equal(speechLine({ ready: false, busy: false, error: null }), "none");
});

test("percent", () => {
  assert.equal(percent(0, 0), null);
  assert.equal(percent(42, 100), 42);
  assert.equal(percent(150, 100), 100);
});
