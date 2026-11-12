//! The person's ChatGPT plan through OpenAI's own Codex CLI, driven over `codex app-server`. Pointcast starts the
//! CLI the person installed and signed in to, asks it the question and reads the answer as it streams; it never
//! sees a token. Codex is a whole coding agent, so the conversation is fenced in to question and answer: a thread
//! that is not saved, a read-only sandbox, an empty folder of Pointcast's own, the tool features and MCP servers
//! turned off, OpenAI's own provider whatever the person's config says, every request Codex makes of us declined
//! at once, and a turn that starts a tool anyway stopped and failed as refused.

use super::child::{self, Launch, Proc, Tail};
use super::jsonl::{CallError, Client, Incoming, RpcError};
use super::{detect, replay, Caps, Chat, Event, Failure, Health, Kind, Limits, Part, Provider, Turn, TurnId, Usage, Window};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ID: &str = "codex";

/// Every method Pointcast sends. The sender refuses anything else, so no later change can reach for a sign-in
/// call without a test failing.
pub const SENDS: &[&str] = &["initialize", "initialized", "account/read", "model/list", "config/read", "thread/start", "turn/start", "turn/interrupt", "account/rateLimits/read"];

/// Every method a sign-in sends, on a line of its own that never asks anything: the greeting, and the start of Codex's
/// own ChatGPT sign-in, whose page the browser shows.
pub const SIGN_IN: &[&str] = &["initialize", "initialized", "account/login/start"];

/// The features of Codex that act on the computer or reach beyond the conversation, by their names in
/// `codex features list` (0.164.0).
pub const FEATURES_OFF: &[&str] = &[
    "apps", "browser_use", "computer_use", "hooks", "image_generation", "multi_agent", "shell_tool", "unified_exec", "view_image", "plugins",
    "sleep_tool", "tool_suggest", "skill_search", "memories", "goals",
];

/// The kinds of item that mean Codex is doing something other than answering.
const ACTS: &[&str] = &["commandExecution", "fileChange", "mcpToolCall", "dynamicToolCall", "collabAgentToolCall", "webSearch", "imageGeneration", "imageView"];

/// The provider a question must go to for what the person agreed to stay true.
const OPENAI: &str = "openai";

const NO_TOOLS: &str = "You are answering a person's question inside Pointcast, a study tool. No tools exist here: you cannot run commands, read or change files, search the web or call any tool, so never try. Answer only from the words and pictures of the conversation.";

const START_WITHIN: Duration = Duration::from_secs(30);
const ANSWER_WITHIN: Duration = Duration::from_secs(15);
const LIMITS_WITHIN: Duration = Duration::from_secs(3);

/// How long a turn has to end once it was interrupted, before the helper itself is ended.
pub const INTERRUPT_GRACE: Duration = Duration::from_millis(1500);

/// The pictures of how many earlier questions ride with a resumed conversation.
pub const KEEP_PICTURES: usize = 2;

/// How an entry of what Codex added to a conversation names an MCP server of the person's that started in it.
pub const MCP_STARTED: &str = "MCP server started by Codex: ";

/// The names of the MCP servers the person's own Codex is set up with, from the answer to `config/read`. Only the
/// names are read: what a server runs and the environment it is given never leave that answer.
fn servers_of(read: &Value) -> Vec<String> {
    // Measured on the real app-server (0.164.0, 2026-11-11): `config/read` answers with the person's servers under
    // `config.mcp_servers`, one entry a server. The schema does not name that key.
    let mut names: Vec<String> = read["config"]["mcp_servers"].as_object().into_iter().flatten().map(|(name, _)| name.clone()).collect();
    names.sort();
    names
}

/// The parameters of `thread/start`: the fence around the conversation. `servers` are the MCP servers the person's
/// Codex is set up with: each is switched off for this thread by name.
fn thread_start(cwd: &Path, system: &str, model: Option<&str>, servers: &[String]) -> Value {
    // Checked against Codex 0.164.0: its generated schema takes `config` here as a free table and names
    // `model_provider` and `web_search` among the keys of a config; every name in `FEATURES_OFF` is in
    // `codex features list`. UNCONFIRMED, since no schema lists them and no real app-server has been asked: that
    // `mcp_servers`, `notify` and `project_doc_max_bytes` are honoured when they arrive this way, and that a dotted
    // `features.<name>` key is read as that feature's switch.
    // Measured on the real CLI: an empty `mcp_servers` table is laid over the person's config, not put in its place,
    // so their servers still start. So each one is named with `enabled = false`, which the same laying-over carries
    // onto the person's entry. Measured the same day: with the empty table the person's server was reported
    // starting; with it named `enabled = false` (nested, as here, or as a dotted key) no start was reported.
    let off: serde_json::Map<String, Value> = servers.iter().map(|name| (name.clone(), json!({ "enabled": false }))).collect();
    let mut config = serde_json::Map::new();
    config.insert("mcp_servers".into(), Value::Object(off));
    config.insert("model_provider".into(), json!(OPENAI));
    config.insert("web_search".into(), json!("disabled"));
    config.insert("notify".into(), json!([]));
    config.insert("project_doc_max_bytes".into(), json!(0));
    for f in FEATURES_OFF {
        config.insert(format!("features.{f}"), json!(false));
    }
    let instructions = if system.trim().is_empty() { NO_TOOLS.to_owned() } else { format!("{NO_TOOLS}\n\n{}", system.trim()) };
    let mut p = json!({
        "ephemeral": true,
        "sandbox": "read-only",
        "approvalPolicy": "untrusted",
        "cwd": cwd.to_string_lossy(),
        "developerInstructions": instructions,
        "config": config,
    });
    if let Some(m) = model {
        p["model"] = json!(m);
    }
    p
}

fn input_of(parts: &[Part]) -> Vec<Value> {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(t) => json!({ "type": "text", "text": t }),
            Part::Picture { path, .. } => json!({ "type": "localImage", "path": path.to_string_lossy() }),
        })
        .collect()
}

fn has_picture(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Picture { .. }))
}

enum Reply {
    Result(Value),
    Error(i64, &'static str),
}

/// What a request from Codex is told: no, in the shape that request takes.
fn decline(method: &str) -> Reply {
    match method {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => Reply::Result(json!({ "decision": "decline" })),
        "item/permissions/requestApproval" => Reply::Result(json!({ "permissions": {}, "scope": "turn" })),
        "item/tool/call" => Reply::Result(json!({ "contentItems": [], "success": false })),
        "item/tool/requestUserInput" => Reply::Result(json!({ "answers": {} })),
        "mcpServer/elicitation/request" => Reply::Result(json!({ "action": "decline" })),
        "applyPatchApproval" | "execCommandApproval" => Reply::Result(json!({ "decision": "denied" })),
        "account/chatgptAuthTokens/refresh" => Reply::Error(-32000, "Pointcast does not hold or refresh tokens."),
        _ => Reply::Error(-32601, "Method not found"),
    }
}

/// What a failed turn's error means for the person. A limit's reset time is read afterwards.
fn failure_of(error: &Value) -> Failure {
    let message = error["message"].as_str().filter(|m| !m.is_empty()).unwrap_or("Codex could not answer.").to_owned();
    let info = &error["codexErrorInfo"];
    match info.as_str() {
        Some("usageLimitExceeded" | "rateLimitExceeded") => Failure::Limit { resets: None },
        Some("unauthorized") => Failure::SignedOut,
        Some("contextWindowExceeded") => Failure::ContextFull,
        Some("cyberPolicy" | "misalignmentPolicyViolation") => Failure::Refused,
        Some(_) => Failure::Broken(message),
        None => {
            let net = ["httpConnectionFailed", "responseStreamConnectionFailed", "responseStreamDisconnected", "responseTooManyFailedAttempts"];
            if net.iter().any(|k| info.get(k).is_some()) {
                Failure::Offline
            } else {
                Failure::Broken(message)
            }
        }
    }
}

fn window_of(v: &Value) -> Option<Window> {
    let used = v.get("usedPercent")?.as_u64()?;
    Some(Window { used_percent: used as u32, minutes: v["windowDurationMins"].as_i64(), resets: v["resetsAt"].as_i64() })
}

fn limits_of(v: &Value) -> Limits {
    Limits { primary: window_of(&v["primary"]), secondary: window_of(&v["secondary"]) }
}

/// Whether a model takes pictures. A model that does not list what it takes counts as words and pictures.
fn takes_pictures(model: &Value) -> bool {
    match model.get("inputModalities").and_then(Value::as_array) {
        Some(list) => list.iter().any(|m| m.as_str() == Some("image")),
        None => true,
    }
}

/// The model a conversation runs on: the one asked for, else the one Codex marks as its default.
fn chosen<'a>(models: &'a [Value], want: Option<&str>) -> Option<&'a Value> {
    let named = want.and_then(|w| models.iter().find(|m| m["id"].as_str() == Some(w) || m["model"].as_str() == Some(w)));
    named.or_else(|| models.iter().find(|m| m["isDefault"].as_bool() == Some(true))).or(models.first())
}

fn offers_effort(model: &Value, effort: &str) -> bool {
    model["supportedReasoningEfforts"].as_array().is_some_and(|l| l.iter().any(|e| e["reasoningEffort"].as_str() == Some(effort)))
}

fn stopped(said: &str) -> String {
    if said.is_empty() {
        "Codex stopped before it finished.".into()
    } else {
        format!("Codex stopped before it finished. It said: {said}")
    }
}

/// Whether the account is signed in, and under which plan. The address it is under is never read.
#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub plan: Option<String>,
    pub api_key: bool,
}

fn account_of(v: &Value) -> Option<Account> {
    let a = v.get("account").filter(|a| a.is_object())?;
    Some(Account { plan: a["planType"].as_str().map(str::to_owned), api_key: a["type"].as_str() == Some("apiKey") })
}

/// The turn being answered.
struct Active {
    id: TurnId,
    out: Sender<Event>,
    /// The id of our `turn/start`, and the id Codex gave the turn once it answered.
    start: u64,
    turn: Option<String>,
    /// An interrupt was asked for: nothing more of this turn is shown, and this is how it ends.
    stopping: Option<Failure>,
    thought: bool,
    streamed: HashSet<String>,
    error: Option<Value>,
}

impl Active {
    fn tell(&self, kind: Kind) {
        let _ = self.out.send(Event { turn: self.id, kind });
    }
}

/// A turn asked while another still ran: it is sent once that one has ended.
struct Queued {
    id: TurnId,
    out: Sender<Event>,
    params: Value,
}

#[derive(Default)]
struct Inner {
    thread: String,
    active: Option<Active>,
    next: Option<Queued>,
    serial: u64,
    limits: Option<Limits>,
    dead: bool,
    /// Turns of Codex that have ended: a late word of one is not the next turn's.
    over: HashSet<String>,
    /// The MCP servers of the person's that Codex said it started or had ready, although each was switched off.
    servers: Vec<String>,
}

struct Shared {
    rpc: Client,
    inner: Mutex<Inner>,
    proc: Mutex<Option<Proc>>,
    tail: Option<Arc<Tail>>,
    /// Notifications that were not for us to act on.
    ignored: AtomicU64,
    /// What this line may send: `SENDS` for a conversation, `SIGN_IN` for a sign-in.
    sends: &'static [&'static str],
    /// A sign-in waiting to hear that it ended.
    login: Mutex<Option<Sender<Value>>>,
}

impl Shared {
    fn allowed(method: &str) -> std::io::Result<()> {
        if SENDS.contains(&method) {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("Pointcast does not send {method}")))
        }
    }

    /// Whether this line may send `method`.
    fn allows(&self, method: &str) -> std::io::Result<()> {
        if self.sends.contains(&method) {
            Ok(())
        } else {
            Err(std::io::Error::other(format!("Pointcast does not send {method}")))
        }
    }

    fn call(&self, method: &str, params: Value, within: Duration) -> Result<Value, Failure> {
        self.allows(method).map_err(|e| Failure::Broken(e.to_string()))?;
        self.rpc.call(method, params, within).map_err(|e| {
            Failure::Broken(match e {
                CallError::Rpc(e) => format!("Codex refused {method}: {}", e.message),
                CallError::Slow => format!("Codex did not answer {method} in time."),
                CallError::Closed => stopped(&self.last_words()),
            })
        })
    }

    fn post_as(&self, id: u64, method: &str, params: Value) -> std::io::Result<()> {
        self.allows(method)?;
        self.rpc.post_as(id, method, params)
    }

    fn last_words(&self) -> String {
        self.tail.as_ref().map(|t| t.settled(Duration::from_millis(500))).unwrap_or_default()
    }

    fn interrupt(&self, thread: &str, turn: &str) {
        let _ = self.post_as(self.rpc.reserve(), "turn/interrupt", json!({ "threadId": thread, "turnId": turn }));
    }

    fn run(self: Arc<Self>, incoming: Receiver<Incoming>) {
        for m in incoming {
            match m {
                Incoming::Request { id, method, .. } => {
                    let _ = match decline(&method) {
                        Reply::Result(v) => self.rpc.answer(&id, v),
                        Reply::Error(code, why) => self.rpc.refuse(&id, code, why),
                    };
                }
                Incoming::Notification { method, params } => self.noted(&method, &params),
                Incoming::Reply { id, result } => self.replied(id, result),
                Incoming::Closed => break,
            }
        }
        self.gone();
    }

    /// The helper is gone: whatever was being answered, or waited to be, fails with what the helper said last.
    fn gone(&self) {
        // A sign-in waiting to hear how it ended hears that it did not.
        self.login.lock().unwrap().take();
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            s.dead = true;
            (s.active.take(), s.next.take())
        };
        if a.is_none() && q.is_none() {
            return;
        }
        let why = stopped(&self.last_words());
        if let Some(a) = a {
            a.tell(Kind::Failed(a.stopping.clone().unwrap_or(Failure::Broken(why.clone()))));
        }
        if let Some(q) = q {
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Broken(why)) });
        }
    }

    /// Send the turn that waited, now that the one before it has ended. Called with the turn just taken out.
    fn send_next(&self, q: Option<Queued>) {
        let Some(q) = q else { return };
        let start = self.rpc.reserve();
        {
            let mut s = self.inner.lock().unwrap();
            s.active = Some(Active { id: q.id, out: q.out, start, turn: None, stopping: None, thought: false, streamed: HashSet::new(), error: None });
        }
        if self.post_as(start, "turn/start", q.params).is_err() {
            let a = self.inner.lock().unwrap().active.take_if(|a| a.start == start);
            if let Some(a) = a {
                a.tell(Kind::Failed(Failure::Broken(stopped(&self.last_words()))));
            }
        }
    }

    fn replied(&self, id: u64, result: Result<Value, RpcError>) {
        let mut s = self.inner.lock().unwrap();
        if !s.active.as_ref().is_some_and(|a| a.start == id) {
            return;
        }
        match result {
            Ok(v) => {
                let thread = s.thread.clone();
                let turn = v["turn"]["id"].as_str().unwrap_or_default().to_owned();
                let stop = s.active.as_mut().is_some_and(|a| {
                    a.turn = Some(turn.clone());
                    a.stopping.is_some()
                });
                drop(s);
                // Stopped before it had an id to stop it by.
                if stop {
                    self.interrupt(&thread, &turn);
                }
            }
            Err(e) => {
                let (a, q) = (s.active.take(), s.next.take());
                drop(s);
                if let Some(a) = a {
                    a.tell(Kind::Failed(a.stopping.clone().unwrap_or(Failure::Broken(format!("Codex did not take the question: {}", e.message)))));
                }
                self.send_next(q);
            }
        }
    }

    fn noted(self: &Arc<Self>, method: &str, p: &Value) {
        const HEARD: &[&str] = &[
            "account/rateLimits/updated", "item/agentMessage/delta", "item/completed", "item/started", "item/reasoning/summaryTextDelta",
            "item/reasoning/textDelta", "thread/tokenUsage/updated", "error", "turn/completed",
        ];
        if method == "account/login/completed" {
            if let Some(tx) = self.login.lock().unwrap().as_ref() {
                let _ = tx.send(p.clone());
                return;
            }
        }
        // A server of the person's that starts in Pointcast's thread is through the fence: it is not a reason to fail
        // the answer (a tool it offers still ends the turn when called), but it is said with what was sent.
        if method == "mcpServer/startupStatus/updated" {
            let name = p["name"].as_str().unwrap_or_default();
            if matches!(p["status"].as_str(), Some("starting" | "ready")) && !name.is_empty() {
                let mut s = self.inner.lock().unwrap();
                if !s.servers.iter().any(|n| n == name) {
                    s.servers.push(name.to_owned());
                    eprintln!("pointcast: Codex started the MCP server \"{name}\" in a conversation that switched it off");
                }
            }
            return;
        }
        if !HEARD.contains(&method) {
            self.ignored.fetch_add(1, Ordering::SeqCst);
            return;
        }
        let mut s = self.inner.lock().unwrap();
        if method == "account/rateLimits/updated" {
            let l = limits_of(&p["rateLimits"]);
            s.limits = Some(l);
            if let Some(a) = &s.active {
                a.tell(Kind::Limits(l));
            }
            return;
        }
        let theirs = p["turnId"].as_str().or(p["turn"]["id"].as_str());
        if theirs.is_some_and(|t| s.over.contains(t)) {
            return;
        }
        let Some(a) = s.active.as_mut() else { return };
        if let (Some(mine), Some(theirs)) = (a.turn.as_deref(), theirs) {
            if mine != theirs {
                return;
            }
        }
        let quiet = a.stopping.is_some();
        match method {
            "item/agentMessage/delta" if !quiet => {
                if let Some(d) = p["delta"].as_str() {
                    a.streamed.insert(p["itemId"].as_str().unwrap_or_default().to_owned());
                    a.tell(Kind::Delta(d.to_owned()));
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" if !quiet && !a.thought => {
                a.thought = true;
                a.tell(Kind::Thinking);
            }
            "item/started" if !quiet => {
                let kind = p["item"]["type"].as_str().unwrap_or_default();
                if ACTS.contains(&kind) {
                    drop(s);
                    self.stop(Failure::Refused);
                } else if kind == "reasoning" && !a.thought {
                    a.thought = true;
                    a.tell(Kind::Thinking);
                }
            }
            // A message that was streamed is not shown again whole; one that was not is shown once.
            "item/completed" if !quiet => {
                let item = &p["item"];
                let text = item["text"].as_str().unwrap_or_default();
                if item["type"].as_str() == Some("agentMessage") && !text.is_empty() && a.streamed.insert(item["id"].as_str().unwrap_or_default().to_owned()) {
                    a.tell(Kind::Delta(text.to_owned()));
                }
            }
            "thread/tokenUsage/updated" => {
                let last = &p["tokenUsage"]["last"];
                let n = |k: &str| last[k].as_u64().unwrap_or(0);
                a.tell(Kind::Usage(Usage { input: n("inputTokens"), cached: n("cachedInputTokens"), output: n("outputTokens") }));
            }
            "error" => {
                if p["willRetry"].as_bool() != Some(true) {
                    a.error = Some(p["error"].clone());
                }
            }
            "turn/completed" => {
                let turn = &p["turn"];
                let end = match (a.stopping.clone(), turn["status"].as_str()) {
                    (Some(why), Some("completed" | "interrupted" | "failed")) => Kind::Failed(why),
                    (None, Some("completed")) => Kind::Done,
                    (None, Some("interrupted")) => Kind::Failed(Failure::Stopped),
                    (None, Some("failed")) => {
                        let e = if turn["error"].is_null() { a.error.clone().unwrap_or(Value::Null) } else { turn["error"].clone() };
                        Kind::Failed(failure_of(&e))
                    }
                    _ => return,
                };
                if let Some(t) = turn["id"].as_str().or(a.turn.as_deref()) {
                    let t = t.to_owned();
                    s.over.insert(t);
                }
                let (Some(a), q, known) = (s.active.take(), s.next.take(), s.limits) else { return };
                drop(s);
                let end = match end {
                    Kind::Failed(Failure::Limit { .. }) => {
                        let read = self.call("account/rateLimits/read", Value::Null, LIMITS_WITHIN).ok().map(|v| limits_of(&v["rateLimits"]));
                        Kind::Failed(Failure::Limit { resets: read.or(known).and_then(|l| l.resets()) })
                    }
                    e => e,
                };
                a.tell(end);
                self.send_next(q);
            }
            _ => {}
        }
    }

    /// Stop the answer under way, to end as `why`: interrupt it, and end the helper if it will not stop. A turn
    /// that waited behind it is stopped with it, unless it is what stops the first (`keep_next`).
    fn stop_for(self: &Arc<Self>, why: Failure, keep_next: bool) {
        let mut s = self.inner.lock().unwrap();
        let q = if keep_next { None } else { s.next.take() };
        if let Some(q) = q {
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Stopped) });
        }
        let thread = s.thread.clone();
        let Some(a) = s.active.as_mut() else { return };
        if a.stopping.is_some() {
            return;
        }
        a.stopping = Some(why);
        let (id, turn) = (a.id, a.turn.clone());
        drop(s);
        if let Some(t) = turn {
            self.interrupt(&thread, &t);
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let still = |s: &Inner| s.active.as_ref().is_some_and(|a| a.id == id);
            let t = Instant::now();
            while t.elapsed() < INTERRUPT_GRACE {
                std::thread::sleep(Duration::from_millis(20));
                if !still(&me.inner.lock().unwrap()) {
                    return;
                }
            }
            // It did not stop: end the helper, then say so, so that whoever hears "stopped" finds it gone.
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
                a.tell(Kind::Failed(a.stopping.clone().unwrap_or(Failure::Stopped)));
            }
            if let Some(q) = q {
                let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Broken("Codex would not stop, so Pointcast ended it. Ask again.".into())) });
            }
        });
    }

    fn stop(self: &Arc<Self>, why: Failure) {
        self.stop_for(why, false)
    }

    /// End the conversation: what was being answered ends as stopped, stdin closes, and the helper gets `grace`
    /// to leave before it and its tree are ended.
    fn shut(&self, grace: Duration) {
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            s.dead = true;
            (s.active.take(), s.next.take())
        };
        if let Some(a) = a {
            a.tell(Kind::Failed(Failure::Stopped));
        }
        if let Some(q) = q {
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Stopped) });
        }
        self.rpc.hang_up();
        let p = self.proc.lock().unwrap().take();
        if let Some(mut p) = p {
            p.end_within(grace);
        }
    }
}

/// A conversation with Codex: first only the greeting (`connect`), then a thread of its own (`begin`).
pub struct CodexChat {
    shared: Arc<Shared>,
    pub account: Account,
    models: Vec<Value>,
    model: Option<String>,
    effort: Option<String>,
    pictures: bool,
    /// What a resumed conversation starts from, sent with its first question.
    context: Vec<Part>,
    /// The files Codex itself adds to every conversation, as its `thread/start` named them (the person's own Codex
    /// instructions file). Pointcast cannot leave them out, so it says that they go along.
    pub adds: Vec<String>,
}

/// What a conversation is started with.
pub struct Setup<'a> {
    pub cwd: &'a Path,
    pub system: &'a str,
    pub earlier: &'a [Turn],
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
}

impl CodexChat {
    /// Greet an app-server over these streams and learn who is signed in and which models there are. No thread is
    /// started and nothing is spent.
    pub fn connect<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W, proc: Option<Proc>) -> Result<CodexChat, Failure> {
        let (rpc, incoming) = Client::over(reader, writer);
        let tail = proc.as_ref().map(Proc::tail);
        let shared = Arc::new(Shared { rpc, inner: Mutex::default(), proc: Mutex::new(proc), tail, ignored: AtomicU64::new(0), sends: SENDS, login: Mutex::default() });
        let pump = shared.clone();
        std::thread::spawn(move || pump.run(incoming));
        let greet = || -> Result<(Account, Vec<Value>), Failure> {
            shared.call("initialize", json!({ "clientInfo": { "name": "pointcast", "title": "Pointcast", "version": env!("CARGO_PKG_VERSION") } }), START_WITHIN)?;
            Shared::allowed("initialized").and_then(|_| shared.rpc.notify("initialized")).map_err(|_| Failure::Broken(stopped(&shared.last_words())))?;
            let who = shared.call("account/read", json!({ "refreshToken": false }), ANSWER_WITHIN)?;
            let account = account_of(&who).ok_or(Failure::SignedOut)?;
            // Without the list a model is taken to read pictures, as one that lists nothing is.
            let models = shared.call("model/list", json!({}), ANSWER_WITHIN).ok().and_then(|m| m["data"].as_array().cloned()).unwrap_or_default();
            Ok((account, models))
        };
        match greet() {
            Ok((account, models)) => Ok(CodexChat { shared, account, models, model: None, effort: None, pictures: true, context: Vec::new(), adds: Vec::new() }),
            Err(f) => {
                shared.shut(Duration::ZERO);
                Err(f)
            }
        }
    }

    /// Start this conversation's own thread, fenced in to question and answer.
    pub fn begin(&mut self, setup: &Setup) -> Result<(), Failure> {
        // A model is always named: without one Codex takes the one in the person's own config, which their plan may
        // not offer. The one asked for when Codex lists it, else the one it marks as its default.
        let Some(model) = chosen(&self.models, setup.model) else {
            return Err(Failure::Broken("Codex did not list its models, so Pointcast did not ask.".into()));
        };
        let id = model["id"].as_str().or(model["model"].as_str()).unwrap_or_default().to_owned();
        self.pictures = takes_pictures(model);
        self.effort = setup.effort.filter(|e| offers_effort(model, e)).map(str::to_owned);
        self.context = replay(setup.earlier, if self.pictures { KEEP_PICTURES } else { 0 });
        // The person's own MCP servers, by name, so that each can be switched off for this thread. Read only: nothing
        // is written, and only the names are kept. A Codex that does not answer starts the thread with none named; a
        // server that starts then is still said (`noted`).
        let read = self.shared.call("config/read", json!({ "includeLayers": false, "cwd": setup.cwd.to_string_lossy() }), ANSWER_WITHIN);
        let servers = read.as_ref().map(servers_of).unwrap_or_default();
        let started = self.shared.call("thread/start", thread_start(setup.cwd, setup.system, Some(&id), &servers), ANSWER_WITHIN)?;
        self.model = Some(id);
        self.adds = started["instructionSources"].as_array().into_iter().flatten().filter_map(|s| s.as_str().map(str::to_owned)).collect();
        // The person agreed to a question going to OpenAI. A config that points Codex elsewhere must not make that false.
        if let Some(other) = started["modelProvider"].as_str().filter(|p| *p != OPENAI) {
            self.shared.shut(Duration::ZERO);
            return Err(Failure::Broken(format!("Codex is set up to send questions to \"{other}\", not to OpenAI, so Pointcast did not ask.")));
        }
        let thread = started["thread"]["id"].as_str().unwrap_or_default().to_owned();
        if thread.is_empty() {
            return Err(Failure::Broken("Codex started no conversation.".into()));
        }
        self.shared.inner.lock().unwrap().thread = thread;
        Ok(())
    }

    /// Over these streams, greeted and with its thread.
    pub fn over<R: Read + Send + 'static, W: Write + Send + 'static>(reader: R, writer: W, proc: Option<Proc>, setup: &Setup) -> Result<CodexChat, Failure> {
        let mut chat = Self::connect(reader, writer, proc)?;
        chat.begin(setup)?;
        Ok(chat)
    }

    pub fn pid(&self) -> Option<u32> {
        self.shared.proc.lock().unwrap().as_ref().map(Proc::pid)
    }

    pub fn thread(&self) -> String {
        self.shared.inner.lock().unwrap().thread.clone()
    }

    fn usable(&self) -> bool {
        !self.shared.inner.lock().unwrap().dead
    }
}

impl Chat for CodexChat {
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure> {
        if !self.pictures && has_picture(&turn.parts) {
            return Err(Failure::NoPictures);
        }
        let mut input = input_of(&self.context);
        input.extend(input_of(&turn.parts));
        let start = self.shared.rpc.reserve();
        let mut s = self.shared.inner.lock().unwrap();
        if s.dead || s.thread.is_empty() {
            return Err(Failure::Broken("Codex is not running.".into()));
        }
        let mut params = json!({ "threadId": s.thread, "input": input });
        if let Some(e) = &self.effort {
            params["effort"] = json!(e);
        }
        s.serial += 1;
        let id = TurnId(s.serial);
        self.context.clear();
        if s.active.is_some() {
            // An answer still comes: stop it, and send this turn once Codex says that one has ended.
            if let Some(old) = s.next.replace(Queued { id, out, params }) {
                let _ = old.out.send(Event { turn: old.id, kind: Kind::Failed(Failure::Stopped) });
            }
            drop(s);
            self.shared.stop_for(Failure::Stopped, true);
            return Ok(id);
        }
        s.active = Some(Active { id, out, start, turn: None, stopping: None, thought: false, streamed: HashSet::new(), error: None });
        drop(s);
        if self.shared.post_as(start, "turn/start", params).is_err() {
            // If the line's end was seen first, the turn has already been failed there, once.
            if self.shared.inner.lock().unwrap().active.take_if(|a| a.start == start).is_some() {
                return Err(Failure::Broken(stopped(&self.shared.last_words())));
            }
        }
        Ok(id)
    }

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let shared = self.shared.clone();
        Arc::new(move || shared.stop(Failure::Stopped))
    }

    fn close(&mut self) {
        self.shared.shut(child::CLOSE_GRACE);
    }

    fn adds(&self) -> Vec<String> {
        let started = self.shared.inner.lock().unwrap().servers.clone();
        self.adds.iter().cloned().chain(started.iter().map(|name| format!("{MCP_STARTED}{name}"))).collect()
    }
}

/// Sign in to ChatGPT through Codex's own flow over these streams: Codex makes the sign-in page, `open` shows it in the
/// browser, and Codex says when the person is done there. Pointcast never sees what the sign-in keeps, and this line
/// can send nothing but what `SIGN_IN` lists.
pub fn sign_in_over<R: Read + Send + 'static, W: Write + Send + 'static>(
    reader: R,
    writer: W,
    proc: Option<Proc>,
    open: &dyn Fn(&str) -> Result<(), String>,
    within: Duration,
) -> Result<(), String> {
    let (rpc, incoming) = Client::over(reader, writer);
    let tail = proc.as_ref().map(Proc::tail);
    let (tx, rx) = std::sync::mpsc::channel();
    let login = Mutex::new(Some(tx));
    let shared = Arc::new(Shared { rpc, inner: Mutex::default(), proc: Mutex::new(proc), tail, ignored: AtomicU64::new(0), sends: SIGN_IN, login });
    let pump = shared.clone();
    std::thread::spawn(move || pump.run(incoming));
    let words = |f: Failure| if let Failure::Broken(why) = f { why } else { "Codex could not start the sign-in.".into() };
    let run = || -> Result<(), String> {
        shared.call("initialize", json!({ "clientInfo": { "name": "pointcast", "title": "Pointcast", "version": env!("CARGO_PKG_VERSION") } }), START_WITHIN).map_err(words)?;
        shared.allows("initialized").and_then(|_| shared.rpc.notify("initialized")).map_err(|_| stopped(&shared.last_words()))?;
        let started = shared.call("account/login/start", json!({ "type": "chatgpt" }), ANSWER_WITHIN).map_err(words)?;
        let url = started["authUrl"].as_str().filter(|u| u.starts_with("https://")).ok_or("Codex gave no sign-in page to open.")?;
        let id = started["loginId"].as_str().map(str::to_owned);
        open(url)?;
        let deadline = Instant::now() + within;
        loop {
            match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
                Ok(done) if id.is_some() && done["loginId"].as_str() != id.as_deref() => {}
                Ok(done) if done["success"].as_bool() == Some(true) => return Ok(()),
                Ok(done) => {
                    let why = done["error"].as_str().filter(|e| !e.is_empty()).unwrap_or("it was not completed");
                    return Err(format!("The sign-in did not finish: {why}."));
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => return Err("The sign-in was not finished in time. Try again.".into()),
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(stopped(&shared.last_words())),
            }
        }
    };
    let said = run();
    shared.shut(child::CLOSE_GRACE);
    said
}

impl Drop for CodexChat {
    fn drop(&mut self) {
        self.shared.shut(Duration::ZERO);
    }
}

/// The folder Codex is started in: Pointcast's own, and empty, so there is no project for an agent to read.
fn empty_folder(home: &Path) -> Result<PathBuf, Failure> {
    let d = home.join("empty");
    std::fs::create_dir_all(&d).map_err(|e| Failure::Broken(format!("Pointcast could not make its folder for Codex: {e}")))?;
    if std::fs::read_dir(&d).map(|mut l| l.next().is_some()).unwrap_or(true) {
        return Err(Failure::Broken(format!("The folder Pointcast keeps empty for Codex has files in it: {}", d.display())));
    }
    Ok(d)
}

pub struct Codex {
    /// A folder of Pointcast's own: the empty folder and the pid file live in it.
    home: PathBuf,
    pub model: Option<String>,
    /// How hard the model thinks before it answers; a study answer wants the first words soon.
    pub effort: Option<String>,
    launch: Option<Launch>,
    warmed: Mutex<Option<CodexChat>>,
    models: Mutex<Vec<Value>>,
    /// The files Codex added to the last conversation it started (`CodexChat::adds`).
    adds: Mutex<Vec<String>>,
}

impl Codex {
    pub fn new(home: PathBuf) -> Codex {
        Codex { home, model: None, effort: Some("low".into()), launch: None, warmed: Mutex::default(), models: Mutex::default(), adds: Mutex::default() }
    }

    /// Run `launch` in place of the `codex` found on this computer.
    pub fn with(launch: Launch, home: PathBuf) -> Codex {
        Codex { launch: Some(launch), ..Codex::new(home) }
    }

    /// Start the helper in Pointcast's empty folder.
    fn spawn(&self) -> Result<(Proc, std::process::ChildStdin, std::process::ChildStdout), Failure> {
        let launch = match &self.launch {
            Some(l) => l.clone(),
            None => match detect::codex() {
                Ok(found) => found.launch(),
                Err(detect::Missing::NotInstalled) => return Err(Failure::NotInstalled),
                Err(detect::Missing::TooOld(why)) => return Err(Failure::Broken(why)),
                Err(detect::Missing::Silent(why)) => return Err(Failure::Broken(format!("Pointcast could not read Codex's version: {why}."))),
            },
        };
        let cwd = empty_folder(&self.home)?;
        let pidfile = self.home.join("codex.pid");
        child::reap_stale(&pidfile);
        Proc::start(&launch, &cwd, Some(&pidfile)).map_err(|e| Failure::Broken(format!("Codex did not start: {e}")))
    }

    /// Start the helper and greet it.
    fn connect(&self) -> Result<CodexChat, Failure> {
        let (proc, stdin, stdout) = self.spawn()?;
        let chat = CodexChat::connect(stdout, stdin, Some(proc))?;
        *self.models.lock().unwrap() = chat.models.clone();
        Ok(chat)
    }

    /// Sign in to ChatGPT through Codex's own flow (`sign_in_over`), on a helper of its own that ends with it.
    pub fn sign_in(&self, open: &dyn Fn(&str) -> Result<(), String>, within: Duration) -> Result<(), String> {
        let (proc, stdin, stdout) = self.spawn().map_err(|f| if let Failure::Broken(why) = f { why } else { "Codex is not installed.".into() })?;
        sign_in_over(stdout, stdin, Some(proc), open, within)
    }

    /// Start the helper if it does not run, and say which models it lists. Nothing is spent.
    pub fn list(&self) -> Result<Vec<super::catalog::Model>, Failure> {
        self.warm_now()?;
        Ok(super::Provider::models(self))
    }

    fn warm_now(&self) -> Result<(), Failure> {
        let mut w = self.warmed.lock().unwrap();
        if !w.as_ref().is_some_and(CodexChat::usable) {
            *w = Some(self.connect()?);
        }
        Ok(())
    }

    /// A conversation on a helper of its own: the warmed one if it still runs, else a new one.
    pub fn start(&self, system: &str, earlier: &[Turn]) -> Result<CodexChat, Failure> {
        let warm = self.warmed.lock().unwrap().take().filter(CodexChat::usable);
        let mut chat = match warm {
            Some(c) => c,
            None => self.connect()?,
        };
        let cwd = empty_folder(&self.home)?;
        chat.begin(&Setup { cwd: &cwd, system, earlier, model: self.model.as_deref(), effort: self.effort.as_deref() })?;
        *self.adds.lock().unwrap() = chat.adds.clone();
        Ok(chat)
    }

    /// The files Codex added to the last conversation it started, once one has started.
    pub fn adds(&self) -> Vec<String> {
        self.adds.lock().unwrap().clone()
    }

    /// The helper `warm` started and no conversation has taken yet.
    pub fn warm_pid(&self) -> Option<u32> {
        self.warmed.lock().unwrap().as_ref().and_then(CodexChat::pid)
    }

    /// End the helper `warm` started, if no conversation took it.
    pub fn cool(&self) {
        self.warmed.lock().unwrap().take();
    }
}

impl Provider for Codex {
    fn id(&self) -> &'static str {
        ID
    }

    fn caps(&self, model: Option<&str>) -> Caps {
        let models = self.models.lock().unwrap();
        let images = chosen(&models, model.or(self.model.as_deref())).is_none_or(takes_pictures);
        Caps { images, max_images: 4, max_edge: 2048, offline: false }
    }

    fn health(&self) -> Health {
        detect::health()
    }

    /// Start the helper and greet it (who is signed in, which models), so the first question does not wait for
    /// that. No thread is started and nothing is spent.
    fn warm(&self) {
        let _ = self.warm_now();
    }

    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
        self.start(system, earlier).map(|c| Box::new(c) as Box<dyn Chat>)
    }

    fn cool(&self) {
        Codex::cool(self);
    }

    fn adds(&self) -> Vec<String> {
        Codex::adds(self)
    }

    /// The models of the last greeting, none before one.
    fn models(&self) -> Vec<super::catalog::Model> {
        let models = self.models.lock().unwrap();
        models
            .iter()
            .filter_map(|m| {
                let id = m["id"].as_str().or(m["model"].as_str())?.to_owned();
                let name = m["displayName"].as_str().filter(|n| !n.is_empty()).map_or_else(|| id.clone(), str::to_owned);
                Some(super::catalog::Model { id, name, images: takes_pictures(m) })
            })
            .collect()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::assist::child::tests::gone;
    use crate::assist::tests::{collect, endings, said, scratch, shot, words};
    use crate::assist::{close_if_idle, fake, Role, IDLE};
    use std::sync::mpsc::channel;

    type Seen = Arc<Mutex<Vec<Value>>>;

    const CWD: &str = "/pointcast/empty";

    /// The stand-in served inside this process, and what it was sent.
    fn link(mode: &'static str) -> (Result<CodexChat, Failure>, Seen) {
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            fake::serve(mode, std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone()));
        });
        (CodexChat::connect(our_in, our_out, None), seen)
    }

    fn wire_with(mode: &'static str, earlier: &[Turn]) -> (Result<CodexChat, Failure>, Seen) {
        let (chat, seen) = link(mode);
        let chat = chat.and_then(|mut c| {
            c.begin(&Setup { cwd: Path::new(CWD), system: "Be brief.", earlier, model: None, effort: Some("low") })?;
            Ok(c)
        });
        (chat, seen)
    }

    fn wire(mode: &'static str) -> (CodexChat, Seen) {
        let (chat, seen) = wire_with(mode, &[]);
        (chat.ok().expect("the stand-in greets and starts a thread"), seen)
    }

    /// The stand-in as a real child: this test program run again as `stand_in` below.
    fn child(mode: &str, name: &str) -> Codex {
        let launch = Launch {
            exe: std::env::current_exe().unwrap(),
            args: ["--exact", "assist::codex::tests::stand_in", "--ignored", "--nocapture"].map(String::from).to_vec(),
            env: vec![("POINTCAST_FAKE_CODEX".into(), mode.into())],
        };
        Codex::with(launch, scratch(name))
    }

    /// Acts as the app-server when a test starts this program as its helper. Does nothing when run with the rest.
    #[test]
    #[ignore]
    fn stand_in() {
        let Ok(mode) = std::env::var("POINTCAST_FAKE_CODEX") else { return };
        // On one test thread the harness leaves "test <name> ... " unfinished on stdout: end that line, so the
        // first answer is a line of its own.
        println!();
        match fake::serve(&mode, std::io::stdin().lock(), std::io::stdout(), |_| {}) {
            fake::Exit::Crash => {
                eprintln!("{}", fake::LAST_WORDS);
                std::process::exit(3);
            }
            fake::Exit::Stubborn => std::thread::sleep(Duration::from_secs(60)),
            fake::Exit::Eof => std::process::exit(0),
        }
    }

    fn methods(seen: &Seen) -> Vec<String> {
        seen.lock().unwrap().iter().filter_map(|m| m["method"].as_str().map(str::to_owned)).collect()
    }

    /// The parameters of every `method` sent.
    fn sent(seen: &Seen, method: &str) -> Vec<Value> {
        seen.lock().unwrap().iter().filter(|m| m["method"].as_str() == Some(method)).map(|m| m["params"].clone()).collect()
    }

    fn answer(chat: &mut CodexChat, turn: Turn) -> Vec<Kind> {
        let (tx, rx) = channel();
        chat.ask(turn, tx).unwrap();
        collect(&rx)
    }

    fn first_word(rx: &Receiver<Event>) {
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).map(|e| e.kind), Ok(Kind::Delta(fake::WORDS[0].into())));
    }

    fn with_picture() -> Turn {
        Turn { role: Role::Person, parts: vec![Part::Text("What is two plus two?".into()), shot("/pointcast/turn/01.png")] }
    }

    #[test]
    fn the_greeting_and_the_question_go_out_in_order_with_every_field_and_the_answer_streams_once() {
        let (mut chat, seen) = wire("normal");
        let got = answer(&mut chat, with_picture());
        assert_eq!(words(&got), fake::WORDS, "three pieces in order, and the whole message not shown again");
        assert_eq!((endings(&got), got.last()), (1, Some(&Kind::Done)));

        assert_eq!(methods(&seen), ["initialize", "initialized", "account/read", "model/list", "config/read", "thread/start", "turn/start"]);
        let all = seen.lock().unwrap().clone();
        assert!(all.iter().all(|m| m.get("jsonrpc").is_none()));
        assert_eq!(all[0]["params"]["clientInfo"]["name"], "pointcast");
        assert_eq!(all[1], json!({ "method": "initialized" }));
        assert_eq!(all[2]["params"], json!({ "refreshToken": false }));
        assert_eq!(all[4]["params"], json!({ "includeLayers": false, "cwd": CWD }));
        let t = &all[5]["params"];
        assert_eq!((&t["ephemeral"], &t["sandbox"], &t["approvalPolicy"], &t["cwd"]), (&json!(true), &json!("read-only"), &json!("untrusted"), &json!(CWD)));
        let told = t["developerInstructions"].as_str().unwrap();
        assert!(told.contains("No tools exist") && told.ends_with("Be brief."), "{told}");
        let c = &t["config"];
        assert_eq!((&c["mcp_servers"], &c["model_provider"], &c["web_search"], &c["notify"], &c["project_doc_max_bytes"]), (&json!({}), &json!("openai"), &json!("disabled"), &json!([]), &json!(0)));
        for f in FEATURES_OFF {
            assert_eq!(c[format!("features.{f}")], false, "{f} is off");
        }
        for f in ["shell_tool", "unified_exec", "view_image", "apps", "browser_use", "computer_use", "hooks", "image_generation", "multi_agent"] {
            assert!(FEATURES_OFF.contains(&f), "{f} is among those turned off");
        }
        assert_eq!(
            all[6]["params"],
            json!({ "threadId": chat.thread(), "effort": "low", "input": [{ "type": "text", "text": "What is two plus two?" }, { "type": "localImage", "path": "/pointcast/turn/01.png" }] })
        );
    }

    #[test]
    fn a_stop_interrupts_the_turn_by_its_ids_and_the_answer_ends_stopped() {
        let (mut chat, seen) = wire("slow");
        let (tx, rx) = channel();
        chat.ask(said("slowly"), tx).unwrap();
        first_word(&rx);
        // From another thread, as the panel does while the turn runs.
        let stop = chat.stopper();
        std::thread::spawn(move || stop()).join().unwrap();
        assert_eq!(collect(&rx), [Kind::Failed(Failure::Stopped)]);
        assert_eq!(sent(&seen, "turn/interrupt"), [json!({ "threadId": chat.thread(), "turnId": "turn-1" })]);
        assert!(chat.usable(), "the helper stopped the turn itself, so it is kept");
    }

    #[test]
    fn a_helper_that_ignores_the_interrupt_is_ended_within_2_seconds() {
        let p = child("stubborn", "stubborn");
        let mut chat = p.start("", &[]).ok().unwrap();
        let pid = chat.pid().unwrap();
        let (tx, rx) = channel();
        chat.ask(said("anything"), tx).unwrap();
        first_word(&rx);
        let t = Instant::now();
        chat.cancel();
        assert_eq!(rx.recv_timeout(Duration::from_secs(20)).map(|e| e.kind), Ok(Kind::Failed(Failure::Stopped)));
        assert!(child::start_time(pid).is_none(), "the helper is gone by the time the answer says stopped");
        assert!(t.elapsed() < Duration::from_secs(2), "took {:?}", t.elapsed());
        assert!(collect_rest(&rx).is_empty(), "and it ends once");
        assert_eq!(chat.ask(said("again"), channel().0).err(), Some(Failure::Broken("Codex is not running.".into())));
    }

    fn collect_rest(rx: &Receiver<Event>) -> Vec<Kind> {
        let mut all = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_millis(300)) {
            all.push(e.kind);
        }
        all
    }

    #[test]
    fn a_turn_that_fails_before_its_start_is_answered_fails_once() {
        let (mut chat, _seen) = wire("fast-fail");
        let got = answer(&mut chat, said("anything"));
        assert_eq!(got, [Kind::Failed(Failure::Broken("The request was refused before it began".into()))]);
        // The late answer to the start did not disturb the next turn.
        assert_eq!(answer(&mut chat, said("again")).len(), 1);
    }

    #[test]
    fn a_start_that_codex_refuses_fails_once() {
        let (mut chat, _seen) = wire("refused-start");
        assert_eq!(answer(&mut chat, said("anything")), [Kind::Failed(Failure::Broken("Codex did not take the question: Invalid turn".into()))]);
    }

    #[test]
    fn signed_out_is_said_when_the_conversation_opens_and_no_thread_or_turn_is_sent() {
        let (chat, seen) = wire_with("signed-out", &[]);
        assert_eq!(chat.err(), Some(Failure::SignedOut));
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(methods(&seen), ["initialize", "initialized", "account/read"]);
    }

    #[test]
    fn a_usage_limit_carries_the_reset_time_read_from_the_limits() {
        let (mut chat, seen) = wire("limit");
        assert_eq!(answer(&mut chat, said("anything")), [Kind::Failed(Failure::Limit { resets: Some(fake::RESETS) })]);
        assert_eq!(methods(&seen).last().map(String::as_str), Some("account/rateLimits/read"));
    }

    #[test]
    fn each_kind_of_error_is_the_failure_the_person_can_act_on() {
        let f = |info: Value| failure_of(&json!({ "message": "why", "codexErrorInfo": info }));
        assert_eq!(f(json!("usageLimitExceeded")), Failure::Limit { resets: None });
        assert_eq!(f(json!("rateLimitExceeded")), Failure::Limit { resets: None });
        assert_eq!(f(json!("unauthorized")), Failure::SignedOut);
        assert_eq!(f(json!("contextWindowExceeded")), Failure::ContextFull);
        assert_eq!(f(json!("cyberPolicy")), Failure::Refused);
        assert_eq!(f(json!({ "httpConnectionFailed": { "httpStatusCode": null } })), Failure::Offline);
        assert_eq!(f(json!({ "responseStreamDisconnected": {} })), Failure::Offline);
        assert_eq!(f(json!("serverOverloaded")), Failure::Broken("why".into()));
        assert_eq!(f(Value::Null), Failure::Broken("why".into()));
        assert_eq!(failure_of(&Value::Null), Failure::Broken("Codex could not answer.".into()));
    }

    #[test]
    fn every_request_codex_makes_is_declined_or_refused_at_once_and_nothing_waits() {
        let (mut chat, seen) = wire("approval");
        // The stand-in goes on only when all six are answered, so an answer at all means none was left waiting.
        let got = answer(&mut chat, said("anything"));
        assert_eq!((words(&got).concat(), got.last()), (fake::WORDS.concat(), Some(&Kind::Done)));
        let all = seen.lock().unwrap().clone();
        let back = |id: Value| all.iter().find(|m| m.get("method").is_none() && m["id"] == id).cloned().expect("answered");
        assert_eq!(back(json!(900)), json!({ "id": 900, "result": { "decision": "decline" } }));
        assert_eq!(back(json!(901)), json!({ "id": 901, "result": { "decision": "decline" } }));
        assert_eq!(back(json!(902)), json!({ "id": 902, "result": { "permissions": {}, "scope": "turn" } }));
        assert_eq!(back(json!(903)), json!({ "id": 903, "result": { "contentItems": [], "success": false } }));
        let refresh = back(json!("refresh-1"));
        assert!(refresh.get("result").is_none() && refresh["error"]["code"] == -32000, "{refresh}");
        assert_eq!(back(json!(905))["error"]["code"], -32601);
    }

    #[test]
    fn a_turn_in_which_codex_starts_a_tool_is_interrupted_and_fails_as_refused() {
        let (mut chat, seen) = wire("tool");
        assert_eq!(answer(&mut chat, said("list my files")), [Kind::Failed(Failure::Refused)]);
        assert_eq!(sent(&seen, "turn/interrupt"), [json!({ "threadId": chat.thread(), "turnId": "turn-1" })]);
    }

    #[test]
    fn a_config_that_sends_questions_elsewhere_than_openai_is_refused_before_any_question() {
        let (chat, seen) = wire_with("foreign", &[]);
        let Some(Failure::Broken(why)) = chat.err() else { panic!("a foreign provider is a failure") };
        assert!(why.contains("\"elsewhere\"") && why.contains("not to OpenAI"), "{why}");
        std::thread::sleep(Duration::from_millis(200));
        assert!(sent(&seen, "turn/start").is_empty());
    }

    #[test]
    fn a_model_without_pictures_refuses_a_picture_before_anything_is_sent() {
        let (mut chat, seen) = wire("text-only");
        assert_eq!(chat.ask(with_picture(), channel().0).err(), Some(Failure::NoPictures));
        std::thread::sleep(Duration::from_millis(200));
        assert!(sent(&seen, "turn/start").is_empty());
        // Words alone still go.
        assert_eq!(answer(&mut chat, said("words")).last(), Some(&Kind::Done));
    }

    #[test]
    fn a_model_that_lists_no_modalities_takes_pictures() {
        let (mut chat, seen) = wire("no-modalities");
        assert_eq!(answer(&mut chat, with_picture()).last(), Some(&Kind::Done));
        assert_eq!(sent(&seen, "turn/start")[0]["input"][1]["type"], "localImage");
        assert!(takes_pictures(&json!({ "id": "m" })) && takes_pictures(&json!({ "inputModalities": ["text", "image"] })));
        assert!(!takes_pictures(&json!({ "inputModalities": ["text"] })) && !takes_pictures(&json!({ "inputModalities": [] })));
    }

    #[test]
    fn unknown_notifications_are_ignored() {
        let (mut plain, _) = wire("normal");
        let (mut noisy, _) = wire("noise");
        let (a, b) = (answer(&mut plain, said("anything")), answer(&mut noisy, said("anything")));
        assert_eq!(a, b, "the three planted notes changed nothing: {b:?}");
        assert!(!words(&b).concat().contains(fake::NEVER_SHOWN));
        let ignored = |c: &CodexChat| c.shared.ignored.load(Ordering::SeqCst);
        assert_eq!(ignored(&noisy), ignored(&plain) + 3, "and three more were passed over");
    }

    #[test]
    fn thinking_usage_and_limits_reach_the_stream_between_the_words() {
        let (mut chat, _) = wire("normal");
        let got = answer(&mut chat, said("anything"));
        assert_eq!(got.first(), Some(&Kind::Thinking));
        assert!(got.contains(&Kind::Usage(Usage { input: 120, cached: 20, output: 7 })));
        let Some(Kind::Limits(l)) = got.iter().find(|k| matches!(k, Kind::Limits(_))) else { panic!("limits: {got:?}") };
        assert_eq!(l.primary, Some(Window { used_percent: 12, minutes: Some(300), resets: Some(fake::RESETS) }));
    }

    #[test]
    fn a_message_that_was_not_streamed_is_shown_once() {
        let (mut chat, _) = wire("whole");
        assert_eq!(answer(&mut chat, said("anything")), [Kind::Delta(fake::WORDS.concat()), Kind::Done]);
    }

    #[test]
    fn a_new_question_waits_for_the_stopped_one_to_end_and_its_late_words_never_reach_the_new_stream() {
        let (mut chat, seen) = wire("late");
        let (tx, rx) = channel();
        let first = chat.ask(said("one"), tx.clone()).unwrap();
        first_word(&rx);
        // Asked while the first still answers. The stand-in refuses a turn sent before it has said the first ended.
        let second = chat.ask(said("two"), tx).unwrap();
        let mut got = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(5)) {
            got.push(e);
        }
        let of = |t: TurnId| got.iter().filter(|e| e.turn == t).map(|e| e.kind.clone()).collect::<Vec<_>>();
        assert_eq!(of(first), [Kind::Failed(Failure::Stopped)]);
        let new = of(second);
        assert_eq!((words(&new), new.last(), endings(&new)), (fake::WORDS.map(String::from).to_vec(), Some(&Kind::Done), 1));
        assert!(!format!("{got:?}").contains(fake::NEVER_SHOWN), "the first turn's late words were dropped");
        assert_eq!(methods(&seen)[6..], ["turn/start", "turn/interrupt", "turn/start"]);
    }

    #[test]
    fn two_conversations_never_share_a_thread() {
        let ((mut a, seen_a), (mut b, seen_b)) = (wire("normal"), wire("normal"));
        assert_ne!(a.thread(), b.thread());
        answer(&mut a, said("study"));
        answer(&mut b, said("summarize"));
        assert_eq!(sent(&seen_a, "turn/start")[0]["threadId"], a.thread());
        assert_eq!(sent(&seen_b, "turn/start")[0]["threadId"], b.thread());
    }

    #[test]
    fn once_runs_on_a_helper_of_its_own_beside_an_open_conversation() {
        let p = child("normal", "once");
        let mut study = p.start("", &[]).ok().unwrap();
        let pid = study.pid().unwrap();
        let mut summary = p.once("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        summary.ask(said("summarize"), tx).unwrap();
        assert_eq!(collect(&rx).last(), Some(&Kind::Done));
        // Closing a conversation ends its helper, so one that shared the study conversation's would take it along.
        summary.close();
        assert!(child::start_time(pid).is_some(), "the open conversation's helper is not the summary's");
        assert_eq!(answer(&mut study, said("study")).last(), Some(&Kind::Done));
        study.close();
        assert!(child::start_time(pid).is_none());
    }

    #[test]
    fn earlier_turns_ride_with_the_first_question_only_as_the_replay_says() {
        let earlier = [
            Turn { role: Role::Person, parts: vec![Part::Text("What is a mole?".into()), shot("/pointcast/turn/00.png")] },
            Turn { role: Role::Assistant, parts: vec![Part::Text("A count of particles.".into())] },
        ];
        let (chat, seen) = wire_with("normal", &earlier);
        let mut chat = chat.ok().unwrap();
        answer(&mut chat, said("How many?"));
        answer(&mut chat, said("Why?"));
        let turns = sent(&seen, "turn/start");
        let mut want = input_of(&replay(&earlier, KEEP_PICTURES));
        want.push(json!({ "type": "text", "text": "How many?" }));
        assert_eq!(turns[0]["input"], json!(want));
        assert_eq!(turns[0]["input"][1], json!({ "type": "localImage", "path": "/pointcast/turn/00.png" }));
        assert_eq!(turns[1]["input"], json!([{ "type": "text", "text": "Why?" }]));
    }

    /// Whether anything in `v` asks for a token: a key that names one, set to anything but false.
    fn asks_for_a_token(v: &Value) -> bool {
        match v {
            Value::Object(m) => m.iter().any(|(k, x)| (k.to_lowercase().contains("token") && !matches!(x, Value::Bool(false) | Value::Null)) || asks_for_a_token(x)),
            Value::Array(l) => l.iter().any(asks_for_a_token),
            _ => false,
        }
    }

    #[test]
    fn only_the_listed_methods_are_ever_sent_and_none_asks_for_a_token() {
        assert!(asks_for_a_token(&json!({ "params": [{ "refreshToken": true }] })) && asks_for_a_token(&json!({ "includeToken": 1 })));
        assert!(!asks_for_a_token(&json!({ "refreshToken": false, "inputTokens": null })));
        let mut all = Vec::new();
        for mode in ["normal", "slow", "limit", "approval", "tool"] {
            let (mut chat, seen) = wire(mode);
            let (tx, rx) = channel();
            chat.ask(said("anything"), tx).unwrap();
            if mode == "slow" {
                first_word(&rx);
                chat.cancel();
            }
            collect(&rx);
            // What the sender itself refuses never reaches the line.
            assert!(chat.shared.call("account/login/start", json!({ "type": "chatgpt" }), ANSWER_WITHIN).is_err());
            assert!(chat.shared.post_as(99, "thread/resume", json!({})).is_err());
            all.extend(seen.lock().unwrap().clone());
        }
        let sent: HashSet<String> = all.iter().filter_map(|m| m["method"].as_str().map(str::to_owned)).collect();
        assert!(sent.iter().all(|m| SENDS.contains(&m.as_str())), "{sent:?}");
        assert_eq!(sent.len(), SENDS.len(), "every listed method was exercised, so the list holds nothing unused: {sent:?}");
        assert!(!all.iter().any(asks_for_a_token));
        for m in ["account/login/start", "account/logout", "account/chatgptAuthTokens/refresh", "thread/resume", "config/value/write"] {
            assert!(Shared::allowed(m).is_err(), "{m} is not sent");
        }
    }

    #[test]
    fn the_thread_always_names_a_model_codex_lists_and_never_falls_to_the_one_in_the_persons_config() {
        // Nothing asked for: the listed default, not the config's.
        let (chat, seen) = wire("normal");
        assert_eq!(sent(&seen, "thread/start")[0]["model"], "fake-model");
        assert_eq!(chat.model.as_deref(), Some("fake-model"));
        // A model this plan does not list: the listed default again.
        let (chat, seen) = link("normal");
        let mut chat = chat.ok().unwrap();
        chat.begin(&Setup { cwd: Path::new(CWD), system: "", earlier: &[], model: Some(fake::CONFIG_MODEL), effort: None }).unwrap();
        assert_eq!(sent(&seen, "thread/start")[0]["model"], "fake-model");
        // No list to choose from: a plain error, and no thread is started for Codex to pick a model for.
        let (chat, seen) = wire_with("no-models", &[]);
        assert_eq!(chat.err(), Some(Failure::Broken("Codex did not list its models, so Pointcast did not ask.".into())));
        assert!(sent(&seen, "thread/start").is_empty());
    }

    #[test]
    fn each_mcp_server_of_the_persons_codex_is_switched_off_by_name_when_the_thread_starts() {
        assert_eq!(servers_of(&json!({ "config": { "mcp_servers": { "b": { "command": "x" }, "a": {} } } })), ["a", "b"]);
        assert!(servers_of(&json!({ "config": {} })).is_empty() && servers_of(&Value::Null).is_empty());
        let (chat, seen) = wire("servers");
        let started = &sent(&seen, "thread/start")[0];
        let off = json!({ (fake::SERVERS[0]): { "enabled": false }, (fake::SERVERS[1]): { "enabled": false } });
        assert_eq!(started["config"]["mcp_servers"], off, "each by its name, whatever the name holds");
        // Only the names were taken: what a server runs and is given stays in Codex's answer.
        let all = format!("{:?}", seen.lock().unwrap());
        assert!(!all.contains(fake::SERVER_SECRET) && !all.contains("notes-mcp"), "{all}");
        assert!(Chat::adds(&chat).iter().all(|a| !a.starts_with(MCP_STARTED)), "none started, so none is said");
        // A Codex with no servers, and one too old to say: the thread starts with none named.
        for mode in ["normal", "old"] {
            let (_chat, seen) = wire(mode);
            assert_eq!(sent(&seen, "thread/start")[0]["config"]["mcp_servers"], json!({}), "{mode}");
            assert_eq!(methods(&seen)[4..], ["config/read", "thread/start"], "{mode}: asked once, before the thread");
        }
    }

    #[test]
    fn an_mcp_server_that_starts_anyway_is_said_with_what_was_sent_and_the_answer_still_comes() {
        let (mut chat, _) = wire("servers-start");
        let got = answer(&mut chat, said("anything"));
        assert_eq!((words(&got), got.last()), (fake::WORDS.map(String::from).to_vec(), Some(&Kind::Done)), "the turn is not failed for it");
        // The one that started or was ready, once; the one that failed to start is not through the fence.
        assert_eq!(Chat::adds(&chat), [fake::INSTRUCTIONS.to_owned(), format!("{MCP_STARTED}{}", fake::SERVERS[1])]);
        assert_eq!(chat.adds, [fake::INSTRUCTIONS.to_owned()], "the files Codex adds are still only the files");
    }

    #[test]
    fn the_files_codex_adds_to_a_conversation_are_said() {
        let (chat, _) = wire("normal");
        assert_eq!((chat.adds.clone(), Chat::adds(&chat)), (vec![fake::INSTRUCTIONS.to_owned()], vec![fake::INSTRUCTIONS.to_owned()]));
    }

    /// A sign-in served by the stand-in inside this process: what it said, what was sent, and which pages were opened.
    fn signing_in(mode: &'static str, within: Duration) -> (Result<(), String>, Seen, Vec<String>) {
        let (their_in, our_out) = std::io::pipe().unwrap();
        let (our_in, their_out) = std::io::pipe().unwrap();
        let seen = Seen::default();
        let log = seen.clone();
        std::thread::spawn(move || {
            fake::serve(mode, std::io::BufReader::new(their_in), their_out, |m| log.lock().unwrap().push(m.clone()));
        });
        let opened = Mutex::new(Vec::new());
        let said = sign_in_over(
            our_in,
            our_out,
            None,
            &|page: &str| {
                opened.lock().unwrap().push(page.to_owned());
                Ok(())
            },
            within,
        );
        (said, seen, opened.into_inner().unwrap())
    }

    #[test]
    fn a_sign_in_is_codexs_own_flow_its_page_opened_and_it_ends_when_codex_says() {
        let (said, seen, opened) = signing_in("login", Duration::from_secs(20));
        assert_eq!(said, Ok(()));
        assert_eq!(opened, [fake::SIGN_IN_PAGE]);
        assert_eq!(methods(&seen), ["initialize", "initialized", "account/login/start"]);
        assert_eq!(sent(&seen, "account/login/start"), [json!({ "type": "chatgpt" })]);
        assert!(!seen.lock().unwrap().iter().any(asks_for_a_token));
        // The sign-in line sends nothing of a conversation, and a conversation never starts a sign-in.
        for m in ["account/read", "model/list", "thread/start", "turn/start", "account/logout"] {
            assert!(!SIGN_IN.contains(&m), "{m}");
        }
        assert!(Shared::allowed("account/login/start").is_err());
        let (said, _, _) = signing_in("login-fail", Duration::from_secs(20));
        assert_eq!(said, Err("The sign-in did not finish: denied.".into()));
        let (said, _, opened) = signing_in("login-never", Duration::from_millis(300));
        assert_eq!((said, opened.len()), (Err("The sign-in was not finished in time. Try again.".into()), 1));
    }

    #[test]
    fn the_address_of_the_account_is_never_kept() {
        let (chat, _) = wire("normal");
        assert_eq!(chat.account, Account { plan: Some("plus".into()), api_key: false });
        assert!(!format!("{:?} {:?}", chat.account, chat.models).contains(fake::PLANTED));
    }

    #[test]
    fn no_helper_is_left_after_close_or_after_the_idle_time() {
        let p = child("normal", "idle");
        let mut chat = p.start("", &[]).ok().unwrap();
        let pid = chat.pid().unwrap();
        assert_eq!(answer(&mut chat, said("anything")).last(), Some(&Kind::Done));
        let t = Instant::now();
        chat.close();
        assert!(child::start_time(pid).is_none(), "none after close");
        assert!(t.elapsed() < child::CLOSE_GRACE, "it left when stdin closed, without being ended");
        assert!(!p.home.join("codex.pid").exists());

        let mut chat = p.start("", &[]).ok().unwrap();
        let pid = chat.pid().unwrap();
        let last = Instant::now();
        assert!(!close_if_idle(&mut chat, last + IDLE - Duration::from_secs(1), last));
        assert!(child::start_time(pid).is_some(), "kept while it may be asked again");
        assert!(close_if_idle(&mut chat, last + IDLE, last));
        assert!(child::start_time(pid).is_none(), "none after the idle time");
    }

    #[test]
    fn warming_starts_the_helper_and_greets_it_without_a_thread_and_the_next_conversation_takes_it() {
        let p = child("normal", "warm");
        p.warm();
        let pid = p.warm_pid().expect("a helper is running");
        assert!(p.caps(None).images);
        let mut chat = p.start("", &[]).ok().unwrap();
        assert_eq!((chat.pid(), p.warm_pid()), (Some(pid), None));
        assert_eq!(answer(&mut chat, said("anything")).last(), Some(&Kind::Done));
        drop(chat);
        assert!(gone(pid), "dropping the conversation ends its helper");
        p.warm();
        let pid = p.warm_pid().unwrap();
        p.cool();
        assert!(gone(pid), "a warmed helper nobody took is ended");
    }

    #[test]
    fn a_helper_that_dies_fails_the_answer_once_with_what_it_said_last() {
        let p = child("crash", "crash");
        let mut chat = p.start("", &[]).ok().unwrap();
        let got = answer(&mut chat, said("anything"));
        let [Kind::Failed(Failure::Broken(why))] = got.as_slice() else { panic!("one failure: {got:?}") };
        assert!(why.contains(fake::LAST_WORDS), "{why}");
    }

    #[test]
    fn the_folder_codex_starts_in_is_made_empty_and_one_with_files_is_refused() {
        let home = scratch("empty");
        let d = empty_folder(&home).unwrap();
        assert!(d.is_dir() && d.starts_with(&home));
        std::fs::write(d.join("notes.txt"), b"x").unwrap();
        assert!(matches!(empty_folder(&home), Err(Failure::Broken(_))));
    }
}
