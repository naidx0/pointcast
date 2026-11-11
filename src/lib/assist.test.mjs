import test from "node:test";
import assert from "node:assert/strict";
import {
  CODEX_INSTALL,
  GROUPS,
  PREVIEW_KINDS,
  askReady,
  chosenRoute,
  consentText,
  defaultModel,
  goesLine,
  groups,
  modelLabel,
  needsConsent,
  normalizeAssist,
  nowView,
  previewApply,
  previewAssist,
  routeView,
  visibleRoutes,
} from "./assist.ts";

const STATES = ["ready", "signed_out", "not_installed", "too_old", "no_key", "not_running", "unsupported"];
const route = (o = {}) => ({
  id: "codex",
  name: "ChatGPT",
  kind: "plan",
  state: "ready",
  line: "",
  goes_to: "OpenAI",
  offline: false,
  consented: false,
  models: [],
  ...o,
});
const none = { chosen: null, offline_only: false };

test("every state of every kind has a plain line, and an action only with its label", () => {
  for (const kind of ["plan", "key", "local"])
    for (const state of STATES) {
      const v = routeView(route({ kind, state, id: `${kind}-x`, name: "X" }), none);
      assert.ok(v.line && !v.line.endsWith(".") && !v.line.includes("!"), `${kind}/${state}: one plain line, no full stop`);
      assert.equal(v.action === null, v.label === null, `${kind}/${state}: an action and its label go together`);
      assert.equal(v.label === null || v.label[0] === v.label[0].toUpperCase(), true);
      if (v.label) assert.equal(v.label.slice(1), v.label.slice(1).toLowerCase(), `${v.label} is sentence case`);
    }
});

test("each state offers the action that fits it", () => {
  const act = (o, a = none) => {
    const v = routeView(route(o), a);
    return [v.action, v.label];
  };
  assert.deepEqual(act({ state: "ready" }), ["use", "Use"]);
  assert.deepEqual(act({ state: "signed_out" }), ["sign_in", "Sign in"]);
  assert.deepEqual(act({ kind: "key", id: "openai", state: "no_key" }), ["add_key", "Add key"]);
  assert.deepEqual(act({ state: "not_installed" }), ["get", "How to get it"]);
  assert.deepEqual(act({ state: "too_old" }), ["get", "How to update"]);
  assert.deepEqual(act({ kind: "local", id: "ollama", name: "Ollama on this computer", state: "not_running" }), ["check", "Check again"]);
  assert.deepEqual(act({ state: "unsupported" }), [null, null]);
  assert.deepEqual(act({ state: "ready" }, { chosen: "codex", offline_only: false }), [null, null], "the chosen route offers nothing to press");
});

test("the lines say the state in plain words", () => {
  assert.equal(routeView(route({ kind: "key", id: "openai", state: "ready" }), none).line, "Key saved");
  assert.equal(routeView(route({ kind: "local", id: "ollama", state: "not_running" }), none).line, "Start Ollama, then check again");
  assert.equal(routeView(route({ kind: "local", id: "lmstudio", state: "not_running" }), none).line, "Start LM Studio's server, then check again");
  assert.equal(routeView(route({ state: "not_installed" }), none).line, "Codex is not on this computer");
  assert.equal(routeView(route({ state: "too_old", line: "Codex 0.9 is too old; 0.12 or newer is needed" }), none).line, "Codex 0.9 is too old; 0.12 or newer is needed");
  assert.equal(routeView(route({ state: "ready", line: "engine words" }), none).line, "Signed in", "the engine's line is used only where the page cannot know");
});

test("Codex's help is OpenAI's own one-line install; the others say where to get the app", () => {
  const get = routeView(route({ state: "not_installed" }), none).help;
  assert.equal(get.command, CODEX_INSTALL);
  assert.equal(CODEX_INSTALL, "npm i -g @openai/codex");
  assert.match(get.text, /check again/);
  assert.match(routeView(route({ state: "too_old" }), none).help.text, /^Update/);
  const ollama = routeView(route({ kind: "local", id: "ollama", name: "Ollama on this computer", state: "not_installed" }), none);
  assert.equal(ollama.help.command, null);
  assert.match(ollama.help.text, /ollama\.com/);
  assert.equal(routeView(route({ state: "ready" }), none).help, null);
});

test("forget is offered only when there is something to forget", () => {
  assert.equal(routeView(route({ state: "ready" }), none).forget, false, "signed in but never allowed");
  assert.equal(routeView(route({ state: "ready", consented: true }), none).forget, true);
  assert.equal(routeView(route({ kind: "key", id: "openai", state: "ready" }), none).forget, true, "a saved key");
  assert.equal(routeView(route({ kind: "key", id: "openai", state: "no_key" }), none).forget, false);
  assert.equal(routeView(route({ state: "ready" }), { chosen: "codex", offline_only: false }).forget, true);
});

test("routes are grouped plainly, in order, and empty groups are left out", () => {
  const a = previewAssist("fresh");
  assert.deepEqual(GROUPS.map(([, t]) => t), ["Your plan", "A key", "This computer"]);
  assert.deepEqual(groups(a).map((g) => [g.title, g.routes.map((r) => r.id)]), [
    ["Your plan", ["codex"]],
    ["A key", ["openai", "openrouter", "gemini"]],
    ["This computer", ["ollama", "lmstudio"]],
  ]);
  const ids = groups(a).flatMap((g) => g.routes.map((r) => r.id));
  assert.equal(new Set(ids).size, a.providers.length, "every route once");
});

test("only on this computer hides every route that leaves it, and the remote choice stops answering", () => {
  const a = { ...previewAssist("codex-ready"), offline_only: true };
  const shown = visibleRoutes(a);
  assert.ok(shown.length > 0 && shown.every((p) => p.offline));
  assert.deepEqual(groups(a).map((g) => g.title), ["This computer"]);
  assert.equal(chosenRoute(a), null);
  assert.equal(askReady(a), false);
  assert.match(nowView(a).line, /off for now/);
  const local = previewAssist("offline");
  assert.equal(chosenRoute(local)?.id, "ollama");
  assert.equal(askReady(local), true);
  assert.equal(nowView(local).line, "Stays on this computer");
});

test("the top says which AI answers and where the words go", () => {
  assert.deepEqual(nowView(previewAssist("fresh")), { title: "Nothing chosen", line: "Choose an AI below. Until then nothing leaves this computer." });
  assert.deepEqual(nowView(previewAssist("codex-ready")), { title: "ChatGPT", line: "Your words and pictures go to OpenAI, through Codex on this computer" });
  assert.equal(nowView(previewAssist("key")).line, "Your words and pictures go to OpenAI, with your key");
  assert.equal(nowView(previewAssist("local")).line, "Stays on this computer");
  const words = { ...previewAssist("local"), offline_only: false };
  assert.equal(goesLine({ id: "openrouter", kind: "key", goes_to: "OpenRouter", offline: false }, false), "Your words go to OpenRouter, with your key");
  assert.equal(nowView({ ...words, model: "qwen2.5:7b" }).line, "Stays on this computer");
});

test("consent names the destination and the account; a model on this computer says nothing leaves", () => {
  const c = consentText(route());
  assert.equal(c.title, "Use ChatGPT for Ask?");
  assert.equal(
    c.body,
    "Ask sends your words and the picture you take to OpenAI, on your account. Nothing goes to Pointcast. You see each picture before it leaves.",
  );
  assert.equal(consentText(route({ kind: "local", offline: true, name: "Ollama on this computer" })).body, "Nothing leaves this computer.");
  assert.equal(needsConsent(route()), true);
  assert.equal(needsConsent(route({ consented: true })), false);
});

test("a route starts on a model that can see pictures, and one that cannot is marked", () => {
  const models = [
    { id: "a", name: "A", images: false },
    { id: "b", name: "B", images: true },
  ];
  assert.equal(defaultModel({ models }), "b");
  assert.equal(defaultModel({ models: [models[0]] }), "a");
  assert.equal(defaultModel({ models: [] }), null);
  assert.equal(modelLabel(models[0]), "A (words only)");
  assert.equal(modelLabel(models[1]), "B");
});

test("a reply of any shape reads as a whole one", () => {
  assert.deepEqual(normalizeAssist(null), { chosen: null, model: null, offline_only: false, providers: [] });
  const a = normalizeAssist({
    chosen: "gone",
    providers: [{ id: "x", kind: "local", state: "weird" }, { id: "", kind: "key" }, { id: "y", kind: "nope" }, { id: "z", kind: "key", models: [{ id: "m" }, null] }],
  });
  assert.equal(a.chosen, null, "a chosen id the list does not have is no choice");
  assert.deepEqual(a.providers.map((p) => [p.id, p.state, p.offline]), [
    ["x", "unsupported", true],
    ["z", "unsupported", false],
  ]);
  assert.deepEqual(a.providers[1].models, [{ id: "m", name: "m", images: false }]);
});

test("every preview kind is a whole reply, and the preview commands change it as the engine would", () => {
  for (const k of PREVIEW_KINDS) assert.deepEqual(normalizeAssist(previewAssist(k)), previewAssist(k), k);
  let a = previewAssist("codex-signed-out");
  a = previewApply(a, { cmd: "signed_in", id: "codex" });
  assert.equal(a.providers[0].state, "ready");
  assert.equal(previewApply(a, { cmd: "choose", id: "codex", model: null, consent: false }), "Allow ChatGPT in Settings first");
  a = previewApply(a, { cmd: "choose", id: "codex", model: null, consent: true });
  assert.equal(a.chosen, "codex");
  assert.equal(a.model, "gpt-5.5");
  assert.equal(a.providers[0].consented, true);
  a = previewApply(a, { cmd: "forget", id: "codex" });
  assert.equal(a.chosen, null);
  assert.equal(a.providers[0].consented, false);
  a = previewApply(a, { cmd: "key_set", id: "openai" });
  assert.equal(routeView(a.providers[1], a).line, "Key saved");
  a = previewApply(a, { cmd: "forget", id: "openai" });
  assert.equal(a.providers[1].state, "no_key", "forget clears the key too");
});
