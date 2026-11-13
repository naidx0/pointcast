//! A subscription through a vendor's own agent that speaks the Agent Client Protocol (JSON-RPC 2.0 over its stdio):
//! the Gemini CLI, GitHub Copilot's CLI and OpenCode, each found on this computer and signed in by the person through
//! that vendor's own CLI. Pointcast never sees a token. Like Codex, each is a whole coding agent, so the conversation
//! is fenced in to question and answer: Pointcast offers it no files and no terminal, starts the session in an empty
//! folder of its own with no MCP servers, rejects every permission it asks for, refuses every other request, and a
//! turn that starts a tool anyway is cancelled and failed as refused.

use super::child::{self, Launch, Proc, Tail};
use super::jsonl::{CallError, Client, Incoming, RpcError};
use super::catalog::Choice;
use super::claude::{Computer, Lease};
use super::{detect, replay, Caps, Chat, Event, Failure, Health, Kind, Mime, Part, Provider, Role, State, Turn, TurnId};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// An agent Pointcast can speak to, and the constant command that starts it as an ACP server.
#[derive(Debug, PartialEq)]
pub struct Agent {
    pub id: &'static str,
    /// What people call it.
    pub name: &'static str,
    /// What its command-line tool is called.
    pub cli: &'static str,
    pub program: &'static str,
    pub args: &'static [&'static str],
    pub goes_to: &'static str,
}

/// The agents offered. Checked offline: `copilot --help` of GitHub Copilot CLI 1.0.105 lists `--acp` ("Start as Agent
/// Client Protocol server"), and no `--stdio` (ACP over stdio is what `--acp` starts). UNCONFIRMED, since neither is installed here: the Gemini CLI's `--experimental-acp` and
/// `opencode acp`, as their documentation named them.
pub const AGENTS: &[Agent] = &[
    Agent { id: "gemini-cli", name: "Gemini", cli: "Gemini CLI", program: "gemini", args: &["--experimental-acp"], goes_to: "Google, through your Gemini CLI sign-in" },
    Agent { id: "copilot", name: "GitHub Copilot", cli: "GitHub Copilot CLI", program: "copilot", args: &["--acp"], goes_to: "GitHub, through your Copilot plan" },
    Agent { id: "opencode", name: "OpenCode", cli: "OpenCode", program: "opencode", args: &["acp"], goes_to: "the AI you set up in OpenCode" },
];

pub fn agent(id: &str) -> Option<&'static Agent> {
    AGENTS.iter().find(|a| a.id == id)
}

/// Every method Pointcast sends. The sender refuses anything else, so no later change can reach for a sign-in
/// (`authenticate`) without a test failing.
pub const SENDS: &[&str] = &["initialize", "session/new", "session/set_config_option", "session/set_model", "session/prompt", "session/cancel"];

/// The value an agent may list for "its own choice": never sent.
pub const DEFAULT: &str = "default";

/// The models and thinking efforts an agent offers, as its `session/new` reported them: `configOptions` of category
/// "model" and "thought_level", else (an older agent) its `models` list. The recommended one of each is the value the
/// session started with. A list of one is no choice and is left empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    pub models: Vec<Choice>,
    pub efforts: Vec<Choice>,
    /// The config ids they are set by; a model without one is set by `session/set_model`.
    pub model_id: Option<String>,
    pub effort_id: Option<String>,
    /// Whether its `initialize` said it takes an MCP server over HTTP (`mcpCapabilities.http`), which is how it is
    /// given Pointcast's computer tools; none before it was greeted.
    #[serde(default)]
    pub http: Option<bool>,
}

/// A select's options, groups flattened and each value once, labelled by its name.
fn choices_of(options: &Value, current: Option<&str>) -> Vec<Choice> {
    let mut all: Vec<Choice> = Vec::new();
    for o in options.as_array().into_iter().flatten() {
        let inner: Vec<&Value> = match o["options"].as_array() {
            Some(group) => group.iter().collect(),
            None => vec![o],
        };
        for c in inner {
            let Some(id) = c["value"].as_str().or(c["modelId"].as_str()).filter(|v| !v.is_empty()) else { continue };
            if !all.iter().any(|a| a.id == id) {
                all.push(Choice { id: id.to_owned(), label: c["name"].as_str().filter(|n| !n.is_empty()).unwrap_or(id).to_owned(), default: current == Some(id) });
            }
        }
    }
    all
}

/// What a `session/new` (or `session/set_config_option`) result offers.
fn options_of(made: &Value) -> Options {
    let mut o = Options::default();
    for c in made["configOptions"].as_array().into_iter().flatten().filter(|c| c["type"].as_str().is_none_or(|t| t == "select")) {
        let (Some(id), current) = (c["id"].as_str(), c["currentValue"].as_str()) else { continue };
        match c["category"].as_str() {
            Some("model") if o.model_id.is_none() => (o.model_id, o.models) = (Some(id.to_owned()), choices_of(&c["options"], current)),
            Some("thought_level") if o.effort_id.is_none() => (o.effort_id, o.efforts) = (Some(id.to_owned()), choices_of(&c["options"], current)),
            _ => {}
        }
    }
    if o.model_id.is_none() {
        o.models = choices_of(&made["models"]["availableModels"], made["models"]["currentModelId"].as_str());
    }
    for l in [&mut o.models, &mut o.efforts] {
        if l.len() < 2 {
            l.clear();
        }
    }
    o
}

/// The file the last options an agent reported are kept in, under its folder of Pointcast's own.
fn options_file(home: &Path) -> PathBuf {
    home.join("options.json")
}

/// The options agent `home` last reported, so Settings shows them without starting a session; none before one.
pub fn cached(home: &Path) -> Options {
    std::fs::read(options_file(home)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// The version of the protocol Pointcast speaks.
pub const PROTOCOL: u64 = 1;

const NO_TOOLS: &str = "You are answering a person's question inside Pointcast, a study tool. No tools exist here: you cannot run commands, read or change files, search the web or call any tool, so never try. Answer only from the words and pictures of the conversation.";

const START_WITHIN: Duration = Duration::from_secs(30);
const ANSWER_WITHIN: Duration = Duration::from_secs(30);

/// How long a turn has to end once it was cancelled, before the agent itself is ended.
pub const CANCEL_GRACE: Duration = Duration::from_millis(1500);

/// The pictures of how many earlier questions ride with a resumed conversation.
pub const KEEP_PICTURES: usize = 2;

/// What Pointcast tells the agent it can do: nothing on the computer.
fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL,
        "clientCapabilities": { "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false },
        "clientInfo": { "name": "pointcast", "title": "Pointcast", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn blocks(parts: &[Part]) -> Result<Vec<Value>, Failure> {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => Ok(json!({ "type": "text", "text": t })),
            Part::Picture { path, mime, .. } => {
                let bytes = std::fs::read(path).map_err(|_| Failure::Broken("A picture of this question could not be read.".into()))?;
                let kind = match mime {
                    Mime::Png => "image/png",
                    Mime::Jpeg => "image/jpeg",
                };
                Ok(json!({ "type": "image", "mimeType": kind, "data": base64::engine::general_purpose::STANDARD.encode(bytes) }))
            }
        })
        .collect()
}

fn has_picture(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Picture { .. }))
}

/// The answer to a permission the agent asks for: its reject option, else no choice at all.
fn reject(params: &Value) -> Value {
    let options = params["options"].as_array().cloned().unwrap_or_default();
    let pick = ["reject_once", "reject_always"].iter().find_map(|k| options.iter().find(|o| o["kind"].as_str() == Some(k)));
    match pick.and_then(|o| o["optionId"].as_str()) {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => json!({ "outcome": { "outcome": "cancelled" } }),
    }
}

/// The answer to a permission for one of Pointcast's computer tools: its allow-once option, else a rejection. Never
/// "always": each call is asked about again.
fn allow(params: &Value) -> Value {
    let options = params["options"].as_array().cloned().unwrap_or_default();
    match options.iter().find(|o| o["kind"].as_str() == Some("allow_once")).and_then(|o| o["optionId"].as_str()) {
        Some(id) => json!({ "outcome": { "outcome": "selected", "optionId": id } }),
        None => reject(params),
    }
}

/// The name Pointcast's own MCP server goes by in a session that may act. Not "pointcast": the agent's own settings may
/// give that name to the same server (with this install's token, for coding).
pub const SERVER: &str = super::codex::SERVER;

/// Whether a tool call an agent reports or asks permission for is one of the ten computer tools on Pointcast's own
/// server. ACP has no field for a tool's name, so each agent says it its own way: an agent's own tool (a shell, an edit,
/// a fetch) is told apart by its `kind`, and Pointcast's tool by a name the agent gives an MCP tool, as its title (whole,
/// or before " (") or the start of its id (before "-"), or in its `_meta`. UNCONFIRMED, since no agent here has been
/// given a server: which of these each agent uses.
fn pointcast_call(call: &Value) -> bool {
    if !matches!(call["kind"].as_str(), None | Some("other")) {
        return false;
    }
    let forms = |n: &str| [n.to_owned(), format!("{SERVER}__{n}"), format!("mcp__{SERVER}__{n}"), format!("{SERVER}_{n}"), format!("{SERVER}/{n}"), format!("{SERVER}.{n}")];
    // Whether `said` is one of those names whole, or followed by `after`.
    let names = |said: &str, after: Option<&str>| {
        let fits = |f: &str| said == f || after.is_some_and(|a| said.strip_prefix(f).is_some_and(|rest| rest.starts_with(a)));
        crate::computer::tools::NAMES.iter().any(|n| forms(n).iter().any(|f| fits(f)))
    };
    let meta = &call["_meta"];
    let in_meta = [&meta["toolName"], &meta["tool_name"], &meta["claudeCode"]["toolName"]].iter().filter_map(|v| v.as_str()).any(|t| names(t, None));
    in_meta || call["title"].as_str().is_some_and(|t| names(t, Some(" ("))) || call["toolCallId"].as_str().is_some_and(|t| names(t, Some("-")))
}

/// Pointcast's server as `session/new` carries it: over HTTP, with the turn's grant secret as the key.
fn servers_of(lease: &Lease) -> Value {
    json!([{ "type": "http", "name": SERVER, "url": lease.url, "headers": [{ "name": "Authorization", "value": format!("Bearer {}", lease.key) }] }])
}

/// What an error the agent answered with means for the person.
fn failure_of(e: &RpcError) -> Failure {
    let low = e.message.to_lowercase();
    let says = |words: &[&str]| words.iter().any(|w| low.contains(w));
    if says(&["auth", "sign in", "signed in", "log in", "login"]) {
        Failure::SignedOut
    } else if says(&["quota", "rate limit", "too many requests", "usage limit", "429"]) {
        Failure::Limit { resets: None }
    } else if says(&["context length", "context window", "too long", "too many tokens"]) {
        Failure::ContextFull
    } else if e.message.is_empty() {
        Failure::Broken(format!("The agent refused the question ({}).", e.code))
    } else {
        Failure::Broken(e.message.clone())
    }
}

fn stopped(name: &str, said: &str) -> String {
    if said.is_empty() {
        format!("{name} stopped before it finished.")
    } else {
        format!("{name} stopped before it finished. It said: {said}")
    }
}

/// The turn being answered.
struct Active {
    id: TurnId,
    out: Sender<Event>,
    /// The id of our `session/prompt`.
    call: u64,
    /// It was cancelled: nothing more of it is shown, and this is how it ends.
    stopping: Option<Failure>,
    thought: bool,
    /// The session it runs in: a turn that may act has one of its own.
    session: String,
    /// The turn's hold on the computer tools, when it may act: let go before the turn says how it ended.
    lease: Option<Lease>,
    /// The ids of the calls of Pointcast's tools the agent made in this turn.
    tools: HashSet<String>,
    /// The answer so far, for a session started later to be told; `kept` once it has been.
    said: String,
    kept: bool,
}

impl Active {
    fn new(id: TurnId, out: Sender<Event>, call: u64, session: String, lease: Option<Lease>) -> Active {
        Active { id, out, call, stopping: None, thought: false, session, lease, tools: HashSet::new(), said: String::new(), kept: false }
    }

    fn tell(&self, kind: Kind) {
        let _ = self.out.send(Event { turn: self.id, kind });
    }

    /// The turn ends as `kind`: the computer is let go first.
    fn end(mut self, kind: Kind) {
        self.lease.take();
        self.tell(kind);
    }
}

/// A turn asked while another still ran: it is sent once that one has ended.
struct Queued {
    id: TurnId,
    out: Sender<Event>,
    params: Value,
    session: String,
    lease: Option<Lease>,
}

#[derive(Default)]
struct Inner {
    session: String,
    active: Option<Active>,
    next: Option<Queued>,
    serial: u64,
    dead: bool,
    /// The conversation so far, which a session started for a turn that may act is told.
    history: Vec<Turn>,
}

impl Inner {
    /// Keep what `a` has answered, once, for a session started later.
    fn keep(&mut self, a: &mut Active) {
        if !a.kept && !a.said.is_empty() {
            self.history.push(Turn { role: Role::Assistant, parts: vec![Part::Text(a.said.clone())] });
        }
        a.kept = true;
    }
}

struct Shared {
    rpc: Client,
    name: &'static str,
    inner: Mutex<Inner>,
    proc: Mutex<Option<Proc>>,
    tail: Option<Arc<Tail>>,
}

impl Shared {
    fn allows(method: &str) -> std::io::Result<()> {
        if SENDS.contains(&method) {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("Pointcast does not send {method}")))
        }
    }

    fn call(&self, method: &str, params: Value, within: Duration) -> Result<Value, Failure> {
        Self::allows(method).map_err(|e| Failure::Broken(e.to_string()))?;
        self.rpc.call(method, params, within).map_err(|e| match e {
            CallError::Rpc(e) => failure_of(&e),
            CallError::Slow => Failure::Broken(format!("{} did not answer {method} in time.", self.name)),
            CallError::Closed => Failure::Broken(stopped(self.name, &self.last_words())),
        })
    }

    fn post_as(&self, id: u64, method: &str, params: Value) -> std::io::Result<()> {
        Self::allows(method)?;
        self.rpc.post_as(id, method, params)
    }

    fn cancel_turn(&self, session: &str) {
        let _ = Self::allows("session/cancel").and_then(|_| self.rpc.notify_with("session/cancel", json!({ "sessionId": session })));
    }

    fn last_words(&self) -> String {
        self.tail.as_ref().map(|t| t.settled(Duration::from_millis(500))).unwrap_or_default()
    }

    fn run(self: Arc<Self>, incoming: Receiver<Incoming>) {
        for m in incoming {
            match m {
                Incoming::Request { id, method, params } => {
                    let _ = if method == "session/request_permission" {
                        self.rpc.answer(&id, if self.permits(&params) { allow(&params) } else { reject(&params) })
                    } else {
                        self.rpc.refuse(&id, -32601, "Pointcast offers no files, terminal or tools.")
                    };
                }
                Incoming::Notification { method, params } if method == "session/update" => self.updated(params["sessionId"].as_str(), &params["update"]),
                Incoming::Notification { .. } => {}
                Incoming::Reply { id, result } => self.replied(id, result),
                Incoming::Closed => break,
            }
        }
        self.gone();
    }

    /// Whether a permission the agent asks for passes: only for a call of one of Pointcast's computer tools, asked in the
    /// session of the turn under way, which holds the tools and is not being cancelled.
    fn permits(&self, p: &Value) -> bool {
        let mut s = self.inner.lock().unwrap();
        let Some(a) = s.active.as_mut() else { return false };
        if a.lease.is_none() || a.stopping.is_some() || p["sessionId"].as_str() != Some(a.session.as_str()) {
            return false;
        }
        let call = &p["toolCall"];
        let id = call["toolCallId"].as_str().unwrap_or_default();
        let ours = (!id.is_empty() && a.tools.contains(id)) || pointcast_call(call);
        if ours && !id.is_empty() {
            a.tools.insert(id.to_owned());
        }
        ours
    }

    /// The agent is gone: whatever was being answered, or waited to be, fails with what it said last.
    fn gone(&self) {
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            s.dead = true;
            (s.active.take(), s.next.take())
        };
        if a.is_none() && q.is_none() {
            return;
        }
        let why = stopped(self.name, &self.last_words());
        if let Some(a) = a {
            let kind = Kind::Failed(a.stopping.clone().unwrap_or(Failure::Broken(why.clone())));
            a.end(kind);
        }
        if let Some(q) = q {
            drop(q.lease);
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Broken(why)) });
        }
    }

    fn send_next(&self, q: Option<Queued>) {
        let Some(q) = q else { return };
        let call = self.rpc.reserve();
        self.inner.lock().unwrap().active = Some(Active::new(q.id, q.out, call, q.session, q.lease));
        if self.post_as(call, "session/prompt", q.params).is_err() {
            let a = self.inner.lock().unwrap().active.take_if(|a| a.call == call);
            if let Some(a) = a {
                a.end(Kind::Failed(Failure::Broken(stopped(self.name, &self.last_words()))));
            }
        }
    }

    fn replied(&self, id: u64, result: Result<Value, RpcError>) {
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            if !s.active.as_ref().is_some_and(|a| a.call == id) {
                return;
            }
            let mut a = s.active.take();
            if let Some(a) = a.as_mut() {
                s.keep(a);
            }
            (a, s.next.take())
        };
        let Some(a) = a else { return };
        let end = match (a.stopping.clone(), result) {
            (Some(why), _) => Kind::Failed(why),
            (None, Ok(v)) => match v["stopReason"].as_str() {
                Some("refusal" | "max_turn_requests") => Kind::Failed(Failure::Refused),
                Some("cancelled") => Kind::Failed(Failure::Stopped),
                _ => Kind::Done,
            },
            (None, Err(e)) => Kind::Failed(failure_of(&e)),
        };
        a.end(end);
        self.send_next(q);
    }

    fn updated(self: &Arc<Self>, session: Option<&str>, u: &Value) {
        let mut s = self.inner.lock().unwrap();
        let Some(a) = s.active.as_mut() else { return };
        // A word of an earlier session is not this turn's.
        if a.stopping.is_some() || session.is_some_and(|id| id != a.session) {
            return;
        }
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                if let Some(t) = u["content"]["text"].as_str().filter(|t| !t.is_empty()) {
                    if !a.kept {
                        a.said.push_str(t);
                    }
                    a.tell(Kind::Delta(t.to_owned()));
                }
            }
            Some("agent_thought_chunk") if !a.thought => {
                a.thought = true;
                a.tell(Kind::Thinking);
            }
            // One of Pointcast's computer tools, in a turn that holds them: the agent is working, which keeps the panel's
            // wait alive.
            Some("tool_call" | "tool_call_update") if a.lease.is_some() && (u["toolCallId"].as_str().is_some_and(|id| a.tools.contains(id)) || pointcast_call(u)) => {
                if let Some(id) = u["toolCallId"].as_str().filter(|id| !id.is_empty()) {
                    a.tools.insert(id.to_owned());
                }
                if !a.thought {
                    a.thought = true;
                    a.tell(Kind::Thinking);
                }
            }
            // The agent began to act on the computer any other way: cancel the turn, which ends refused.
            Some("tool_call" | "tool_call_update") => {
                drop(s);
                self.stop_for(Failure::Refused, false);
            }
            _ => {}
        }
    }

    /// Cancel the answer under way, to end as `why`, and end the agent if it will not stop. A turn that waited
    /// behind it is stopped with it, unless it is what stops the first (`keep_next`).
    fn stop_for(self: &Arc<Self>, why: Failure, keep_next: bool) {
        let mut s = self.inner.lock().unwrap();
        let q = if keep_next { None } else { s.next.take() };
        if let Some(q) = q {
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Stopped) });
        }
        let Some(a) = s.active.as_mut() else { return };
        if a.stopping.is_some() {
            return;
        }
        a.stopping = Some(why);
        // The computer first, at once: a tool call after a stop is refused.
        let lease = a.lease.take();
        let (id, session) = (a.id, a.session.clone());
        drop(s);
        drop(lease);
        self.cancel_turn(&session);
        let me = self.clone();
        std::thread::spawn(move || {
            let still = |s: &Inner| s.active.as_ref().is_some_and(|a| a.id == id);
            let t = Instant::now();
            while t.elapsed() < CANCEL_GRACE {
                std::thread::sleep(Duration::from_millis(20));
                if !still(&me.inner.lock().unwrap()) {
                    return;
                }
            }
            // It did not stop: end the agent, then say so.
            let (a, q) = {
                let mut s = me.inner.lock().unwrap();
                if !still(&s) {
                    return;
                }
                s.dead = true;
                (s.active.take(), s.next.take())
            };
            me.rpc.hang_up();
            let p = me.proc.lock().unwrap().take();
            if let Some(mut p) = p {
                p.end();
            }
            if let Some(a) = a {
                let kind = Kind::Failed(a.stopping.clone().unwrap_or(Failure::Stopped));
                a.end(kind);
            }
            if let Some(q) = q {
                drop(q.lease);
                let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Broken(format!("{} would not stop, so Pointcast ended it. Ask again.", me.name))) });
            }
        });
    }

    /// End the conversation: what was being answered ends as stopped, stdin closes, and the agent gets `grace` to
    /// leave before it and its tree are ended.
    fn shut(&self, grace: Duration) {
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            s.dead = true;
            (s.active.take(), s.next.take())
        };
        if let Some(a) = a {
            a.end(Kind::Failed(Failure::Stopped));
        }
        if let Some(q) = q {
            drop(q.lease);
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Stopped) });
        }
        self.rpc.hang_up();
        let p = self.proc.lock().unwrap().take();
        if let Some(mut p) = p {
            p.end_within(grace);
        }
    }
}

/// What a session is started with: the instructions, what a resumed conversation said, and the model and effort the
/// person chose (none leaves the agent's own).
pub struct Start<'a> {
    pub system: &'a str,
    pub earlier: &'a [Turn],
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    /// The conversation may see and act, when the agent can carry Pointcast's server; none keeps the fence of today.
    pub computer: Option<Computer>,
}

/// Start a session in `cwd` with the MCP servers `servers`, and set the model, then the effort, each only when chosen,
/// offered by this session and not what it started with. Its id, and what it offered before anything was set.
fn session_in(shared: &Shared, cwd: &Path, servers: Value, model: Option<&str>, effort: Option<&str>) -> Result<(String, Options), Failure> {
    let made = shared.call("session/new", json!({ "cwd": cwd.to_string_lossy(), "mcpServers": servers }), ANSWER_WITHIN)?;
    let session = made["sessionId"].as_str().unwrap_or_default().to_owned();
    if session.is_empty() {
        return Err(Failure::Broken(format!("{} started no conversation.", shared.name)));
    }
    let offered = options_of(&made);
    let mut now = offered.clone();
    if let Some(m) = model.filter(|m| *m != DEFAULT && now.models.iter().any(|c| c.id == *m && !c.default)) {
        let set = match &now.model_id {
            Some(id) => shared.call("session/set_config_option", json!({ "sessionId": session, "configId": id, "value": m }), ANSWER_WITHIN)?,
            None => shared.call("session/set_model", json!({ "sessionId": session, "modelId": m }), ANSWER_WITHIN)?,
        };
        // A model can change which efforts there are: the agent's answer says them all again.
        if set["configOptions"].is_array() {
            now = options_of(&set);
        }
    }
    if let (Some(e), Some(id)) = (effort.filter(|e| *e != DEFAULT && now.efforts.iter().any(|c| c.id == *e && !c.default)), &now.effort_id) {
        shared.call("session/set_config_option", json!({ "sessionId": session, "configId": id, "value": e }), ANSWER_WITHIN)?;
    }
    Ok((session, offered))
}

/// What goes before a session's first question: `fence` with the instructions, then what was said before.
fn preamble_of(fence: &str, system: &str, earlier: &[Turn], pictures: bool) -> Vec<Part> {
    let fence = if system.trim().is_empty() { fence.to_owned() } else { format!("{fence}\n\n{}", system.trim()) };
    let mut preamble = vec![Part::Text(fence)];
    preamble.extend(replay(earlier, if pictures { KEEP_PICTURES } else { 0 }));
    preamble
}

/// A conversation with an ACP agent: greeted, with a session of its own.
pub struct AcpChat {
    shared: Arc<Shared>,
    /// Whether the agent said it reads pictures.
    pub pictures: bool,
    /// The models and efforts its `session/new` offered, before anything was set.
    pub options: Options,
    /// What goes before the first question: the fence, the instructions and what a resumed conversation said.
    preamble: Vec<Part>,
    /// The conversation may act (and the agent can carry Pointcast's server): each turn holds a grant and runs in a
    /// session of its own, given that grant's key.
    computer: Option<Computer>,
    /// What each session that may act is started with.
    cwd: PathBuf,
    system: String,
    model: Option<String>,
    effort: Option<String>,
    /// Where the options a session offered are kept for Settings, once one has started.
    cache: Option<PathBuf>,
}

impl AcpChat {
    /// Greet an agent over these streams and start a session in `cwd`. Nothing is asked and nothing is spent.
    pub fn over<R: Read + Send + 'static, W: Write + Send + 'static>(
        reader: R,
        writer: W,
        proc: Option<Proc>,
        name: &'static str,
        cwd: &Path,
        start: Start,
    ) -> Result<AcpChat, Failure> {
        let Start { system, earlier, model, effort, computer } = start;
        let (rpc, incoming) = Client::rpc2(reader, writer);
        let tail = proc.as_ref().map(Proc::tail);
        let shared = Arc::new(Shared { rpc, name, inner: Mutex::default(), proc: Mutex::new(proc), tail });
        let pump = shared.clone();
        std::thread::spawn(move || pump.run(incoming));
        let greet = || -> Result<(bool, String, Options, Option<Computer>), Failure> {
            let hello = shared.call("initialize", initialize(), START_WITHIN)?;
            let pictures = hello["agentCapabilities"]["promptCapabilities"]["image"].as_bool() == Some(true);
            // Pointcast's server goes over HTTP, so only an agent that says it takes one can be given it. One that does
            // not answers as if the setting were off, and the AI page says it can't act yet.
            let http = hello["agentCapabilities"]["mcpCapabilities"]["http"].as_bool() == Some(true);
            let computer = computer.filter(|_| http);
            // One that may act starts its session with its first turn, once that turn holds the tools: the session is
            // given the turn's key as it starts.
            if computer.is_some() {
                return Ok((pictures, String::new(), Options { http: Some(http), ..Options::default() }, computer));
            }
            let (session, offered) = session_in(&shared, cwd, json!([]), model, effort)?;
            Ok((pictures, session, Options { http: Some(http), ..offered }, None))
        };
        match greet() {
            Ok((pictures, session, options, computer)) => {
                {
                    let mut s = shared.inner.lock().unwrap();
                    s.session = session;
                    s.history = earlier.to_vec();
                }
                let preamble = if computer.is_some() { Vec::new() } else { preamble_of(NO_TOOLS, system, earlier, pictures) };
                Ok(AcpChat {
                    shared,
                    pictures,
                    options,
                    preamble,
                    computer,
                    cwd: cwd.to_owned(),
                    system: system.to_owned(),
                    model: model.map(str::to_owned),
                    effort: effort.map(str::to_owned),
                    cache: None,
                })
            }
            Err(f) => {
                shared.shut(Duration::ZERO);
                Err(f)
            }
        }
    }

    pub fn pid(&self) -> Option<u32> {
        self.shared.proc.lock().unwrap().as_ref().map(Proc::pid)
    }
}

impl Chat for AcpChat {
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure> {
        if !self.pictures && has_picture(&turn.parts) {
            return Err(Failure::NoPictures);
        }
        // A turn that may act holds the tools from before its session can list them until it ends. An earlier turn's
        // session was given that turn's key, which opens nothing now: this turn gets a session of its own, given the new
        // key and told the conversation so far (with what an answer still coming has said).
        let lease = match &self.computer {
            None => None,
            Some(c) => {
                let lease = c.lease()?;
                let earlier = {
                    let mut s = self.shared.inner.lock().unwrap();
                    if s.dead {
                        return Err(Failure::Broken(format!("{} is not running.", self.shared.name)));
                    }
                    let mut a = s.active.take();
                    if let Some(a) = a.as_mut() {
                        s.keep(a);
                    }
                    s.active = a;
                    s.history.clone()
                };
                let (session, offered) = session_in(&self.shared, &self.cwd, servers_of(&lease), self.model.as_deref(), self.effort.as_deref())?;
                self.options = Options { http: Some(true), ..offered };
                if let (Some(f), Ok(b)) = (&self.cache, serde_json::to_vec(&self.options)) {
                    let _ = std::fs::write(f, b);
                }
                self.shared.inner.lock().unwrap().session = session;
                self.preamble = preamble_of(super::claude::ACTING, &self.system, &earlier, self.pictures);
                Some(lease)
            }
        };
        let mut prompt = blocks(&self.preamble)?;
        prompt.extend(blocks(&turn.parts)?);
        let call = self.shared.rpc.reserve();
        let mut s = self.shared.inner.lock().unwrap();
        if s.dead || s.session.is_empty() {
            return Err(Failure::Broken(format!("{} is not running.", self.shared.name)));
        }
        let session = s.session.clone();
        let params = json!({ "sessionId": session, "prompt": prompt });
        s.serial += 1;
        let id = TurnId(s.serial);
        self.preamble.clear();
        s.history.push(turn);
        if s.active.is_some() {
            // An answer still comes: cancel it, and send this turn once the agent says that one has ended.
            if let Some(old) = s.next.replace(Queued { id, out, params, session, lease }) {
                let _ = old.out.send(Event { turn: old.id, kind: Kind::Failed(Failure::Stopped) });
            }
            drop(s);
            self.shared.stop_for(Failure::Stopped, true);
            return Ok(id);
        }
        s.active = Some(Active::new(id, out, call, session, lease));
        drop(s);
        if self.shared.post_as(call, "session/prompt", params).is_err() {
            let mut s = self.shared.inner.lock().unwrap();
            if s.active.take_if(|a| a.call == call).is_some() {
                s.history.pop();
                return Err(Failure::Broken(stopped(self.shared.name, &self.shared.last_words())));
            }
        }
        Ok(id)
    }

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move || shared.stop_for(Failure::Stopped, false))
    }

    fn close(&mut self) {
        self.shared.shut(child::CLOSE_GRACE);
    }

    /// The preamble without its fence (the instructions, not anything said before).
    fn context(&self) -> Vec<Part> {
        self.preamble.iter().skip(1).cloned().collect()
    }
}

impl Drop for AcpChat {
    fn drop(&mut self) {
        self.shared.shut(Duration::ZERO);
    }
}

/// The folder an agent is started in: Pointcast's own, and empty, so there is no project for it to read.
fn empty_folder(home: &Path, name: &str) -> Result<PathBuf, Failure> {
    let d = home.join("empty");
    std::fs::create_dir_all(&d).map_err(|e| Failure::Broken(format!("Pointcast could not make its folder for {name}: {e}")))?;
    if std::fs::read_dir(&d).map(|mut l| l.next().is_some()).unwrap_or(true) {
        return Err(Failure::Broken(format!("The folder Pointcast keeps empty for {name} has files in it: {}", d.display())));
    }
    Ok(d)
}

/// How `a` is started: the program found on this computer, with its constant arguments.
pub fn launch_of(a: &Agent) -> Option<Launch> {
    let exe = detect::on_path(a.program)?;
    Some(Launch { exe, args: a.args.iter().map(|s| s.to_string()).collect(), env: Vec::new() })
}

/// Whether `a` is installed and which version, in one line. Spends nothing: its `--version` only. Whether the person
/// is signed in is not asked, since none of these says so without reaching its service.
pub fn health(a: &Agent) -> Health {
    let (state, line) = match detect::on_path(a.program) {
        None => (State::NotInstalled, format!("{} is not installed on this computer. Choose Install.", a.cli)),
        Some(exe) => match detect::version_of(&exe) {
            Ok(said) => {
                let v = detect::version_in(&said).map(|(x, y, z)| format!(" {x}.{y}.{z}")).unwrap_or_default();
                (State::Ready, format!("{}{v} is installed.", a.cli))
            }
            Err(why) => (State::Unknown, format!("{} is installed, but Pointcast could not read its version: {why}.", a.cli)),
        },
    };
    Health { state, line, goes_to: a.goes_to }
}

pub struct Acp {
    agent: &'static Agent,
    /// A folder of Pointcast's own: the empty folder and the pid file live in it.
    home: PathBuf,
    launch: Option<Launch>,
    /// Whether the agent said it reads pictures, once one has been greeted.
    pictures: Mutex<Option<bool>>,
    /// The model and effort the person chose; none leaves the agent's own.
    model: Option<String>,
    effort: Option<String>,
    /// Each turn may see and act through this, when the agent can carry Pointcast's server; none keeps the fence.
    computer: Option<Computer>,
}

impl Acp {
    pub fn new(agent: &'static Agent, home: PathBuf) -> Acp {
        Acp { agent, home, launch: None, pictures: Mutex::default(), model: None, effort: None, computer: None }
    }

    /// The same, letting each turn see and act through `computer`; none keeps the fence of today.
    pub fn acting(self, computer: Option<Computer>) -> Acp {
        Acp { computer, ..self }
    }

    /// The same, with the model and effort the person chose.
    pub fn choosing(self, model: Option<String>, effort: Option<String>) -> Acp {
        Acp { model, effort, ..self }
    }

    /// Run `launch` in place of the program found on this computer.
    pub fn with(agent: &'static Agent, launch: Launch, home: PathBuf) -> Acp {
        Acp { launch: Some(launch), ..Acp::new(agent, home) }
    }

    pub fn start(&self, system: &str, earlier: &[Turn]) -> Result<AcpChat, Failure> {
        let launch = self.launch.clone().or_else(|| launch_of(self.agent)).ok_or(Failure::NotInstalled)?;
        let cwd = empty_folder(&self.home, self.agent.name)?;
        let pidfile = self.home.join(format!("{}.pid", self.agent.id));
        child::reap_stale(&pidfile);
        let (proc, stdin, stdout) = Proc::start(&launch, &cwd, Some(&pidfile)).map_err(|e| Failure::Broken(format!("{} did not start: {e}", self.agent.name)))?;
        let start = Start { system, earlier, model: self.model.as_deref(), effort: self.effort.as_deref(), computer: self.computer.clone() };
        let mut chat = AcpChat::over(stdout, stdin, Some(proc), self.agent.name, &cwd, start)?;
        *self.pictures.lock().unwrap() = Some(chat.pictures);
        // Kept for Settings; a cache that cannot be written only means the pane waits for the next session. One that may
        // act has no session yet: its first turn's session writes it.
        chat.cache = Some(options_file(&self.home));
        if chat.computer.is_none() {
            if let Ok(b) = serde_json::to_vec(&chat.options) {
                let _ = std::fs::write(options_file(&self.home), b);
            }
        }
        Ok(chat)
    }
}

impl Provider for Acp {
    fn id(&self) -> &'static str {
        self.agent.id
    }

    /// Pictures are assumed until the agent says otherwise: one that cannot take them refuses one before it is sent.
    fn caps(&self, _model: Option<&str>) -> Caps {
        Caps { images: self.pictures.lock().unwrap().unwrap_or(true), max_images: 4, max_edge: 2048, offline: false }
    }

    fn health(&self) -> Health {
        health(self.agent)
    }

    fn warm(&self) {}

    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
        self.start(system, earlier).map(|c| Box::new(c) as Box<dyn Chat>)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::assist::child::tests::gone;
    use crate::assist::tests::{collect, endings, said, scratch, shot, words};
    use crate::assist::Role;
    use std::io::BufRead;
    use std::sync::mpsc::channel;

    type Seen = Arc<Mutex<Vec<Value>>>;

    pub const WORDS: [&str; 3] = ["Two ", "plus two ", "is four."];
    pub const LAST_WORDS: &str = "fake_acp: out of cheese";
    const CWD: &str = "/pointcast/empty";
    const SESSION: &str = "sess-1";
    /// The ids of the requests the stand-in makes of Pointcast.
    const PERMISSION: u64 = 900;
    const READ_FILE: u64 = 901;

    /// What the stand-in offers in "options" mode: a grouped model select, a thinking select with a "default", and a
    /// mode select that is neither.
    static OPTIONS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| {
        json!([
            { "type": "select", "id": "mode", "name": "Mode", "category": "mode", "currentValue": "agent", "options": [{ "value": "agent", "name": "Agent" }, { "value": "plan", "name": "Plan" }] },
            { "type": "select", "id": "model", "name": "Model", "category": "model", "currentValue": "pro-1", "options": [{ "group": "g", "name": "Google", "options": [{ "value": "pro-1", "name": "Pro 1" }, { "value": "flash-1", "name": "Flash 1" }] }, { "value": "flash-1", "name": "Flash 1" }] },
            { "type": "select", "id": "thinking", "name": "Thinking", "category": "thought_level", "currentValue": "default", "options": [{ "value": "default", "name": "Default" }, { "value": "low", "name": "Low" }, { "value": "high", "name": "High" }] },
        ])
    });

    pub enum Exit {
        Eof,
        Crash,
    }

    fn update(say: &dyn Fn(Value), kind: &str, more: Value) {
        update_in(say, SESSION, kind, more)
    }

    fn update_in(say: &dyn Fn(Value), session: &str, kind: &str, more: Value) {
        let mut u = json!({ "sessionUpdate": kind });
        for (k, v) in more.as_object().cloned().unwrap_or_default() {
            u[k] = v;
        }
        say(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": session, "update": u } }));
    }

    /// The tool calls and permissions of a turn that may act, in "acting" mode, as what it is asked says. A permission
    /// is asked by its id here; the turn ends once every one is answered (`finish`).
    fn act(say: &dyn Fn(Value), session: &str, asked: &str) -> (Vec<u64>, bool) {
        let options = json!([{ "optionId": "yes", "name": "Allow", "kind": "allow_once" }, { "optionId": "always", "name": "Always", "kind": "allow_always" }, { "optionId": "no", "name": "Reject", "kind": "reject_once" }]);
        let permit = |id: u64, in_session: &str, call: Value| say(json!({ "jsonrpc": "2.0", "id": id, "method": "session/request_permission", "params": { "sessionId": in_session, "toolCall": call, "options": options } }));
        if asked.contains("bash") {
            update_in(say, session, "tool_call", json!({ "toolCallId": "shell-1", "title": "ls", "kind": "execute", "status": "pending" }));
            return (Vec::new(), false);
        }
        let ours = json!({ "toolCallId": "pointcast_apps-1", "title": "pointcast_apps (pointcast_turn MCP Server)", "kind": "other", "status": "pending" });
        update_in(say, session, "tool_call", ours);
        if asked.contains("slowly") {
            update_in(say, session, "agent_message_chunk", json!({ "content": { "type": "text", "text": WORDS[0] } }));
            return (Vec::new(), false);
        }
        permit(920, session, json!({ "toolCallId": "pointcast_apps-1" }));
        permit(921, session, json!({ "toolCallId": "shell-2", "title": "pointcast_click (pointcast_turn MCP Server)", "kind": "execute" }));
        permit(922, session, json!({ "toolCallId": "other-1", "title": "rm -rf ~", "kind": "other" }));
        permit(923, "sess-earlier", json!({ "toolCallId": "pointcast_click-9", "title": "pointcast_click", "kind": "other" }));
        update_in(say, session, "tool_call_update", json!({ "toolCallId": "pointcast_apps-1", "status": "completed" }));
        (vec![920, 921, 922, 923], true)
    }

    fn chunk(say: &dyn Fn(Value), kind: &str, text: &str) {
        update(say, kind, json!({ "content": { "type": "text", "text": text } }));
    }

    /// A stand-in ACP agent, as `mode` says, until its end of the line closes. `saw` is shown every message sent to it.
    pub fn serve(mode: &str, input: impl BufRead, out: impl Write + Send + 'static, mut saw: impl FnMut(&Value)) -> Exit {
        let out = Mutex::new(out);
        let say = move |v: Value| {
            let mut o = out.lock().unwrap();
            let _ = writeln!(o, "{v}");
            let _ = o.flush();
        };
        let mut pending: Option<Value> = None;
        // Like a real agent, whose tool call waits on its permission, a cancelled turn is answered only once
        // Pointcast has answered every request the stand-in made of it.
        let mut asked: Vec<u64> = Vec::new();
        let mut cancelled = false;
        // In "acting" mode: the session made last, and a turn that ends once its permissions are answered.
        let (mut sessions, mut session) = (0u32, SESSION.to_owned());
        let mut finish: Option<Value> = None;
        for line in input.lines() {
            let Ok(line) = line else { break };
            let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
            saw(&m);
            let id = m.get("id").cloned().unwrap_or(Value::Null);
            if m.get("method").is_none() {
                asked.retain(|a| id != *a);
            }
            let reply = |result: Value| say(json!({ "jsonrpc": "2.0", "id": id, "result": result }));
            match m["method"].as_str() {
                Some("initialize") if mode == "acting" => {
                    reply(json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": false, "promptCapabilities": { "image": true }, "mcpCapabilities": { "http": true, "sse": false } }, "authMethods": [] }))
                }
                Some("initialize") => {
                    reply(json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": false, "promptCapabilities": { "image": mode != "noimage" } }, "authMethods": [] }))
                }
                Some("session/new") if mode == "acting" => {
                    sessions += 1;
                    session = format!("sess-{sessions}");
                    reply(json!({ "sessionId": session }));
                }
                Some("session/new") if mode == "signedout" => say(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Authentication required" } })),
                Some("session/new") if mode == "options" => reply(json!({ "sessionId": SESSION, "configOptions": OPTIONS.clone() })),
                Some("session/new") if mode == "old-models" => reply(json!({ "sessionId": SESSION, "models": { "currentModelId": "auto", "availableModels": [{ "modelId": "auto", "name": "Auto" }, { "modelId": "auto", "name": "Auto" }, { "modelId": "fast-1", "name": "Fast 1" }] } })),
                Some("session/new") => reply(json!({ "sessionId": SESSION })),
                Some("session/set_config_option") => reply(json!({ "configOptions": OPTIONS.clone() })),
                Some("session/set_model") => reply(json!({})),
                Some("session/prompt") => match mode {
                    "acting" => {
                        let asked_text = m["params"]["prompt"].as_array().and_then(|p| p.last()).and_then(|b| b["text"].as_str()).unwrap_or_default().to_owned();
                        let (ids, ends) = act(&say, &session, &asked_text);
                        asked = ids;
                        if ends {
                            finish = Some(id.clone());
                        } else {
                            pending = Some(id.clone());
                        }
                    }
                    "tool" => {
                        chunk(&say, "agent_message_chunk", "Let me look");
                        update(&say, "tool_call", json!({ "toolCallId": "t1", "title": "Read file", "kind": "read", "status": "pending" }));
                        let options = json!([{ "optionId": "yes", "name": "Allow", "kind": "allow_once" }, { "optionId": "no", "name": "Reject", "kind": "reject_once" }]);
                        say(json!({ "jsonrpc": "2.0", "id": PERMISSION, "method": "session/request_permission", "params": { "sessionId": SESSION, "toolCall": { "toolCallId": "t1" }, "options": options } }));
                        say(json!({ "jsonrpc": "2.0", "id": READ_FILE, "method": "fs/read_text_file", "params": { "sessionId": SESSION, "path": "/etc/hosts" } }));
                        asked = vec![PERMISSION, READ_FILE];
                        pending = Some(id.clone());
                    }
                    "slow" => {
                        chunk(&say, "agent_message_chunk", "first");
                        pending = Some(id.clone());
                    }
                    "crash" => {
                        chunk(&say, "agent_message_chunk", "half");
                        return Exit::Crash;
                    }
                    _ => {
                        chunk(&say, "agent_thought_chunk", "Adding.");
                        chunk(&say, "agent_thought_chunk", "Still adding.");
                        update(&say, "plan", json!({ "entries": [{ "content": "add", "priority": "high", "status": "pending" }] }));
                        for w in WORDS {
                            chunk(&say, "agent_message_chunk", w);
                        }
                        reply(json!({ "stopReason": "end_turn" }));
                    }
                },
                Some("session/cancel") => cancelled = true,
                _ => {}
            }
            if asked.is_empty() {
                if let Some(p) = finish.take() {
                    for w in WORDS {
                        update_in(&say, &session, "agent_message_chunk", json!({ "content": { "type": "text", "text": w } }));
                    }
                    say(json!({ "jsonrpc": "2.0", "id": p, "result": { "stopReason": "end_turn" } }));
                }
            }
            if cancelled && asked.is_empty() {
                cancelled = false;
                if let Some(p) = pending.take() {
                    say(json!({ "jsonrpc": "2.0", "id": p, "result": { "stopReason": "cancelled" } }));
                }
            }
        }
        Exit::Eof
    }

    /// The stand-in served inside this process, greeted and with its session, and what it was sent.
    fn wire_with(mode: &'static str, earlier: &[Turn]) -> (Result<AcpChat, Failure>, Seen) {
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            serve(mode, std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone()));
        });
        (AcpChat::over(our_in, our_out, None, "Gemini", Path::new(CWD), Start { system: "Be brief.", earlier, model: None, effort: None, computer: None }), seen)
    }

    fn wire(mode: &'static str) -> (AcpChat, Seen) {
        let (chat, seen) = wire_with(mode, &[]);
        (chat.ok().expect("the stand-in greets and starts a session"), seen)
    }

    fn sent(seen: &Seen, method: &str) -> Vec<Value> {
        seen.lock().unwrap().iter().filter(|m| m["method"].as_str() == Some(method)).cloned().collect()
    }

    fn answer(chat: &mut dyn Chat, turn: Turn) -> Vec<Kind> {
        let (tx, rx) = channel();
        chat.ask(turn, tx).unwrap();
        collect(&rx)
    }

    /// The stand-in as a real child: this test program run again as `stand_in` below.
    fn child(mode: &str, name: &str) -> Acp {
        let launch = Launch {
            exe: std::env::current_exe().unwrap(),
            args: ["--exact", "assist::acp::tests::stand_in", "--ignored", "--nocapture"].map(String::from).to_vec(),
            env: vec![("POINTCAST_FAKE_ACP".into(), mode.into())],
        };
        Acp::with(&AGENTS[0], launch, scratch(name))
    }

    /// Acts as the agent when a test starts this program as its helper. Does nothing when run with the rest.
    #[test]
    #[ignore]
    fn stand_in() {
        let Ok(mode) = std::env::var("POINTCAST_FAKE_ACP") else { return };
        // On one test thread the harness leaves "test <name> ... " unfinished on stdout: end that line first.
        println!();
        match serve(&mode, std::io::stdin().lock(), std::io::stdout(), |_| {}) {
            Exit::Crash => {
                eprintln!("{LAST_WORDS}");
                std::process::exit(1);
            }
            Exit::Eof => std::process::exit(0),
        }
    }

    #[test]
    fn the_handshake_offers_no_files_no_terminal_and_no_servers_and_the_prompt_carries_the_fence_and_the_picture() {
        let (mut chat, seen) = wire("ok");
        let d = scratch("acp-shape");
        let bytes = [1u8, 2, 3];
        std::fs::write(d.join("q.png"), bytes).unwrap();
        let picture = Part::Picture { path: d.join("q.png"), w: 2, h: 2, mime: Mime::Png };
        let got = answer(&mut chat, Turn { role: Role::Person, parts: vec![Part::Text("What is it?".into()), picture] });
        assert_eq!(got.last(), Some(&Kind::Done));
        let all = seen.lock().unwrap().clone();
        assert!(all.iter().all(|m| m["jsonrpc"] == "2.0"), "strict JSON-RPC 2.0: {all:?}");
        let methods: Vec<&str> = all.iter().filter_map(|m| m["method"].as_str()).collect();
        assert_eq!(methods, ["initialize", "session/new", "session/prompt"]);
        let hello = &all[0]["params"];
        assert_eq!(hello["protocolVersion"], PROTOCOL);
        assert_eq!(hello["clientCapabilities"], json!({ "fs": { "readTextFile": false, "writeTextFile": false }, "terminal": false }));
        assert_eq!(all[1]["params"], json!({ "cwd": CWD, "mcpServers": [] }));
        let p = &all[2]["params"];
        assert_eq!(p["sessionId"], SESSION);
        let prompt = p["prompt"].as_array().unwrap();
        let fence = prompt[0]["text"].as_str().unwrap();
        assert!(fence.starts_with(NO_TOOLS) && fence.ends_with("Be brief."), "{fence}");
        assert_eq!(prompt[1], json!({ "type": "text", "text": "What is it?" }));
        assert_eq!(prompt[2], json!({ "type": "image", "mimeType": "image/png", "data": base64::engine::general_purpose::STANDARD.encode(bytes) }));
        // The fence and the instructions go once: the agent keeps the session.
        answer(&mut chat, said("again"));
        assert_eq!(sent(&seen, "session/prompt")[1]["params"]["prompt"], json!([{ "type": "text", "text": "again" }]));
    }

    #[test]
    fn the_chosen_model_and_effort_are_set_before_the_first_prompt_and_none_is_sent_when_none_is_chosen() {
        let agent = |mode: &str, name: &str, model: Option<&str>, effort: Option<&str>| {
            let a = child(mode, name).choosing(model.map(str::to_owned), effort.map(str::to_owned));
            let mut chat = a.start("", &[]).unwrap();
            assert_eq!(words(&answer(&mut chat, said("hi"))), WORDS);
            chat.close();
            (chat.options.clone(), cached(&a.home))
        };
        // Through the real child, with what it was sent read back from its log.
        let (options, kept) = agent("options", "acp-options", Some("flash-1"), Some("high"));
        fn ids(l: &[Choice]) -> Vec<(&str, &str, bool)> {
            l.iter().map(|c| (c.id.as_str(), c.label.as_str(), c.default)).collect()
        }
        assert_eq!(ids(&options.models), [("pro-1", "Pro 1", true), ("flash-1", "Flash 1", false)]);
        assert_eq!(ids(&options.efforts), [("default", "Default", true), ("low", "Low", false), ("high", "High", false)]);
        assert_eq!(kept, options, "Settings reads what the agent last offered without a session");

        let (chat, seen) = {
            let (their_in, our_out) = std::io::pipe().unwrap();
            let (our_in, their_out) = std::io::pipe().unwrap();
            let seen = Seen::default();
            let log = seen.clone();
            std::thread::spawn(move || serve("options", std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone())));
            (AcpChat::over(our_in, our_out, None, "Gemini", Path::new(CWD), Start { system: "", earlier: &[], model: Some("flash-1"), effort: Some("high"), computer: None }), seen)
        };
        let mut chat = chat.unwrap();
        answer(&mut chat, said("hi"));
        let methods: Vec<String> = seen.lock().unwrap().iter().filter_map(|m| m["method"].as_str().map(str::to_owned)).collect();
        assert_eq!(methods, ["initialize", "session/new", "session/set_config_option", "session/set_config_option", "session/prompt"]);
        let set: Vec<Value> = sent(&seen, "session/set_config_option").iter().map(|m| m["params"].clone()).collect();
        assert_eq!(set, [json!({ "sessionId": SESSION, "configId": "model", "value": "flash-1" }), json!({ "sessionId": SESSION, "configId": "thinking", "value": "high" })]);

        // Nothing chosen, "default" chosen, the value it started with, or one it does not offer: nothing is set.
        for (model, effort) in [(None, None), (Some("pro-1"), Some(DEFAULT)), (Some("gone-1"), Some("ultra"))] {
            let (their_in, our_out) = std::io::pipe().unwrap();
            let (our_in, their_out) = std::io::pipe().unwrap();
            let seen = Seen::default();
            let log = seen.clone();
            std::thread::spawn(move || serve("options", std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone())));
            let mut chat = AcpChat::over(our_in, our_out, None, "Gemini", Path::new(CWD), Start { system: "", earlier: &[], model, effort, computer: None }).unwrap();
            answer(&mut chat, said("hi"));
            assert!(sent(&seen, "session/set_config_option").is_empty() && sent(&seen, "session/set_model").is_empty(), "{model:?} {effort:?}");
        }

        // An older agent lists `models` only: duplicates are one choice, and the model is set by `session/set_model`.
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || serve("old-models", std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone())));
        let chat = AcpChat::over(our_in, our_out, None, "Copilot", Path::new(CWD), Start { system: "", earlier: &[], model: Some("fast-1"), effort: Some("high"), computer: None }).unwrap();
        assert_eq!(ids(&chat.options.models), [("auto", "Auto", true), ("fast-1", "Fast 1", false)]);
        assert!(chat.options.efforts.is_empty());
        assert_eq!(sent(&seen, "session/set_model").iter().map(|m| m["params"].clone()).collect::<Vec<_>>(), [json!({ "sessionId": SESSION, "modelId": "fast-1" })]);
        assert!(sent(&seen, "session/set_config_option").is_empty());
        // An agent that offers nothing has nothing to pick.
        assert_eq!(options_of(&json!({ "sessionId": "s" })), Options::default());
        assert_eq!(options_of(&json!({ "configOptions": [{ "type": "select", "id": "model", "category": "model", "currentValue": "auto", "options": [{ "value": "auto", "name": "Auto" }, { "value": "auto", "name": "Auto" }] }] })).models, []);
    }

    #[test]
    fn chunks_stream_in_order_thoughts_show_once_as_thinking_a_plan_is_ignored_and_the_answer_ends_once() {
        let (mut chat, _) = wire("ok");
        let got = answer(&mut chat, said("count"));
        assert_eq!(got[0], Kind::Thinking);
        assert_eq!(got.iter().filter(|k| **k == Kind::Thinking).count(), 1);
        assert_eq!(words(&got), WORDS);
        assert_eq!((got.last(), endings(&got)), (Some(&Kind::Done), 1));
    }

    #[test]
    fn a_tool_call_cancels_the_turn_which_fails_refused_its_permission_is_rejected_and_a_file_request_refused() {
        let (mut chat, seen) = wire("tool");
        let got = answer(&mut chat, said("look at my files"));
        assert_eq!(got, [Kind::Delta("Let me look".into()), Kind::Failed(Failure::Refused)]);
        assert_eq!(sent(&seen, "session/cancel"), [json!({ "jsonrpc": "2.0", "method": "session/cancel", "params": { "sessionId": SESSION } })]);
        let all = seen.lock().unwrap().clone();
        let to = |id: u64| all.iter().find(|m| m["id"] == id && m.get("method").is_none()).cloned().unwrap_or_else(|| panic!("no answer to {id}: {all:?}"));
        assert_eq!(to(PERMISSION)["result"], json!({ "outcome": { "outcome": "selected", "optionId": "no" } }));
        assert_eq!(to(READ_FILE)["error"]["code"], -32601);
        assert_eq!(reject(&json!({ "options": [{ "optionId": "y", "kind": "allow_always" }] })), json!({ "outcome": { "outcome": "cancelled" } }));
    }

    #[test]
    fn a_cancel_ends_the_turn_stopped_and_a_newer_question_is_sent_after_it() {
        let (mut chat, seen) = wire("slow");
        let (tx, rx) = channel();
        let first = chat.ask(said("one"), tx).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).map(|e| e.kind), Ok(Kind::Delta("first".into())));
        let stop = chat.stopper();
        std::thread::spawn(move || stop()).join().unwrap();
        assert_eq!(collect(&rx), [Kind::Failed(Failure::Stopped)]);
        assert_eq!(sent(&seen, "session/cancel").len(), 1);
        // A newer question stops the one under way, and goes once that one has ended.
        let (tx, rx) = channel();
        let second = chat.ask(said("two"), tx.clone()).unwrap();
        let third = chat.ask(said("three"), tx).unwrap();
        assert!(first < second && second < third);
        let stop = chat.stopper();
        let mut got = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(10)) {
            // The slow stand-in ends a turn only when it is cancelled.
            if e.turn == third && e.kind == Kind::Delta("first".into()) {
                stop();
            }
            let end = e.turn == third && e.kind.ends();
            got.push(e);
            if end {
                break;
            }
        }
        let of = |t: TurnId| got.iter().filter(|e| e.turn == t).map(|e| e.kind.clone()).collect::<Vec<_>>();
        assert_eq!(of(second).last(), Some(&Kind::Failed(Failure::Stopped)), "{got:?}");
        assert_eq!(of(third), [Kind::Delta("first".into()), Kind::Failed(Failure::Stopped)]);
        assert_eq!(sent(&seen, "session/prompt").len(), 3, "the third went once the second had ended");
        assert_eq!(sent(&seen, "session/cancel").len(), 3);
    }

    #[test]
    fn an_agent_without_pictures_refuses_one_before_anything_is_sent() {
        let (mut chat, seen) = wire("noimage");
        let (tx, _rx) = channel();
        assert_eq!(chat.ask(Turn { role: Role::Person, parts: vec![shot("a.png")] }, tx), Err(Failure::NoPictures));
        assert!(sent(&seen, "session/prompt").is_empty());
    }

    #[test]
    fn a_signed_out_agent_says_so_and_a_resumed_conversation_rides_with_the_first_question() {
        assert!(matches!(wire_with("signedout", &[]).0, Err(Failure::SignedOut)));
        let earlier = [said("before"), Turn { role: Role::Assistant, parts: vec![Part::Text("Yes.".into())] }];
        let (chat, seen) = wire_with("ok", &earlier);
        let mut chat = chat.ok().unwrap();
        answer(&mut chat, said("now"));
        let prompt = sent(&seen, "session/prompt")[0]["params"]["prompt"].clone();
        assert!(prompt[1]["text"].as_str().unwrap().contains("You: before"), "{prompt}");
        assert_eq!(prompt[2]["text"], "now");
        assert_eq!(failure_of(&RpcError { code: 429, message: "Quota exceeded".into() }), Failure::Limit { resets: None });
        assert_eq!(failure_of(&RpcError { code: 1, message: "prompt is too long".into() }), Failure::ContextFull);
    }

    #[test]
    fn a_crashed_agent_fails_once_with_what_it_said_last_and_none_is_left_after_close() {
        let p = child("crash", "acp-crash");
        let mut chat = p.start("", &[]).ok().expect("the stand-in starts");
        let got = answer(&mut chat, said("anything"));
        assert_eq!(endings(&got), 1, "{got:?}");
        let Some(Kind::Failed(Failure::Broken(why))) = got.last() else { panic!("{got:?}") };
        assert!(why.contains(LAST_WORDS) && why.starts_with("Gemini stopped"), "{why}");

        let p = child("ok", "acp-close");
        let mut chat = p.start("", &[]).ok().expect("the stand-in starts");
        assert_eq!(words(&answer(&mut chat, said("count"))), WORDS);
        assert!(p.caps(None).images);
        let pid = chat.pid().unwrap();
        chat.close();
        assert!(gone(pid), "no agent is left after close");
    }

    #[test]
    fn each_agent_is_started_with_constant_arguments_and_a_missing_one_says_so() {
        let ids: Vec<&str> = AGENTS.iter().map(|a| a.id).collect();
        assert_eq!(ids, ["gemini-cli", "copilot", "opencode"]);
        for a in AGENTS {
            assert!(!a.args.is_empty() && !a.goes_to.is_empty() && agent(a.id) == Some(a));
        }
        let missing = Agent { id: "x", name: "Nobody", cli: "Nobody CLI", program: "pointcast-no-such-program", args: &["acp"], goes_to: "nowhere" };
        let h = health(&missing);
        assert_eq!((h.state, h.line.as_str(), h.goes_to), (State::NotInstalled, "Nobody CLI is not installed on this computer. Choose Install.", "nowhere"));
        let leaked: &'static Agent = Box::leak(Box::new(missing));
        assert!(matches!(Acp::new(leaked, scratch("acp-missing")).start("", &[]), Err(Failure::NotInstalled)));
        assert!(Shared::allows("authenticate").is_err() && Shared::allows("session/prompt").is_ok());
    }

    // Computer use: "Let the AI see and act".

    #[test]
    fn with_computer_use_off_the_wire_is_byte_for_byte_what_it_was() {
        let (mut chat, seen) = wire("ok");
        assert_eq!(answer(&mut chat, said("What is it?")).last(), Some(&Kind::Done));
        let all = seen.lock().unwrap().clone();
        let line = |i: usize| {
            let mut m = all[i].clone();
            m.as_object_mut().unwrap().remove("id");
            m.to_string()
        };
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(line(0), format!(r#"{{"jsonrpc":"2.0","method":"initialize","params":{{"clientCapabilities":{{"fs":{{"readTextFile":false,"writeTextFile":false}},"terminal":false}},"clientInfo":{{"name":"pointcast","title":"Pointcast","version":"{version}"}},"protocolVersion":1}}}}"#));
        assert_eq!(line(1), r#"{"jsonrpc":"2.0","method":"session/new","params":{"cwd":"/pointcast/empty","mcpServers":[]}}"#);
        assert_eq!(line(2), format!(r#"{{"jsonrpc":"2.0","method":"session/prompt","params":{{"prompt":[{{"text":"{NO_TOOLS}\n\nBe brief.","type":"text"}},{{"text":"What is it?","type":"text"}}],"sessionId":"sess-1"}}}}"#));
        assert_eq!(all.len(), 3);
        assert!(Acp::new(agent("gemini-cli").unwrap(), "unused".into()).computer.is_none(), "off unless the setting is on");
    }

    /// A conversation with the stand-in in `mode` that may act, and its desk.
    fn acting(mode: &'static str) -> (AcpChat, Seen, &'static crate::computer::Desk) {
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            serve(mode, std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone()));
        });
        let (c, desk) = crate::assist::codex::tests::computer();
        let start = Start { system: "Be brief.", earlier: &[], model: None, effort: None, computer: Some(c) };
        (AcpChat::over(our_in, our_out, None, "Gemini", Path::new(CWD), start).unwrap(), seen, desk)
    }

    /// The key each session was given, in order. Never printed.
    fn keys(seen: &Seen) -> Vec<String> {
        let auth = |m: &Value| m["params"]["mcpServers"][0]["headers"][0]["value"].as_str().map(|a| a.trim_start_matches("Bearer ").to_owned());
        sent(seen, "session/new").iter().filter_map(auth).collect()
    }

    /// What Pointcast answered the request `id` of the stand-in.
    fn answered(seen: &Seen, id: u64) -> Value {
        let all = seen.lock().unwrap().clone();
        all.into_iter().find(|m| m.get("method").is_none() && m["id"] == id).unwrap_or_else(|| panic!("{id} was answered"))["result"].clone()
    }

    fn picked(option: &str) -> Value {
        json!({ "outcome": { "outcome": "selected", "optionId": option } })
    }

    #[test]
    fn a_turn_that_may_act_gets_a_session_of_its_own_given_pointcast_over_http_and_only_its_tool_permissions_pass() {
        let (mut chat, seen, desk) = acting("acting");
        assert_eq!(sent(&seen, "session/new").len(), 0, "no session before the turn");
        assert!(!desk.granted() && chat.options.http == Some(true));
        let got = answer(&mut chat, said("open TextEdit"));
        assert_eq!(got.first(), Some(&Kind::Thinking), "a computer tool reads as working: {got:?}");
        assert_eq!((words(&got), got.last()), (WORDS.map(String::from).to_vec(), Some(&Kind::Done)), "and is not refused: {got:?}");
        assert!(!desk.granted(), "the grant goes with the turn");
        let made = &sent(&seen, "session/new")[0]["params"];
        let key = keys(&seen)[0].clone();
        assert!(key.len() == 64 && key.bytes().all(|c| c.is_ascii_hexdigit()), "the grant's secret, 32 bytes as hex");
        let want = json!({ "cwd": CWD, "mcpServers": [{ "type": "http", "name": "pointcast_turn", "url": "http://127.0.0.1:9/mcp", "headers": [{ "name": "Authorization", "value": format!("Bearer {key}") }] }] });
        assert_eq!(made, &want);
        let prompt = sent(&seen, "session/prompt")[0]["params"].clone();
        assert_eq!(prompt["sessionId"], "sess-1");
        let fence = prompt["prompt"][0]["text"].as_str().unwrap();
        assert!(fence.starts_with(crate::assist::claude::ACTING) && fence.ends_with("Be brief.") && !fence.contains("No tools exist"), "{fence}");
        // Only a call of Pointcast's tool, in the turn's own session, passes, and only once.
        assert_eq!(answered(&seen, 920), picked("yes"));
        for id in [921, 922, 923] {
            assert_eq!(answered(&seen, id), picked("no"), "{id}");
        }

        // The next turn: a new grant, and a new session given its key and told the conversation so far.
        assert_eq!(answer(&mut chat, said("and type hello")).last(), Some(&Kind::Done));
        let both = keys(&seen);
        assert_eq!(both.len(), 2);
        assert_ne!(both[0], both[1], "each turn's key is its own");
        let second = sent(&seen, "session/prompt")[1]["params"].clone();
        assert_eq!(second["sessionId"], "sess-2");
        let text: String = second["prompt"].as_array().unwrap().iter().filter_map(|b| b["text"].as_str()).collect();
        assert!(text.starts_with(crate::assist::claude::ACTING) && text.contains("open TextEdit") && text.contains(&WORDS.concat()) && text.ends_with("and type hello"), "{text}");
    }

    #[test]
    fn a_stale_key_opens_nothing_once_a_new_turn_holds_the_tools_and_a_stop_drops_the_grant_at_once() {
        let (mut chat, seen, desk) = acting("acting");
        assert_eq!(answer(&mut chat, said("open TextEdit")).last(), Some(&Kind::Done));
        let (tx, rx) = channel();
        chat.ask(said("type slowly"), tx).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).map(|e| e.kind), Ok(Kind::Thinking));
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).map(|e| e.kind), Ok(Kind::Delta(WORDS[0].into())));
        let k = keys(&seen);
        assert!(desk.granted());
        assert!(desk.grants.by_secret(k[0].as_bytes()).is_none(), "the first turn's key is refused");
        assert!(desk.grants.by_secret(k[1].as_bytes()).is_some(), "the turn under way holds its own");
        (chat.stopper())();
        assert!(!desk.granted() && desk.grants.by_secret(k[1].as_bytes()).is_none(), "a stop drops the grant at once");
        assert_eq!(collect(&rx), [Kind::Failed(Failure::Stopped)]);
        assert_eq!(sent(&seen, "session/cancel").last().unwrap()["params"], json!({ "sessionId": "sess-2" }));
    }

    #[test]
    fn in_a_turn_that_may_act_any_other_tool_still_ends_it_as_refused() {
        let (mut chat, _seen, desk) = acting("acting");
        assert_eq!(answer(&mut chat, said("run bash")), [Kind::Failed(Failure::Refused)]);
        assert!(!desk.granted());
    }

    #[test]
    fn an_agent_that_cannot_take_a_server_over_http_is_asked_as_if_the_setting_were_off() {
        let (mut chat, seen, desk) = acting("ok");
        assert_eq!(chat.options.http, Some(false), "the AI page says it can't act yet");
        assert_eq!(answer(&mut chat, said("open TextEdit")).last(), Some(&Kind::Done));
        assert!(!desk.granted());
        assert_eq!(sent(&seen, "session/new")[0]["params"], json!({ "cwd": CWD, "mcpServers": [] }));
        assert!(sent(&seen, "session/prompt")[0]["params"]["prompt"][0]["text"].as_str().unwrap().starts_with(NO_TOOLS));
    }

    #[test]
    fn a_tool_call_is_pointcasts_only_when_an_mcp_name_of_one_of_the_ten_tools_says_so_and_its_kind_is_not_the_agents_own() {
        for ours in [
            json!({ "toolCallId": "c1", "title": "pointcast_click", "kind": "other" }),
            json!({ "toolCallId": "c1", "title": "pointcast_click (pointcast_turn MCP Server)", "kind": "other" }),
            json!({ "toolCallId": "pointcast_type-1731", "title": "{\"text\":\"hi\"}", "kind": "other" }),
            json!({ "toolCallId": "c1", "title": "pointcast_turn_pointcast_apps" }),
            json!({ "toolCallId": "c1", "title": "mcp__pointcast_turn__pointcast_screen" }),
            json!({ "toolCallId": "c1", "title": "Screen", "_meta": { "claudeCode": { "toolName": "mcp__pointcast_turn__pointcast_screen" } } }),
        ] {
            assert!(pointcast_call(&ours), "{ours}");
        }
        for theirs in [
            json!({ "toolCallId": "c1", "title": "pointcast_click", "kind": "execute" }),
            json!({ "toolCallId": "c1", "title": "pointcast_click (pointcast_turn MCP Server)", "kind": "edit" }),
            json!({ "toolCallId": "c1", "title": "echo pointcast_click", "kind": "other" }),
            json!({ "toolCallId": "c1", "title": "pointcast_clicker", "kind": "other" }),
            json!({ "toolCallId": "c1", "title": "pointcast_click; rm -rf ~", "kind": "other" }),
            json!({ "toolCallId": "c1", "title": "pointcast_latest", "kind": "other" }),
            json!({ "toolCallId": "notes_pointcast_click-1", "kind": "other" }),
            json!({ "toolCallId": "c1" }),
        ] {
            assert!(!pointcast_call(&theirs), "{theirs}");
        }
    }
}
