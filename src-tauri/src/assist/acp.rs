//! A subscription through a vendor's own agent that speaks the Agent Client Protocol (JSON-RPC 2.0 over its stdio):
//! the Gemini CLI, GitHub Copilot's CLI and OpenCode, each found on this computer and signed in by the person through
//! that vendor's own CLI. Pointcast never sees a token. Like Codex, each is a whole coding agent, so the conversation
//! is fenced in to question and answer: Pointcast offers it no files and no terminal, starts the session in an empty
//! folder of its own with no MCP servers, rejects every permission it asks for, refuses every other request, and a
//! turn that starts a tool anyway is cancelled and failed as refused.

use super::child::{self, Launch, Proc, Tail};
use super::jsonl::{CallError, Client, Incoming, RpcError};
use super::{detect, replay, Caps, Chat, Event, Failure, Health, Kind, Mime, Part, Provider, State, Turn, TurnId};
use base64::Engine;
use serde_json::{json, Value};
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
pub const SENDS: &[&str] = &["initialize", "session/new", "session/prompt", "session/cancel"];

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
    session: String,
    active: Option<Active>,
    next: Option<Queued>,
    serial: u64,
    dead: bool,
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
                        self.rpc.answer(&id, reject(&params))
                    } else {
                        self.rpc.refuse(&id, -32601, "Pointcast offers no files, terminal or tools.")
                    };
                }
                Incoming::Notification { method, params } if method == "session/update" => self.updated(&params["update"]),
                Incoming::Notification { .. } => {}
                Incoming::Reply { id, result } => self.replied(id, result),
                Incoming::Closed => break,
            }
        }
        self.gone();
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
            a.tell(Kind::Failed(a.stopping.clone().unwrap_or(Failure::Broken(why.clone()))));
        }
        if let Some(q) = q {
            let _ = q.out.send(Event { turn: q.id, kind: Kind::Failed(Failure::Broken(why)) });
        }
    }

    fn send_next(&self, q: Option<Queued>) {
        let Some(q) = q else { return };
        let call = self.rpc.reserve();
        self.inner.lock().unwrap().active = Some(Active { id: q.id, out: q.out, call, stopping: None, thought: false });
        if self.post_as(call, "session/prompt", q.params).is_err() {
            let a = self.inner.lock().unwrap().active.take_if(|a| a.call == call);
            if let Some(a) = a {
                a.tell(Kind::Failed(Failure::Broken(stopped(self.name, &self.last_words()))));
            }
        }
    }

    fn replied(&self, id: u64, result: Result<Value, RpcError>) {
        let (a, q) = {
            let mut s = self.inner.lock().unwrap();
            if !s.active.as_ref().is_some_and(|a| a.call == id) {
                return;
            }
            (s.active.take(), s.next.take())
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
        a.tell(end);
        self.send_next(q);
    }

    fn updated(self: &Arc<Self>, u: &Value) {
        let mut s = self.inner.lock().unwrap();
        let Some(a) = s.active.as_mut() else { return };
        if a.stopping.is_some() {
            return;
        }
        match u["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => {
                if let Some(t) = u["content"]["text"].as_str().filter(|t| !t.is_empty()) {
                    a.tell(Kind::Delta(t.to_owned()));
                }
            }
            Some("agent_thought_chunk") if !a.thought => {
                a.thought = true;
                a.tell(Kind::Thinking);
            }
            // The agent began to act on the computer: cancel the turn, which ends refused.
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
        let session = s.session.clone();
        let Some(a) = s.active.as_mut() else { return };
        if a.stopping.is_some() {
            return;
        }
        a.stopping = Some(why);
        let id = a.id;
        drop(s);
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
                a.tell(Kind::Failed(a.stopping.clone().unwrap_or(Failure::Stopped)));
            }
            if let Some(q) = q {
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

/// A conversation with an ACP agent: greeted, with a session of its own.
pub struct AcpChat {
    shared: Arc<Shared>,
    /// Whether the agent said it reads pictures.
    pub pictures: bool,
    /// What goes before the first question: the fence, the instructions and what a resumed conversation said.
    preamble: Vec<Part>,
}

impl AcpChat {
    /// Greet an agent over these streams and start a session in `cwd`. Nothing is asked and nothing is spent.
    pub fn over<R: Read + Send + 'static, W: Write + Send + 'static>(
        reader: R,
        writer: W,
        proc: Option<Proc>,
        name: &'static str,
        cwd: &Path,
        system: &str,
        earlier: &[Turn],
    ) -> Result<AcpChat, Failure> {
        let (rpc, incoming) = Client::rpc2(reader, writer);
        let tail = proc.as_ref().map(Proc::tail);
        let shared = Arc::new(Shared { rpc, name, inner: Mutex::default(), proc: Mutex::new(proc), tail });
        let pump = shared.clone();
        std::thread::spawn(move || pump.run(incoming));
        let greet = || -> Result<(bool, String), Failure> {
            let hello = shared.call("initialize", initialize(), START_WITHIN)?;
            let pictures = hello["agentCapabilities"]["promptCapabilities"]["image"].as_bool() == Some(true);
            let made = shared.call("session/new", json!({ "cwd": cwd.to_string_lossy(), "mcpServers": [] }), ANSWER_WITHIN)?;
            let session = made["sessionId"].as_str().unwrap_or_default().to_owned();
            if session.is_empty() {
                return Err(Failure::Broken(format!("{name} started no conversation.")));
            }
            Ok((pictures, session))
        };
        match greet() {
            Ok((pictures, session)) => {
                shared.inner.lock().unwrap().session = session;
                let fence = if system.trim().is_empty() { NO_TOOLS.to_owned() } else { format!("{NO_TOOLS}\n\n{}", system.trim()) };
                let mut preamble = vec![Part::Text(fence)];
                preamble.extend(replay(earlier, if pictures { KEEP_PICTURES } else { 0 }));
                Ok(AcpChat { shared, pictures, preamble })
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
        let mut prompt = blocks(&self.preamble)?;
        prompt.extend(blocks(&turn.parts)?);
        let call = self.shared.rpc.reserve();
        let mut s = self.shared.inner.lock().unwrap();
        if s.dead || s.session.is_empty() {
            return Err(Failure::Broken(format!("{} is not running.", self.shared.name)));
        }
        let params = json!({ "sessionId": s.session, "prompt": prompt });
        s.serial += 1;
        let id = TurnId(s.serial);
        self.preamble.clear();
        if s.active.is_some() {
            // An answer still comes: cancel it, and send this turn once the agent says that one has ended.
            if let Some(old) = s.next.replace(Queued { id, out, params }) {
                let _ = old.out.send(Event { turn: old.id, kind: Kind::Failed(Failure::Stopped) });
            }
            drop(s);
            self.shared.stop_for(Failure::Stopped, true);
            return Ok(id);
        }
        s.active = Some(Active { id, out, call, stopping: None, thought: false });
        drop(s);
        if self.shared.post_as(call, "session/prompt", params).is_err() && self.shared.inner.lock().unwrap().active.take_if(|a| a.call == call).is_some() {
            return Err(Failure::Broken(stopped(self.shared.name, &self.shared.last_words())));
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
}

impl Acp {
    pub fn new(agent: &'static Agent, home: PathBuf) -> Acp {
        Acp { agent, home, launch: None, pictures: Mutex::default() }
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
        let chat = AcpChat::over(stdout, stdin, Some(proc), self.agent.name, &cwd, system, earlier)?;
        *self.pictures.lock().unwrap() = Some(chat.pictures);
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

    pub enum Exit {
        Eof,
        Crash,
    }

    fn update(say: &dyn Fn(Value), kind: &str, more: Value) {
        let mut u = json!({ "sessionUpdate": kind });
        for (k, v) in more.as_object().cloned().unwrap_or_default() {
            u[k] = v;
        }
        say(json!({ "jsonrpc": "2.0", "method": "session/update", "params": { "sessionId": SESSION, "update": u } }));
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
                Some("initialize") => {
                    reply(json!({ "protocolVersion": 1, "agentCapabilities": { "loadSession": false, "promptCapabilities": { "image": mode != "noimage" } }, "authMethods": [] }))
                }
                Some("session/new") if mode == "signedout" => say(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": -32000, "message": "Authentication required" } })),
                Some("session/new") => reply(json!({ "sessionId": SESSION })),
                Some("session/prompt") => match mode {
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
        (AcpChat::over(our_in, our_out, None, "Gemini", Path::new(CWD), "Be brief.", earlier), seen)
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
}
