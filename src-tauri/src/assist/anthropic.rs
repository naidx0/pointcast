//! Anthropic's own API with a key the person pasted ("Claude API"): the Messages API (`POST {base}/messages`, streamed
//! as server-sent events). The line, the history, the stop and the one ending are the OpenAI adapter's (`openai`);
//! what differs is here: how a request is written (the instructions apart from the messages, a picture as a base64
//! block) and how each event of the answer is read. The key rides in `x-api-key` (`http::Wire::Anthropic`).

use super::http::Wire;
use super::openai::{refusal, Answer, Bit, Config, Said, KEEP_PICTURES};
use super::{Failure, Kind, Role, Usage};
use serde_json::{json, Value};

pub const ID: &str = "anthropic";
pub const BASE: &str = "https://api.anthropic.com/v1";

/// The longest answer asked for, in tokens: the API wants a limit, and a study answer is far shorter.
pub const MAX_TOKENS: u32 = 4096;

/// A service asked over the Messages API with the key the person pasted for `id`.
pub fn config(id: &'static str, base_url: &str, model: &str) -> Config {
    Config { wire: Wire::Anthropic, ..super::openai::remote(id, base_url, model) }
}

fn block(b: &Bit, with_pictures: bool) -> Option<Value> {
    match b {
        Bit::Words(w) if w.is_empty() => None,
        Bit::Words(w) => Some(json!({ "type": "text", "text": w })),
        Bit::Picture(url) if with_pictures => {
            let (head, data) = url.split_once(";base64,").unwrap_or(("", ""));
            Some(json!({ "type": "image", "source": { "type": "base64", "media_type": head.trim_start_matches("data:"), "data": data } }))
        }
        Bit::Picture(_) => Some(json!({ "type": "text", "text": "[a picture, not sent again]" })),
    }
}

/// One message: its pictures first, as Anthropic advises, then its words.
fn message(said: &Said, with_pictures: bool) -> Value {
    let role = if said.by == Role::Person { "user" } else { "assistant" };
    let (pictures, words): (Vec<&Bit>, Vec<&Bit>) = said.bits.iter().partition(|b| with_pictures && matches!(b, Bit::Picture(_)));
    let content: Vec<Value> = pictures.into_iter().chain(words).filter_map(|b| block(b, with_pictures)).collect();
    json!({ "role": role, "content": content })
}

/// The whole request for one question: what was said before, then the question; the instructions apart.
pub(super) fn request(model: &str, system: &str, history: &[Said], new: &Said) -> Value {
    let all: Vec<&Said> = history.iter().chain(Some(new)).collect();
    let asked: Vec<usize> = (0..all.len()).filter(|i| all[*i].by == Role::Person).collect();
    let kept = &asked[asked.len().saturating_sub(KEEP_PICTURES)..];
    let messages: Vec<Value> = all.iter().enumerate().map(|(i, s)| message(s, kept.contains(&i))).collect();
    let mut body = json!({ "model": model, "max_tokens": MAX_TOKENS, "messages": messages, "stream": true });
    if !system.is_empty() {
        body["system"] = json!(system);
    }
    body
}

/// The status an error type of Anthropic's stands for, so an error inside the stream reads as one before it.
fn code_of(kind: &str) -> u16 {
    match kind {
        "invalid_request_error" => 400,
        "authentication_error" => 401,
        "permission_error" => 403,
        "not_found_error" => 404,
        "request_too_large" => 413,
        "rate_limit_error" => 429,
        "overloaded_error" => 529,
        _ => 500,
    }
}

fn think(a: &mut Answer) {
    if !a.thought && a.words.is_empty() {
        a.thought = true;
        a.tell(Kind::Thinking);
    }
}

/// Take one event of the answer.
pub(super) fn take(a: &mut Answer, v: &Value) {
    match v["type"].as_str() {
        Some("message_start") => {
            let u = &v["message"]["usage"];
            if let Some(input) = u["input_tokens"].as_u64() {
                a.usage = Some(Usage { input, cached: u["cache_read_input_tokens"].as_u64().unwrap_or(0), output: u["output_tokens"].as_u64().unwrap_or(0) });
            }
        }
        Some("content_block_start") if matches!(v["content_block"]["type"].as_str(), Some("thinking" | "redacted_thinking")) => think(a),
        Some("content_block_delta") => {
            let d = &v["delta"];
            match d["type"].as_str() {
                Some("text_delta") => {
                    if let Some(w) = d["text"].as_str().filter(|w| !w.is_empty()) {
                        a.words.push_str(w);
                        a.tell(Kind::Delta(w.to_owned()));
                    }
                }
                Some("thinking_delta") => think(a),
                _ => {}
            }
        }
        Some("message_delta") => {
            if let (Some(u), Some(out)) = (a.usage.as_mut(), v["usage"]["output_tokens"].as_u64()) {
                u.output = out;
            }
            match v["delta"]["stop_reason"].as_str() {
                Some("refusal") => a.end(Some(Failure::Refused)),
                Some(_) => a.finished = true,
                None => {}
            }
        }
        Some("message_stop") => a.end(None),
        Some("error") => {
            let e = &v["error"];
            let f = refusal(code_of(e["type"].as_str().unwrap_or_default()), None, e["message"].as_str().unwrap_or_default(), a.pictures);
            a.end(Some(f));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::http::tests::{event, events, stub, Reply, KEY};
    use crate::assist::tests::{collect, endings, said, scratch, words};
    use crate::assist::vault::Secret;
    use crate::assist::{Chat, Mime, Part, Provider, Turn};
    use base64::Engine;
    use std::sync::mpsc::channel;

    fn ev(v: Value) -> String {
        v.to_string()
    }

    fn text(words: &str) -> String {
        ev(json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": words } }))
    }

    fn start() -> String {
        ev(json!({ "type": "message_start", "message": { "usage": { "input_tokens": 50, "cache_read_input_tokens": 8, "output_tokens": 1 } } }))
    }

    fn stop(reason: &str) -> String {
        ev(json!({ "type": "message_delta", "delta": { "stop_reason": reason }, "usage": { "output_tokens": 6 } }))
    }

    const END: &str = r#"{"type":"message_stop"}"#;

    fn claude(base: &str) -> crate::assist::openai::OpenAi {
        config("stub", base, "claude-test").provider(false).unwrap().with_key(Secret::new(KEY))
    }

    fn answer(chat: &mut dyn Chat, turn: Turn) -> Vec<Kind> {
        let (tx, rx) = channel();
        chat.ask(turn, tx).unwrap();
        collect(&rx)
    }

    #[test]
    fn a_question_goes_to_messages_with_the_key_in_its_own_header_the_instructions_apart_and_the_picture_as_base64() {
        let pieces = vec![
            (2, event(&start())),
            // Anthropic names each event on a line of its own; only the data is read.
            (2, format!("event: ping\ndata: {}\n\n", json!({ "type": "ping" }))),
            (2, event(&text("Two "))),
            (2, event(&text("is two."))),
            (2, event(&stop("end_turn"))),
            (2, event(END)),
        ];
        let s = stub(move |_| Reply::Stream(pieces.clone()));
        let d = scratch("anthropic-shape");
        let bytes = [9u8, 8, 7, 6];
        std::fs::write(d.join("q.png"), bytes).unwrap();
        let picture = Part::Picture { path: d.join("q.png"), w: 2, h: 2, mime: Mime::Png };
        let mut chat = claude(&s.base).open("Be brief.", &[]).ok().unwrap();
        let got = answer(&mut *chat, Turn { role: Role::Person, parts: vec![Part::Text("What is it?".into()), picture] });
        assert_eq!(words(&got), ["Two ", "is two."]);
        assert_eq!(got[got.len() - 2..], [Kind::Usage(Usage { input: 50, cached: 8, output: 6 }), Kind::Done]);
        assert_eq!(endings(&got), 1);
        let seen = s.seen.lock().unwrap();
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", "/v1/messages"));
        let head = seen[0].head.to_lowercase();
        assert!(head.contains(&format!("x-api-key: {KEY}")) && head.contains("anthropic-version: 2023-06-01"), "{head}");
        assert!(!head.contains("authorization"), "the key is not also a bearer: {head}");
        let body: Value = serde_json::from_str(&seen[0].body).unwrap();
        assert_eq!((body["model"].as_str(), body["system"].as_str(), body["stream"].as_bool()), (Some("claude-test"), Some("Be brief."), Some(true)));
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        assert_eq!(
            body["messages"],
            json!([{ "role": "user", "content": [
                { "type": "image", "source": { "type": "base64", "media_type": "image/png", "data": encoded } },
                { "type": "text", "text": "What is it?" },
            ] }])
        );
        assert!(!seen[0].body.contains(KEY));
    }

    #[test]
    fn the_next_question_carries_what_was_said_and_thinking_shows_once_before_the_words() {
        let s = stub(|_| {
            let think = ev(json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "thinking", "thinking": "" } }));
            let more = ev(json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "thinking_delta", "thinking": "Hmm." } }));
            events(&[&start(), &think, &more, &text("ok"), &stop("end_turn"), END])
        });
        let mut chat = claude(&s.base).open("", &[]).ok().unwrap();
        let first = answer(&mut *chat, said("one"));
        assert_eq!(first[..2], [Kind::Thinking, Kind::Delta("ok".into())]);
        answer(&mut *chat, said("two"));
        let body = &s.bodies()[1];
        assert!(body.get("system").is_none(), "no instructions, no system field");
        let roles: Vec<&str> = body["messages"].as_array().unwrap().iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(roles, ["user", "assistant", "user"]);
        assert_eq!(body["messages"][1]["content"], json!([{ "type": "text", "text": "ok" }]));
    }

    #[test]
    fn each_refusal_and_error_in_the_stream_is_the_failure_the_person_can_act_on() {
        let refused = |reply: fn() -> Reply| {
            let s = stub(move |_| reply());
            let mut chat = claude(&s.base).open("", &[]).ok().unwrap();
            let got = answer(&mut *chat, said("anything"));
            assert_eq!(endings(&got), 1, "{got:?}");
            got.last().cloned().unwrap()
        };
        fn err(kind: &str, message: &str) -> String {
            ev(json!({ "type": "error", "error": { "type": kind, "message": message } }))
        }
        assert_eq!(refused(|| Reply::Whole(401, "", r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#.into())), Kind::Failed(Failure::SignedOut));
        assert!(matches!(refused(|| Reply::Whole(429, "retry-after: 30\r\n", "{}".into())), Kind::Failed(Failure::Limit { resets: Some(_) })));
        assert_eq!(
            refused(|| Reply::Whole(400, "", r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#.into())),
            Kind::Failed(Failure::ContextFull)
        );
        assert_eq!(
            refused(|| Reply::Whole(400, "", r#"{"type":"error","error":{"type":"invalid_request_error","message":"Your credit balance is too low to access the Anthropic API."}}"#.into())),
            Kind::Failed(Failure::Limit { resets: None })
        );
        assert_eq!(refused(|| events(&[&start(), &text("I can't"), &stop("refusal"), END])), Kind::Failed(Failure::Refused));
        assert_eq!(refused(|| events(&[&start(), &err("overloaded_error", "Overloaded")])), Kind::Failed(Failure::Broken("Overloaded".into())));
        assert_eq!(refused(|| events(&[&start(), &err("rate_limit_error", "Slow down")])), Kind::Failed(Failure::Limit { resets: None }));
    }

    #[test]
    fn the_models_are_listed_with_the_key_in_its_own_header() {
        let s = stub(|_| Reply::Whole(200, "", r#"{"data":[{"type":"model","id":"claude-b","display_name":"B"},{"type":"model","id":"claude-a","display_name":"A"}]}"#.into()));
        let listed = claude(&s.base).listed().unwrap();
        assert_eq!(listed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["claude-a", "claude-b"]);
        assert!(listed.iter().all(|m| m.images));
        let seen = s.seen.lock().unwrap();
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("GET", "/v1/models"));
        assert!(seen[0].head.to_lowercase().contains(&format!("x-api-key: {KEY}")));
    }
}
