// Run: node --experimental-strip-types --test src/lib/talkback.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { RATES, languageName, nearestRate, previewVoices, voiceGroups, voiceLabel } from "./talkback.ts";

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
