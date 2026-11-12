//! The AIs a person can choose in Settings, and what Pointcast keeps of that choice: which one, which model, whether
//! only this computer may be asked, and each destination the person agreed to. `providers` reads only what is known
//! without spending anything; a pasted key goes straight to `vault` and is never in a setting, an event or an error.
//! The address the person types for "Other OpenAI-compatible" is kept as the destination they agreed to.

use super::acp;
use super::anthropic;
use super::codex::{self, Codex};
use super::vault::{self, Secret};
use super::{openai, Failure, Health, Provider, State};
use crate::settings::{Consent, Settings};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const NONE_CHOSEN: &str = "Choose an AI in Settings first";
pub const OFFLINE_ONLY: &str = "Only on this computer is on, so nothing is sent to another one.";
pub const UNKNOWN: &str = "Pointcast does not offer that AI.";
const KEYS_NOT_HERE: &str = "Keys are not supported on Linux yet. A model on this computer needs no key.";

/// The route whose address the person types.
pub const CUSTOM: &str = "custom";
pub const CLAUDE_CODE: &str = "claude-code";
pub const CLAUDE_CODE_NOT_OK: &str = "Needs your OK first: Anthropic's terms for using Claude Code inside another app";
const NO_ADDRESS: &str = "Type the address of the service first.";

/// How a route is paid for: the person's plan through Codex, another subscription through its vendor's own agent, a
/// pasted key, or nothing (a model on this computer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Way {
    Plan,
    Agent,
    Key,
    Local,
}

pub struct Route {
    pub id: &'static str,
    pub name: &'static str,
    pub way: Way,
    pub base: &'static str,
    pub goes_to: &'static str,
}

/// Every route Pointcast offers, in the order Settings lists them: the subscriptions first (ChatGPT, then the agents),
/// then the keys, then this computer. An agent's destination is the one `acp::AGENTS` names.
pub const ROUTES: &[Route] = &[
    Route { id: codex::ID, name: "ChatGPT", way: Way::Plan, base: "", goes_to: super::detect::GOES_TO },
    Route { id: "gemini-cli", name: "Gemini", way: Way::Agent, base: "", goes_to: "Google, through your Gemini CLI sign-in" },
    Route { id: "copilot", name: "GitHub Copilot", way: Way::Agent, base: "", goes_to: "GitHub, through your Copilot plan" },
    Route { id: "opencode", name: "OpenCode", way: Way::Agent, base: "", goes_to: "the AI you set up in OpenCode" },
    Route { id: CLAUDE_CODE, name: "Claude", way: Way::Agent, base: "", goes_to: "Anthropic, through your Claude plan" },
    Route { id: "openai", name: "OpenAI API", way: Way::Key, base: "https://api.openai.com/v1", goes_to: "api.openai.com" },
    Route { id: anthropic::ID, name: "Claude API", way: Way::Key, base: anthropic::BASE, goes_to: "api.anthropic.com" },
    Route { id: "xai", name: "Grok (xAI API)", way: Way::Key, base: "https://api.x.ai/v1", goes_to: "api.x.ai" },
    Route { id: "gemini", name: "Gemini API", way: Way::Key, base: "https://generativelanguage.googleapis.com/v1beta/openai", goes_to: "generativelanguage.googleapis.com" },
    Route { id: "groq", name: "Groq", way: Way::Key, base: "https://api.groq.com/openai/v1", goes_to: "api.groq.com" },
    Route { id: "mistral", name: "Mistral", way: Way::Key, base: "https://api.mistral.ai/v1", goes_to: "api.mistral.ai" },
    Route { id: "deepseek", name: "DeepSeek", way: Way::Key, base: "https://api.deepseek.com/v1", goes_to: "api.deepseek.com" },
    Route { id: "openrouter", name: "OpenRouter", way: Way::Key, base: "https://openrouter.ai/api/v1", goes_to: "openrouter.ai" },
    Route { id: CUSTOM, name: "Other OpenAI-compatible", way: Way::Key, base: "", goes_to: "" },
    Route { id: "ollama", name: "Ollama on this computer", way: Way::Local, base: openai::LOCAL[0].1, goes_to: "this computer" },
    Route { id: "lmstudio", name: "LM Studio on this computer", way: Way::Local, base: openai::LOCAL[1].1, goes_to: "this computer" },
];

pub fn route(id: &str) -> Option<&'static Route> {
    ROUTES.iter().find(|r| r.id == id)
}

/// What the person calls the AI behind `id`; nothing for an id that is not offered.
pub fn name_of(id: &str) -> &'static str {
    route(id).map_or("", |r| r.name)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Ready,
    SignedOut,
    NotInstalled,
    TooOld,
    NoKey,
    NotRunning,
    Unsupported,
    /// Its probe has not answered yet: the pane draws at once and fills this in when it does.
    Checking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub images: bool,
    /// The thinking efforts this model offers, as its vendor lists them, lowest first; empty when it says none.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    /// The vendor marks it as the one it starts with.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub default: bool,
}

impl Model {
    /// A model known only by its id, offering no effort choice.
    pub fn plain(id: String, images: bool) -> Model {
        Model { name: id.clone(), id, images, efforts: Vec::new(), default: false }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    pub id: &'static str,
    pub name: &'static str,
    pub kind: Way,
    pub state: Status,
    pub line: String,
    pub goes_to: String,
    pub offline: bool,
    /// Whether it can be chosen now.
    pub available: bool,
    /// The address the person typed, for the route that takes one.
    pub base: Option<String>,
    pub consented: bool,
    pub models: Vec<Model>,
    /// The thinking efforts the route offers to choose from, lowest first; none where it offers no choice.
    pub efforts: Vec<&'static str>,
    /// Files the AI's own app sends along with every question (Codex: the person's own instructions file).
    pub adds: Vec<String>,
    /// The plan Codex says the person is signed in with ("plus"), and the address its `account/read` names: shown
    /// only in the pane, so the person sees whom they are signed in as.
    pub plan: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct List {
    pub chosen: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub offline_only: bool,
    pub providers: Vec<Row>,
}

/// What is known of each route without spending anything.
#[derive(Debug, Clone, Default)]
pub struct Look {
    /// Codex as `detect::health` found it, and whether `codex login status` says it is signed in.
    pub codex: Option<(Health, Option<bool>)>,
    /// The models and added files of a Codex already running, none otherwise.
    pub codex_models: Vec<Model>,
    pub adds: Vec<String>,
    /// The routes with a key saved.
    pub keys: Vec<&'static str>,
    pub keys_here: bool,
    pub local: Vec<openai::Found>,
    /// Each agent as `acp::health` found it.
    pub agents: Vec<(&'static str, Health)>,
    /// Who Codex says is signed in, once it was asked.
    pub account: Option<codex::Account>,
    /// The routes whose probe has not answered yet.
    pub pending: Vec<&'static str>,
}

/// Gather a `Look`: the model servers at `places` asked for their lists (two seconds), Codex's version and sign-in
/// asked of `codex`, and the store asked which keys it holds; all at once.
pub fn gather(places: &[(&'static str, &str)], codex: impl FnOnce() -> (Health, Option<bool>) + Send, has_key: impl Fn(&str) -> bool, held: Option<Arc<dyn Provider>>) -> Look {
    let (local, codex) = std::thread::scope(|s| {
        let local = s.spawn(|| openai::detect_at(places));
        let codex = codex();
        (local.join().unwrap_or_default(), codex)
    });
    let running = held.filter(|p| p.id() == codex::ID);
    Look {
        codex: Some(codex),
        codex_models: running.as_ref().map(|p| p.models()).unwrap_or_default(),
        adds: running.as_ref().map(|p| p.adds()).unwrap_or_default(),
        keys: ROUTES.iter().filter(|r| r.way == Way::Key && has_key(r.id)).map(|r| r.id).collect(),
        keys_here: cfg!(any(windows, target_os = "macos")),
        local,
        agents: Vec::new(),
        account: None,
        pending: Vec::new(),
    }
}

/// Claude Code, as far as finding it goes.
static CLAUDE_CLI: acp::Agent = acp::Agent { id: CLAUDE_CODE, name: "Claude", cli: "Claude Code", program: "claude", args: &[], goes_to: "Anthropic, through your Claude plan" };

/// Each agent's `--version`, all at once.
fn agents_now() -> Vec<(&'static str, Health)> {
    let all: Vec<&'static acp::Agent> = acp::AGENTS.iter().chain([&CLAUDE_CLI]).collect();
    std::thread::scope(|sc| {
        let found: Vec<_> = all.iter().map(|a| sc.spawn(move || (a.id, acp::health(a)))).collect();
        found.into_iter().filter_map(|h| h.join().ok()).collect()
    })
}

/// The `Look` of this computer.
pub fn look(held: &Held) -> Look {
    let codex = || {
        let found = super::detect::codex();
        let signed = found.as_ref().ok().and_then(|f| super::detect::signed_in(&f.exe));
        (super::detect::health_of(&found), signed)
    };
    let agents = std::thread::spawn(agents_now);
    let mut look = gather(openai::LOCAL, codex, vault::has, held.held());
    look.agents = agents.join().unwrap_or_default();
    look
}

/// What a probe found, put into the session's `Look`.
pub type Patch = Box<dyn FnOnce(&mut Look) + Send>;

/// One look at this computer for the routes `ids`, run on a thread of its own.
pub struct Probe {
    pub ids: Vec<&'static str>,
    pub run: Box<dyn FnOnce() -> Patch + Send>,
}

/// The routes as last seen, kept for the session: the pane draws from it at once and every probe fills in its own
/// routes as it answers. Before the first answer a route says it is being checked; a key is read from the store each
/// time, which is quick.
pub struct Survey {
    look: Mutex<Look>,
    began: Mutex<Option<std::time::Instant>>,
    out: std::sync::atomic::AtomicUsize,
}

impl Default for Survey {
    fn default() -> Self {
        let pending = ROUTES.iter().filter(|r| r.way != Way::Key).map(|r| r.id).collect();
        Survey { look: Mutex::new(Look { pending, ..Look::default() }), began: Mutex::default(), out: Default::default() }
    }
}

impl Survey {
    /// The list for `s` from what is known now; never waits on a probe.
    pub fn now(&self, s: &Settings, has_key: impl Fn(&str) -> bool, held: Option<Arc<dyn Provider>>) -> List {
        let mut look = self.look.lock().unwrap().clone();
        look.keys = ROUTES.iter().filter(|r| r.way == Way::Key && has_key(r.id)).map(|r| r.id).collect();
        look.keys_here = cfg!(any(windows, target_os = "macos"));
        // A Codex already running for Ask knows its models and who is signed in better than the last probe.
        if let Some(p) = held.filter(|p| p.id() == codex::ID) {
            let models = p.models();
            if !models.is_empty() {
                look.codex_models = models;
            }
            look.adds = p.adds();
            look.account = p.account().or(look.account);
        }
        providers(s, &look)
    }

    /// Whether a round of probes is out now.
    pub fn busy(&self) -> bool {
        self.out.load(std::sync::atomic::Ordering::SeqCst) > 0
    }

    /// Start `probes`, each on its own thread: once per run of the app (at launch), and again only when the person
    /// asks (Refresh, Check again, a sign-in: `force`); never on a timer and never because the pane opened. `told` is
    /// called after each probe's answer is in. Returns whether a round started.
    pub fn refresh(self: &Arc<Self>, probes: Vec<Probe>, force: bool, told: Arc<dyn Fn() + Send + Sync>) -> bool {
        {
            let mut began = self.began.lock().unwrap();
            if self.busy() || (began.is_some() && !force) {
                return false;
            }
            *began = Some(std::time::Instant::now());
            self.out.fetch_add(probes.len(), std::sync::atomic::Ordering::SeqCst);
        }
        for p in probes {
            let me = self.clone();
            let told = told.clone();
            std::thread::spawn(move || {
                let patch = (p.run)();
                {
                    let mut look = me.look.lock().unwrap();
                    patch(&mut look);
                    look.pending.retain(|id| !p.ids.contains(id));
                }
                me.out.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                told();
            });
        }
        true
    }
}

/// Every probe of this computer, one per thing that can be slow: Codex (its version, its sign-in, and who is signed in
/// as its own `account/read` says), each agent's `--version`, and each model server's port.
pub fn probes(held: Arc<Held>, home: PathBuf) -> Vec<Probe> {
    let mut all = vec![Probe { ids: vec![codex::ID], run: Box::new(move || codex_probe(&held, &home)) }];
    for a in acp::AGENTS.iter().chain([&CLAUDE_CLI]) {
        let a: &'static acp::Agent = a;
        all.push(Probe {
            ids: vec![a.id],
            run: Box::new(move || {
                let h = acp::health(a);
                Box::new(move |l: &mut Look| {
                    l.agents.retain(|(id, _)| *id != a.id);
                    l.agents.push((a.id, h));
                })
            }),
        });
    }
    for (i, place) in openai::LOCAL.iter().enumerate() {
        let id = ROUTES.iter().filter(|r| r.way == Way::Local).nth(i).map_or("", |r| r.id);
        all.push(Probe {
            ids: vec![id],
            run: Box::new(move || {
                let found = openai::detect_at(&[*place]);
                Box::new(move |l: &mut Look| {
                    l.local.retain(|f| f.base_url != place.1);
                    l.local.extend(found);
                })
            }),
        });
    }
    all
}

fn codex_probe(held: &Held, home: &Path) -> Patch {
    let found = super::detect::codex();
    let signed = found.as_ref().ok().and_then(|f| super::detect::signed_in(&f.exe));
    let health = super::detect::health_of(&found);
    // Who is signed in: the Codex running for Ask already knows; else one is started and greeted, which spends nothing.
    let (account, models) = if health.state == State::Ready && signed != Some(false) {
        match held.held().filter(|p| p.id() == codex::ID) {
            Some(p) => {
                p.warm();
                (p.account(), p.models())
            }
            None => {
                let c = Codex::new(home.join("codex"));
                let models = c.list().unwrap_or_default();
                (super::Provider::account(&c), models)
            }
        }
    } else {
        (None, Vec::new())
    };
    Box::new(move |l: &mut Look| {
        l.codex = Some((health, signed));
        l.account = account;
        l.codex_models = models;
    })
}

/// The address the person typed for the other service, checked: https, or http only to this computer, and nothing
/// but the address (no name, password or query, which could carry a key into the settings).
pub fn base_of(text: &str) -> Result<String, String> {
    let url = reqwest::Url::parse(text.trim()).map_err(|_| "That is not a web address.".to_owned())?;
    if !url.username().is_empty() || url.password().is_some() || url.query().is_some() || url.fragment().is_some() {
        return Err("Type only the address of the service, with no name, password or query in it.".into());
    }
    match url.scheme() {
        "https" => {}
        "http" if openai::loopback(&url) => {}
        _ => return Err("The address must start with https:// (http:// only for this computer).".into()),
    }
    Ok(url.as_str().trim_end_matches('/').to_owned())
}

fn here(base: &str) -> bool {
    reqwest::Url::parse(base).is_ok_and(|u| openai::loopback(&u))
}

fn host_of(base: &str) -> String {
    reqwest::Url::parse(base).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default()
}

/// The address the person agreed to send questions to through the other service, if they did.
pub fn custom_base(s: &Settings) -> Option<String> {
    s.assist_consent.iter().filter(|c| c.provider == CUSTOM).find_map(|c| base_of(&c.goes_to).ok())
}

/// Whether nothing sent through `r` leaves this computer.
fn stays_here(s: &Settings, r: &Route) -> bool {
    r.way == Way::Local || (r.id == CUSTOM && custom_base(s).is_some_and(|b| here(&b)))
}

pub fn consented(s: &Settings, r: &Route) -> bool {
    if r.id == CUSTOM {
        return custom_base(s).is_some();
    }
    s.assist_consent.iter().any(|c| c.provider == r.id && c.goes_to == r.goes_to)
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', std::path::MAIN_SEPARATOR]).next().unwrap_or(path)
}

/// Every route as Settings shows it.
pub fn providers(s: &Settings, look: &Look) -> List {
    let custom = custom_base(s);
    let rows = ROUTES
        .iter()
        .map(|r| {
            let (mut models, mut adds) = (Vec::new(), Vec::new());
            let (mut plan, mut email) = (None, None);
            let checking = look.pending.contains(&r.id) && !(r.id == CLAUDE_CODE && !s.assist_claude_code_ok);
            let (state, line) = match r.way {
                _ if checking => (Status::Checking, "Checking…".to_owned()),
                Way::Plan => match &look.codex {
                    Some((h, signed)) => match (h.state, signed) {
                        (State::NotInstalled, _) => (Status::NotInstalled, h.line.clone()),
                        (State::TooOld, _) => (Status::TooOld, h.line.clone()),
                        (State::Unknown, _) => (Status::Unsupported, h.line.clone()),
                        (State::Ready, Some(false)) => (Status::SignedOut, "Codex is installed. Sign in with your ChatGPT account.".to_owned()),
                        (State::Ready, signed) => {
                            (models, adds) = (look.codex_models.clone(), look.adds.clone());
                            if let Some(a) = &look.account {
                                (plan, email) = (a.plan.clone(), a.email.clone());
                            }
                            let mut line = if *signed == Some(true) { "Signed in to ChatGPT through Codex.".to_owned() } else { h.line.clone() };
                            if !adds.is_empty() {
                                let names: Vec<&str> = adds.iter().map(|a| file_name(a)).collect();
                                line.push_str(&format!(" Codex also sends your own Codex instructions ({}) with every question.", names.join(", ")));
                            }
                            (Status::Ready, line)
                        }
                    },
                    None => (Status::NotInstalled, "Codex is not installed on this computer.".to_owned()),
                },
                Way::Agent if r.id == CLAUDE_CODE && !s.assist_claude_code_ok => (Status::Unsupported, CLAUDE_CODE_NOT_OK.to_owned()),
                Way::Agent => match look.agents.iter().find(|(id, _)| *id == r.id).map(|(_, h)| h) {
                    Some(h) => match h.state {
                        State::Ready => (Status::Ready, h.line.clone()),
                        State::NotInstalled => (Status::NotInstalled, h.line.clone()),
                        State::TooOld => (Status::TooOld, h.line.clone()),
                        State::Unknown => (Status::Unsupported, h.line.clone()),
                    },
                    None => (Status::NotInstalled, format!("{} is not installed on this computer.", r.name)),
                },
                Way::Key if r.id == CUSTOM && custom.is_none() && look.keys_here => (if look.keys.contains(&r.id) { Status::Ready } else { Status::NoKey }, NO_ADDRESS.to_owned()),
                Way::Key if !look.keys_here => (Status::Unsupported, KEYS_NOT_HERE.to_owned()),
                Way::Key if look.keys.contains(&r.id) => (Status::Ready, "Key saved.".to_owned()),
                Way::Key => (Status::NoKey, "No key yet.".to_owned()),
                Way::Local => match look.local.iter().find(|f| f.base_url == r.base) {
                    Some(f) => {
                        models = f.models.iter().map(|m| Model::plain(m.clone(), true)).collect();
                        let line = if models.len() == 1 { "Running, 1 model.".to_owned() } else { format!("Running, {} models.", models.len()) };
                        (Status::Ready, line)
                    }
                    None => (Status::NotRunning, "Not running.".to_owned()),
                },
            };
            let (goes_to, base) = match (r.id, &custom) {
                (CUSTOM, Some(b)) => (host_of(b), Some(b.clone())),
                (CUSTOM, None) => ("the address you type".to_owned(), None),
                _ => (r.goes_to.to_owned(), None),
            };
            let available = !(r.id == CLAUDE_CODE && !s.assist_claude_code_ok);
            Row { id: r.id, name: r.name, kind: r.way, state, line, goes_to, offline: stays_here(s, r), available, base, consented: consented(s, r), models, efforts: efforts_of(r.id).to_vec(), adds, plan, email }
        })
        .collect();
    List { chosen: s.assist_provider.clone(), model: s.assist_model.clone(), effort: effort_of(s), offline_only: s.assist_offline_only, providers: rows }
}

/// Every effort Codex names, lowest first; the first is what Pointcast starts with. Each model offers some of them
/// (its `supportedReasoningEfforts`); one the chosen model does not offer is not sent, and the model's own applies.
pub const CODEX_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max", "ultra"];

/// The thinking efforts route `id` offers; none for a route that offers no choice.
pub fn efforts_of(id: &str) -> &'static [&'static str] {
    if id == codex::ID {
        CODEX_EFFORTS
    } else {
        &[]
    }
}

/// The effort saved for the chosen route, if that route offers it.
pub fn effort_of(s: &Settings) -> Option<String> {
    let offered = efforts_of(s.assist_provider.as_deref().unwrap_or_default());
    s.assist_effort.clone().filter(|e| offered.contains(&e.as_str()))
}

/// Save the thinking effort for the chosen route; refused when it offers no such choice.
pub fn set_effort(s: &mut Settings, effort: &str) -> Result<(), String> {
    let id = s.assist_provider.as_deref().unwrap_or_default();
    if !efforts_of(id).contains(&effort) {
        return Err(format!("{} offers no \"{effort}\" effort.", route(id).map_or("That AI", |r| r.name)));
    }
    s.assist_effort = Some(effort.to_owned());
    Ok(())
}

/// Choose the AI `id` and its `model`. Refused unless the person agrees now (`consent`) or agreed before to this
/// route and destination; the agreement is kept. A route off this computer is refused while "Only on this computer" is on.
pub fn choose(s: &mut Settings, id: &str, model: Option<String>, consent: bool, now_ms: u64) -> Result<(), String> {
    choose_at(s, id, None, model, consent, now_ms)
}

/// `choose`, with the address the person typed for the other service (`CUSTOM`); none keeps the one agreed to. A new
/// address is a new destination and needs a new yes. Claude through Claude Code is refused until the owner's OK.
pub fn choose_at(s: &mut Settings, id: &str, base: Option<&str>, model: Option<String>, consent: bool, now_ms: u64) -> Result<(), String> {
    let r = route(id).ok_or(UNKNOWN)?;
    if r.id == CLAUDE_CODE && !s.assist_claude_code_ok {
        return Err(CLAUDE_CODE_NOT_OK.into());
    }
    let goes_to = match (r.id, base) {
        (CUSTOM, Some(b)) => base_of(b)?,
        (CUSTOM, None) => custom_base(s).ok_or(NO_ADDRESS)?,
        _ => r.goes_to.to_owned(),
    };
    if s.assist_offline_only && !(r.way == Way::Local || (r.id == CUSTOM && here(&goes_to))) {
        return Err(OFFLINE_ONLY.into());
    }
    let model = model.map(|m| m.trim().to_owned()).filter(|m| !m.is_empty());
    if matches!(r.way, Way::Key | Way::Local) && model.is_none() {
        return Err(format!("Choose a model for {} first.", r.name));
    }
    if !s.assist_consent.iter().any(|c| c.provider == r.id && c.goes_to == goes_to) {
        if !consent {
            let shown = if r.id == CUSTOM { host_of(&goes_to) } else { goes_to };
            return Err(format!("Agree to send your questions to {shown} first."));
        }
        s.assist_consent.retain(|c| c.provider != r.id);
        s.assist_consent.push(Consent { provider: r.id.into(), goes_to, at_ms: now_ms });
    }
    s.assist_provider = Some(r.id.into());
    s.assist_model = model;
    Ok(())
}

/// Forget `id`: its agreement, the choice if it was chosen, and its saved key. The settings change even when the store
/// could not forget the key; the error says so.
pub fn forget(s: &mut Settings, keys: &Keys, id: &str) -> Result<(), String> {
    let r = route(id).ok_or(UNKNOWN)?;
    s.assist_consent.retain(|c| c.provider != r.id);
    if s.assist_provider.as_deref() == Some(r.id) {
        (s.assist_provider, s.assist_model) = (None, None);
    }
    if r.way == Way::Key {
        keys.delete(r.id)?;
    }
    Ok(())
}

/// The pasted keys, in the system's store, under items whose names start with `ns` (none for the person's own; a
/// test's own name in a test, so a test never touches a real key).
pub struct Keys {
    ns: String,
}

impl Keys {
    pub fn store() -> Keys {
        Keys { ns: String::new() }
    }

    #[cfg(test)]
    pub fn named(ns: &str) -> Keys {
        Keys { ns: ns.to_owned() }
    }

    fn item(&self, id: &str) -> String {
        format!("{}{id}", self.ns)
    }

    fn keyed(id: &str) -> Result<&'static Route, String> {
        let r = route(id).ok_or(UNKNOWN)?;
        if r.way == Way::Key {
            Ok(r)
        } else {
            Err(format!("{} takes no key.", r.name))
        }
    }

    pub fn has(&self, id: &str) -> bool {
        vault::has(&self.item(id))
    }

    pub fn set(&self, id: &str, key: String) -> Result<(), String> {
        let r = Self::keyed(id)?;
        vault::set(&self.item(r.id), Secret::new(key))
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let r = Self::keyed(id)?;
        vault::delete(&self.item(r.id))
    }
}

/// What an Ask is sent through, as the settings say now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub id: &'static str,
    pub model: Option<String>,
    pub offline_only: bool,
    /// The address agreed to, for the other service.
    pub base: Option<String>,
    /// The thinking effort chosen, where the route offers one.
    pub effort: Option<String>,
}

/// The chosen AI, or the line the panel shows instead: none chosen, not agreed to, or off this computer while only
/// this computer may be asked.
pub fn picked(s: &Settings) -> Result<Picked, String> {
    let Some(r) = s.assist_provider.as_deref().and_then(route) else {
        return Err(NONE_CHOSEN.into());
    };
    if r.id == CLAUDE_CODE && !s.assist_claude_code_ok {
        return Err(CLAUDE_CODE_NOT_OK.into());
    }
    if !consented(s, r) {
        return Err(format!("Allow {} in Settings first", r.name));
    }
    if s.assist_offline_only && !stays_here(s, r) {
        return Err(OFFLINE_ONLY.into());
    }
    let base = if r.id == CUSTOM { custom_base(s) } else { None };
    Ok(Picked { id: r.id, model: s.assist_model.clone(), offline_only: s.assist_offline_only, base, effort: effort_of(s) })
}

/// Make the provider for `p`. Codex keeps its empty folder and its pid file under `home`.
pub fn build(p: &Picked, home: &Path) -> Result<Arc<dyn Provider>, String> {
    let r = route(p.id).ok_or(UNKNOWN)?;
    let model = p.model.as_deref().unwrap_or_default();
    match r.way {
        Way::Plan => Ok(Arc::new(codex_for(p, home))),
        Way::Agent => match acp::agent(r.id) {
            Some(a) => Ok(Arc::new(acp::Acp::new(a, home.join(a.id)))),
            // Claude through Claude Code. A `Picked` that names it exists only with the owner's OK: `picked` refuses
            // it before, and so nothing is made.
            None => Ok(Arc::new(super::claude::Claude::new(home.join(CLAUDE_CODE)))),
        },
        _ if model.is_empty() => Err(format!("Choose a model for {} in Settings first.", r.name)),
        Way::Key => Ok(Arc::new(keyed(r, p.base.as_deref(), model)?.provider(p.offline_only)?)),
        Way::Local => Ok(Arc::new(openai::local(r.id, r.base, model).provider(p.offline_only)?)),
    }
}

/// Codex for `p`: its model, and the effort chosen (low, Codex's start here, when none is).
pub fn codex_for(p: &Picked, home: &Path) -> Codex {
    let mut c = Codex::new(home.join("codex"));
    c.model = p.model.clone();
    if let Some(e) = &p.effort {
        c.effort = Some(e.clone());
    }
    c
}

/// The service behind a key route: Anthropic's own API, the address typed for the other service (with a key only if
/// one is saved, when that is this computer), or OpenAI's chat API at the route's own address.
fn keyed(r: &Route, base: Option<&str>, model: &str) -> Result<openai::Config, String> {
    Ok(match r.id {
        anthropic::ID => anthropic::config(r.id, r.base, model),
        CUSTOM => {
            let base = base.ok_or(NO_ADDRESS)?;
            openai::Config { needs_key: !here(base) || vault::has(r.id), ..openai::remote(r.id, base, model) }
        }
        _ => openai::remote(r.id, r.base, model),
    })
}

pub type Build = Box<dyn Fn(&Picked) -> Result<Arc<dyn Provider>, String> + Send + Sync>;

/// The provider of the chosen AI, made once per choice: a warmed helper and a model list outlive one question, and a
/// changed choice takes effect at the next Ask.
pub struct Held {
    built: Mutex<Option<(Picked, Arc<dyn Provider>)>>,
    build: Build,
}

impl Held {
    pub fn new(home: PathBuf) -> Held {
        Held::with(Box::new(move |p| build(p, &home)))
    }

    pub fn with(build: Build) -> Held {
        Held { built: Mutex::default(), build }
    }

    /// The provider for what the settings say now; nothing is made when the answer is no.
    pub fn provider(&self, s: &Settings) -> Result<Arc<dyn Provider>, String> {
        let p = picked(s)?;
        let mut b = self.built.lock().unwrap();
        if let Some((was, made)) = b.as_ref() {
            if *was == p {
                return Ok(made.clone());
            }
        }
        let made = (self.build)(&p)?;
        *b = Some((p, made.clone()));
        Ok(made)
    }

    pub fn held(&self) -> Option<Arc<dyn Provider>> {
        self.built.lock().unwrap().as_ref().map(|(_, p)| p.clone())
    }

    /// Let go of a helper warmed for a question that never came.
    pub fn cool(&self) {
        if let Some(p) = self.held() {
            p.cool();
        }
    }
}

/// The models `id` offers, asked of it now: Codex is started and greeted (its `model/list`), a service is asked for its
/// list. Never a question.
pub fn models_of(id: &str, s: &Settings, held: &Held, home: &Path) -> Result<Vec<Model>, String> {
    let r = route(id).ok_or(UNKNOWN)?;
    match r.way {
        Way::Plan => {
            let said = |f: Failure| match f {
                Failure::NotInstalled => "Codex is not installed on this computer.".to_owned(),
                Failure::SignedOut => "Sign in to ChatGPT through Codex first.".to_owned(),
                Failure::Broken(why) => why,
                _ => "Codex did not list its models.".to_owned(),
            };
            match held.held().filter(|p| p.id() == codex::ID) {
                Some(p) => {
                    p.warm();
                    Some(p.models()).filter(|m| !m.is_empty()).ok_or_else(|| "Codex did not list its models.".to_owned())
                }
                None => Codex::new(home.join("codex")).list().map_err(said),
            }
        }
        // An agent picks its own model, from what the person set up in it.
        Way::Agent => Ok(Vec::new()),
        Way::Key => keyed(r, custom_base(s).as_deref(), "")?.provider(s.assist_offline_only)?.listed(),
        Way::Local => openai::local(r.id, r.base, "").provider(s.assist_offline_only)?.listed(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::http::tests::{stub, Reply};
    use crate::assist::Scripted;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const FAKE: &str = "test-key-not-real-planted";

    fn ready_codex() -> (Health, Option<bool>) {
        (Health { state: State::Ready, line: "Codex 0.164.0 is installed.".into(), goes_to: super::super::detect::GOES_TO }, Some(true))
    }

    #[test]
    fn every_route_says_where_a_question_goes_and_a_service_is_named_by_its_own_host() {
        let ids: Vec<&str> = ROUTES.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            [
                "codex", "gemini-cli", "copilot", "opencode", "claude-code", "openai", "anthropic", "xai", "gemini", "groq", "mistral", "deepseek", "openrouter",
                "custom", "ollama", "lmstudio"
            ]
        );
        for r in ROUTES {
            assert!(!r.name.is_empty() && name_of(r.id) == r.name);
            match r.way {
                Way::Plan => assert!(r.base.is_empty()),
                Way::Agent => {
                    assert!(r.base.is_empty() && !r.goes_to.is_empty(), "{}", r.id);
                    let named = acp::agent(r.id).map_or(CLAUDE_CLI.goes_to, |a| a.goes_to);
                    assert_eq!(r.goes_to, named, "{}", r.id);
                }
                Way::Key if r.id == CUSTOM => assert!(r.base.is_empty() && r.goes_to.is_empty()),
                Way::Key => assert_eq!(reqwest::Url::parse(r.base).unwrap().host_str(), Some(r.goes_to), "{}", r.id),
                Way::Local => assert_eq!((reqwest::Url::parse(r.base).unwrap().host_str(), r.goes_to), (Some("127.0.0.1"), "this computer")),
            }
        }
        assert_eq!((name_of("codex"), name_of("claude"), route("claude").is_none()), ("ChatGPT", "", true));
    }

    #[test]
    fn choosing_needs_the_persons_yes_once_per_route_and_destination() {
        let mut s = Settings::default();
        assert_eq!(picked(&s), Err(NONE_CHOSEN.into()));
        assert_eq!(choose(&mut s, "openai", Some("gpt-x".into()), false, 1), Err("Agree to send your questions to api.openai.com first.".into()));
        assert_eq!((s.assist_provider.clone(), s.assist_consent.len()), (None, 0), "nothing kept on a no");
        assert_eq!(choose(&mut s, "openai", None, true, 1), Err("Choose a model for OpenAI API first.".into()));
        assert_eq!(choose(&mut s, "openai", Some(" gpt-x ".into()), true, 7), Ok(()));
        assert_eq!(s.assist_consent, [Consent { provider: "openai".into(), goes_to: "api.openai.com".into(), at_ms: 7 }]);
        assert_eq!((s.assist_provider.as_deref(), s.assist_model.as_deref()), (Some("openai"), Some("gpt-x")));
        // Remembered: the next choice of it needs no new yes, and Codex takes its own default model.
        assert_eq!(choose(&mut s, "openai", Some("gpt-y".into()), false, 9), Ok(()));
        assert_eq!(s.assist_consent.len(), 1);
        assert_eq!(picked(&s), Ok(Picked { id: "openai", model: Some("gpt-y".into()), offline_only: false, base: None, effort: None }));
        assert_eq!(choose(&mut s, "codex", None, true, 10), Ok(()));
        assert_eq!(s.assist_model, None);
        assert_eq!(choose(&mut s, "nope", None, true, 1), Err(UNKNOWN.into()));
        // A destination agreed to before is not this one: it asks again, and an Ask is refused meanwhile.
        let mut moved = Settings { assist_provider: Some("gemini".into()), assist_model: Some("m".into()), ..Settings::default() };
        moved.assist_consent.push(Consent { provider: "gemini".into(), goes_to: "old.example.test".into(), at_ms: 1 });
        assert_eq!(picked(&moved), Err("Allow Gemini API in Settings first".into()));
        assert!(choose(&mut moved, "gemini", Some("m".into()), false, 2).is_err());
        assert_eq!(choose(&mut moved, "gemini", Some("m".into()), true, 2), Ok(()));
        assert_eq!(moved.assist_consent, [Consent { provider: "gemini".into(), goes_to: "generativelanguage.googleapis.com".into(), at_ms: 2 }]);
    }

    #[test]
    fn only_on_this_computer_refuses_a_remote_choice_and_a_remote_ask_and_builds_nothing() {
        let mut s = Settings::default();
        choose(&mut s, "openrouter", Some("m".into()), true, 1).unwrap();
        s.assist_offline_only = true;
        assert_eq!(choose(&mut s, "codex", None, true, 1), Err(OFFLINE_ONLY.into()));
        assert_eq!(choose(&mut s, "openai", Some("m".into()), true, 1), Err(OFFLINE_ONLY.into()));
        assert_eq!(picked(&s), Err(OFFLINE_ONLY.into()));
        let builds = Arc::new(AtomicUsize::new(0));
        let n = builds.clone();
        let held = Held::with(Box::new(move |_| {
            n.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Scripted::new(&[])) as Arc<dyn Provider>)
        }));
        assert_eq!(held.provider(&s).err(), Some(OFFLINE_ONLY.into()));
        // A model on this computer is still allowed, and even a remote route built by hand makes no client.
        assert_eq!(choose(&mut s, "ollama", Some("llava".into()), true, 2), Ok(()));
        assert!(held.provider(&s).is_ok());
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        let remote = Picked { id: "openai", model: Some("m".into()), offline_only: true, base: None, effort: None };
        assert_eq!(build(&remote, Path::new("unused")).err(), Some(OFFLINE_ONLY.into()));
    }

    #[test]
    fn the_provider_is_made_once_per_choice_and_none_without_one() {
        let builds = Arc::new(AtomicUsize::new(0));
        let n = builds.clone();
        let held = Held::with(Box::new(move |p| {
            n.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Scripted { id: p.id, ..Scripted::new(&[]) }) as Arc<dyn Provider>)
        }));
        let mut s = Settings::default();
        assert_eq!(held.provider(&s).err(), Some(NONE_CHOSEN.into()));
        s.assist_provider = Some("lmstudio".into());
        s.assist_model = Some("m".into());
        assert_eq!(held.provider(&s).err(), Some("Allow LM Studio on this computer in Settings first".into()));
        assert_eq!(builds.load(Ordering::SeqCst), 0, "no yes, nothing made");
        choose(&mut s, "lmstudio", Some("m".into()), true, 1).unwrap();
        let a = held.provider(&s).unwrap();
        let b = held.provider(&s).unwrap();
        assert!(Arc::ptr_eq(&a, &b) && a.id() == "lmstudio");
        s.assist_model = Some("other".into());
        assert!(!Arc::ptr_eq(&a, &held.provider(&s).unwrap()), "a new model is a new provider");
        assert_eq!(builds.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn the_list_spends_nothing_a_local_server_is_asked_only_for_its_models_and_codex_s_added_file_is_said() {
        let server = stub(|_| Reply::Whole(200, "", r#"{"data":[{"id":"llava"},{"id":"qwen"}]}"#.into()));
        let place: &'static str = Box::leak(server.base.clone().into_boxed_str());
        let mut held_codex = Scripted::new(&[]);
        held_codex.id = codex::ID;
        let look = gather(&[("Ollama", place)], ready_codex, |id| id == "openrouter", Some(Arc::new(held_codex)));
        assert_eq!(server.paths(), ["/v1/models"], "one list, never a question");
        let mut look = Look { local: look.local.iter().map(|f| openai::Found { base_url: openai::LOCAL[0].1.into(), ..f.clone() }).collect(), ..look };
        look.adds = vec!["/home/someone/.codex/AGENTS.md".into()];
        let mut s = Settings::default();
        choose(&mut s, "codex", None, true, 1).unwrap();
        let list = providers(&s, &look);
        assert_eq!((list.chosen.as_deref(), list.offline_only), (Some("codex"), false));
        let row = |id: &str| list.providers.iter().find(|r| r.id == id).unwrap().clone();
        let codex = row("codex");
        assert_eq!((codex.state, codex.consented, codex.kind, codex.adds.len()), (Status::Ready, true, Way::Plan, 1));
        assert!(codex.line.contains("Signed in") && codex.line.contains("(AGENTS.md)"), "{}", codex.line);
        let ollama = row("ollama");
        assert_eq!((ollama.state, ollama.offline, ollama.models.len(), ollama.line.as_str()), (Status::Ready, true, 2, "Running, 2 models."));
        assert_eq!(row("lmstudio").state, Status::NotRunning);
        let keyed = if cfg!(any(windows, target_os = "macos")) { (Status::Ready, Status::NoKey) } else { (Status::Unsupported, Status::Unsupported) };
        assert_eq!((row("openrouter").state, row("openai").state), keyed);
        let json = serde_json::to_value(&list).unwrap();
        let at = |id: &str| list.providers.iter().position(|r| r.id == id).unwrap();
        assert_eq!((json["providers"][0]["kind"].as_str(), json["providers"][at("openai")]["state"].as_str()), (Some("plan"), Some(if keyed.1 == Status::NoKey { "no_key" } else { "unsupported" })));
        assert_eq!(json["providers"][at("copilot")]["kind"].as_str(), Some("agent"));
        // Codex signed out, missing, too old: one plain state each.
        let codex_as = |h: State, signed: Option<bool>| {
            let look = Look { codex: Some((Health { state: h, line: "said".into(), goes_to: "" }, signed)), ..Look::default() };
            providers(&Settings::default(), &look).providers[0].state
        };
        assert_eq!(
            [codex_as(State::Ready, Some(false)), codex_as(State::NotInstalled, None), codex_as(State::TooOld, None), codex_as(State::Unknown, None)],
            [Status::SignedOut, Status::NotInstalled, Status::TooOld, Status::Unsupported]
        );
    }

    #[test]
    fn the_other_service_takes_an_address_the_person_types_and_a_new_address_asks_again() {
        let mut s = Settings::default();
        assert_eq!(choose(&mut s, CUSTOM, Some("m".into()), true, 1), Err(NO_ADDRESS.into()));
        for bad in ["ftp://x.example.test/v1", "http://x.example.test/v1", "https://user:pw@x.example.test/v1", "https://x.example.test/v1?key=abc", "not an address"] {
            assert!(choose_at(&mut s, CUSTOM, Some(bad), Some("m".into()), true, 1).is_err(), "{bad}");
        }
        assert!(s.assist_consent.is_empty() && s.assist_provider.is_none());
        let first = "https://llm.example.test/v1/";
        assert_eq!(choose_at(&mut s, CUSTOM, Some(first), Some("m".into()), false, 1), Err("Agree to send your questions to llm.example.test first.".into()));
        assert_eq!(choose_at(&mut s, CUSTOM, Some(first), Some("m".into()), true, 2), Ok(()));
        assert_eq!(s.assist_consent, [Consent { provider: CUSTOM.into(), goes_to: "https://llm.example.test/v1".into(), at_ms: 2 }]);
        assert_eq!(picked(&s).unwrap().base.as_deref(), Some("https://llm.example.test/v1"));
        // The same address needs no new yes; another one does, and replaces the first.
        assert_eq!(choose(&mut s, CUSTOM, Some("n".into()), false, 3), Ok(()));
        let other = "https://other.example.test/v1";
        assert!(choose_at(&mut s, CUSTOM, Some(other), Some("m".into()), false, 4).is_err());
        assert_eq!(picked(&s).unwrap().base.as_deref(), Some("https://llm.example.test/v1"), "a no changes nothing");
        assert_eq!(choose_at(&mut s, CUSTOM, Some(other), Some("m".into()), true, 4), Ok(()));
        assert_eq!(s.assist_consent.len(), 1);
        let row = providers(&s, &Look::default()).providers.into_iter().find(|r| r.id == CUSTOM).unwrap();
        assert_eq!((row.goes_to.as_str(), row.base.as_deref(), row.offline), ("other.example.test", Some(other), false));
        // http only to this computer, which "Only on this computer" allows.
        s.assist_offline_only = true;
        assert_eq!(picked(&s), Err(OFFLINE_ONLY.into()));
        assert_eq!(choose_at(&mut s, CUSTOM, Some(other), Some("m".into()), true, 5), Err(OFFLINE_ONLY.into()));
        assert_eq!(choose_at(&mut s, CUSTOM, Some("http://127.0.0.1:8000/v1"), Some("m".into()), true, 5), Ok(()));
        let p = picked(&s).unwrap();
        assert_eq!(p.base.as_deref(), Some("http://127.0.0.1:8000/v1"));
        assert_eq!(build(&p, Path::new("unused")).map(|b| b.id()), Ok(CUSTOM));
    }

    #[test]
    fn claude_through_claude_code_cannot_be_chosen_or_asked_until_the_owner_s_ok() {
        let mut s = Settings::default();
        assert_eq!(choose(&mut s, CLAUDE_CODE, None, true, 1), Err(CLAUDE_CODE_NOT_OK.into()));
        assert!(s.assist_provider.is_none() && s.assist_consent.is_empty());
        let row = providers(&s, &Look::default()).providers.into_iter().find(|r| r.id == CLAUDE_CODE).unwrap();
        assert_eq!((row.available, row.kind, row.state, row.line.as_str()), (false, Way::Agent, Status::Unsupported, CLAUDE_CODE_NOT_OK));
        // Chosen by hand in the settings file, it is still refused at the Ask, and nothing is made.
        s.assist_provider = Some(CLAUDE_CODE.into());
        s.assist_consent.push(Consent { provider: CLAUDE_CODE.into(), goes_to: CLAUDE_CLI.goes_to.into(), at_ms: 1 });
        assert_eq!(picked(&s), Err(CLAUDE_CODE_NOT_OK.into()));
        let made = |s: &Settings| picked(s).and_then(|p| build(&p, Path::new("unused"))).map(|b| b.id());
        assert_eq!(made(&s), Err(CLAUDE_CODE_NOT_OK.into()));
        s.assist_claude_code_ok = true;
        assert_eq!(choose(&mut s, CLAUDE_CODE, None, false, 2), Ok(()));
        assert_eq!(made(&s), Ok(CLAUDE_CODE));
        assert!(providers(&s, &Look::default()).providers.iter().all(|r| r.available));
    }

    #[test]
    fn an_agent_is_a_subscription_that_needs_no_model_and_says_how_it_was_found() {
        let h = |state, line: &str| Health { state, line: line.into(), goes_to: "" };
        let ready = "Gemini CLI 0.9.0 is installed. Sign in with the Gemini CLI first.";
        let look = Look { agents: vec![("gemini-cli", h(State::Ready, ready)), ("copilot", h(State::Unknown, "said"))], ..Look::default() };
        let mut s = Settings::default();
        let list = providers(&s, &look);
        let row = |id: &str| list.providers.iter().find(|r| r.id == id).unwrap().clone();
        let gemini = row("gemini-cli");
        assert_eq!((gemini.state, gemini.kind, gemini.offline, gemini.line.as_str()), (Status::Ready, Way::Agent, false, ready));
        assert_eq!(gemini.goes_to, "Google, through your Gemini CLI sign-in");
        assert_eq!((row("copilot").state, row("opencode").state), (Status::Unsupported, Status::NotInstalled));
        assert_eq!(choose(&mut s, "gemini-cli", None, false, 1), Err("Agree to send your questions to Google, through your Gemini CLI sign-in first.".into()));
        assert_eq!(choose(&mut s, "gemini-cli", None, true, 1), Ok(()));
        assert_eq!((s.assist_provider.as_deref(), s.assist_model.as_deref()), (Some("gemini-cli"), None));
        let p = picked(&s).unwrap();
        assert_eq!(build(&p, Path::new("unused")).map(|b| b.id()), Ok("gemini-cli"));
        let held = Held::with(Box::new(|_| Err("unused".into())));
        assert_eq!(models_of("opencode", &s, &held, Path::new("unused")), Ok(Vec::new()));
        let claude_api = Picked { id: anthropic::ID, model: Some("claude-x".into()), offline_only: false, base: None, effort: None };
        assert_eq!(build(&claude_api, Path::new("unused")).map(|b| b.id()), Ok("anthropic"));
        s.assist_offline_only = true;
        assert_eq!(choose(&mut s, "copilot", None, true, 2), Err(OFFLINE_ONLY.into()));
        assert_eq!(picked(&s), Err(OFFLINE_ONLY.into()));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn forgetting_a_route_removes_its_key_its_yes_and_the_choice_and_a_key_is_never_in_a_setting_list_or_error() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let keys = Keys::named(&format!("pointcast-test-{}-{nanos}/", std::process::id()));
        struct Gone<'a>(&'a Keys);
        impl Drop for Gone<'_> {
            fn drop(&mut self) {
                let _ = self.0.delete("openai");
            }
        }
        let _gone = Gone(&keys);
        assert_eq!(keys.set("ollama", FAKE.into()), Err("Ollama on this computer takes no key.".into()));
        assert_eq!(keys.set("openai", FAKE.into()), Ok(()));
        assert!(keys.has("openai"));
        let mut s = Settings::default();
        choose(&mut s, "openai", Some("m".into()), true, 1).unwrap();
        let mut errors = vec![choose(&mut s, "nope", Some(FAKE.into()), true, 1).unwrap_err(), keys.set("nope", FAKE.into()).unwrap_err()];
        let look = gather(&[], ready_codex, |id| keys.has(id), None);
        let list = serde_json::to_string(&providers(&s, &look)).unwrap();
        let file = std::env::temp_dir().join(format!("pc-catalog-{}-{nanos}.json", std::process::id()));
        s.save(&file).unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        let _ = std::fs::remove_file(&file);
        assert_eq!(forget(&mut s, &keys, "openai"), Ok(()));
        assert!(!keys.has("openai") && s.assist_provider.is_none() && s.assist_model.is_none() && s.assist_consent.is_empty());
        errors.push(picked(&s).unwrap_err());
        let back: serde_json::Value = serde_json::from_str(&saved).unwrap();
        assert_eq!(back["assist_provider"], "openai", "the scan read the saved settings");
        for text in [saved.as_str(), list.as_str()].into_iter().chain(errors.iter().map(String::as_str)) {
            assert!(!text.contains(FAKE), "{text}");
        }
        assert!(list.contains("Key saved."), "the list read the planted key's item: {list}");
    }

    /// How long each probe of this computer takes, printed: `cargo test --lib time_each_probe -- --ignored --nocapture`.
    /// Spends nothing (versions, `codex login status`, the store, the local ports).
    #[test]
    #[ignore]
    fn time_each_probe_on_this_computer() {
        let t = std::time::Instant::now();
        let found = super::super::detect::codex();
        println!("probe codex --version: {:?} {:?}", t.elapsed(), found.as_ref().map(|f| f.version));
        let t = std::time::Instant::now();
        let signed = found.as_ref().ok().and_then(|f| super::super::detect::signed_in(&f.exe));
        println!("probe codex login status: {:?} {signed:?}", t.elapsed());
        for a in acp::AGENTS.iter().chain([&CLAUDE_CLI]) {
            let t = std::time::Instant::now();
            let h = acp::health(a);
            println!("probe {} --version: {:?} {:?}", a.id, t.elapsed(), h.state);
        }
        let t = std::time::Instant::now();
        let local = openai::detect_local();
        println!("probe local ports: {:?} {}", t.elapsed(), local.len());
        let t = std::time::Instant::now();
        let keys = ROUTES.iter().filter(|r| r.way == Way::Key && vault::has(r.id)).count();
        println!("probe store keys: {:?} {keys}", t.elapsed());
        let t = std::time::Instant::now();
        let held = Held::with(Box::new(|_| Err("unused".into())));
        let _ = providers(&Settings::default(), &look(&held));
        println!("whole look (the old first answer): {:?}", t.elapsed());
        let held = Arc::new(held);
        let survey = Arc::new(Survey::default());
        let t = std::time::Instant::now();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        survey.refresh(probes(held.clone(), std::env::temp_dir().join("pc-probe-timing")), false, Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        }));
        let _ = survey.now(&Settings::default(), vault::has, held.held());
        println!("survey first answer (the new first answer): {:?}", t.elapsed());
        while survey.busy() {
            let _ = rx.recv_timeout(std::time::Duration::from_secs(20));
        }
        println!("survey every route filled: {:?}", t.elapsed());
    }

    #[test]
    fn the_first_answer_never_waits_on_a_slow_probe_and_each_route_fills_in_as_its_probe_answers() {
        use std::time::{Duration, Instant};
        let survey = Arc::new(Survey::default());
        let mut s = Settings::default();
        let slow = Probe {
            ids: vec![codex::ID],
            run: Box::new(|| {
                std::thread::sleep(Duration::from_millis(1500));
                Box::new(|l: &mut Look| l.codex = Some(ready_codex()))
            }),
        };
        let quick = Probe {
            ids: vec!["ollama"],
            run: Box::new(|| Box::new(|l: &mut Look| l.local.push(openai::Found { name: "Ollama", base_url: openai::LOCAL[0].1.into(), models: vec!["m".into()] }))),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        let t = Instant::now();
        assert!(survey.refresh(vec![slow, quick], false, Arc::new(move || {
            let _ = tx.lock().unwrap().send(());
        })));
        let first = survey.now(&s, |_| false, None);
        assert!(t.elapsed() < Duration::from_millis(300), "the first answer waited: {:?}", t.elapsed());
        let state = |l: &List, id: &str| l.providers.iter().find(|r| r.id == id).unwrap().state;
        assert_eq!((state(&first, "codex"), state(&first, "gemini-cli"), state(&first, "lmstudio")), (Status::Checking, Status::Checking, Status::Checking));
        assert_ne!(state(&first, "openai"), Status::Checking, "a key is read at once");
        assert_eq!(state(&first, CLAUDE_CODE), Status::Unsupported, "waiting for the owner's OK is known at once");
        // "Only on this computer" changes the list at once, while the slow probe is still out.
        s.assist_offline_only = true;
        let t = Instant::now();
        assert!(survey.now(&s, |_| false, None).offline_only && t.elapsed() < Duration::from_millis(300));
        assert!(survey.busy());
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let mid = survey.now(&s, |_| false, None);
        assert_eq!((state(&mid, "ollama"), state(&mid, "codex")), (Status::Ready, Status::Checking), "the quick probe fills in first");
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        while survey.busy() {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(state(&survey.now(&s, |_| false, None), "codex"), Status::Ready);
        // Kept for the session: a new round only when asked or once it is stale.
        assert!(!survey.refresh(Vec::new(), false, Arc::new(|| ())), "once per run: opening the pane asks nothing");
        assert!(survey.refresh(Vec::new(), true, Arc::new(|| ())));
    }

    #[test]
    fn only_codex_offers_an_effort_and_the_choice_is_kept_for_it_alone() {
        let mut s = Settings::default();
        choose(&mut s, "codex", None, true, 1).unwrap();
        assert_eq!(picked(&s).unwrap().effort, None);
        assert_eq!(set_effort(&mut s, "high"), Ok(()));
        assert_eq!((picked(&s).unwrap().effort.as_deref(), providers(&s, &Look::default()).effort.as_deref()), (Some("high"), Some("high")));
        assert!(set_effort(&mut s, "extreme").is_err());
        let list = providers(&s, &Look::default());
        for r in &list.providers {
            assert_eq!(r.efforts.is_empty(), r.id != codex::ID, "{}", r.id);
        }
        choose(&mut s, "ollama", Some("m".into()), true, 2).unwrap();
        assert_eq!((picked(&s).unwrap().effort, providers(&s, &Look::default()).effort), (None, None), "a route without the choice sends none");
        assert!(set_effort(&mut s, "low").is_err());
    }

    #[test]
    fn the_codex_row_names_the_plan_and_the_address_codex_says_it_is_signed_in_with() {
        let look = Look { codex: Some(ready_codex()), account: Some(codex::Account { plan: Some("plus".into()), api_key: false, email: Some("someone@example.test".into()) }), ..Look::default() };
        let row = providers(&Settings::default(), &look).providers.remove(0);
        assert_eq!((row.state, row.plan.as_deref(), row.email.as_deref()), (Status::Ready, Some("plus"), Some("someone@example.test")));
        let signed_out = Look { codex: Some((ready_codex().0, Some(false))), ..look };
        let row = providers(&Settings::default(), &signed_out).providers.remove(0);
        assert_eq!((row.state, row.plan, row.email), (Status::SignedOut, None, None), "nobody named once signed out");
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn on_linux_a_key_is_refused_and_forgetting_still_clears_the_choice() {
        let keys = Keys::named("pointcast-test-linux/");
        assert!(keys.set("openai", FAKE.into()).unwrap_err().contains("not supported on Linux yet"));
        let mut s = Settings::default();
        choose(&mut s, "openai", Some("m".into()), true, 1).unwrap();
        assert_eq!(forget(&mut s, &keys, "openai"), Ok(()));
        assert!(s.assist_provider.is_none() && s.assist_consent.is_empty());
    }
}
