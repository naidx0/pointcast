// Run: node --experimental-strip-types --test src/lib/talkback.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { RATES, RECOMMENDED, defaultLabel, fetchLine, isKokoro, languageName, megabytes, nearestRate, previewVoices, voiceGroups, voiceLabel } from "./talkback.ts";

const v = (name, lang, natural = false) => ({ id: `id.${name}`, name, lang, natural });

test("a natural voice says so in the Voice select, a standard one does not", () => {
  assert.equal(voiceLabel(v("Ava", "en-US", true), "en"), "Ava (natural) · American English");
  assert.equal(voiceLabel(v("Samantha", "en-US"), "en"), "Samantha · American English");
});

test("a language with no name known is shown by its tag", () => {
  assert.equal(languageName("xx-unknown-tag-zz", "en"), "xx-unknown-tag-zz");
  assert.equal(languageName("not a tag", "en"), "not a tag");
});

test("voices group as natural, the person's language, then the rest, keeping their order and leaving out empty groups", () => {
  const voices = [v("Ava", "en-US", true), v("Kyoko", "ja-JP", true), v("Samantha", "en-US"), v("Daniel", "en-GB"), v("Thomas", "fr-FR")];
  const groups = voiceGroups(voices, "en-US");
  assert.deepEqual(
    groups.map((g) => [g.title, g.voices.map((x) => x.name)]),
    [
      ["Natural", ["Ava", "Kyoko"]],
      ["English", ["Samantha", "Daniel"]],
      ["Other languages", ["Thomas"]],
    ],
  );
  assert.deepEqual(
    voiceGroups([v("Samantha", "en-US")], "en-GB").map((g) => g.title),
    ["English"],
  );
  assert.ok(previewVoices.some(isKokoro), "the preview shows a Kokoro voice");
  assert.equal(voiceGroups(voices, "en-US").flatMap((g) => g.voices).length, voices.length, "every voice is offered once");
});

test("a saved rate shows as the nearest one offered, and none saved is normal", () => {
  assert.equal(nearestRate(undefined), 1);
  assert.equal(nearestRate(null), 1);
  assert.equal(nearestRate(Number.NaN), 1);
  assert.equal(nearestRate(1.3), 1.25);
  assert.equal(nearestRate(0.5), 0.8);
  assert.equal(nearestRate(2), 1.5);
  for (const [r] of RATES) assert.equal(nearestRate(r), r);
  assert.ok(RATES.some(([r, name]) => r === 1 && name === "Normal"));
});

test("the preview voices have a natural one to show", () => {
  assert.ok(previewVoices.some((x) => x.natural) && previewVoices.some((x) => !x.natural));
});

const kokoro = (key, name, lang, download) => ({ id: `kokoro:${key}`, name, lang, natural: true, ...(download ? { download } : {}) });

test("Kokoro's voices come first in a group of their own, before this computer's natural voices", () => {
  const voices = [kokoro("af_heart", "Heart", "en-US", 132_353_410), kokoro("bf_emma", "Emma", "en-GB", 132_353_410), v("Ava", "en-US", true), v("Samantha", "en-US")];
  const groups = voiceGroups(voices, "en-US");
  assert.deepEqual(
    groups.map((g) => [g.title, g.voices.map((x) => x.name)]),
    [
      ["Natural (Kokoro)", ["Heart", "Emma"]],
      ["Natural", ["Ava"]],
      ["English", ["Samantha"]],
    ],
  );
  assert.equal(voiceGroups([v("Ava", "en-US", true)], "en-US")[0].title, "Natural", "no Kokoro group without its voices");
});

test("a Kokoro voice shows its download size until it is downloaded, and the best one says it is recommended", () => {
  assert.equal(voiceLabel(kokoro("af_heart", "Heart", "en-US", 132_353_410), "en"), "Heart (recommended) · American English · 132 MB download");
  assert.equal(voiceLabel(kokoro("bf_emma", "Emma", "en-GB"), "en"), "Emma · British English");
  assert.equal(megabytes(9_000), "1 MB");
  assert.equal(megabytes(0), "0 MB");
});

test("no voice chosen is the system's until Kokoro is downloaded, then Kokoro's recommended voice", () => {
  assert.equal(defaultLabel([]), "System default");
  assert.equal(defaultLabel([kokoro("af_heart", "Heart", "en-US", 1000), v("Ava", "en-US", true)]), "System default");
  assert.equal(defaultLabel([kokoro("af_heart", "Heart", "en-US"), v("Ava", "en-US", true)]), "Recommended · Heart (natural)");
  assert.equal(RECOMMENDED, "kokoro:af_heart");
});

test("the download's status line says how far it is, then that it is ready or why it failed", () => {
  assert.equal(fetchLine(null), null);
  assert.equal(fetchLine({ state: "downloading", received: 45_000_000, total: 141_000_000 }), "Downloading the natural voice… 45 MB of 141 MB");
  assert.equal(fetchLine({ state: "downloading", received: 0, total: null }), "Downloading the natural voice…");
  assert.equal(fetchLine({ state: "unpacking" }), "Setting up the natural voice…");
  assert.equal(fetchLine({ state: "ready" }), "The natural voice is ready.");
  assert.equal(fetchLine({ state: "failed", reason: "The disk is full." }), "The natural voice didn't download: The disk is full.");
});
