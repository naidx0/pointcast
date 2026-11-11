//! Any service that speaks OpenAI's chat API (`POST {base}/chat/completions`, streamed): OpenAI, OpenRouter, a
//! Gemini key through Google's compatible endpoint, and a model on this computer served by Ollama, LM Studio or
//! llama.cpp, which costs nothing and sends nothing anywhere. The API keeps no conversation, so the adapter does:
//! every question goes out with the instructions, what was said before it and the pictures of only the last
//! questions. A pasted key comes from the system's store (`vault`) and is never held in a setting. Anthropic's own
//! API differs only in how a request is written and an answer read (`anthropic`), so it rides the same machinery.

use super::http::{self, Ask, Line, Piece, Stopper, Trouble, Wire};
use super::vault::{self, Secret};
use super::{replay, Caps, Chat, Event, Failure, Health, Kind, Mime, Part, Provider, Role, State, Turn, TurnId, Usage};
use base64::Engine;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// The pictures of how many questions ride with each request: the new one and the one before it.
pub const KEEP_PICTURES: usize = 2;

/// How long a server on this computer gets to say whether it runs.
const PROBE: Duration = Duration::from_secs(2);

const WARM_WITHIN: Duration = Duration::from_secs(5);

/// A model on this computer may first have to be loaded: its answer gets longer to start than one from a service.
const LOCAL_QUIET: Duration = Duration::from_secs(120);

/// Where the model servers people run on their own computer listen.
pub const LOCAL: &[(&str, &str)] = &[("Ollama", "http://127.0.0.1:11434/v1"), ("LM Studio", "http://127.0.0.1:1234/v1")];

/// One service: what `settings.json` may keep of it. The key is not here.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub id: &'static str,
    pub base_url: String,
    pub model: String,
    pub needs_key: bool,
    pub wire: Wire,
}

/// A service asked with the key the person pasted for `id`.
pub fn remote(id: &'static str, base_url: &str, model: &str) -> Config {
    Config { id, base_url: base_url.into(), model: model.into(), needs_key: true, wire: Wire::OpenAi }
}

/// A model server that takes no key.
pub fn local(id: &'static str, base_url: &str, model: &str) -> Config {
    Config { id, base_url: base_url.into(), model: model.into(), needs_key: false, wire: Wire::OpenAi }
}

/// Whether `url` names this computer itself.
pub(crate) fn loopback(url: &reqwest::Url) -> bool {
    let host = url.host_str().unwrap_or("").trim_start_matches('[').trim_end_matches(']').trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback())
}

impl Config {
    /// The provider for this service. With `offline_only` ("Only on this computer") a service anywhere else is
    /// refused here, so nothing that could reach it is ever made. A key is sent only over https.
    pub fn provider(self, offline_only: bool) -> Result<OpenAi, String> {
        let url = reqwest::Url::parse(&self.base_url).map_err(|_| "That is not a web address.".to_owned())?;
        let offline = loopback(&url);
        if offline_only && !offline {
            return Err("Only on this computer is on, so nothing is sent to another one.".into());
        }
        match url.scheme() {
            "https" => {}
            "http" if offline || !self.needs_key => {}
            _ => return Err("A key is only sent over https.".into()),
        }
        // Named once for as long as Pointcast runs: a provider is made when Settings change, not per question.
        let goes_to: &'static str = if offline { "this computer" } else { Box::leak(url.host_str().unwrap_or("").to_owned().into_boxed_str()) };
        Ok(OpenAi {
            base: self.base_url.trim_end_matches('/').to_owned(),
            quiet: if offline { LOCAL_QUIET } else { http::QUIET },
            cfg: self,
            offline,
            goes_to,
            held: None,
            line: OnceLock::new(),
            known: Mutex::default(),
        })
    }
}

/// A model server found on this computer.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub name: &'static str,
    pub base_url: String,
    pub models: Vec<String>,
}

/// The model servers that run on this computer right now, each with its models: both usual ports asked at once,
/// two seconds in all.
pub fn detect_local() -> Vec<Found> {
    detect_at(LOCAL)
}

pub(crate) fn detect_at(places: &[(&'static str, &str)]) -> Vec<Found> {
    let probes: Vec<_> = places
        .iter()
        .map(|(name, base)| {
            let (name, base_url) = (*name, base.to_string());
            std::thread::spawn(move || {
                let ask = Ask { url: format!("{base_url}/models"), key: None, body: None, within: PROBE };
                let list = models(&Line::new(true)?.whole(&ask).ok()?);
                Some(Found { name, base_url, models: list.into_iter().map(|m| m.0).collect() })
            })
        })
        .collect();
    probes.into_iter().filter_map(|p| p.join().ok().flatten()).collect()
}

/// The models a list names, and for each whether it takes pictures when the list says.
fn models(body: &str) -> Vec<(String, Option<bool>)> {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let list = v["data"].as_array().or(v["models"].as_array()).cloned().unwrap_or_default();
    list.iter().filter_map(|m| Some((m["id"].as_str().or(m["name"].as_str())?.to_owned(), takes_pictures(m)))).collect()
}

fn takes_pictures(model: &Value) -> Option<bool> {
    let arch = &model["architecture"];
    if let Some(inputs) = arch["input_modalities"].as_array() {
        return Some(inputs.iter().any(|i| i.as_str() == Some("image")));
    }
    if let Some(way) = arch["modality"].as_str() {
        return Some(way.split("->").next().unwrap_or("").contains("image"));
    }
    model["capabilities"].as_array().map(|c| c.iter().any(|i| i.as_str() == Some("vision")))
}

pub struct OpenAi {
    cfg: Config,
    base: String,
    offline: bool,
    goes_to: &'static str,
    /// A key handed over directly, used before the store's.
    held: Option<Arc<Secret>>,
    /// Made at the first request, never before.
    line: OnceLock<Option<Line>>,
    /// What is known of each model's pictures; `None` when its server did not say.
    known: Mutex<HashMap<String, Option<bool>>>,
    pub(crate) quiet: Duration,
}

impl OpenAi {
    /// Use `key` and leave the store alone.
    pub fn with_key(mut self, key: Secret) -> OpenAi {
        self.held = Some(Arc::new(key));
        self
    }

    fn key(&self) -> Option<Arc<Secret>> {
        if !self.cfg.needs_key {
            return None;
        }
        self.held.clone().or_else(|| vault::get(self.cfg.id).map(Arc::new))
    }

    fn line(&self) -> Result<&Line, Failure> {
        self.line.get_or_init(|| Line::new(self.offline).map(|l| l.speaking(self.cfg.wire))).as_ref().ok_or(Failure::Offline)
    }

    /// Read the service's list of models; how many it has, or `None` when it did not answer.
    fn list(&self, within: Duration) -> Option<usize> {
        let ask = Ask { url: format!("{}/models", self.base), key: self.key(), body: None, within };
        let found = models(&self.line().ok()?.whole(&ask).ok()?);
        let n = found.len();
        let mut known = self.known.lock().unwrap();
        for (id, pictures) in found {
            // A list that does not say keeps what was learned another way.
            if pictures.is_some() || !known.contains_key(&id) {
                known.insert(id, pictures);
            }
        }
        Some(n)
    }

    /// Ask the service for its list of models now: one request for the list, never a question.
    pub fn listed(&self) -> Result<Vec<super::catalog::Model>, String> {
        self.list(WARM_WITHIN).ok_or_else(|| "The service did not list its models. Check the key, or that it runs.".to_owned())?;
        Ok(Provider::models(self))
    }

    /// Ollama says what a model takes when asked about it; its list does not. One short request on this computer.
    fn show(&self, model: &str) {
        if self.cfg.needs_key || !self.offline || self.known.lock().unwrap().get(model).copied().flatten().is_some() {
            return;
        }
        let root = self.base.strip_suffix("/v1").unwrap_or(&self.base);
        let ask = Ask { url: format!("{root}/api/show"), key: None, body: Some(json!({ "model": model })), within: PROBE };
        let Some(said) = self.line().ok().and_then(|l| l.whole(&ask).ok()) else { return };
        let v: Value = serde_json::from_str(&said).unwrap_or(Value::Null);
        if let Some(can) = v["capabilities"].as_array() {
            self.known.lock().unwrap().insert(model.to_owned(), Some(can.iter().any(|c| c.as_str() == Some("vision"))));
        }
    }
}

impl Provider for OpenAi {
    fn id(&self) -> &'static str {
        self.cfg.id
    }

    /// Pictures are assumed where the service does not say: it refuses one it cannot take.
    fn caps(&self, model: Option<&str>) -> Caps {
        let images = self.known.lock().unwrap().get(model.unwrap_or(&self.cfg.model)).copied().flatten().unwrap_or(true);
        Caps { images, max_images: 4, max_edge: 2048, offline: self.offline }
    }

    /// A service with a key is never asked: the key is saved or it is not. A server on this computer is asked
    /// once, for its list of models.
    fn health(&self) -> Health {
        let (state, line) = if self.cfg.needs_key {
            match self.key() {
                Some(_) => (State::Ready, "Key saved.".to_owned()),
                None => (State::Unknown, "No key yet.".to_owned()),
            }
        } else {
            match self.list(PROBE) {
                Some(1) => (State::Ready, "Running, 1 model.".to_owned()),
                Some(n) => (State::Ready, format!("Running, {n} models.")),
                None => (State::Unknown, "Not running.".to_owned()),
            }
        };
        Health { state, line, goes_to: self.goes_to }
    }

    /// Open the connection to a service, so the question does not wait for it. Nothing is spent.
    fn warm(&self) {
        if !self.offline {
            let _ = self.list(WARM_WITHIN);
        }
    }

    /// What the service's list said; a model whose pictures it did not mention is taken to read them, as `caps` does.
    fn models(&self) -> Vec<super::catalog::Model> {
        let mut v: Vec<_> = self.known.lock().unwrap().iter().map(|(id, p)| super::catalog::Model { id: id.clone(), name: id.clone(), images: p.unwrap_or(true) }).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
        let key = self.key();
        if self.cfg.needs_key && key.is_none() {
            return Err(Failure::SignedOut);
        }
        self.show(&self.cfg.model);
        let pictures = self.caps(None).images;
        Ok(Box::new(OpenAiChat {
            line: self.line()?.clone(),
            url: match self.cfg.wire {
                Wire::OpenAi => format!("{}/chat/completions", self.base),
                Wire::Anthropic => format!("{}/messages", self.base),
            },
            wire: self.cfg.wire,
            key,
            quiet: self.quiet,
            model: self.cfg.model.clone(),
            system: system.to_owned(),
            context: replay(earlier, if pictures { KEEP_PICTURES } else { 0 }),
            pictures,
            history: Arc::default(),
            serial: 0,
            live: Arc::default(),
        }))
    }
}

/// One piece of what was said, as it is sent: a picture is already the address that holds its bytes.
#[derive(Clone)]
pub(super) enum Bit {
    Words(String),
    Picture(String),
}

#[derive(Clone)]
pub(super) struct Said {
    pub(super) by: Role,
    pub(super) bits: Vec<Bit>,
}

pub(super) fn has_picture(parts: &[Part]) -> bool {
    parts.iter().any(|p| matches!(p, Part::Picture { .. }))
}

fn bits(parts: &[Part]) -> Result<Vec<Bit>, Failure> {
    parts
        .iter()
        .map(|p| match p {
            Part::Text(words) => Ok(Bit::Words(words.clone())),
            Part::Picture { path, mime, .. } => {
                let bytes = std::fs::read(path).map_err(|_| Failure::Broken("A picture of this question could not be read.".into()))?;
                let kind = match mime {
                    Mime::Png => "image/png",
                    Mime::Jpeg => "image/jpeg",
                };
                Ok(Bit::Picture(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))))
            }
        })
        .collect()
}

fn message(said: &Said, with_pictures: bool) -> Value {
    let (mut words, mut pictures) = (Vec::new(), Vec::new());
    for b in &said.bits {
        match b {
            Bit::Words(w) => words.push(w.clone()),
            Bit::Picture(url) if with_pictures => pictures.push(json!({ "type": "image_url", "image_url": { "url": url } })),
            Bit::Picture(_) => words.push("[a picture, not sent again]".into()),
        }
    }
    let role = if said.by == Role::Person { "user" } else { "assistant" };
    let text = words.join("\n\n");
    if pictures.is_empty() {
        return json!({ "role": role, "content": text });
    }
    let mut content = Vec::new();
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    content.extend(pictures);
    json!({ "role": role, "content": content })
}

/// The whole request for one question: the instructions, what was said before, then the question.
fn request(model: &str, system: &str, history: &[Said], new: &Said) -> Value {
    let all: Vec<&Said> = history.iter().chain(Some(new)).collect();
    let asked: Vec<usize> = (0..all.len()).filter(|i| all[*i].by == Role::Person).collect();
    let kept = &asked[asked.len().saturating_sub(KEEP_PICTURES)..];
    let mut messages = Vec::new();
    if !system.is_empty() {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.extend(all.iter().enumerate().map(|(i, s)| message(s, kept.contains(&i))));
    json!({ "model": model, "messages": messages, "stream": true, "stream_options": { "include_usage": true } })
}

fn usage_of(u: &Value) -> Option<Usage> {
    Some(Usage {
        input: u["prompt_tokens"].as_u64()?,
        cached: u["prompt_tokens_details"]["cached_tokens"].as_u64().unwrap_or(0),
        output: u["completion_tokens"].as_u64().unwrap_or(0),
    })
}

/// What a refusal means for the person. `pictures` says the question carried one.
pub(super) fn refusal(code: u16, resets: Option<i64>, said: &str, pictures: bool) -> Failure {
    let low = said.to_lowercase();
    let says = |words: &[&str]| words.iter().any(|w| low.contains(w));
    match code {
        401 | 403 => Failure::SignedOut,
        400 if says(&["api key", "api_key"]) => Failure::SignedOut,
        429 => Failure::Limit { resets },
        // Out of credit: asking again does not help until the person adds some.
        402 => Failure::Limit { resets: None },
        _ if says(&["credit balance"]) => Failure::Limit { resets: None },
        413 => Failure::ContextFull,
        _ if says(&["context length", "context_length", "context size", "context window", "context overflow", "maximum number of tokens", "too many tokens", "prompt is too long"]) => {
            Failure::ContextFull
        }
        _ if pictures && code < 500 && says(&["image", "vision", "multimodal", "multi-modal"]) => Failure::NoPictures,
        _ if said.is_empty() => Failure::Broken(format!("The service answered {code}.")),
        _ => Failure::Broken(said.to_owned()),
    }
}

fn failure_of(t: Trouble, pictures: bool) -> Failure {
    match t {
        Trouble::Stopped => Failure::Stopped,
        Trouble::Offline => Failure::Offline,
        Trouble::Status { code, resets, said } => refusal(code, resets, &said, pictures),
        Trouble::Broken(why) => Failure::Broken(why),
    }
}

/// One answer as it streams in.
pub(super) struct Answer {
    id: TurnId,
    out: Sender<Event>,
    history: Arc<Mutex<Vec<Said>>>,
    asked: Option<Said>,
    pub(super) pictures: bool,
    pub(super) words: String,
    pub(super) usage: Option<Usage>,
    pub(super) thought: bool,
    /// The model said it was done; the count of what it cost may still follow.
    pub(super) finished: bool,
    over: bool,
    wire: Wire,
}

impl Answer {
    pub(super) fn tell(&self, kind: Kind) {
        let _ = self.out.send(Event { turn: self.id, kind });
    }

    /// Take one piece of the stream; `false` once the answer has ended.
    fn take(&mut self, piece: Piece) -> bool {
        if self.over {
            return false;
        }
        match piece {
            Piece::Data(d) => self.data(&d),
            // It said all it had and the line then closed, or never did.
            Piece::End(Ok(())) | Piece::End(Err(Trouble::Offline)) if self.finished => self.end(None),
            Piece::End(Ok(())) => self.end(Some(Failure::Broken("The answer ended early.".into()))),
            Piece::End(Err(t)) => self.end(Some(failure_of(t, self.pictures))),
        }
        !self.over
    }

    fn data(&mut self, d: &str) {
        if self.wire == Wire::Anthropic {
            if let Ok(v) = serde_json::from_str::<Value>(d) {
                super::anthropic::take(self, &v);
            }
            return;
        }
        if d.trim() == "[DONE]" {
            return self.end(None);
        }
        let Ok(v) = serde_json::from_str::<Value>(d) else { return };
        let error = &v["error"];
        if !error.is_null() {
            let code = error["code"].as_u64().unwrap_or(0) as u16;
            let said = error["message"].as_str().or(error.as_str()).unwrap_or("");
            return self.end(Some(refusal(code, None, said, self.pictures)));
        }
        if let Some(u) = usage_of(&v["usage"]) {
            self.usage = Some(u);
        }
        let choice = &v["choices"][0];
        let delta = &choice["delta"];
        let thinks = ["reasoning", "reasoning_content"].iter().any(|k| delta[*k].as_str().is_some_and(|t| !t.is_empty()));
        if thinks && !self.thought && self.words.is_empty() {
            self.thought = true;
            self.tell(Kind::Thinking);
        }
        if let Some(words) = delta["content"].as_str().filter(|w| !w.is_empty()) {
            self.words.push_str(words);
            self.tell(Kind::Delta(words.to_owned()));
        }
        match choice["finish_reason"].as_str() {
            Some("content_filter") => self.end(Some(Failure::Refused)),
            Some(_) => self.finished = true,
            None => {}
        }
    }

    /// End the answer, once. What the person saw of it is what the next question follows: a question that got no
    /// words back is not kept.
    pub(super) fn end(&mut self, failed: Option<Failure>) {
        self.over = true;
        if let Some(asked) = self.asked.take().filter(|_| !self.words.is_empty()) {
            let mut history = self.history.lock().unwrap();
            history.push(asked);
            history.push(Said { by: Role::Assistant, bits: vec![Bit::Words(self.words.clone())] });
        }
        match failed {
            Some(f) => self.tell(Kind::Failed(f)),
            None => {
                if let Some(u) = self.usage {
                    self.tell(Kind::Usage(u));
                }
                self.tell(Kind::Done);
            }
        }
    }
}

pub struct OpenAiChat {
    line: Line,
    url: String,
    wire: Wire,
    key: Option<Arc<Secret>>,
    quiet: Duration,
    model: String,
    system: String,
    /// What a resumed conversation said before: it rides with the first question, and stays in the history.
    context: Vec<Part>,
    pictures: bool,
    history: Arc<Mutex<Vec<Said>>>,
    serial: u64,
    /// The answer under way, if one is.
    live: Arc<Mutex<Option<Stopper>>>,
}

impl Chat for OpenAiChat {
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure> {
        if !self.pictures && has_picture(&turn.parts) {
            return Err(Failure::NoPictures);
        }
        let pictures = has_picture(&turn.parts) || has_picture(&self.context);
        let mut parts = self.context.clone();
        parts.extend(turn.parts);
        let new = Said { by: Role::Person, bits: bits(&parts)? };
        // An answer still comes: it ends as stopped here and now, and its words so far stay in the history.
        let old = self.live.lock().unwrap().take();
        if let Some(old) = old {
            old.stop();
        }
        let body = match self.wire {
            Wire::OpenAi => request(&self.model, &self.system, &self.history.lock().unwrap(), &new),
            Wire::Anthropic => super::anthropic::request(&self.model, &self.system, &self.history.lock().unwrap(), &new),
        };
        self.context.clear();
        self.serial += 1;
        let id = TurnId(self.serial);
        let history = self.history.clone();
        let mut answer = Answer { id, out, history, asked: Some(new), pictures, words: String::new(), usage: None, thought: false, finished: false, over: false, wire: self.wire };
        let ask = Ask { url: self.url.clone(), key: self.key.clone(), body: Some(body), within: self.quiet };
        let stopper = self.line.stream(ask, Box::new(move |piece| answer.take(piece)));
        *self.live.lock().unwrap() = Some(stopper);
        Ok(id)
    }

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let live = self.live.clone();
        Arc::new(move || {
            let now = live.lock().unwrap().clone();
            if let Some(answer) = now {
                answer.stop();
            }
        })
    }

    fn close(&mut self) {
        self.cancel();
    }
}

impl Drop for OpenAiChat {
    fn drop(&mut self) {
        self.cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::http::tests::{event, events, nobody, stub, Reply, Stub, KEY};
    use crate::assist::tests::{collect, said, scratch};
    use crate::assist::allowed;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc::channel;
    use std::time::Instant;

    const STOP: &str = r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#;
    const DONE: &str = "[DONE]";

    fn delta(words: &str) -> String {
        json!({ "choices": [{ "delta": { "content": words } }] }).to_string()
    }

    /// A service with a key, served by the stand-in.
    fn keyed(base: &str) -> OpenAi {
        remote("stub", base, "m").provider(false).unwrap().with_key(Secret::new(KEY))
    }

    fn answer(chat: &mut dyn Chat, turn: Turn) -> Vec<Kind> {
        let (tx, rx) = channel();
        chat.ask(turn, tx).unwrap();
        collect(&rx)
    }

    /// A server that says `ok` to every question.
    fn agreeable() -> Stub {
        stub(|_| events(&[&delta("ok"), STOP, DONE]))
    }

    fn picture(dir: &std::path::Path, name: &str, bytes: &[u8], mime: Mime) -> Part {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        Part::Picture { path, w: 4, h: 3, mime }
    }

    #[test]
    fn a_question_goes_out_with_the_instructions_the_replay_and_its_picture_as_a_data_url_and_the_key_only_in_the_header() {
        let d = scratch("openai-body");
        let earlier = [
            Turn { role: Role::Person, parts: vec![Part::Text("What is this?".into()), picture(&d, "1.png", b"first picture", Mime::Png)] },
            Turn { role: Role::Assistant, parts: vec![Part::Text("A cell.".into())] },
        ];
        let s = agreeable();
        let mut chat = keyed(&s.base).open("Be brief.", &earlier).ok().unwrap();
        let turn = Turn { role: Role::Person, parts: vec![Part::Text("And this?".into()), picture(&d, "2.jpg", b"second picture", Mime::Jpeg)] };
        assert_eq!(answer(chat.as_mut(), turn), [Kind::Delta("ok".into()), Kind::Done]);

        let replayed = replay(&earlier, KEEP_PICTURES);
        let Part::Text(transcript) = &replayed[0] else { panic!("the transcript comes first") };
        let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
        let want = json!({
            "model": "m",
            "stream": true,
            "stream_options": { "include_usage": true },
            "messages": [
                { "role": "system", "content": "Be brief." },
                { "role": "user", "content": [
                    { "type": "text", "text": format!("{transcript}\n\nAnd this?") },
                    { "type": "image_url", "image_url": { "url": format!("data:image/png;base64,{}", b64(b"first picture")) } },
                    { "type": "image_url", "image_url": { "url": format!("data:image/jpeg;base64,{}", b64(b"second picture")) } },
                ] },
            ],
        });
        assert_eq!(s.bodies(), [want]);
        let seen = s.seen.lock().unwrap();
        assert_eq!((seen[0].method.as_str(), seen[0].path.as_str()), ("POST", "/v1/chat/completions"));
        assert!(seen[0].head.to_lowercase().contains(&format!("authorization: bearer {KEY}")));
        assert!(!seen[0].body.contains(KEY), "the key is in the header and nowhere in the body");
    }

    #[test]
    fn words_arrive_in_order_then_what_they_cost_and_the_answer_ends_once() {
        let cost = r#"{"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":4}}}"#;
        let s = stub(move |_| events(&[&delta("one "), &delta("two"), STOP, cost, DONE]));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        let usage = Usage { input: 12, cached: 4, output: 3 };
        assert_eq!(answer(chat.as_mut(), said("count")), [Kind::Delta("one ".into()), Kind::Delta("two".into()), Kind::Usage(usage), Kind::Done]);
        // A server that says it is finished and closes the line without the last marker has also ended well.
        let s = stub(|_| events(&[&delta("all"), STOP]));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        assert_eq!(answer(chat.as_mut(), said("count")), [Kind::Delta("all".into()), Kind::Done]);
        // One that only stops talking has not.
        let s = stub(|_| events(&[&delta("half")]));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        assert_eq!(answer(chat.as_mut(), said("count")), [Kind::Delta("half".into()), Kind::Failed(Failure::Broken("The answer ended early.".into()))]);
    }

    #[test]
    fn a_stop_in_the_middle_ends_the_answer_stopped_at_once_and_later_words_are_ignored() {
        let s = stub(|_| Reply::Stream(vec![(1, event(&delta("first"))), (400, event(&delta("late"))), (10, event(STOP)), (10, event(DONE))]));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        let id = chat.ask(said("anything"), tx).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)), Ok(Event { turn: id, kind: Kind::Delta("first".into()) }));
        let t = Instant::now();
        // From another thread, as the panel's Esc does while the turn runs.
        let stop = chat.stopper();
        std::thread::spawn(move || stop()).join().unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)), Ok(Event { turn: id, kind: Kind::Failed(Failure::Stopped) }));
        assert!(t.elapsed() < Duration::from_millis(200), "stopped in {:?}", t.elapsed());
        assert!(rx.recv_timeout(Duration::from_millis(900)).is_err(), "nothing follows the end");
    }

    #[test]
    fn a_second_question_ends_the_first_as_stopped_and_never_carries_its_late_words() {
        let n = AtomicUsize::new(0);
        let s = stub(move |_| match n.fetch_add(1, Ordering::SeqCst) {
            0 => Reply::Stream(vec![(1, event(&delta("old "))), (300, event(&delta("stale"))), (10, event(STOP)), (10, event(DONE))]),
            _ => events(&[&delta("new"), STOP, DONE]),
        });
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        let first = chat.ask(said("one"), tx.clone()).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(10)).map(|e| e.kind), Ok(Kind::Delta("old ".into())));
        let second = chat.ask(said("two"), tx).unwrap();
        let mut got = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(2)) {
            got.push(e);
        }
        let of = |t: TurnId| got.iter().filter(|e| e.turn == t).map(|e| e.kind.clone()).collect::<Vec<_>>();
        assert_eq!(of(first), [Kind::Failed(Failure::Stopped)]);
        assert_eq!(of(second), [Kind::Delta("new".into()), Kind::Done]);
        assert!(!format!("{got:?}").contains("stale"), "the first answer's late words were dropped");
        // The second question follows what the person saw of the first answer.
        let want = json!([{ "role": "user", "content": "one" }, { "role": "assistant", "content": "old " }, { "role": "user", "content": "two" }]);
        assert_eq!(s.bodies()[1]["messages"], want);
    }

    /// How a question ends when the server answers it with `status`, `headers` and `body`.
    fn refused(status: u16, headers: &'static str, body: &'static str, turn: Turn) -> Kind {
        let s = stub(move |_| Reply::Whole(status, headers, body.to_owned()));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        let got = answer(chat.as_mut(), turn);
        assert_eq!(got.len(), 1, "one ending and nothing else: {got:?}");
        got[0].clone()
    }

    #[test]
    fn each_refusal_is_the_failure_the_person_can_act_on() {
        let failed = |status, headers, body| refused(status, headers, body, said("q"));
        assert_eq!(failed(401, "", r#"{"error":{"message":"Incorrect API key provided."}}"#), Kind::Failed(Failure::SignedOut));
        assert_eq!(failed(403, "", "{}"), Kind::Failed(Failure::SignedOut));
        let now = chrono::Utc::now().timestamp();
        let Kind::Failed(Failure::Limit { resets: Some(at) }) = failed(429, "Retry-After: 30\r\n", r#"{"error":{"message":"Slow down."}}"#) else {
            panic!("a limit with its reset")
        };
        assert!((now + 29..=now + 35).contains(&at), "{at} from {now}");
        assert_eq!(failed(429, "", "{}"), Kind::Failed(Failure::Limit { resets: None }));
        let full = r#"{"error":{"message":"This model's maximum context length is 8192 tokens.","code":"context_length_exceeded"}}"#;
        assert_eq!(failed(400, "", full), Kind::Failed(Failure::ContextFull));
        assert_eq!(failed(500, "", r#"{"error":"the model crashed"}"#), Kind::Failed(Failure::Broken("the model crashed".into())));
        assert_eq!(failed(503, "", ""), Kind::Failed(Failure::Broken("The service answered 503.".into())));

        // Only the server can say that this model takes no pictures: the question was sent, and that is its end.
        let d = scratch("openai-refused");
        let blind = r#"{"error":{"message":"This model does not support image input."}}"#;
        let shown = Turn { role: Role::Person, parts: vec![Part::Text("look".into()), picture(&d, "p.png", b"p", Mime::Png)] };
        assert_eq!(refused(400, "", blind, shown), Kind::Failed(Failure::NoPictures));
        assert!(matches!(failed(400, "", blind), Kind::Failed(Failure::Broken(_))), "without a picture it is not about one");
    }

    #[test]
    fn what_a_broken_server_says_is_shown_without_the_key() {
        let s = stub(|a| Reply::Whole(500, "", json!({ "error": { "message": format!("Failed on: {}", a.head) } }).to_string()));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        let got = answer(chat.as_mut(), said("q"));
        let [Kind::Failed(Failure::Broken(why))] = &got[..] else { panic!("one failure: {got:?}") };
        assert!(why.starts_with("Failed on: POST") && why.contains("[key]"), "{why}");
        assert!(!format!("{got:?}").contains(KEY));
    }

    #[test]
    fn a_refused_connection_is_offline() {
        let mut chat = keyed(&nobody()).open("", &[]).ok().unwrap();
        assert_eq!(answer(chat.as_mut(), said("q")), [Kind::Failed(Failure::Offline)]);
    }

    #[test]
    fn a_service_without_its_key_does_not_open() {
        let p = remote("pointcast-test-no-such-key", "https://api.openai.com/v1", "m").provider(false).unwrap();
        assert_eq!(p.open("", &[]).err(), Some(Failure::SignedOut));
        assert_eq!(p.health().line, "No key yet.");
        assert!(p.line.get().is_none(), "nothing that could reach the service was made");
    }

    #[test]
    fn a_model_known_to_take_no_pictures_refuses_one_before_anything_is_sent() {
        let d = scratch("openai-blind");
        // Ollama's own answer about a text model; its list says nothing about pictures.
        let s = stub(|a| match a.path.as_str() {
            "/api/show" => Reply::Whole(200, "", r#"{"capabilities":["completion"]}"#.into()),
            _ => events(&[&delta("ok"), STOP, DONE]),
        });
        let p = local("ollama", &s.base, "llama3").provider(true).unwrap();
        assert!(p.caps(None).images, "pictures are assumed until the server says");
        let mut chat = p.open("", &[]).ok().unwrap();
        assert!(!p.caps(None).images);
        let (tx, _rx) = channel();
        let shown = Turn { role: Role::Person, parts: vec![picture(&d, "p.png", b"p", Mime::Png)] };
        assert_eq!(chat.ask(shown, tx), Err(Failure::NoPictures));
        assert_eq!(s.paths(), ["/api/show"], "the question never left");
        assert_eq!(s.bodies()[0], json!({ "model": "llama3" }));
        // Words alone still go, without a key.
        assert_eq!(answer(chat.as_mut(), said("words")), [Kind::Delta("ok".into()), Kind::Done]);
        assert!(!s.seen.lock().unwrap()[1].head.to_lowercase().contains("authorization"));

        // A list that names what each model takes is believed too.
        let list = r#"{"data":[{"id":"sees","architecture":{"input_modalities":["text","image"]}},{"id":"reads","architecture":{"modality":"text->text"}},{"id":"plain"}]}"#;
        assert_eq!(models(list), [("sees".to_owned(), Some(true)), ("reads".to_owned(), Some(false)), ("plain".to_owned(), None)]);
        let s = stub(move |_| Reply::Whole(200, "", list.into()));
        let p = keyed(&s.base);
        assert_eq!(p.list(PROBE), Some(3));
        assert_eq!((p.caps(Some("sees")).images, p.caps(Some("reads")).images, p.caps(Some("plain")).images), (true, false, true));
    }

    #[test]
    fn reasoning_shows_as_thinking_once_before_the_words() {
        let think = |key: &str| json!({ "choices": [{ "delta": { key: "hm" } }] }).to_string();
        let (a, b) = (think("reasoning_content"), think("reasoning"));
        let s = stub(move |_| events(&[&a, &b, &delta("so"), STOP, DONE]));
        let mut chat = keyed(&s.base).open("", &[]).ok().unwrap();
        assert_eq!(answer(chat.as_mut(), said("why")), [Kind::Thinking, Kind::Delta("so".into()), Kind::Done]);
    }

    #[test]
    fn the_adapter_keeps_the_history_and_once_shares_none_of_it() {
        let s = agreeable();
        let p = keyed(&s.base);
        let mut study = p.open("Be brief.", &[]).ok().unwrap();
        answer(study.as_mut(), said("study question"));
        let mut summary = p.once("Summarize.", &[]).ok().unwrap();
        answer(summary.as_mut(), said("summary question"));
        answer(study.as_mut(), said("again"));
        let sent = s.bodies();
        let user = |words: &str| json!({ "role": "user", "content": words });
        assert_eq!(sent[1]["messages"], json!([{ "role": "system", "content": "Summarize." }, user("summary question")]));
        let want = json!([{ "role": "system", "content": "Be brief." }, user("study question"), { "role": "assistant", "content": "ok" }, user("again")]);
        assert_eq!(sent[2]["messages"], want);
    }

    #[test]
    fn only_the_pictures_of_the_last_two_questions_ride_with_a_request() {
        let asked = |n: usize| Said { by: Role::Person, bits: vec![Bit::Words(format!("q{n}")), Bit::Picture(format!("data:{n}"))] };
        let told = Said { by: Role::Assistant, bits: vec![Bit::Words("a".into())] };
        let body = request("m", "", &[asked(1), told.clone(), asked(2), told], &asked(3));
        let m = &body["messages"];
        assert_eq!(m[0], json!({ "role": "user", "content": "q1\n\n[a picture, not sent again]" }));
        assert_eq!(m[2]["content"][1]["image_url"]["url"], "data:2");
        assert_eq!(m[4]["content"], json!([{ "type": "text", "text": "q3" }, { "type": "image_url", "image_url": { "url": "data:3" } }]));
        assert_eq!(body.to_string().matches("image_url").count(), 4, "two pictures, each named twice");
    }

    #[test]
    fn this_computer_is_offline_and_a_service_is_not_and_only_on_this_computer_no_service_can_be_made() {
        for base in ["http://127.0.0.1:11434/v1", "http://localhost:1234/v1", "http://[::1]:8080/v1"] {
            let p = local("here", base, "m").provider(true).unwrap();
            assert!(p.caps(None).offline && allowed(p.caps(None), true), "{base}");
            assert_eq!(p.goes_to, "this computer");
        }
        let there = remote("there", "https://api.openai.com/v1", "m");
        let p = there.clone().provider(false).unwrap().with_key(Secret::new(KEY));
        assert!(!p.caps(None).offline);
        assert!(allowed(p.caps(None), false) && !allowed(p.caps(None), true));
        let health = p.health();
        assert_eq!((health.state, health.line.as_str(), health.goes_to), (State::Ready, "Key saved.", "api.openai.com"));
        assert!(p.line.get().is_none(), "saying so reached for nothing");
        // With the switch on, a provider for a service does not exist, so nothing can be sent through one.
        assert!(there.provider(true).is_err());
        assert!(local("lan", "http://192.168.1.20:11434/v1", "m").provider(true).is_err(), "another computer is not this one");
        assert!(local("lan", "http://192.168.1.20:11434/v1", "m").provider(false).is_ok());
        // A key never travels in the clear.
        assert!(remote("clear", "http://example.com/v1", "m").provider(false).is_err());
        assert!(remote("odd", "not an address", "m").provider(false).is_err());
    }

    #[test]
    fn health_asks_a_local_server_for_its_models_once_and_a_service_nothing() {
        let s = stub(|_| Reply::Whole(200, "", r#"{"object":"list","data":[{"id":"llava"},{"id":"llama3"}]}"#.into()));
        let health = local("ollama", &s.base, "llava").provider(true).unwrap().health();
        assert_eq!((health.state, health.line.as_str(), health.goes_to), (State::Ready, "Running, 2 models.", "this computer"));
        assert_eq!(s.paths(), ["/v1/models"], "one question about models and none to a model");
        let health = local("ollama", &nobody(), "llava").provider(true).unwrap().health();
        assert_eq!((health.state, health.line.as_str()), (State::Unknown, "Not running."));
        // A service with a key: nothing is asked at all.
        let s = agreeable();
        assert_eq!(keyed(&s.base).health().line, "Key saved.");
        assert!(s.paths().is_empty());
    }

    #[test]
    fn the_model_servers_on_this_computer_are_found_with_their_models() {
        let s = stub(|_| Reply::Whole(200, "", r#"{"data":[{"id":"llava"},{"id":"llama3"}]}"#.into()));
        let gone = nobody();
        let found = detect_at(&[("Ollama", s.base.as_str()), ("LM Studio", gone.as_str())]);
        assert_eq!(found, [Found { name: "Ollama", base_url: s.base.clone(), models: vec!["llava".into(), "llama3".into()] }]);
        assert_eq!(s.paths(), ["/v1/models"]);
        assert_eq!(LOCAL.iter().map(|l| l.1).collect::<Vec<_>>(), ["http://127.0.0.1:11434/v1", "http://127.0.0.1:1234/v1"]);
    }
}
