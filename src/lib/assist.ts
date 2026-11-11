// Choosing the AI that answers Ask (Settings › AI). Pure: what each route says and offers in each state, the grouping,
// the offline filter, where the words go, and the consent sheet's words, so node --test reads it without a window
// (src/lib/assist.test.mjs). The engine's command names live only in the adapter in api.ts.

export type RouteKind = "plan" | "key" | "local";
export type RouteState = "ready" | "signed_out" | "not_installed" | "too_old" | "no_key" | "not_running" | "unsupported";
export type AssistModel = { id: string; name: string; images: boolean };

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
  /** The person allowed this AI and destination once. */
  consented: boolean;
  models: AssistModel[];
};

/** The reply of `assist_providers`. */
export type Assist = { chosen: string | null; model: string | null; offline_only: boolean; providers: Route[] };

/** The one action a route offers in its state. */
export type RouteAction = "use" | "sign_in" | "add_key" | "get" | "check" | null;

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
};
export type Help = { text: string; command: string | null };

export const GROUPS: readonly (readonly [RouteKind, string])[] = [
  ["plan", "Your plan"],
  ["key", "A key"],
  ["local", "This computer"],
];

/** OpenAI's own install line for its Codex CLI (the same line updates it). */
export const CODEX_INSTALL = "npm i -g @openai/codex";

const KINDS: RouteKind[] = ["plan", "key", "local"];
const STATES: RouteState[] = ["ready", "signed_out", "not_installed", "too_old", "no_key", "not_running", "unsupported"];
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
      consented: p.consented === true,
      models: normalizeModels(p.models),
    }));
  const chosen = str(v.chosen) && providers.some((p) => p.id === v.chosen) ? (v.chosen as string) : null;
  return { chosen, model: str(v.model) || null, offline_only: v.offline_only === true, providers };
}

export function normalizeModels(x: unknown): AssistModel[] {
  return (Array.isArray(x) ? x : [])
    .filter((m) => m && typeof m === "object" && str(m.id))
    .map((m) => ({ id: m.id, name: str(m.name) || m.id, images: m.images === true }));
}

/** The routes the list shows: only those on this computer while "Only on this computer" is on. */
export function visibleRoutes(a: Assist): Route[] {
  return a.offline_only ? a.providers.filter((p) => p.offline) : a.providers;
}

/** The route answering Ask now, or null: nothing chosen, or a route that leaves the computer while only this computer
 * is allowed. */
export function chosenRoute(a: Assist): Route | null {
  const p = a.providers.find((r) => r.id === a.chosen) ?? null;
  return p && (!a.offline_only || p.offline) ? p : null;
}

/** Ask can answer: an AI is chosen and allowed. Home shows the Ask key only then. */
export const askReady = (a: Assist) => chosenRoute(a) != null;

/** The list as it is drawn: the non-empty groups in order, each route with its line and its one action. */
export function groups(a: Assist): { kind: RouteKind; title: string; routes: RouteView[] }[] {
  const shown = visibleRoutes(a);
  return GROUPS.map(([kind, title]) => ({ kind, title, routes: shown.filter((p) => p.kind === kind).map((p) => routeView(p, a)) })).filter(
    (g) => g.routes.length,
  );
}

const app = (p: Route) => (p.id === "lmstudio" ? "LM Studio" : p.id === "ollama" ? "Ollama" : p.id === "codex" ? "Codex" : p.name);

/** A route's one line in plain words, and the one action that fits its state. */
export function routeView(p: Route, a: Pick<Assist, "chosen" | "offline_only">): RouteView {
  const chosen = a.chosen === p.id && (!a.offline_only || p.offline);
  const base = { id: p.id, name: p.name, chosen, forget: chosen || p.consented || (p.kind === "key" && p.state !== "no_key"), help: null };
  const use = (line: string): RouteView => ({ ...base, line, action: chosen ? null : "use", label: chosen ? null : "Use" });
  switch (p.state) {
    case "ready":
      return use(p.kind === "plan" ? "Signed in" : p.kind === "key" ? "Key saved" : "Running, free");
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
      return { ...base, line: `${app(p)} is not on this computer`, action: "get", label: "How to get it", help: installHelp(p) };
    case "too_old":
      return { ...base, line: p.line || `This ${app(p)} is too old`, action: "get", label: "How to update", help: installHelp(p, true) };
    case "unsupported":
      return { ...base, line: p.line || "Not available on this computer", action: null, label: null };
  }
}

/** What "How to get it" shows: Codex's official install line, or where the app is published. */
export function installHelp(p: Pick<Route, "id" | "name">, update = false): Help {
  if (p.id === "codex")
    return { text: `${update ? "Update" : "Install"} Codex in a terminal, then check again. It needs Node.js.`, command: CODEX_INSTALL };
  if (p.id === "ollama") return { text: `Get Ollama from ollama.com, start it, then check again.`, command: null };
  if (p.id === "lmstudio") return { text: "Get LM Studio from lmstudio.ai, start its server, then check again.", command: null };
  return { text: `Install ${p.name}, then check again.`, command: null };
}

/** Where the words go, for the line under the chosen AI. A model that cannot see pictures takes words only. */
export function goesLine(p: Pick<Route, "id" | "kind" | "goes_to" | "offline">, images = true): string {
  if (p.offline) return "Stays on this computer";
  const what = images ? "Your words and pictures go" : "Your words go";
  const to = p.goes_to || "the AI";
  if (p.id === "codex") return `${what} to ${to}, through Codex on this computer`;
  return p.kind === "plan" ? `${what} to ${to}, on your plan` : `${what} to ${to}, with your key`;
}

/** The top of the pane: which AI answers now and where the words go. */
export function nowView(a: Assist): { title: string; line: string } {
  const p = chosenRoute(a);
  if (!p) {
    const off = a.providers.find((r) => r.id === a.chosen);
    if (off) return { title: "Nothing chosen", line: `${off.name} sends words off this computer, so it is off for now` };
    return { title: "Nothing chosen", line: "Choose an AI below. Until then nothing leaves this computer." };
  }
  const m = p.models.find((x) => x.id === a.model);
  return { title: p.name, line: goesLine(p, m ? m.images : true) };
}

/** The consent sheet: once per AI and destination, before its first use. */
export function consentText(p: Pick<Route, "name" | "goes_to" | "offline">): { title: string; body: string } {
  const title = `Use ${p.name} for Ask?`;
  if (p.offline) return { title, body: "Nothing leaves this computer." };
  return {
    title,
    body: `Ask sends your words and the picture you take to ${p.goes_to || p.name}, on your account. Nothing goes to Pointcast. You see each picture before it leaves.`,
  };
}

/** The model a route starts with when chosen: the first that can see pictures, else the first. */
export function defaultModel(p: Pick<Route, "models">): string | null {
  return (p.models.find((m) => m.images) ?? p.models[0])?.id ?? null;
}

/** A model as the select lists it; one that cannot see pictures says so. */
export const modelLabel = (m: AssistModel) => (m.images ? m.name : `${m.name} (words only)`);

/** Whether a route's choice needs the consent sheet first. */
export const needsConsent = (p: Pick<Route, "consented">) => !p.consented;

// —— Browser preview fixtures (settings.html?preview=1&pane=ai&assist=…): data only; api.ts plays them. ——

export const PREVIEW_KINDS = ["fresh", "codex-signed-out", "codex-ready", "key", "local", "offline"] as const;

const MODELS: Record<string, AssistModel[]> = {
  codex: [
    { id: "gpt-5.5", name: "GPT-5.5", images: true },
    { id: "gpt-5.5-mini", name: "GPT-5.5 mini", images: true },
  ],
  openai: [
    { id: "gpt-5.5", name: "GPT-5.5", images: true },
    { id: "gpt-5.5-mini", name: "GPT-5.5 mini", images: true },
  ],
  openrouter: [
    { id: "anthropic/claude-sonnet", name: "Claude Sonnet", images: true },
    { id: "deepseek/deepseek-chat", name: "DeepSeek Chat", images: false },
  ],
  gemini: [{ id: "gemini-flash", name: "Gemini Flash", images: true }],
  ollama: [
    { id: "gemma3:12b", name: "gemma3:12b", images: true },
    { id: "qwen2.5:7b", name: "qwen2.5:7b", images: false },
  ],
  lmstudio: [],
};

/** The routes on a fresh computer: Codex not installed, no keys, Ollama installed but stopped, no LM Studio. */
export function previewAssist(kind: string): Assist {
  const r = (id: string, name: string, k: RouteKind, state: RouteState, goes_to: string): Route => ({
    id,
    name,
    kind: k,
    state,
    line: "",
    goes_to,
    offline: k === "local",
    consented: false,
    models: state === "ready" ? MODELS[id] ?? [] : [],
  });
  const a: Assist = {
    chosen: null,
    model: null,
    offline_only: false,
    providers: [
      r("codex", "ChatGPT", "plan", "not_installed", "OpenAI"),
      r("openai", "OpenAI API", "key", "no_key", "OpenAI"),
      r("openrouter", "OpenRouter", "key", "no_key", "OpenRouter"),
      r("gemini", "Gemini API", "key", "no_key", "Google"),
      r("ollama", "Ollama on this computer", "local", "not_running", ""),
      r("lmstudio", "LM Studio on this computer", "local", "not_installed", ""),
    ],
  };
  const set = (id: string, p: Partial<Route>) => {
    a.providers = a.providers.map((x) => (x.id === id ? { ...x, ...p, models: p.state === "ready" ? MODELS[id] ?? [] : x.models } : x));
  };
  const choose = (id: string) => {
    set(id, { state: "ready", consented: true });
    a.chosen = id;
    a.model = defaultModel(a.providers.find((x) => x.id === id)!);
  };
  if (kind === "codex-signed-out") set("codex", { state: "signed_out" });
  if (kind === "codex-ready") choose("codex");
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
  c: { cmd: "choose"; id: string; model: string | null; consent: boolean } | { cmd: "forget" | "key_set" | "key_delete" | "signed_in"; id: string } | { cmd: "offline"; on: boolean },
): Assist | string {
  const find = (id: string) => a.providers.find((p) => p.id === id);
  const map = (id: string, f: (p: Route) => Route) => ({ ...a, providers: a.providers.map((p) => (p.id === id ? f(p) : p)) });
  switch (c.cmd) {
    case "offline":
      return { ...a, offline_only: c.on };
    case "choose": {
      const p = find(c.id);
      if (!p || p.state !== "ready") return "That AI is not ready";
      if (!p.consented && !c.consent) return `Allow ${p.name} in Settings first`;
      if (a.offline_only && !p.offline) return "Only this computer is allowed";
      return { ...map(c.id, (x) => ({ ...x, consented: true })), chosen: c.id, model: c.model ?? defaultModel(p) };
    }
    case "forget": {
      const next = map(c.id, (p) => ({ ...p, consented: false, ...(p.kind === "key" ? { state: "no_key" as const, models: [] } : {}) }));
      return a.chosen === c.id ? { ...next, chosen: null, model: null } : next;
    }
    case "key_set":
      return map(c.id, (p) => ({ ...p, state: "ready", models: MODELS[p.id] ?? [] }));
    case "key_delete":
      return { ...map(c.id, (p) => ({ ...p, state: "no_key", models: [] })), ...(a.chosen === c.id ? { chosen: null, model: null } : {}) };
    case "signed_in":
      return map(c.id, (p) => ({ ...p, state: "ready", models: MODELS[p.id] ?? [] }));
  }
}

export const previewModels = (id: string): AssistModel[] => MODELS[id] ?? [];
