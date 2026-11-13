//! A stand-in for `codex app-server`: it speaks the same lines and can be made to misbehave. Tests run it inside
//! their own process over pipes, and as a real child by running the test program again (see `codex::tests`), so
//! nothing here ever reaches a real `codex`, an account or the network.

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How the stand-in's work ended, for the process that hosts it to act out.
pub enum Exit {
    Eof,
    /// Die with `LAST_WORDS` on stderr.
    Crash,
    /// Stay alive although stdin closed.
    Stubborn,
}

pub const WORDS: [&str; 3] = ["Two ", "plus two ", "is four."];
pub const RESETS: i64 = 1_900_000_000;
pub const LAST_WORDS: &str = "fake_codex: out of cheese";
/// Stands where an account's address would be, so a test can look for it in what the adapter keeps.
pub const PLANTED: &str = "planted-address";
pub const NEVER_SHOWN: &str = "NOT SHOWN";
/// The model the person's own Codex config names, which their plan does not offer: a thread started without a model
/// runs on it.
pub const CONFIG_MODEL: &str = "config-only-model";
/// The file Codex itself adds to every conversation.
pub const INSTRUCTIONS: &str = "/home/someone/.codex/AGENTS.md";
pub const SIGN_IN_PAGE: &str = "https://auth.example.test/start";
/// The MCP servers the person's own Codex is set up with, in the `servers` modes.
pub const SERVERS: [&str; 2] = ["notes", "code.repl 2"];
/// The name Pointcast's own server goes by in a thread that may act.
pub const ACTING_SERVER: &str = "pointcast_turn";
/// What one of those servers is given in its environment: never Pointcast's to keep or send.
pub const SERVER_SECRET: &str = "planted-server-secret";

static THREADS: AtomicU64 = AtomicU64::new(0);

type Say = Arc<dyn Fn(Value) + Send + Sync>;

fn note(say: &Say, method: &str, params: Value) {
    say(json!({ "method": method, "params": params }));
}

fn ended(say: &Say, thread: &str, turn: &str, status: &str, error: Value) {
    note(say, "turn/completed", json!({ "threadId": thread, "turn": { "id": turn, "items": [], "status": status, "error": error } }));
}

fn delta(say: &Say, thread: &str, turn: &str, words: &str) {
    note(say, "item/agentMessage/delta", json!({ "threadId": thread, "turnId": turn, "itemId": format!("msg-{turn}"), "delta": words }));
}

fn limits(used: u32) -> Value {
    json!({ "primary": { "usedPercent": used, "windowDurationMins": 300, "resetsAt": RESETS }, "secondary": { "usedPercent": 40, "windowDurationMins": 10080, "resetsAt": RESETS + 500_000 } })
}

/// The whole of a good answer.
fn answer(say: &Say, mode: &str, thread: &str, turn: &str) {
    let item = format!("msg-{turn}");
    note(say, "item/reasoning/summaryTextDelta", json!({ "threadId": thread, "turnId": turn, "itemId": "think-1", "delta": "Adding.", "summaryIndex": 0 }));
    note(say, "item/started", json!({ "threadId": thread, "turnId": turn, "item": { "type": "agentMessage", "id": item, "text": "" } }));
    delta(say, thread, turn, WORDS[0]);
    if mode == "noise" {
        say(json!({ "method": "pointcast/planted/two" }));
    }
    delta(say, thread, turn, WORDS[1]);
    if mode == "noise" {
        note(say, "pointcast/planted/three", json!({ "threadId": thread, "turnId": turn, "itemId": item, "delta": NEVER_SHOWN }));
    }
    delta(say, thread, turn, WORDS[2]);
    note(say, "item/completed", json!({ "threadId": thread, "turnId": turn, "completedAtMs": 1, "item": { "type": "agentMessage", "id": item, "text": WORDS.concat() } }));
    note(say, "thread/tokenUsage/updated", json!({ "threadId": thread, "turnId": turn, "tokenUsage": { "last": { "inputTokens": 120, "cachedInputTokens": 20, "outputTokens": 7, "reasoningOutputTokens": 0, "totalTokens": 127 }, "total": {} } }));
    note(say, "account/rateLimits/updated", json!({ "rateLimits": limits(12) }));
    ended(say, thread, turn, "completed", Value::Null);
}

/// Serve one client until its end of the line closes. `saw` is shown every message the client sent.
pub fn serve(mode: &str, input: impl BufRead, out: impl Write + Send + 'static, mut saw: impl FnMut(&Value)) -> Exit {
    let out = Mutex::new(out);
    let say: Say = Arc::new(move |v: Value| {
        let mut o = out.lock().unwrap();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    });
    let new_thread = || format!("thread-{}-{}", std::process::id(), THREADS.fetch_add(1, Ordering::SeqCst) + 1);
    let mut thread = new_thread();
    let mut threads = 0u32;
    let running = Arc::new(AtomicBool::new(false));
    let (mut ready, mut turns, mut owed) = (false, 0u32, 0usize);
    let mut turn = String::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        let Ok(m) = serde_json::from_str::<Value>(&line) else { continue };
        saw(&m);
        let id = m.get("id").cloned().unwrap_or(Value::Null);
        let reply = |result: Value| say(json!({ "id": id, "result": result }));
        let refuse = |code: i64, why: &str| say(json!({ "id": id, "error": { "code": code, "message": why } }));
        let Some(method) = m.get("method").and_then(Value::as_str) else {
            // An answer to a request of ours: once all are in, the turn goes on.
            if owed > 0 {
                owed -= 1;
                if owed == 0 {
                    running.store(false, Ordering::SeqCst);
                    answer(&say, mode, &thread, &turn);
                }
            }
            continue;
        };
        if !ready && method != "initialize" {
            if !id.is_null() {
                refuse(-32600, "Not initialized");
            }
            continue;
        }
        match method {
            "initialize" => {
                ready = true;
                reply(json!({ "userAgent": "fake_codex/0.164.0" }));
            }
            "initialized" => {
                if mode == "noise" {
                    note(&say, "pointcast/planted/one", json!({ "n": 1 }));
                }
            }
            "account/read" => {
                let account = if mode == "signed-out" { Value::Null } else { json!({ "type": "chatgpt", "email": PLANTED, "planType": "plus" }) };
                reply(json!({ "account": account, "requiresOpenaiAuth": true }));
            }
            "model/list" => {
                let mut model = json!({ "id": "fake-model", "model": "fake-model", "displayName": "Fake", "description": "", "hidden": false, "isDefault": true, "defaultReasoningEffort": "medium", "supportedReasoningEfforts": [{ "reasoningEffort": "low", "description": "" }, { "reasoningEffort": "medium", "description": "" }], "inputModalities": ["text", "image"] });
                match mode {
                    "text-only" => model["inputModalities"] = json!(["text"]),
                    "tiers" => {
                        model["serviceTiers"] = json!(["flex"]);
                        model["additionalSpeedTiers"] = json!([{ "id": "priority", "name": "Fast" }, "flex"]);
                    }
                    "no-modalities" => {
                        model.as_object_mut().unwrap().remove("inputModalities");
                    }
                    _ => {}
                }
                let data = if mode == "no-models" { json!([]) } else { json!([model]) };
                reply(json!({ "data": data, "nextCursor": null }));
            }
            // The person's own config, as `config/read` says it: in the `servers` modes two MCP servers of theirs, one
            // with a name no dotted key could carry and one given a secret in its environment.
            "config/read" if mode != "old" => {
                let servers = if mode.starts_with("servers") {
                    json!({ (SERVERS[0]): { "command": "notes-mcp", "env": { "NOTES_KEY": SERVER_SECRET } }, (SERVERS[1]): { "command": "python", "args": ["-m", "repl"] } })
                } else {
                    json!({})
                };
                reply(json!({ "config": { "model": CONFIG_MODEL, "mcp_servers": servers }, "origins": {} }));
            }
            "account/rateLimits/read" => reply(json!({ "rateLimits": limits(if mode == "limit" { 100 } else { 12 }) })),
            "thread/start" => {
                // Each thread after the first has an id of its own.
                threads += 1;
                if threads > 1 {
                    thread = new_thread();
                }
                let provider = if mode == "foreign" { "elsewhere" } else { "openai" };
                let model = m["params"]["model"].as_str().unwrap_or(CONFIG_MODEL).to_owned();
                reply(json!({ "thread": { "id": thread }, "model": model, "modelProvider": provider, "instructionSources": [INSTRUCTIONS] }));
                note(&say, "thread/started", json!({ "thread": { "id": thread } }));
            }
            "turn/start" => {
                if running.load(Ordering::SeqCst) {
                    refuse(-32600, "A turn is already running");
                    continue;
                }
                turns += 1;
                let last = std::mem::replace(&mut turn, format!("turn-{turns}"));
                let started = json!({ "turn": { "id": turn, "items": [], "status": "inProgress" } });
                match mode {
                    "crash" => return Exit::Crash,
                    "refused-start" => {
                        refuse(-32602, "Invalid turn");
                        continue;
                    }
                    "fast-fail" => {
                        ended(&say, &thread, &turn, "failed", json!({ "message": "The request was refused before it began", "codexErrorInfo": "badRequest" }));
                        reply(started);
                        continue;
                    }
                    _ => {}
                }
                reply(started);
                note(&say, "turn/started", json!({ "threadId": thread, "turn": { "id": turn } }));
                if mode == "servers-start" {
                    // A Codex that starts the person's servers although the thread switched them off: one is seen
                    // starting and then ready, one fails to start.
                    let status = |name: &str, status: &str| note(&say, "mcpServer/startupStatus/updated", json!({ "name": name, "status": status, "threadId": thread, "error": null }));
                    status(SERVERS[1], "starting");
                    status(SERVERS[0], "failed");
                    status(SERVERS[1], "ready");
                }
                match mode {
                    "limit" => ended(&say, &thread, &turn, "failed", json!({ "message": "You have hit your usage limit", "codexErrorInfo": "usageLimitExceeded" })),
                    "approval" => {
                        running.store(true, Ordering::SeqCst);
                        let ask = |id: Value, method: &str| say(json!({ "id": id, "method": method, "params": { "threadId": thread, "turnId": turn, "itemId": "ask" } }));
                        ask(json!(900), "item/commandExecution/requestApproval");
                        ask(json!(901), "item/fileChange/requestApproval");
                        ask(json!(902), "item/permissions/requestApproval");
                        ask(json!(903), "item/tool/call");
                        ask(json!("refresh-1"), "account/chatgptAuthTokens/refresh");
                        ask(json!(905), "pointcast/unknown/request");
                        owed = 6;
                    }
                    "tool" => {
                        running.store(true, Ordering::SeqCst);
                        note(&say, "item/started", json!({ "threadId": thread, "turnId": turn, "item": { "type": "commandExecution", "id": "cmd-1", "command": "ls" } }));
                    }
                    // A turn that may act: what it is asked says which tool it reaches for.
                    "acting" => {
                        let asked = m["params"]["input"].as_array().and_then(|i| i.last()).and_then(|q| q["text"].as_str()).unwrap_or_default().to_owned();
                        let item = |server: &str, tool: &str| note(&say, "item/started", json!({ "threadId": thread, "turnId": turn, "item": { "type": "mcpToolCall", "id": "mcp-1", "server": server, "tool": tool, "status": "inProgress", "arguments": {} } }));
                        running.store(true, Ordering::SeqCst);
                        if asked.contains("bash") {
                            note(&say, "item/started", json!({ "threadId": thread, "turnId": turn, "item": { "type": "commandExecution", "id": "cmd-1", "command": "ls" } }));
                        } else if asked.contains("notes") {
                            item(SERVERS[0], "search");
                        } else if asked.contains("slowly") {
                            item(ACTING_SERVER, "pointcast_type");
                            delta(&say, &thread, &turn, WORDS[0]);
                        } else {
                            item(ACTING_SERVER, "pointcast_apps");
                            // Codex asks before a tool runs, in the words of 0.167.2: only a computer tool on Pointcast's
                            // server, in this thread, may pass.
                            let ask = |id: u64, server: &str, in_thread: &str, tool: &str| {
                                let message = format!("Allow the {server} MCP server to run tool \"{tool}\"?");
                                say(json!({ "id": id, "method": "mcpServer/elicitation/request", "params": { "threadId": in_thread, "turnId": turn, "serverName": server, "mode": "form", "message": message, "requestedSchema": { "type": "object", "properties": {} }, "_meta": { "codex_approval_kind": "mcp_tool_call" } } }));
                            };
                            ask(910, ACTING_SERVER, &thread, "pointcast_click");
                            ask(911, SERVERS[0], &thread, "pointcast_click");
                            ask(912, ACTING_SERVER, &thread, "shell");
                            ask(913, ACTING_SERVER, "thread-earlier", "pointcast_click");
                            say(json!({ "id": 914, "method": "item/commandExecution/requestApproval", "params": { "threadId": thread, "turnId": turn, "itemId": "ask" } }));
                            owed = 5;
                        }
                    }
                    "whole" => {
                        note(&say, "item/completed", json!({ "threadId": thread, "turnId": turn, "completedAtMs": 1, "item": { "type": "agentMessage", "id": "msg-whole", "text": WORDS.concat() } }));
                        ended(&say, &thread, &turn, "completed", Value::Null);
                    }
                    "stubborn" => {
                        running.store(true, Ordering::SeqCst);
                        delta(&say, &thread, &turn, WORDS[0]);
                    }
                    "slow" | "late" if turns == 1 => {
                        running.store(true, Ordering::SeqCst);
                        delta(&say, &thread, &turn, WORDS[0]);
                    }
                    "late" => {
                        // Words of the turn before, arriving after it ended.
                        delta(&say, &thread, &last, NEVER_SHOWN);
                        answer(&say, mode, &thread, &turn);
                    }
                    _ => answer(&say, mode, &thread, &turn),
                }
            }
            "account/login/start" if mode.starts_with("login") => {
                reply(json!({ "type": "chatgpt", "loginId": "login-1", "authUrl": SIGN_IN_PAGE }));
                // Another sign-in's end is not this one's.
                note(&say, "account/login/completed", json!({ "loginId": "login-0", "success": true, "error": null }));
                match mode {
                    "login" => note(&say, "account/login/completed", json!({ "loginId": "login-1", "success": true, "error": null })),
                    "login-fail" => note(&say, "account/login/completed", json!({ "loginId": "login-1", "success": false, "error": "denied" })),
                    _ => {}
                }
            }
            "turn/interrupt" => {
                if mode == "stubborn" {
                    continue;
                }
                reply(json!({}));
                if !running.load(Ordering::SeqCst) {
                    continue;
                }
                // The turn ends a moment after the interrupt is answered, as a real one does: a client that sends
                // its next turn before this arrives is told a turn is already running.
                let (say, running, thread, turn) = (say.clone(), running.clone(), thread.clone(), turn.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(150));
                    running.store(false, Ordering::SeqCst);
                    ended(&say, &thread, &turn, "interrupted", Value::Null);
                });
            }
            _ => {
                if !id.is_null() {
                    refuse(-32601, "Method not found");
                }
            }
        }
    }
    if mode == "stubborn" {
        Exit::Stubborn
    } else {
        Exit::Eof
    }
}
