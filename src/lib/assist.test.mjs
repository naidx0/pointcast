import test from "node:test";
import assert from "node:assert/strict";
import {
  BASE_ERRORS,
  CLAUDE_CODE_NOT_OK,
  CODEX_INSTALL,
  CUSTOM,
  GROUPS,
  PREVIEW_KINDS,
  CODEX_SIGN_OUT,
  checkingAssist,
  modelName,
  askReady,
  baseOf,
  chosenRoute,
  consentText,
  customCheck,
  effortView,
  normalizeModels,
  defaultModel,
  goesLine,
  groups,
  hostOf,
  installHelp,
  isHere,
  modelLabel,
  needsConsent,
  needsConsentAt,
  normalizeAssist,
  nowView,
  offlineSaid,
  withOffline,
  previewApply,
  previewAssist,
  routeView,
  visibleRoutes,
} from "./assist.ts";

const STATES = ["ready", "signed_out", "not_installed", "too_old", "no_key", "not_running", "unsupported", "checking"];
const route = (o = {}) => ({
  id: "codex",
  name: "ChatGPT",
  kind: "plan",
  state: "ready",
  line: "",
  goes_to: "OpenAI",
  offline: false,
  available: true,
  base: "",
  consented: false,
  models: [],
  efforts: [],
  plan: null,
  email: null,
  adds: [],
  ...o,
});
const none = { chosen: null, offline_only: false };
const byId = (a, id) => a.providers.find((p) => p.id === id);

test("every state of every kind has a plain line, and an action only with its label", () => {
  for (const kind of ["plan", "agent", "key", "local"])
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
  assert.deepEqual(act({ state: "not_installed" }), ["get", "Install"]);
  assert.deepEqual(act({ state: "too_old" }), ["get", "How to update"]);
  assert.deepEqual(act({ kind: "local", id: "ollama", name: "Ollama on this computer", state: "not_running" }), ["check", "Check again"]);
  assert.deepEqual(act({ state: "unsupported" }), [null, null]);
  assert.deepEqual(act({ state: "ready" }, { chosen: "codex", offline_only: false }), [null, null], "the chosen route offers nothing to press");
});

test("a signed-in plan says which plan, in plain words", () => {
  assert.equal(routeView(route({ state: "ready", plan: "free" }), none).line, "ChatGPT Free, through Codex");
  assert.equal(routeView(route({ state: "ready", plan: "PLUS" }), none).line, "ChatGPT Plus, through Codex");
  assert.equal(routeView(route({ state: "ready" }), none).line, "Signed in", "no plan said: no plan guessed");
  assert.equal(normalizeAssist({ providers: [{ id: "codex", kind: "plan", planType: "free" }] }).providers[0].plan, "free");
  assert.equal(routeView(previewAssist("codex-ready").providers[0], previewAssist("codex-ready")).line, "Signed in as you@example.com", "the address Codex names comes first");
});

test("a route on this computer drops the words its group already says", () => {
  const v = routeView(route({ kind: "local", id: "ollama", name: "Ollama on this computer", offline: true }), none);
  assert.equal(v.name, "Ollama");
  assert.equal(nowView(previewAssist("local")).title, "Ollama");
  assert.equal(routeView(route(), none).name, "ChatGPT");
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
  assert.match(get.account, /Sign in/, "then the account, through Codex's own sign-in");
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
  assert.deepEqual(GROUPS.map(([, t]) => t), ["Your plan", "Your subscriptions", "A key", "This computer"]);
  assert.deepEqual(groups(a).map((g) => [g.title, g.routes.map((r) => r.id)]), [
    ["Your plan", ["codex"]],
    ["Your subscriptions", ["gemini-cli", "copilot", "opencode", "claude-code"]],
    ["A key", ["openai", "anthropic", "xai", "gemini", "groq", "mistral", "deepseek", "openrouter", "custom"]],
    ["This computer", ["ollama", "lmstudio"]],
  ]);
  const ids = groups(a).flatMap((g) => g.routes.map((r) => r.id));
  assert.equal(new Set(ids).size, a.providers.length, "every route once");
  // The fixture lists the routes in the engine's own order (catalog::ROUTES), which is the order on screen.
  assert.deepEqual(ids, a.providers.map((p) => p.id));
  assert.deepEqual(groups({ ...a, providers: a.providers.filter((p) => p.kind !== "agent") }).map((g) => g.title), ["Your plan", "A key", "This computer"]);
});

test("an agent shows the engine's line as is, offers Use when installed, Check again when not, and no model", () => {
  const a = previewAssist("fresh");
  const gemini = routeView(byId(a, "gemini-cli"), a);
  assert.deepEqual([gemini.name, gemini.line, gemini.action, gemini.label], ["Gemini", "Gemini CLI 0.9.0 is installed. Sign in with the Gemini CLI first.", "use", "Use"]);
  const copilot = routeView(byId(a, "copilot"), a);
  assert.deepEqual([copilot.line, copilot.action, copilot.label], ["GitHub Copilot CLI is not installed on this computer. Install it first.", "get", "Install"]);
  assert.deepEqual([copilot.help.command, copilot.help.page], ["npm install -g @github/copilot", "https://github.com/github/copilot-cli"]);
  assert.match(copilot.help.account, /sign in/);
  for (const id of ["gemini-cli", "copilot", "opencode", "claude-code"]) {
    const h = installHelp({ id, name: id });
    assert.ok(h.command.startsWith("npm install -g ") && h.page.startsWith("https://") && h.account, id);
  }
  assert.equal(routeView(route({ kind: "agent", id: "opencode", name: "OpenCode", state: "unsupported", line: "OpenCode is installed, but…" }), none).action, null);
  assert.equal(defaultModel(byId(a, "gemini-cli")), null, "an agent picks its own model");
  // Chosen, it says where the words go, with the destination the engine names.
  const chosen = previewApply(a, { cmd: "choose", id: "gemini-cli", model: null, consent: true });
  assert.equal(chosen.chosen, "gemini-cli");
  assert.equal(chosen.model, null);
  assert.deepEqual(nowView(chosen), { title: "Gemini", line: "Your words and pictures go to Google, through your Gemini CLI sign-in" });
  assert.equal(goesLine({ id: "copilot", kind: "agent", goes_to: "GitHub, through your Copilot plan", offline: false }), "Your words and pictures go to GitHub on your Copilot plan");
  assert.equal(consentText(byId(a, "opencode")).title, "Use OpenCode for Ask?");
});

test("a route that cannot be chosen yet says why, offers nothing, and never answers Ask", () => {
  const a = previewAssist("fresh");
  const p = byId(a, "claude-code");
  assert.deepEqual([p.available, p.kind, p.state], [false, "agent", "unsupported"]);
  const v = routeView(p, a);
  assert.deepEqual([v.name, v.line, v.action, v.label, v.chosen, v.forget], ["Claude", CLAUDE_CODE_NOT_OK, null, null, false, false]);
  // Chosen by hand in the settings file, it still does not answer, and the top says why.
  const forced = { ...a, chosen: "claude-code" };
  assert.equal(chosenRoute(forced), null);
  assert.equal(askReady(forced), false);
  assert.equal(routeView(p, forced).chosen, false);
  assert.deepEqual(nowView(forced), { title: "Nothing chosen", line: CLAUDE_CODE_NOT_OK });
  assert.equal(previewApply(a, { cmd: "choose", id: "claude-code", model: null, consent: true }), CLAUDE_CODE_NOT_OK);
  assert.equal(routeView(route({ available: false }), none).line, "Not available yet", "no line from the engine: still one plain line");
});

test("the other service's address is checked as the engine checks it (catalog::base_of)", () => {
  // The engine's own refusals, each for the reason it gives.
  for (const [bad, why] of [
    ["not an address", BASE_ERRORS.parse],
    ["", BASE_ERRORS.parse],
    ["ftp://x.example.test/v1", BASE_ERRORS.scheme],
    ["http://x.example.test/v1", BASE_ERRORS.scheme],
    ["localhost:8000/v1", BASE_ERRORS.scheme],
    ["https://user:pw@x.example.test/v1", BASE_ERRORS.extra],
    ["https://user@x.example.test/v1", BASE_ERRORS.extra],
    ["https://:pw@x.example.test/v1", BASE_ERRORS.extra],
    ["https://x.example.test/v1?key=abc", BASE_ERRORS.extra],
    ["https://x.example.test/v1?", BASE_ERRORS.extra],
    ["https://x.example.test/v1#key", BASE_ERRORS.extra],
    ["http://user:pw@127.0.0.1:8000/v1", BASE_ERRORS.extra],
  ])
    assert.deepEqual(baseOf(bad), { why }, bad);
  // Taken: https anywhere, http only to this computer; the address as the engine keeps it, with no trailing slash.
  for (const [good, base] of [
    ["https://llm.example.test/v1/", "https://llm.example.test/v1"],
    ["  https://LLM.Example.test  ", "https://llm.example.test"],
    ["https://llm.example.test:8443/v1//", "https://llm.example.test:8443/v1"],
    ["http://127.0.0.1:8000/v1", "http://127.0.0.1:8000/v1"],
    ["http://127.5.6.7/v1", "http://127.5.6.7/v1"],
    ["http://localhost:1234/v1/", "http://localhost:1234/v1"],
    ["http://[::1]:8000/v1", "http://[::1]:8000/v1"],
  ])
    assert.deepEqual(baseOf(good), { base }, good);
  assert.deepEqual([isHere("http://127.0.0.1:8000/v1"), isHere("http://localhost/v1"), isHere("http://[::1]/v1")], [true, true, true]);
  assert.deepEqual([isHere("https://llm.example.test/v1"), isHere("http://127.0.0.1.example.test/v1"), isHere("http://0.0.0.0/v1"), isHere("nope")], [false, false, false, false]);
  assert.deepEqual([hostOf("https://llm.example.test/v1"), hostOf("http://[::1]:8000/v1"), hostOf("nope")], ["llm.example.test", "[::1]", ""]);
});

test("the other service's row asks for an address first, then says where the words go", () => {
  const a = previewAssist("fresh");
  const fresh = routeView(byId(a, CUSTOM), a);
  assert.deepEqual([fresh.name, fresh.line, fresh.action, fresh.label, fresh.forget], ["Other OpenAI-compatible", "No address yet", "add_key", "Set up", false]);
  const custom = (o) => routeView(route({ id: CUSTOM, kind: "key", name: "Other OpenAI-compatible", ...o }), none);
  const at = "https://llm.example.test/v1";
  assert.deepEqual([custom({ state: "ready", base: at, consented: true }).line, custom({ state: "ready", base: at }).action], ["Key saved for llm.example.test", "use"]);
  assert.deepEqual([custom({ state: "no_key", base: at, consented: true }).line, custom({ state: "no_key", base: at }).label], ["No key yet for llm.example.test", "Add key"]);
  const here = custom({ state: "no_key", base: "http://127.0.0.1:8000/v1", offline: true, consented: true });
  assert.deepEqual([here.line, here.action], ["At 127.0.0.1, no key needed", "use"], "this computer needs no key");
  assert.equal(custom({ state: "ready" }).label, "Set up", "a key saved but no address: the address comes first");
  assert.equal(custom({ state: "unsupported", line: "Keys are not supported on Linux yet. A model on this computer needs no key." }).action, null);
});

test("the other service's form needs an address, a model and a key unless this computer; a new address asks again", () => {
  const f = (o) => ({ address: "https://llm.example.test/v1/", model: " m ", key: true, ...o });
  assert.deepEqual(customCheck({ state: "no_key" }, f()), { base: "https://llm.example.test/v1", model: "m" });
  assert.deepEqual(customCheck({ state: "no_key" }, f({ address: "http://llm.example.test" })), { why: BASE_ERRORS.scheme });
  assert.deepEqual(customCheck({ state: "no_key" }, f({ model: "  " })), { why: "Type the name of a model too." });
  assert.deepEqual(customCheck({ state: "no_key" }, f({ key: false })), { why: "Paste the service's key too." });
  assert.deepEqual(customCheck({ state: "ready" }, f({ key: false })), { base: "https://llm.example.test/v1", model: "m" }, "a saved key will do");
  assert.deepEqual(customCheck({ state: "no_key" }, f({ key: false, address: "http://127.0.0.1:8000/v1" })), { base: "http://127.0.0.1:8000/v1", model: "m" });
  const agreed = { consented: true, base: "https://llm.example.test/v1" };
  assert.equal(needsConsentAt(agreed, "https://llm.example.test/v1"), false, "the address agreed to");
  assert.equal(needsConsentAt(agreed, "https://other.example.test/v1"), true, "a changed address is a new destination");
  assert.equal(needsConsentAt({ consented: false, base: "" }, "https://llm.example.test/v1"), true);
});

test("the preview takes an address as the engine does: checked, agreed to once, and asked again when it changes", () => {
  let a = previewApply(previewAssist("fresh"), { cmd: "key_set", id: CUSTOM });
  const choose = (o) => previewApply(a, { cmd: "choose", id: CUSTOM, model: "m", consent: false, ...o });
  assert.equal(choose({ base: undefined }), "Type the address of the service first.");
  assert.equal(choose({ base: "http://llm.example.test/v1", consent: true }), BASE_ERRORS.scheme);
  assert.equal(choose({ base: "https://llm.example.test/v1" }), "Allow Other OpenAI-compatible in Settings first");
  assert.equal(choose({ base: "https://llm.example.test/v1", model: null, consent: true }), "Choose a model for Other OpenAI-compatible first.");
  a = choose({ base: "https://llm.example.test/v1/", consent: true });
  assert.deepEqual([a.chosen, a.model, byId(a, CUSTOM).base, byId(a, CUSTOM).goes_to], [CUSTOM, "m", "https://llm.example.test/v1", "llm.example.test"]);
  assert.equal(nowView(a).line, "Your words and pictures go to llm.example.test, with your key");
  assert.equal(typeof choose({ base: "https://llm.example.test/v1" }), "object", "the same address needs no new yes");
  assert.equal(typeof choose({ base: undefined, model: "n" }), "object", "no address sent: the one agreed to");
  assert.equal(choose({ base: "https://other.example.test/v1" }), "Allow Other OpenAI-compatible in Settings first");
  // On this computer it needs no key and stays here, so "Only on this computer" allows it.
  a = previewApply({ ...a, offline_only: true }, { cmd: "choose", id: CUSTOM, model: "m", consent: true, base: "http://127.0.0.1:8000/v1" });
  assert.deepEqual([a.chosen, byId(a, CUSTOM).offline, nowView(a).line], [CUSTOM, true, "Stays on this computer"]);
  a = previewApply(a, { cmd: "forget", id: CUSTOM });
  assert.deepEqual([a.chosen, byId(a, CUSTOM).base, byId(a, CUSTOM).consented, byId(a, CUSTOM).state], [null, "", false, "no_key"]);
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
  assert.deepEqual(nowView(previewAssist("codex-ready")), { title: "ChatGPT", line: "Your words and pictures go to OpenAI on your ChatGPT plan, through Codex on this computer" });
  // The engine names the plan as a way through too: the line says "through" once.
  const viaPlan = { id: "codex", kind: "plan", goes_to: "OpenAI, through your ChatGPT plan", offline: false };
  assert.equal(goesLine(viaPlan), "Your words and pictures go to OpenAI on your ChatGPT plan, through Codex on this computer");
  assert.equal(goesLine(viaPlan).split("through").length - 1, 1);
  assert.equal(nowView(previewAssist("key")).line, "Your words and pictures go to api.openai.com, with your key");
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
  assert.equal(
    consentText(route({ adds: ["AGENTS.md"] })).body,
    "Ask sends your words and the picture you take to OpenAI, on your account. Nothing goes to Pointcast. You see each picture before it leaves. Codex also sends your Codex instructions file (AGENTS.md).",
    "what Codex always adds is said, in one more sentence",
  );
  assert.doesNotMatch(consentText(route()).body, /also sends/, "only when the engine reports it");
  assert.match(consentText(previewAssist("codex-signed-out").providers[0]).body, /AGENTS\.md/, "the fixture reports it for Codex");
  assert.equal(consentText(route({ kind: "local", id: "ollama", offline: true, name: "Ollama on this computer", adds: ["x"] })).title, "Use Ollama for Ask?");
  assert.equal(needsConsent(route()), true);
  assert.equal(needsConsent(route({ consented: true })), false);
});

test("a route starts on a model that can see pictures, and one that cannot is marked", () => {
  const models = [
    { id: "a", name: "A", images: false },
    { id: "b", name: "B", images: true },
  ];
  assert.equal(defaultModel({ models }), "b");
  assert.equal(defaultModel({ models: [...models, { id: "c", name: "C", images: true, default: true }] }), "c", "the route's own default first");
  assert.equal(defaultModel({ models: [models[0]] }), "a");
  assert.equal(defaultModel({ models: [] }), null);
  assert.equal(modelLabel(models[0]), "A (words only)");
  assert.equal(modelLabel(models[1]), "B");
});

test("a reply of any shape reads as a whole one", () => {
  assert.deepEqual(normalizeAssist(null), { chosen: null, model: null, effort: null, offline_only: false, providers: [] });
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
  // The engine's new fields: available unless it says not, and the agreed address (null when none) as a string.
  const b = normalizeAssist({
    providers: [
      { id: "claude-code", kind: "agent", available: false, base: null },
      { id: CUSTOM, kind: "key", available: true, base: " https://llm.example.test/v1 " },
      { id: "old", kind: "key" },
    ],
  });
  assert.deepEqual(b.providers.map((p) => [p.id, p.kind, p.available, p.base]), [
    ["claude-code", "agent", false, ""],
    [CUSTOM, "key", true, "https://llm.example.test/v1"],
    ["old", "key", true, ""],
  ]);
});

test("every preview kind is a whole reply, and the preview commands change it as the engine would", () => {
  for (const k of PREVIEW_KINDS) assert.deepEqual(normalizeAssist(previewAssist(k)), previewAssist(k), k);
  let a = previewAssist("codex-signed-out");
  a = previewApply(a, { cmd: "signed_in", id: "codex" });
  assert.equal(a.providers[0].state, "ready");
  assert.equal(previewApply(a, { cmd: "choose", id: "codex", model: null, consent: false }), "Allow ChatGPT in Settings first");
  a = previewApply(a, { cmd: "choose", id: "codex", model: null, consent: true });
  assert.equal(a.chosen, "codex");
  assert.equal(a.model, "gpt-6-luna", "the plan's own default");
  assert.equal(a.providers[0].consented, true);
  a = previewApply(a, { cmd: "forget", id: "codex" });
  assert.equal(a.chosen, null);
  assert.equal(a.providers[0].consented, false);
  a = previewApply(a, { cmd: "key_set", id: "openai" });
  assert.equal(routeView(byId(a, "openai"), a).line, "Key saved");
  a = previewApply(a, { cmd: "forget", id: "openai" });
  assert.equal(byId(a, "openai").state, "no_key", "forget clears the key too");
});

test("the first answer draws every route at once: a route being checked says so and offers nothing until its probe answers", () => {
  const filled = previewAssist("codex-ready");
  const first = checkingAssist(filled);
  assert.equal(first.chosen, "codex", "the choice is known at once");
  assert.equal(first.model, "gpt-6-luna");
  for (const p of first.providers) {
    if (p.kind === "key" || !p.available) assert.notEqual(p.state, "checking", `${p.id}: known at once`);
    else assert.equal(p.state, "checking", p.id);
  }
  const v = routeView(byId(first, "codex"), first);
  assert.deepEqual([v.line, v.action, v.label, v.checking, v.chosen], ["Checking…", null, null, true, true]);
  assert.equal(nowView(first).title, "ChatGPT", "the chosen AI heads the pane before its probe answers");
  assert.equal(modelName(first, byId(first, "codex")), "GPT-6-Luna", "the chosen model is named before its list comes");
  assert.equal(normalizeAssist({ providers: [{ id: "x", kind: "agent", state: "checking" }] }).providers[0].state, "checking");
});

test("the ChatGPT row says whom Codex is signed in as and on which plan, and points to Codex's own sign-out", () => {
  const p = route({ state: "ready", plan: "plus", email: "someone@example.test" });
  const v = routeView(p, none);
  assert.equal(v.line, "Signed in as someone@example.test");
  assert.deepEqual(v.account, { who: "someone@example.test", plan: "Plus" });
  assert.deepEqual(routeView(route({ state: "ready", plan: "free" }), none).account, { who: null, plan: "Free" });
  assert.equal(routeView(route({ state: "signed_out", email: "someone@example.test" }), none).account, null, "nobody named once signed out");
  assert.equal(routeView(route({ id: "openai", kind: "key", state: "ready" }), none).account, null);
  assert.equal(normalizeAssist({ providers: [{ id: "codex", kind: "plan", email: " a@b.test " }] }).providers[0].email, "a@b.test");
  assert.equal(CODEX_SIGN_OUT, "codex logout");
});

test("a model is named in plain words, from its list when there is one", () => {
  assert.equal(modelName({ model: "gpt-6-luna" }, null), "GPT-6-Luna");
  assert.equal(modelName({ model: "m1" }, { models: [{ id: "m1", name: "Model One", images: true }] }), "Model One");
  assert.equal(modelName({ model: null }, { models: [{ id: "a", name: "A", images: true, default: true }] }), "A");
  assert.equal(modelName({ model: null }, null), null);
});

test("Only on this computer flips at once and says what it did in one quiet line", () => {
  const a = previewAssist("codex-ready");
  const on = withOffline(a, true);
  assert.equal(on.offline_only, true);
  assert.ok(visibleRoutes(on).every((p) => p.offline), "only this computer's AIs are listed at once");
  assert.equal(chosenRoute(on), null, "ChatGPT is off for now, before the engine answers");
  assert.equal(a.offline_only, false, "the list it came from is untouched");
  assert.equal(withOffline(on, false).providers.length, visibleRoutes(withOffline(on, false)).length);
  assert.equal(offlineSaid(true), "On. Only AIs on this computer are shown and asked.");
  assert.equal(offlineSaid(false), "Off. Every AI is shown again.");
});

test("the effort control shows only for a route that offers one, starting where the route starts", () => {
  let a = previewAssist("codex-ready");
  const codex = byId(a, "codex");
  assert.deepEqual(effortView(a, codex).options.map(([v]) => v), ["low", "medium", "high", "xhigh", "max", "ultra"], "no model named: the route's own list");
  assert.equal(effortView(a, codex).value, "low");
  a = previewApply(a, { cmd: "effort", effort: "high" });
  assert.equal(effortView(a, codex).value, "high");
  assert.equal(typeof previewApply(a, { cmd: "effort", effort: "extreme" }), "string");
  const key = previewAssist("key");
  assert.equal(effortView(key, chosenRoute(key)), null, "a key route offers no effort: no control");
  assert.equal(effortView(a, null), null);
  assert.deepEqual(normalizeAssist({ effort: "medium", providers: [{ id: "codex", kind: "plan", efforts: ["low", 3, "high"] }] }).providers[0].efforts, ["low", "high"]);
});

test("the Thinking menu lists what the chosen model offers, in words, and keeps a saved level only where it is offered", () => {
  const a = previewAssist();
  const codex = { efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] };
  const luna = { efforts: ["low", "medium", "high", "xhigh", "max"] };
  const terra = { efforts: ["low", "medium", "high", "xhigh", "max", "ultra"] };
  assert.deepEqual(effortView(a, codex, luna).options, [["low", "Low"], ["medium", "Medium"], ["high", "High"], ["xhigh", "Extra high"], ["max", "Max"]]);
  assert.deepEqual(effortView({ effort: "ultra" }, codex, terra).value, "ultra");
  assert.equal(effortView({ effort: "ultra" }, codex, luna).value, "low", "Luna has no Ultra: the menu shows where it starts");
  assert.deepEqual(effortView({ effort: null }, codex, { efforts: [] }).options.length, 6, "a model that names none: the route's list");
  assert.deepEqual(normalizeModels([{ id: "m", efforts: ["low", 2, "max"] }])[0].efforts, ["low", "max"]);
  assert.equal("efforts" in normalizeModels([{ id: "m" }])[0], false);
});
