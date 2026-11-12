//! The assistant: a question in words and pictures goes to an AI the person already has, and the answer streams back
//! as text. One contract serves every route (`Provider` opens a conversation, `Chat` asks and cancels); the routes
//! are the vendors' own command-line tools run as a child (`child`, `jsonl`, then `codex`, and the agents that speak
//! the Agent Client Protocol, `acp`), and any service that speaks OpenAI's chat API or Anthropic's Messages API over
//! HTTP (`http`, then `openai` and `anthropic`): one the person pasted a key for (`vault`), or a model on this computer. Pointcast never reads, stores or forwards another program's sign-in: a test below fails the
//! build if this folder names a vendor's sign-in file.

// Nothing calls this module yet: slice 3 wires it to commands and Settings, and removes this line.
#![allow(dead_code)]

pub mod acp;
pub mod anthropic;
pub mod catalog;
pub mod child;
pub mod claude;
pub mod codex;
pub mod detect;
#[cfg(test)]
pub mod fake;
pub mod http;
pub mod jsonl;
pub mod openai;
pub mod vault;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// One piece of a turn. A picture is always a file Pointcast wrote, already flattened and fitted.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Picture { path: PathBuf, w: u32, h: u32, mime: Mime },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mime {
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Person,
    Assistant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Turn {
    pub role: Role,
    pub parts: Vec<Part>,
}

/// What one answer cost, as the provider counted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Usage {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

/// One window of the person's plan: how much is used and when it starts over (Unix seconds).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub used_percent: u32,
    pub minutes: Option<i64>,
    pub resets: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Limits {
    pub primary: Option<Window>,
    pub secondary: Option<Window>,
}

impl Limits {
    /// When the person can ask again: the latest reset among the windows that are used up, since each of them
    /// blocks; with none used up, the next reset of any window.
    pub fn resets(&self) -> Option<i64> {
        let all = [self.primary, self.secondary];
        let full = all.iter().flatten().filter(|w| w.used_percent >= 100).filter_map(|w| w.resets).max();
        full.or_else(|| all.iter().flatten().filter_map(|w| w.resets).min())
    }
}

/// Which question of a conversation an event answers: a turn that was stopped or overtaken says nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TurnId(pub u64);

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub turn: TurnId,
    pub kind: Kind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    Delta(String),
    /// The model is working before its first words, so a wait reads as thinking and not as stuck.
    Thinking,
    Usage(Usage),
    Limits(Limits),
    Done,
    Failed(Failure),
}

impl Kind {
    /// Whether this is the one event that ends a turn.
    pub fn ends(&self) -> bool {
        matches!(self, Kind::Done | Kind::Failed(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Failure {
    NotInstalled,
    SignedOut,
    Limit { resets: Option<i64> },
    Offline,
    Refused,
    NoPictures,
    /// The conversation no longer fits the model: start a new one.
    ContextFull,
    /// The person stopped it, or the conversation was closed under it.
    Stopped,
    Broken(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub images: bool,
    pub max_images: u8,
    pub max_edge: u32,
    pub offline: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Ready,
    NotInstalled,
    TooOld,
    Unknown,
}

/// Whether a provider can be used, in one line for Settings, and where a question sent through it goes.
#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    pub state: State,
    pub line: String,
    pub goes_to: &'static str,
}

pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    /// What `model` takes (the provider's default when none is named).
    fn caps(&self, model: Option<&str>) -> Caps;
    /// Spends nothing: no question is sent and no account is touched.
    fn health(&self) -> Health;
    fn warm(&self);
    /// Start a conversation. `earlier` is what a session resumed from disk already said, whichever side keeps the
    /// thread.
    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure>;
    /// A separate conversation that shares nothing with any open `Chat`: a summary asked through it never lands
    /// in the study conversation.
    fn once(&self, system: &str, turns: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
        self.open(system, turns)
    }
    /// Let go of a helper `warm` started that no conversation took.
    fn cool(&self) {}
    /// The models known without asking anyone; none when nothing is known yet.
    fn models(&self) -> Vec<catalog::Model> {
        Vec::new()
    }
    /// The files the AI's own app adds to every question, once known (Codex: the person's own instructions file).
    fn adds(&self) -> Vec<String> {
        Vec::new()
    }
    /// Who is signed in, as the AI's own app said at its last greeting (Codex's `account/read`); none before one.
    fn account(&self) -> Option<codex::Account> {
        None
    }
}

pub trait Chat: Send {
    /// Ask the new turn. `Err` means the turn never started and nothing is sent on `out`; after `Ok`, `out` gets
    /// the answer under the returned id and that turn ends with exactly one `Done` or `Failed`. Asking while an
    /// answer still comes stops that answer first.
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure>;
    /// Stops the answer under way, from any thread, while another holds the chat.
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync>;
    fn cancel(&self) {
        (self.stopper())()
    }
    fn close(&mut self);
    /// What goes ahead of the next question: the earlier turns this conversation was opened on, as its own `replay`
    /// made them (pictures left out when the model takes none). Empty once that question went, or with none.
    fn context(&self) -> Vec<Part> {
        Vec::new()
    }
    /// The files the AI's own app added to this conversation.
    fn adds(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Earlier turns as the context a new conversation starts from: one transcript in words, then the pictures of
/// only the last `keep_pictures_of_last` turns the person asked. What it returns is exactly what is sent.
pub fn replay(earlier: &[Turn], keep_pictures_of_last: usize) -> Vec<Part> {
    if earlier.is_empty() {
        return Vec::new();
    }
    let asked: Vec<usize> = (0..earlier.len()).filter(|i| earlier[*i].role == Role::Person).collect();
    let kept = &asked[asked.len().saturating_sub(keep_pictures_of_last)..];
    let mut text = String::from("Earlier in this conversation, for context:");
    let mut pictures = Vec::new();
    for (i, t) in earlier.iter().enumerate() {
        let who = if t.role == Role::Person { "You" } else { "Assistant" };
        let mut said = Vec::new();
        for p in &t.parts {
            match p {
                Part::Text(words) => said.push(words.clone()),
                Part::Picture { .. } if kept.contains(&i) => {
                    pictures.push(p.clone());
                    said.push(format!("[picture {}, attached]", pictures.len()));
                }
                Part::Picture { .. } => said.push("[a picture, not sent again]".into()),
            }
        }
        text.push_str(&format!("\n\n{who}: {}", said.join("\n")));
    }
    let mut parts = vec![Part::Text(text)];
    parts.extend(pictures);
    parts
}

/// How long a conversation nobody asks keeps its helper.
pub const IDLE: Duration = Duration::from_secs(5 * 60);

/// Whether a conversation last used at `last_used` has sat idle long enough at `now` to be closed.
pub fn idle_due(now: Instant, last_used: Instant) -> bool {
    now.saturating_duration_since(last_used) >= IDLE
}

/// Close `chat` if it has sat idle long enough. The scheduler that calls this arrives with slice 3.
pub fn close_if_idle(chat: &mut dyn Chat, now: Instant, last_used: Instant) -> bool {
    let due = idle_due(now, last_used);
    if due {
        chat.close();
    }
    due
}

/// Whether a provider that takes `caps` may be offered. With "Only on this computer" on, only one that sends
/// nothing off it.
pub fn allowed(caps: Caps, offline_only: bool) -> bool {
    !offline_only || caps.offline
}

/// The providers Pointcast can offer, by id.
#[derive(Default)]
pub struct Registry {
    list: Vec<Arc<dyn Provider>>,
}

impl Registry {
    /// Add a provider; one with the same id is replaced.
    pub fn add(&mut self, p: Arc<dyn Provider>) {
        self.list.retain(|q| q.id() != p.id());
        self.list.push(p);
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.list.iter().find(|p| p.id() == id).cloned()
    }

    pub fn ids(&self) -> Vec<&'static str> {
        self.list.iter().map(|p| p.id()).collect()
    }
}

/// A provider that says fixed words at fixed delays: the stand-in the later slices test against.
#[derive(Clone)]
pub struct Scripted {
    pub id: &'static str,
    /// Each piece of the answer and how long to wait before it.
    pub steps: Vec<(Duration, String)>,
    /// How the answer ends when it does not end well.
    pub fail: Option<Failure>,
    pub pictures: bool,
    /// What `open` was given, and every turn asked, in order.
    pub opened: Arc<Mutex<Vec<(String, Vec<Turn>)>>>,
    pub asked: Arc<Mutex<Vec<Turn>>>,
    /// Every question as it went on the wire: the replay of the earlier turns, as an adapter makes it from what `open`
    /// was given, ahead of the first one, then the question's own parts.
    pub wire: Arc<Mutex<Vec<Vec<Part>>>>,
}

impl Scripted {
    pub fn new(steps: &[(u64, &str)]) -> Scripted {
        Scripted {
            id: "scripted",
            steps: steps.iter().map(|(ms, s)| (Duration::from_millis(*ms), s.to_string())).collect(),
            fail: None,
            pictures: true,
            opened: Arc::default(),
            asked: Arc::default(),
            wire: Arc::default(),
        }
    }

    pub fn failing(mut self, f: Failure) -> Scripted {
        self.fail = Some(f);
        self
    }
}

impl Provider for Scripted {
    fn id(&self) -> &'static str {
        self.id
    }

    fn caps(&self, _model: Option<&str>) -> Caps {
        Caps { images: self.pictures, max_images: 4, max_edge: 2048, offline: true }
    }

    fn health(&self) -> Health {
        Health { state: State::Ready, line: "A scripted answer.".into(), goes_to: "nowhere" }
    }

    fn warm(&self) {}

    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
        self.opened.lock().unwrap().push((system.to_owned(), earlier.to_vec()));
        // As every adapter does: the earlier turns are replayed once, with no pictures for a model that takes none.
        let context = replay(earlier, if self.pictures { 2 } else { 0 });
        Ok(Box::new(ScriptedChat { script: self.clone(), context, current: Arc::default(), halted: Arc::default() }))
    }
}

struct ScriptedChat {
    script: Scripted,
    context: Vec<Part>,
    /// The turn asked last, and the newest turn that was stopped.
    current: Arc<AtomicU64>,
    halted: Arc<AtomicU64>,
}

impl Chat for ScriptedChat {
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure> {
        if !self.script.pictures && turn.parts.iter().any(|p| matches!(p, Part::Picture { .. })) {
            return Err(Failure::NoPictures);
        }
        let mut wire = std::mem::take(&mut self.context);
        wire.extend(turn.parts.iter().cloned());
        self.script.wire.lock().unwrap().push(wire);
        self.script.asked.lock().unwrap().push(turn);
        // A newer turn stops the one before it.
        let n = self.current.fetch_add(1, Ordering::SeqCst) + 1;
        let id = TurnId(n);
        let (current, halted) = (self.current.clone(), self.halted.clone());
        let (steps, fail) = (self.script.steps.clone(), self.script.fail.clone());
        std::thread::spawn(move || {
            let stopped = || halted.load(Ordering::SeqCst) >= n || current.load(Ordering::SeqCst) > n;
            let say = |kind: Kind| {
                let _ = out.send(Event { turn: id, kind });
            };
            for (wait, words) in steps {
                let t = Instant::now();
                while t.elapsed() < wait && !stopped() {
                    std::thread::sleep(Duration::from_millis(2).min(wait));
                }
                if stopped() {
                    return say(Kind::Failed(Failure::Stopped));
                }
                say(Kind::Delta(words));
            }
            say(fail.map_or(Kind::Done, Kind::Failed));
        });
        Ok(id)
    }

    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
        let (current, halted) = (self.current.clone(), self.halted.clone());
        Arc::new(move || halted.store(current.load(Ordering::SeqCst), Ordering::SeqCst))
    }

    fn close(&mut self) {
        self.cancel();
    }

    fn context(&self) -> Vec<Part> {
        self.context.clone()
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::mpsc::{channel, Receiver};

    /// A folder of this test's own, empty.
    pub fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-assist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Everything a turn sent, up to its end and a moment past it, so a second ending would show.
    pub fn collect(rx: &Receiver<Event>) -> Vec<Kind> {
        let mut all = Vec::new();
        loop {
            match rx.recv_timeout(Duration::from_secs(20)) {
                Ok(e) => {
                    let end = e.kind.ends();
                    all.push(e.kind);
                    if end {
                        break;
                    }
                }
                Err(_) => panic!("the turn never ended: {all:?}"),
            }
        }
        while let Ok(e) = rx.recv_timeout(Duration::from_millis(300)) {
            all.push(e.kind);
        }
        all
    }

    pub fn words(events: &[Kind]) -> Vec<String> {
        events.iter().filter_map(|e| if let Kind::Delta(d) = e { Some(d.clone()) } else { None }).collect()
    }

    /// How many endings a turn had: one is the contract.
    pub fn endings(events: &[Kind]) -> usize {
        events.iter().filter(|e| e.ends()).count()
    }

    pub fn said(text: &str) -> Turn {
        Turn { role: Role::Person, parts: vec![Part::Text(text.into())] }
    }

    #[test]
    fn a_scripted_answer_arrives_in_order_and_ends_once() {
        let p = Scripted::new(&[(1, "one "), (1, "two "), (1, "three")]);
        let mut chat = p.open("Be brief.", &[said("before")]).ok().unwrap();
        let (tx, rx) = channel();
        assert_eq!(chat.ask(said("count"), tx), Ok(TurnId(1)));
        let got = collect(&rx);
        assert_eq!(words(&got), ["one ", "two ", "three"]);
        assert_eq!(got.last(), Some(&Kind::Done));
        assert_eq!(endings(&got), 1);
        assert_eq!(*p.asked.lock().unwrap(), [said("count")]);
        assert_eq!(*p.opened.lock().unwrap(), [("Be brief.".to_owned(), vec![said("before")])]);
    }

    #[test]
    fn a_scripted_failure_ends_the_answer_once_after_its_words() {
        let p = Scripted::new(&[(1, "half")]).failing(Failure::Offline);
        let mut chat = p.open("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        chat.ask(said("anything"), tx).unwrap();
        assert_eq!(collect(&rx), [Kind::Delta("half".into()), Kind::Failed(Failure::Offline)]);
    }

    #[test]
    fn a_scripted_answer_that_is_cancelled_ends_stopped() {
        let p = Scripted::new(&[(1, "first"), (5_000, "never")]);
        let mut chat = p.open("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        chat.ask(said("anything"), tx).unwrap();
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).map(|e| e.kind), Ok(Kind::Delta("first".into())));
        // From another thread, as the panel's Esc does while the turn runs.
        let stop = chat.stopper();
        std::thread::spawn(move || stop()).join().unwrap();
        assert_eq!(collect(&rx), [Kind::Failed(Failure::Stopped)]);
    }

    #[test]
    fn a_newer_scripted_turn_stops_the_one_before_and_each_event_names_its_turn() {
        let p = Scripted::new(&[(40, "slow")]);
        let mut chat = p.open("", &[]).ok().unwrap();
        let (tx, rx) = channel();
        let first = chat.ask(said("one"), tx.clone()).unwrap();
        let second = chat.ask(said("two"), tx).unwrap();
        let mut got = Vec::new();
        while let Ok(e) = rx.recv_timeout(Duration::from_secs(5)) {
            got.push(e);
        }
        let of = |t: TurnId| got.iter().filter(|e| e.turn == t).map(|e| e.kind.clone()).collect::<Vec<_>>();
        assert_eq!(of(first), [Kind::Failed(Failure::Stopped)]);
        assert_eq!(of(second), [Kind::Delta("slow".into()), Kind::Done]);
    }

    pub fn shot(name: &str) -> Part {
        Part::Picture { path: name.into(), w: 4, h: 3, mime: Mime::Png }
    }

    #[test]
    fn a_replay_is_one_transcript_and_the_pictures_of_only_the_last_turns_asked() {
        assert!(replay(&[], 2).is_empty());
        let earlier = [
            Turn { role: Role::Person, parts: vec![Part::Text("What is this?".into()), shot("1.png")] },
            Turn { role: Role::Assistant, parts: vec![Part::Text("A cell.".into())] },
            Turn { role: Role::Person, parts: vec![Part::Text("And this?".into()), shot("2.png")] },
            Turn { role: Role::Assistant, parts: vec![Part::Text("A nucleus.".into())] },
        ];
        let got = replay(&earlier, 1);
        let Part::Text(text) = &got[0] else { panic!("the transcript comes first") };
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines,
            [
                "Earlier in this conversation, for context:",
                "",
                "You: What is this?",
                "[a picture, not sent again]",
                "",
                "Assistant: A cell.",
                "",
                "You: And this?",
                "[picture 1, attached]",
                "",
                "Assistant: A nucleus.",
            ]
        );
        assert_eq!(got[1..], [shot("2.png")]);
        assert_eq!(replay(&earlier, 2)[1..], [shot("1.png"), shot("2.png")]);
        assert_eq!(replay(&earlier, 0).len(), 1);
    }

    #[test]
    fn a_picture_kind_is_written_as_its_media_type() {
        assert_eq!(serde_json::to_value([Mime::Png, Mime::Jpeg]).unwrap(), serde_json::json!(["image/png", "image/jpeg"]));
    }

    #[test]
    fn a_scripted_provider_without_pictures_refuses_one_and_records_nothing() {
        let mut p = Scripted::new(&[(1, "x")]);
        p.pictures = false;
        let mut chat = p.open("", &[]).ok().unwrap();
        let (tx, _rx) = channel();
        let turn = Turn { role: Role::Person, parts: vec![shot("a.png")] };
        assert_eq!(chat.ask(turn, tx), Err(Failure::NoPictures));
        assert!(p.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn the_registry_finds_a_provider_by_id_and_keeps_one_of_each() {
        let mut r = Registry::default();
        r.add(Arc::new(Scripted::new(&[])));
        let mut other = Scripted::new(&[]);
        other.id = "other";
        r.add(Arc::new(other));
        r.add(Arc::new(Scripted::new(&[(1, "newer")])));
        assert_eq!(r.ids(), ["other", "scripted"]);
        assert!(r.get("scripted").is_some());
        assert!(r.get("missing").is_none());
    }

    #[test]
    fn a_conversation_is_due_to_close_only_once_the_idle_time_has_passed() {
        let last = Instant::now();
        assert!(!idle_due(last, last));
        assert!(!idle_due(last + IDLE - Duration::from_secs(1), last));
        assert!(idle_due(last + IDLE, last));
        // A clock that reads earlier than the last use is not idle time.
        assert!(!idle_due(last, last + IDLE));
    }

    #[test]
    fn the_reset_is_the_latest_of_the_used_up_windows_else_the_next_one() {
        let w = |used, resets| Some(Window { used_percent: used, minutes: None, resets });
        assert_eq!(Limits { primary: w(100, Some(50)), secondary: w(40, Some(90)) }.resets(), Some(50));
        assert_eq!(Limits { primary: w(100, Some(50)), secondary: w(100, Some(90)) }.resets(), Some(90));
        assert_eq!(Limits { primary: w(10, Some(50)), secondary: w(40, Some(90)) }.resets(), Some(50));
        assert_eq!(Limits { primary: w(100, None), secondary: None }.resets(), None);
        assert_eq!(Limits::default().resets(), None);
    }

    /// The needles of `needles` found in `text`, whatever their case.
    fn hits(text: &str, needles: &[String]) -> Vec<String> {
        let text = text.to_lowercase();
        needles.iter().filter(|n| text.contains(&n.to_lowercase())).cloned().collect()
    }

    fn sources(dir: &Path, into: &mut Vec<PathBuf>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                sources(&p, into);
            } else if p.extension().is_some_and(|x| x == "rs") {
                into.push(p);
            }
        }
    }

    #[test]
    fn no_source_under_assist_names_a_vendor_sign_in_file() {
        // Built from pieces, so this file does not hold what it looks for.
        // The Gemini CLI's and GitHub Copilot's sign-ins too: an agent spoken to over ACP keeps its own.
        let needles: Vec<String> = [
            [".creden", "tials.json"],
            ["auth", ".json"],
            ["key", "chain"],
            ["Claude Code-cre", "dentials"],
            ["oauth_", "creds"],
            ["github-", "copilot"],
            ["hosts", ".json"],
            // The private endpoints and borrowed app ids a route that impersonates a vendor's own app reaches for.
            ["oauth", "/usage"],
            ["backend", "-api"],
            ["copilot", "_internal"],
            ["Iv", "1."],
            ["app_EMoamEEZ73f0", "CkXaXp7hrann"],
        ]
            .iter()
            .map(|p| p.concat())
            .collect();
        // The scan can find each one, and does not find what is not there.
        for n in &needles {
            let planted = format!("let p = home.join(\"{}\");", n.to_uppercase());
            assert_eq!(hits(&planted, &needles), [n.clone()], "a planted {n} is found");
        }
        assert!(hits("let p = home.join(\"settings.json\");", &needles).is_empty());

        let mut files = Vec::new();
        sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src").join("assist"), &mut files);
        assert!(files.len() >= 12, "the scan read the folder it meant to: {files:?}");
        assert!(["acp.rs", "anthropic.rs", "claude.rs"].iter().all(|n| files.iter().any(|f| f.ends_with(n))), "and the newest files in it");
        for f in files {
            let found = hits(&std::fs::read_to_string(&f).unwrap(), &needles);
            assert!(found.is_empty(), "{} names {found:?}", f.display());
        }
    }
}
