// Choosing the AI that answers Ask (Settings › AI). Pure: what each route says and offers in each state, the grouping,
// the offline filter, where the words go, and the consent sheet's words, so node --test reads it without a window
// (src/lib/assist.test.mjs). The engine's command names live only in the adapter in api.ts.

/** How a route is paid for: the ChatGPT plan through Codex, another subscription through its vendor's own agent, a
 * pasted key, or nothing (a model on this computer). */
export type RouteKind = "plan" | "agent" | "key" | "local";
/** `checking`: its probe has not answered yet; the pane draws at once and the route fills in when it does. */
export type RouteState = "ready" | "signed_out" | "not_installed" | "too_old" | "no_key" | "not_running" | "unsupported" | "checking";
/** One option of a picker: the id the AI is sent, the words shown, and whether it is the one the AI recommends. */
export type Choice = { id: string; label: string; default?: boolean };
/** `default`: the model the route itself starts with. `efforts` and `speeds`: what this model offers, each its own. */
export type AssistModel = { id: string; name: string; images: boolean; default?: boolean; efforts?: Choice[]; speeds?: Choice[] };
/** What was last chosen for one route, kept when another AI is chosen. */
export type RoutePick = { model?: string; effort?: string; speed?: string };

/** One way to an AI, as `assist_providers` lists it. */
export type Route = {
  id: string;
  name: string;
  kind: RouteKind;
  state: RouteState;
  /** The engine's own line: shown only where it carries what the page cannot know (a version, a reason). */
  line: string;
  /** Who receives the words: "OpenAI", "OpenRouter", "Google". */
  goes_to: string;
  /** Nothing leaves this computer. */
  offline: boolean;
  /** It can be chosen now (Claude through Claude Code waits for the owner's OK). */
  available: boolean;
  /** The address the person agreed to, for the route that takes one (`custom`); empty otherwise. */
  base: string;
  /** The person allowed this AI and destination once. */
  consented: boolean;
  models: AssistModel[];
  /** The thinking efforts it offers a model that names none of its own, lowest first; empty where it offers no choice. */
  efforts: Choice[];
  /** A model id the person types is taken as it is (Claude Code), besides those listed. */
  typed: boolean;
  pick: RoutePick;
  /** The ChatGPT plan Codex is signed in with ("free", "plus"); null when not said. Read from `plan` or `planType`. */
  plan: string | null;
  /** The address Codex's own account/read says the person is signed in with; null when not said. Shown, never kept. */
  email: string | null;
  /** Files the route always sends along with each question (Codex: the person's own AGENTS.md); empty when none. */
  adds: string[];
  /** Its readiness is known (signed in as its own app says, a key saved, a server answering), not assumed. */
  sure: boolean;
  /** It can see and act when "Let the AI see and act" is on (Claude, Codex, and an agent that can carry a server). */
  acts: boolean;
};

/** The reply of `assist_providers`. */
export type Assist = { chosen: string | null; model: string | null; effort: string | null; speed?: string | null; offline_only: boolean; providers: Route[]; checking?: boolean };

/** The one action a route offers in its state. */
/** `install` and `update` are done by Pointcast itself (the vendor's own package); `get` only shows where to get an app
 * Pointcast cannot install (Ollama, LM Studio). */
export type RouteAction = "use" | "sign_in" | "add_key" | "get" | "install" | "update" | "check" | null;

export type RouteView = {
  id: string;
  name: string;
  line: string;
  action: RouteAction;
  label: string | null;
  /** This route answers Ask now. */
  chosen: boolean;
  /** Something to forget: a key, a consent, or the choice. */
  forget: boolean;
  /** What "How to get it" opens: a sentence and, for Codex, the official one-line install. */
  help: Help | null;
  /** Signed in through the vendor's own app: whom as (the address, when it says) and on which plan ("Plus"). */
  account: { who: string | null; plan: string | null } | null;
  /** The route is being checked: its line says so and it offers nothing yet. */
  checking: boolean;
  /** An installed agent that cannot say whether it is signed in: a quiet Sign in link sits under its line. */
  signIn?: boolean;
};
/** How to get a missing app: a sentence, the vendor's own install line (Copy), its official page, and how to set up
 * the account through the vendor's own sign-in; then Refresh. Pointcast runs none of it. */
export type Help = { text: string; command: string | null; page?: string; account?: string };

/** Each agent's official install line, page and own sign-in, as its vendor publishes them. */
export const AGENT_INSTALL: Record<string, { command: string; page: string; account: string }> = {
  "gemini-cli": { command: "npm install -g @google/gemini-cli", page: "https://github.com/google-gemini/gemini-cli", account: "Then choose Sign in: Gemini opens Google's own sign-in." },
  copilot: { command: "npm install -g @github/copilot", page: "https://github.com/github/copilot-cli", account: "Then choose Sign in: Copilot opens GitHub's own sign-in." },
  opencode: { command: "npm install -g opencode-ai", page: "https://opencode.ai/docs", account: "Then choose Sign in and pick your AI." },
  "claude-code": { command: "npm install -g @anthropic-ai/claude-code", page: "https://docs.claude.com/en/docs/claude-code/setup", account: "Then choose Sign in: Claude opens its own sign-in." },
};

/** The page's groups. What is ready on this computer comes first; every other route sits under the way it is set up,
 * with its one next step (install, sign in, add a key, start it). No group names an account: a fresh computer has no
 * "plan", only subscriptions the person may bring. */
export type GroupId = "ready" | "subscription" | "key" | "local";
export const GROUPS: readonly (readonly [GroupId, string])[] = [
  ["ready", "Ready to use"],
  ["subscription", "Your subscriptions"],
  ["key", "With a key"],
  ["local", "On this computer"],
];

/** The group a route is drawn in: ready ones together, the rest by how they are set up (a ChatGPT plan through Codex
 * is a subscription like Claude's or Gemini's). */
export const groupOf = (p: Pick<Route, "kind" | "state" | "available" | "sure">): GroupId =>
  p.available && p.state === "ready" && p.sure ? "ready" : p.kind === "plan" || p.kind === "agent" ? "subscription" : p.kind;

/** OpenAI's own install line for its Codex CLI (the same line updates it). */
export const CODEX_INSTALL = "npm i -g @openai/codex";

/** The route whose address the person types: any service that answers like OpenAI's chat API. */
export const CUSTOM = "custom";

const KINDS: RouteKind[] = ["plan", "agent", "key", "local"];
const STATES: RouteState[] = ["ready", "signed_out", "not_installed", "too_old", "no_key", "not_running", "unsupported", "checking"];
const str = (v: unknown) => (typeof v === "string" ? v : "");

/** A reply with every field in its type, whatever arrived; a route without an id or a known kind is dropped. */
export function normalizeAssist(x: unknown): Assist {
  const v = (x && typeof x === "object" ? x : {}) as Partial<Assist>;
  const providers = (Array.isArray(v.providers) ? v.providers : [])
    .filter((p): p is Route => !!p && typeof p === "object" && !!str(p.id) && KINDS.includes(p.kind))
    .map((p) => ({
      id: p.id,
      name: str(p.name) || p.id,
      kind: p.kind,
      state: STATES.includes(p.state) ? p.state : "unsupported",
      line: str(p.line).trim(),
      goes_to: str(p.goes_to),
      offline: p.offline === true || p.kind === "local",
      available: p.available !== false,
      base: str(p.base).trim(),
      consented: p.consented === true,
      models: normalizeModels(p.models),
      efforts: choices(p.efforts),
      typed: p.typed === true,
      pick: pickOf(p.pick),
      plan: str(p.plan) || str((p as { planType?: unknown }).planType) || null,
      email: str(p.email).trim() || null,
      adds: (Array.isArray(p.adds) ? p.adds : []).filter((f) => typeof f === "string" && f.trim()),
      sure: p.sure === true,
      // An older engine says nothing: only Claude acted then.
      acts: typeof p.acts === "boolean" ? p.acts : str(p.id) === "claude-code",
    }));
  const chosen = str(v.chosen) && providers.some((p) => p.id === v.chosen) ? (v.chosen as string) : null;
  return { chosen, model: str(v.model) || null, effort: str(v.effort) || null, speed: str(v.speed) || null, offline_only: v.offline_only === true, providers, ...(v.checking === true ? { checking: true } : {}) };
}

/** A picker's options, whatever arrived: each with an id, its label (an effort's name when none came) and its mark.
 * A bare name, as older engines sent efforts, is an option too. */
export function choices(x: unknown): Choice[] {
  return (Array.isArray(x) ? x : [])
    .map((c): Choice | null => {
      if (typeof c === "string") return c ? { id: c, label: effortWord(c) } : null;
      if (!c || typeof c !== "object" || !str(c.id)) return null;
      return { id: c.id, label: str(c.label) || effortWord(c.id), ...(c.default === true ? { default: true } : {}) };
    })
    .filter((c): c is Choice => !!c);
}

const pickOf = (x: unknown): RoutePick => {
  const v = (x && typeof x === "object" ? x : {}) as Record<string, unknown>;
  const out: RoutePick = {};
  for (const k of ["model", "effort", "speed"] as const) if (str(v[k])) out[k] = v[k] as string;
  return out;
};

export function normalizeModels(x: unknown): AssistModel[] {
  return (Array.isArray(x) ? x : [])
    .filter((m) => m && typeof m === "object" && str(m.id))
    .map((m) => {
      const efforts = choices(m.efforts);
      const speeds = choices(m.speeds);
      return {
        id: m.id,
        name: str(m.name) || m.id,
        images: m.images === true,
        ...(m.default === true ? { default: true } : {}),
        ...(efforts.length ? { efforts } : {}),
        ...(speeds.length ? { speeds } : {}),
      };
    });
}

/** The routes the list shows: only those on this computer while "Only on this computer" is on. */
export function visibleRoutes(a: Assist): Route[] {
  return a.offline_only ? a.providers.filter((p) => p.offline) : a.providers;
}

/** The route answering Ask now, or null: nothing chosen, a route that cannot be chosen yet, or one that leaves the
 * computer while only this computer is allowed. */
export function chosenRoute(a: Assist): Route | null {
  const p = a.providers.find((r) => r.id === a.chosen) ?? null;
  return p && p.available && (!a.offline_only || p.offline) ? p : null;
}

/** "Only on this computer", flipped at once in the pane before the engine confirms it. */
export const withOffline = (a: Assist, on: boolean): Assist => ({ ...a, offline_only: on });

/** What the switch did, in one quiet line under it. */
export const offlineSaid = (on: boolean) => (on ? "On. Only AIs on this computer are shown and asked." : "Off. Every AI is shown again.");

/** Ask can answer: an AI is chosen and allowed. Home shows the Ask key only then. */
export const askReady = (a: Assist) => chosenRoute(a) != null;

/** The list as it is drawn: the non-empty groups in order, each route with its line and its one action. */
export function groups(a: Assist): { kind: GroupId; title: string; routes: RouteView[] }[] {
  const shown = visibleRoutes(a);
  return GROUPS.map(([kind, title]) => ({ kind, title, routes: shown.filter((p) => groupOf(p) === kind).map((p) => routeView(p, a)) })).filter(
    (g) => g.routes.length,
  );
}

const app = (p: Pick<Route, "id" | "name">) => (p.id === "lmstudio" ? "LM Studio" : p.id === "ollama" ? "Ollama" : p.id === "codex" ? "Codex" : p.name);

/** A route's name as the page says it: "Ollama on this computer" is "Ollama" under This computer, where the rest goes
 * without saying. */
export const shortName = (p: Pick<Route, "name">) => p.name.replace(/ on this computer$/i, "") || p.name;

/** The line under "Let the AI see and act": what acting means with the AI chosen, or that it can't act yet. */
export function actLine(r: Pick<Route, "name" | "acts"> | null): string {
  const who = r ? shortName(r) : "The AI";
  return r && !r.acts
    ? `${who} can't act yet.`
    : `${who} clicks and types in apps you allow, while you watch. It asks before anything that can't be undone. Esc stops it.`;
}

/** A ChatGPT plan in plain words: "free" is "Free", "plus" is "Plus". */
const planName = (plan: string) => plan.charAt(0).toUpperCase() + plan.slice(1).toLowerCase();

/** A signed-in plan: "Signed in as someone@example.com" when Codex names the address, else "ChatGPT Free, through
 * Codex". */
function planLine(p: Route): string {
  if (p.email) return `Signed in as ${p.email}`;
  if (!p.plan) return "Signed in";
  const named = `${p.name} ${planName(p.plan)}`;
  return p.id === "codex" ? `${named}, through Codex` : named;
}

/** A route's one line in plain words, and the one action that fits its state. A route that cannot be chosen yet says
 * why and offers nothing; an agent's line is the engine's own, which already ends in its one next step. */
export function routeView(p: Route, a: Pick<Assist, "chosen" | "offline_only">): RouteView {
  const chosen = a.chosen === p.id && p.available && (!a.offline_only || p.offline);
  // Who is signed in, where the app says: ChatGPT through Codex, and Claude through Claude Code.
  const account = (p.kind === "plan" || p.id === "claude-code") && p.state === "ready" && (p.plan || p.email) ? { who: p.email ?? null, plan: p.plan ? planName(p.plan) : null } : null;
  const base = {
    id: p.id,
    name: shortName(p),
    chosen,
    forget: chosen || p.consented || (p.kind === "key" && p.state !== "no_key"),
    help: null,
    account,
    checking: false,
  };
  const use = (line: string): RouteView => ({ ...base, line, action: chosen ? null : "use", label: chosen ? null : "Use" });
  if (!p.available) return { ...base, line: p.line || "Not available yet", action: null, label: null };
  if (p.state === "checking") return { ...base, line: "Checking…", action: null, label: null, checking: true };
  if (p.kind === "agent") {
    const v = stateView(p, base, use);
    const line = p.line || v.line;
    if (p.state === "not_installed" && AGENT_INSTALL[p.id]) return { ...base, line, action: "install", label: "Install", help: installHelp(p) };
    if (p.state === "signed_out") return { ...base, line, action: "sign_in", label: "Sign in", help: installHelp(p) };
    if (p.state === "too_old") return { ...base, line, action: "update", label: "Update", help: installHelp(p, true) };
    if (p.state === "ready" && p.email) return { ...v, line: `Signed in as ${p.email}` };
    if (p.state === "ready") return { ...v, line, signIn: p.id !== "claude-code" && !!AGENT_INSTALL[p.id] };
    return p.state === "unsupported" ? { ...v, line } : { ...base, line, action: "check", label: "Check again" };
  }
  if (p.id === CUSTOM && p.state !== "unsupported") {
    if (!p.base) return { ...base, line: "No address yet", action: "add_key", label: "Set up" };
    const host = hostOf(p.base);
    if (p.state === "ready") return use(`Key saved for ${host}`);
    if (p.offline) return use(`At ${host}, no key needed`);
    return { ...base, line: `No key yet for ${host}`, action: "add_key", label: "Add key" };
  }
  return stateView(p, base, use);
}

function stateView(p: Route, base: Omit<RouteView, "line" | "action" | "label">, use: (line: string) => RouteView): RouteView {
  switch (p.state) {
    case "ready":
      return use(p.kind === "plan" ? planLine(p) : p.kind === "key" ? "Key saved" : "Running, free");
    case "signed_out":
      return { ...base, line: "Not signed in", action: "sign_in", label: "Sign in" };
    case "no_key":
      return { ...base, line: "No key yet", action: "add_key", label: "Add key" };
    case "not_running":
      return {
        ...base,
        line: p.id === "lmstudio" ? "Start LM Studio's server, then check again" : `Start ${app(p)}, then check again`,
        action: "check",
        label: "Check again",
      };
    case "not_installed":
      return { ...base, line: `${app(p)} is not on this computer`, action: p.id === "codex" ? "install" : "get", label: p.id === "codex" ? "Install" : "Download", help: installHelp(p) };
    case "too_old":
      return { ...base, line: p.line || `This ${app(p)} is too old`, action: p.id === "codex" ? "update" : "get", label: "Update", help: installHelp(p, true) };
    case "unsupported":
      return { ...base, line: p.line || "Not available on this computer", action: null, label: null };
    case "checking":
      return { ...base, line: "Checking…", action: null, label: null, checking: true };
  }
}

/** Codex's own way to sign out, said as a pointer: Pointcast never signs anyone out of another app. */
export const CODEX_SIGN_OUT = "codex logout";

/** The first answer while the probes are out: every route a probe finds says it is being checked, with nothing
 * offered yet; keys, the choice and the switch are known at once. */
export function checkingAssist(a: Assist): Assist {
  return { ...a, providers: a.providers.map((p) => (p.kind === "key" || !p.available ? p : { ...p, state: "checking", line: "", models: [] })) };
}

/** A model id in plain words, for when its list has not come yet: "gpt-6-luna" is "GPT-6-Luna". */
export function modelName(a: Pick<Assist, "model">, p: Pick<Route, "models"> | null): string | null {
  if (!a.model) return p ? (p.models.find((m) => m.default)?.name ?? null) : null;
  const known = p?.models.find((m) => m.id === a.model);
  if (known) return known.name;
  return a.model
    .split("-")
    .map((w) => (/^gpt$/i.test(w) ? "GPT" : w.charAt(0).toUpperCase() + w.slice(1)))
    .join("-");
}

/** What "How to get it" shows: Codex's official install line, or where the app is published. */
export function installHelp(p: Pick<Route, "id" | "name">, update = false): Help {
  if (p.id === "codex")
    return {
      text: `${update ? "Update" : "Install"} Codex in a terminal, then check again. It needs Node.js.`,
      command: CODEX_INSTALL,
      page: "https://github.com/openai/codex",
      account: "Then come back and choose Sign in: Codex opens ChatGPT's own sign-in page.",
    };
  const agent = AGENT_INSTALL[p.id];
  if (agent) return { text: `Install ${p.name}'s own app in a terminal, then check again. It needs Node.js.`, ...agent };
  if (p.id === "ollama") return { text: `Get Ollama from ollama.com, start it, then check again.`, command: null, page: "https://ollama.com/download" };
  if (p.id === "lmstudio") return { text: "Get LM Studio from lmstudio.ai, start its server, then check again.", command: null, page: "https://lmstudio.ai/download" };
  return { text: `Install ${p.name}, then check again.`, command: null };
}

/** Where the words go, said with one "through": the engine's "OpenAI, through your ChatGPT plan" reads "OpenAI on
 * your ChatGPT plan", so Codex can be the one way through. */
export function destination(goesTo: string): string {
  return goesTo.trim().replace(/,?\s+through (your .+ plan)$/i, " on $1");
}

/** Where the words go, for the line under the chosen AI. A model that cannot see pictures takes words only. */
export function goesLine(p: Pick<Route, "id" | "kind" | "goes_to" | "offline">, images = true): string {
  if (p.offline) return "Stays on this computer";
  const what = images ? "Your words and pictures go" : "Your words go";
  const to = destination(p.goes_to) || "the AI";
  if (p.id === "codex") return `${what} to ${to}, through Codex on this computer`;
  if (p.kind === "agent") return `${what} to ${to}`;
  return p.kind === "plan" ? `${what} to ${to}, on your plan` : `${what} to ${to}, with your key`;
}

/** The top of the pane: which AI answers now and where the words go. */
export function nowView(a: Assist): { title: string; line: string } {
  const p = chosenRoute(a);
  if (!p) {
    const off = a.providers.find((r) => r.id === a.chosen);
    if (off && !off.available) return { title: "Nothing chosen", line: off.line || `${shortName(off)} is not available yet` };
    if (off) return { title: "Nothing chosen", line: `${off.name} sends words off this computer, so it is off for now` };
    return { title: "Nothing chosen", line: "Choose an AI below. Until then nothing leaves this computer." };
  }
  const m = p.models.find((x) => x.id === a.model);
  return { title: shortName(p), line: goesLine(p, m ? m.images : true) };
}

/** The consent sheet: once per AI and destination, before its first use. What a route always sends along (Codex's
 * AGENTS.md) is said in one more sentence, only when the engine reports it. */
export function consentText(p: Pick<Route, "id" | "name" | "goes_to" | "offline" | "adds">): { title: string; body: string } {
  const title = `Use ${shortName(p)} for Ask?`;
  if (p.offline) return { title, body: "Nothing leaves this computer." };
  const files = p.adds.join(", ");
  const also = !files ? "" : p.id === "codex" ? ` Codex also sends your Codex instructions file (${files}).` : ` ${app(p)} also sends ${files}.`;
  return {
    title,
    body: `Ask sends your words and the picture you take to ${p.goes_to || p.name}, on your account. Nothing goes to Pointcast. You see each picture before it leaves.${also}`,
  };
}

/** The model a route starts with when chosen: its own default, else the first that can see pictures, else the first. */
export function defaultModel(p: Pick<Route, "models">): string | null {
  return (p.models.find((m) => m.default) ?? p.models.find((m) => m.images) ?? p.models[0])?.id ?? null;
}

/** Effort names as the menu says them, for an engine that sent only the name. */
const EFFORT_WORDS: Record<string, string> = { xhigh: "Extra high", max: "Max", ultra: "Ultra" };
export const effortWord = (e: string) => EFFORT_WORDS[e] ?? e.charAt(0).toUpperCase() + e.slice(1);

/** One option's words: its label, and "(recommended)" on the one the AI recommends. */
export const choiceLabel = (c: Choice) => (c.default ? `${c.label} (recommended)` : c.label);

/** A picker drawn from its options alone, for every route alike: the one in use is `now` when offered, else the
 * recommended one, else the first; null when there is nothing to choose, which then shows no control. */
export function pickerView(options: Choice[], now: string | null | undefined): Picker | null {
  if (options.length < 2) return null;
  const value = options.find((c) => c.id === now)?.id ?? options.find((c) => c.default)?.id ?? options[0].id;
  // A segmented control keeps its words short: "recommended" is said once, under the row's label.
  return { value, options: options.map((c): [string, string] => [c.id, c.label]), recommended: options.find((c) => c.default)?.label ?? null };
}

export type Picker = { value: string; options: [string, string][]; recommended: string | null };

/** The Thinking control for the chosen AI: the efforts the chosen model offers (the route's own list when the model
 * names none, as for one typed in) and the one an Ask sends; null for a route that offers no choice. */
export function effortView(
  a: Pick<Assist, "effort">,
  p: Pick<Route, "efforts"> | null,
  model?: Pick<AssistModel, "efforts"> | null,
): Picker | null {
  if (!p) return null;
  return pickerView(model?.efforts?.length ? model.efforts : p.efforts, a.effort);
}

/** The Speed control: only where the chosen model offers speeds (Codex's service tiers); standard first. */
export function speedView(a: Pick<Assist, "speed">, model?: Pick<AssistModel, "speeds"> | null): Picker | null {
  return pickerView(model?.speeds ?? [], a.speed);
}

/** A model as the select lists it: the recommended one says so, and one that cannot see pictures says that. */
export const modelLabel = (m: AssistModel) => `${m.name}${m.default ? " (recommended)" : ""}${m.images ? "" : " (words only)"}`;

/** Whether a route's choice needs the consent sheet first. */
export const needsConsent = (p: Pick<Route, "consented">) => !p.consented;

// —— The other service (`custom`): the address the person types, checked here as the engine checks it
// (catalog::base_of), so a wrong one is said before anything is sent. ——

/** The engine's own words for a refused address. */
export const BASE_ERRORS = {
  parse: "That is not a web address.",
  extra: "Type only the address of the service, with no name, password or query in it.",
  scheme: "The address must start with https:// (http:// only for this computer).",
} as const;

/** This computer, as the engine reads a host (openai::loopback): localhost, 127.x.x.x or ::1. */
function loopback(u: URL): boolean {
  let h = u.hostname;
  while (h.startsWith("[")) h = h.slice(1);
  while (h.endsWith("]") || h.endsWith(".")) h = h.slice(0, -1);
  if (h.toLowerCase() === "localhost" || h === "::1") return true;
  const parts = h.split(".");
  return parts.length === 4 && parts.every((n) => n !== "" && String(Number(n)) === n && Number(n) < 256) && parts[0] === "127";
}

const parsed = (text: string): URL | null => {
  try {
    return new URL(text.trim());
  } catch {
    return null;
  }
};

/** The address typed for the other service: https, or http only to this computer, and nothing but the address (no
 * name, password, query or fragment, which could carry a key into the settings). Trailing slashes go, as the engine
 * drops them. */
export function baseOf(text: string): { base: string } | { why: string } {
  const u = parsed(text);
  if (!u) return { why: BASE_ERRORS.parse };
  // A query or fragment, even an empty one, keeps its ? or # in the address; neither can appear anywhere else in it.
  if (u.username || u.password || u.href.includes("?") || u.href.includes("#")) return { why: BASE_ERRORS.extra };
  if (!(u.protocol === "https:" || (u.protocol === "http:" && loopback(u)))) return { why: BASE_ERRORS.scheme };
  let base = u.href;
  while (base.endsWith("/")) base = base.slice(0, -1);
  return { base };
}

/** Nothing sent to this address leaves this computer. */
export const isHere = (base: string) => {
  const u = parsed(base);
  return !!u && loopback(u);
};

/** The host the words go to, as the engine names it. */
export const hostOf = (base: string) => parsed(base)?.hostname ?? "";

/** The other service's form, checked before anything is sent: an address the engine takes, a model's name, and a key
 * unless one is saved or the address is this computer. */
export function customCheck(
  p: Pick<Route, "state">,
  f: { address: string; model: string; key: boolean },
): { base: string; model: string } | { why: string } {
  const b = baseOf(f.address);
  if ("why" in b) return b;
  const model = f.model.trim();
  if (!model) return { why: "Type the name of a model too." };
  if (!f.key && p.state !== "ready" && !isHere(b.base)) return { why: "Paste the service's key too." };
  return { base: b.base, model };
}

/** An address the person has not agreed to is a new destination: the consent sheet opens again. */
export const needsConsentAt = (p: Pick<Route, "consented" | "base">, base: string) => !p.consented || p.base !== base;

// —— Browser preview fixtures (settings.html?preview=1&pane=ai&assist=…): data only; api.ts plays them. ——

export const PREVIEW_KINDS = ["fresh", "codex-signed-out", "codex-ready", "claude", "key", "local", "offline", "checking"] as const;

const levels = (ids: string[], start?: string): Choice[] => ids.map((id) => ({ id, label: effortWord(id), ...(id === start ? { default: true } : {}) }));

const MODELS: Record<string, AssistModel[]> = {
  // As Codex's model/list answered for a ChatGPT Free plan.
  codex: [
    { id: "gpt-6-luna", name: "GPT-6-Luna", images: true, default: true, efforts: levels(["low", "medium", "high", "xhigh", "max"], "medium") },
    { id: "gpt-5.6-terra", name: "GPT-5.6-Terra", images: true, efforts: levels(["low", "medium", "high", "xhigh", "max", "ultra"], "medium") },
    { id: "gpt-5.6-luna", name: "GPT-5.6-Luna", images: true, efforts: levels(["low", "medium", "high", "xhigh", "max"], "medium") },
  ],
  // Claude Code's own list: its aliases, then the full ids; Sonnet and medium recommended.
  "claude-code": [
    ["opus", "Opus"],
    ["sonnet", "Sonnet"],
    ["haiku", "Haiku"],
    ["claude-opus-5-5", "Claude Opus 5.5"],
    ["claude-sonnet-5-5", "Claude Sonnet 5.5"],
    ["claude-haiku-5-5", "Claude Haiku 5.5"],
  ].map(([id, name]) => ({ id, name, images: true, ...(id === "sonnet" ? { default: true } : {}), efforts: levels(["low", "medium", "high", "xhigh", "max"], "medium") })),
  openai: [
    { id: "gpt-5.5", name: "GPT-5.5", images: true },
    { id: "gpt-5.5-mini", name: "GPT-5.5 mini", images: true },
  ],
  openrouter: [
    { id: "anthropic/claude-sonnet", name: "Claude Sonnet", images: true },
    { id: "deepseek/deepseek-chat", name: "DeepSeek Chat", images: false },
  ],
  gemini: [{ id: "gemini-flash", name: "Gemini Flash", images: true }],
  anthropic: [{ id: "claude-sonnet", name: "Claude Sonnet", images: true }],
  xai: [{ id: "grok-4", name: "Grok 4", images: true }],
  ollama: [
    { id: "gemma3:12b", name: "gemma3:12b", images: true },
    { id: "qwen2.5:7b", name: "qwen2.5:7b", images: false },
  ],
  lmstudio: [],
};


/** The routes on a fresh computer, in the engine's order: Codex not installed; the Gemini CLI installed (the engine
 * cannot tell whether it is signed in), Copilot and OpenCode not installed, Claude Code signed in; no
 * keys and no address; Ollama installed but stopped, no LM Studio. */
export function previewAssist(kind: string): Assist {
  const r = (id: string, name: string, k: RouteKind, state: RouteState, goes_to: string, line = ""): Route => ({
    id,
    name,
    kind: k,
    state,
    line,
    goes_to,
    offline: k === "local",
    available: true,
    base: "",
    consented: false,
    models: state === "ready" ? MODELS[id] ?? [] : [],
    efforts: id === "codex" ? levels(["low", "medium", "high", "xhigh", "max", "ultra"]) : id === "claude-code" ? levels(["low", "medium", "high", "xhigh", "max"], "medium") : [],
    typed: id === "claude-code",
    pick: {},
    plan: null,
    email: null,
    adds: id === "codex" ? ["AGENTS.md"] : [],
    // An installed agent that cannot say whether anyone is signed in (Gemini here) is usable but not sure.
    sure: state === "ready" && (k !== "agent" || id === "claude-code"),
    acts: k === "plan" || k === "agent",
  });
  const missing = (cli: string) => `${cli} is not installed on this computer. Choose Install.`;
  const a: Assist = {
    chosen: null,
    model: null,
    effort: null,
    speed: null,
    offline_only: false,
    providers: [
      r("codex", "ChatGPT", "plan", "not_installed", "OpenAI, through your ChatGPT plan"),
      r("gemini-cli", "Gemini", "agent", "ready", "Google, through your Gemini CLI sign-in", "Gemini CLI 0.9.0 is installed."),
      r("copilot", "GitHub Copilot", "agent", "not_installed", "GitHub, through your Copilot plan", missing("GitHub Copilot CLI")),
      r("opencode", "OpenCode", "agent", "not_installed", "the AI you set up in OpenCode", missing("OpenCode")),
      r("claude-code", "Claude", "agent", "ready", "Anthropic, through your Claude plan", "Signed in to Claude Code."),
      r("openai", "OpenAI API", "key", "no_key", "api.openai.com"),
      r("anthropic", "Claude API", "key", "no_key", "api.anthropic.com"),
      r("xai", "Grok (xAI API)", "key", "no_key", "api.x.ai"),
      r("gemini", "Gemini API", "key", "no_key", "generativelanguage.googleapis.com"),
      r("groq", "Groq", "key", "no_key", "api.groq.com"),
      r("mistral", "Mistral", "key", "no_key", "api.mistral.ai"),
      r("deepseek", "DeepSeek", "key", "no_key", "api.deepseek.com"),
      r("openrouter", "OpenRouter", "key", "no_key", "openrouter.ai"),
      r(CUSTOM, "Other OpenAI-compatible", "key", "no_key", "the address you type", "Type the address of the service first."),
      r("ollama", "Ollama on this computer", "local", "not_running", "this computer"),
      r("lmstudio", "LM Studio on this computer", "local", "not_installed", "this computer"),
    ],
  };
  const set = (id: string, p: Partial<Route>) => {
    a.providers = a.providers.map((x) => (x.id === id ? { ...x, ...p, models: p.state === "ready" ? MODELS[id] ?? [] : x.models, sure: p.state ? p.state === "ready" : x.sure } : x));
  };
  const choose = (id: string) => {
    set(id, { state: "ready", consented: true });
    a.chosen = id;
    a.model = defaultModel(a.providers.find((x) => x.id === id)!);
  };
  if (kind === "codex-signed-out") set("codex", { state: "signed_out" });
  if (kind === "codex-ready" || kind === "checking") {
    choose("codex");
    set("codex", { plan: "plus", email: "you@example.com" });
  }
  if (kind === "claude") choose("claude-code");
  if (kind === "key") {
    set("codex", { state: "signed_out" });
    choose("openai");
  }
  if (kind === "local" || kind === "offline") {
    set("codex", { state: "signed_out" });
    choose("ollama");
  }
  if (kind === "offline") a.offline_only = true;
  return a;
}

/** The preview's answer to a command, as the engine would change the list. */
export function previewApply(
  a: Assist,
  c:
    | { cmd: "choose"; id: string; model: string | null; consent: boolean; base?: string }
    | { cmd: "forget" | "key_set" | "key_delete" | "signed_in"; id: string }
    | { cmd: "offline"; on: boolean }
    | { cmd: "effort"; effort: string }
    | { cmd: "speed"; speed: string },
): Assist | string {
  const find = (id: string) => a.providers.find((p) => p.id === id);
  const map = (id: string, f: (p: Route) => Route) => ({ ...a, providers: a.providers.map((p) => (p.id === id ? f(p) : p)) });
  switch (c.cmd) {
    case "offline":
      return { ...a, offline_only: c.on };
    case "effort": {
      const p = a.providers.find((x) => x.id === a.chosen);
      const m = p?.models.find((x) => x.id === a.model);
      if (!p || ![...p.efforts, ...(m?.efforts ?? [])].some((e) => e.id === c.effort)) return `${p?.name ?? "That AI"} offers no "${c.effort}" effort.`;
      return { ...map(p.id, (x) => ({ ...x, pick: { ...x.pick, effort: c.effort } })), effort: c.effort };
    }
    case "speed": {
      const m = a.providers.find((x) => x.id === a.chosen)?.models.find((x) => x.id === a.model);
      if (!m?.speeds?.some((t) => t.id === c.speed)) return "That AI offers no speed choice.";
      return { ...a, speed: c.speed === "standard" ? null : c.speed };
    }
    case "choose": {
      const p = find(c.id);
      if (!p) return "Pointcast does not offer that AI.";
      if (!p.available) return p.line;
      // The other service: a typed address is checked, and one not agreed to is a new destination.
      let at = p.base;
      if (p.id === CUSTOM && c.base !== undefined) {
        const b = baseOf(c.base);
        if ("why" in b) return b.why;
        at = b.base;
      }
      if (p.id === CUSTOM && !at) return "Type the address of the service first.";
      const here = p.id === CUSTOM ? isHere(at) : p.offline;
      if (p.state !== "ready" && !(p.id === CUSTOM && here)) return "That AI is not ready";
      if (a.offline_only && !here) return "Only this computer is allowed";
      const model = c.model ?? defaultModel(p);
      if ((p.kind === "key" || p.kind === "local") && !model) return `Choose a model for ${p.name} first.`;
      if (needsConsentAt(p, at) && !c.consent) return `Allow ${p.name} in Settings first`;
      const agreed = p.id === CUSTOM ? { base: at, goes_to: hostOf(at), offline: here } : {};
      return { ...map(c.id, (x) => ({ ...x, ...agreed, consented: true })), chosen: c.id, model };
    }
    case "forget": {
      const gone = (p: Route) => (p.id === CUSTOM ? { base: "", goes_to: "the address you type", offline: false } : {});
      const next = map(c.id, (p) => ({ ...p, ...gone(p), consented: false, ...(p.kind === "key" ? { state: "no_key" as const, models: [] } : {}) }));
      return a.chosen === c.id ? { ...next, chosen: null, model: null } : next;
    }
    case "key_set":
      return map(c.id, (p) => ({ ...p, state: "ready", models: MODELS[p.id] ?? [], sure: true }));
    case "key_delete":
      return { ...map(c.id, (p) => ({ ...p, state: "no_key", models: [], sure: false })), ...(a.chosen === c.id ? { chosen: null, model: null } : {}) };
    case "signed_in":
      return map(c.id, (p) => ({ ...p, state: "ready", models: MODELS[p.id] ?? [], plan: p.kind === "plan" ? "free" : p.plan, sure: true }));
  }
}

export const previewModels = (id: string): AssistModel[] => MODELS[id] ?? [];

/** The AIs onboarding offers, each with its one click: the ones ready on this computer first, then the ones a click
 * signs in or updates, then the ones a click installs; at most `max`. Keys, addresses and apps Pointcast cannot
 * install wait in Settings, so the step asks for nothing typed. */
export function onboardingChoices(a: Assist, max = 5): RouteView[] {
  const rank = (v: RouteView) => (v.chosen ? 0 : v.action === "use" ? 1 : v.action === "sign_in" || v.action === "update" ? 2 : v.action === "install" ? 3 : 9);
  return visibleRoutes(a)
    .filter((p) => p.available && p.kind !== "key" && p.id !== CUSTOM)
    .map((p) => routeView(p, a))
    .filter((v) => rank(v) < 9)
    .sort((x, y) => rank(x) - rank(y))
    .slice(0, max);
}
