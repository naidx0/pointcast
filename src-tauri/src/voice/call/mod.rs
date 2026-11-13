//! Call mode: it feels like a phone call. You talk; when your turn ends (voice-activity detection, `vad` and `turn`)
//! it is asked through the normal Ask path (the same AI and session, so the conversation carries on); the answer is read
//! aloud; then it listens again, with no key pressed, until the call ends (`machine`). Talking over the answer stops it
//! at once and becomes the next turn, from just before the first word (the call's microphone keeps a few seconds,
//! `audio::Feed`); its own voice coming back through the speakers does not (`turn::Echo`).
//!
//! One microphone is open for the whole call. The questions record from it (`session::App::lend_feed`), so a turn never
//! waits for a device to open and nothing between turns is lost.

pub mod machine;
pub mod turn;
pub mod vad;

use crate::voice::audio::{self, Feed, Mic, Take};
pub use machine::{Call, Do, Event, Side, Stage, Why};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use turn::{Echo, Heard, Turn};
use vad::{Vad, WINDOW};

/// The call's microphone keeps this much: a turn noticed late, or a barge-in that waits for an answer to stop, still
/// starts from just before its first word.
pub const KEEP_MS: u32 = 4_000;
/// The call looks at its microphone and the Ask side this often.
const POLL: Duration = Duration::from_millis(10);
/// Listening after a barge-in waits at most this long for the answer it stopped to end.
const STOP_WAIT: Duration = Duration::from_millis(2_000);
/// The capsule's line hears the call this often.
const LEVEL_EVERY: Duration = Duration::from_millis(33);

/// The words of a turn that end the call: "end call" or "hang up" said alone (a please or thanks around it is still
/// alone).
pub fn is_hangup(text: &str) -> bool {
    let lower = text.to_lowercase();
    let mut w: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric() && c != '\'').filter(|w| !w.is_empty()).collect();
    while w.first().is_some_and(|x| matches!(*x, "okay" | "ok" | "please")) {
        w.remove(0);
    }
    if w.ends_with(&["thank", "you"]) {
        w.truncate(w.len() - 2);
    }
    while w.last().is_some_and(|x| matches!(*x, "please" | "now" | "thanks")) {
        w.pop();
    }
    matches!(w.as_slice(), ["end", "call"] | ["end", "the", "call"] | ["hang", "up"] | ["hangup"])
}

/// What the panel and the capsule show of the call (`call://state`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct View {
    pub active: bool,
    /// Who has the call; none when no call runs.
    pub side: Option<Side>,
    /// Why the last call ended, once it has.
    pub ended: Option<Why>,
}

/// The Ask side as a call drives it: the panel (`live::Live`) and the voice reading answers (`speak::Reader`).
pub trait Asking: Send + Sync {
    /// Start recording a question, as the Ask key's press does; false when it could not.
    fn listen(&self) -> bool;
    /// The turn ended: finish its words and ask, as the key coming up does.
    fn ask(&self);
    /// Stop the answer on its way.
    fn stop_answer(&self);
    /// Drop the question being recorded, unsaid.
    fn drop_turn(&self);
    fn stop_speech(&self);
    fn stage(&self) -> Stage;
    /// An answer is being read aloud.
    fn speaking(&self) -> bool;
    /// The rms of what the speakers play now; 0 when not known (the system voice plays on its own).
    fn played(&self) -> f32;
    /// Hear each question's words before they are kept; true keeps and sends nothing (`live::Live::hear_words`).
    fn hear_words(&self, words: Option<crate::live::Words>);
}

/// The call's ear: the microphone in windows, the VAD's answer for each, the turn detector, and, while Pointcast speaks,
/// only what is louder than its own voice through the speakers.
pub struct Ear {
    vad: Box<dyn Vad>,
    turn: Turn,
    echo: Echo,
    rest: Vec<f32>,
    side: Side,
}

impl Ear {
    pub fn new(vad: Box<dyn Vad>) -> Ear {
        Ear { vad, turn: Turn::default(), echo: Echo::default(), rest: Vec::new(), side: Side::Listening }
    }

    pub fn vad(&self) -> &'static str {
        self.vad.name()
    }

    /// Hear `x` while `side` has the call: what changed, window by window.
    pub fn hear(&mut self, x: &[f32], side: Side) -> Vec<Heard> {
        let speaking = side == Side::Speaking;
        if speaking && self.side != Side::Speaking {
            self.echo.begin();
        }
        self.side = side;
        self.turn.need(if speaking { turn::BARGE_MS } else { turn::MIN_SPEECH_MS });
        self.rest.extend_from_slice(x);
        let whole = self.rest.len() / WINDOW * WINDOW;
        let mut out = Vec::new();
        for w in self.rest[..whole].chunks(WINDOW) {
            let mut speech = self.vad.speech(w);
            if speaking {
                // Its own voice teaches the bar even where the VAD hears speech in it (it is speech).
                speech &= self.echo.hear(audio::rms(w), (WINDOW * 1000 / audio::RATE as usize) as u32);
            }
            match self.turn.hear(speech, w.len()) {
                Heard::Nothing => {}
                h => out.push(h),
            }
        }
        self.rest.drain(..whole);
        out
    }
}

/// One call, looked at every `POLL`: what came from outside (the switch, Esc), what the microphone heard, how the Ask
/// side stands and the clock go to the machine, and what it says to do is done at once.
pub struct Loop {
    ask: Arc<dyn Asking>,
    feed: Arc<Feed>,
    ear: Ear,
    call: Call,
    /// The turn just asked said only "end call" (set by `Asking::hear_words` before the panel moves on).
    hangup: Arc<AtomicBool>,
    /// What happened, by ms into the call: for the log and the hand-run.
    pub log: Vec<(u64, String)>,
}

impl Loop {
    /// A call starts at `now` (ms): it listens.
    pub fn new(ask: Arc<dyn Asking>, feed: Arc<Feed>, vad: Box<dyn Vad>, hangup: Arc<AtomicBool>, now: u64) -> Loop {
        let (call, first) = Call::start(now);
        let mut l = Loop { ask, feed, ear: Ear::new(vad), call, hangup, log: Vec::new() };
        l.note(now, format!("call starts ({} end of turn)", l.ear.vad()));
        l.act(first, now);
        l
    }

    pub fn side(&self) -> Side {
        self.call.side()
    }

    pub fn over(&self) -> Option<Why> {
        self.call.over()
    }

    fn note(&mut self, now: u64, what: String) {
        self.log.push((now, what));
    }

    /// One look at `now`: `events` from outside, then `x`, what the microphone delivered since the last look.
    pub fn tick(&mut self, events: impl IntoIterator<Item = Event>, x: &[f32], now: u64) {
        for ev in events {
            self.step(ev, now);
        }
        self.feed.push(x);
        for h in self.ear.hear(x, self.call.side()) {
            let ms = |at: u64| at * 1000 / audio::RATE as u64;
            match h {
                Heard::Started { at } => self.note(now, format!("speech from {} ms", ms(at))),
                Heard::Ended { from, to } => self.note(now, format!("turn {}..{} ms ends", ms(from), ms(to))),
                Heard::Noise => self.note(now, "too short for a turn".into()),
                Heard::Nothing => {}
            }
            self.step(Event::Heard(h), now);
        }
        if self.hangup.swap(false, Ordering::SeqCst) {
            self.step(Event::Hangup, now);
        }
        let ev = Event::Ask { stage: self.ask.stage(), speaking: self.ask.speaking() };
        self.step(ev, now);
        self.step(Event::Tick, now);
    }

    fn step(&mut self, ev: Event, now: u64) {
        let side = self.call.side();
        let todo = self.call.step(ev, now);
        self.act(todo, now);
        if self.call.side() != side && self.call.over().is_none() {
            self.note(now, format!("{:?}", self.call.side()).to_lowercase());
        }
    }

    fn act(&mut self, todo: Vec<Do>, now: u64) {
        for d in todo {
            match d {
                Do::Listen { from } => {
                    // From just before the first word, or from this moment: never from whenever the recording opens.
                    let pre = turn::PREROLL_MS as u64 * audio::RATE as u64 / 1000;
                    let at = from.map_or(self.feed.end(), |f| f.saturating_sub(pre));
                    self.feed.start_at(Some(at));
                    let t = Instant::now();
                    while self.ask.stage() == Stage::Asking && t.elapsed() < STOP_WAIT {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    let ok = self.ask.listen();
                    self.note(now, format!("listen from {} ms{}", at * 1000 / audio::RATE as u64, if ok { "" } else { ": refused" }));
                    if !ok {
                        self.step(Event::Failed, now);
                    }
                }
                Do::Ask => {
                    self.note(now, "ask".into());
                    self.ask.ask();
                }
                Do::StopAnswer => {
                    self.note(now, "stop the answer".into());
                    self.ask.stop_answer();
                }
                Do::StopSpeech => {
                    if self.ask.speaking() {
                        self.note(now, "stop speaking".into());
                    }
                    self.ask.stop_speech();
                }
                Do::Discard => {
                    self.note(now, "drop the turn".into());
                    self.ask.drop_turn();
                }
                Do::End(why) => self.note(now, format!("call ends: {why:?}").to_lowercase()),
            }
        }
    }
}

/// How the app runs calls.
pub struct Deps {
    pub ask: Arc<dyn Asking>,
    /// Open the call's microphone into a take.
    pub open: Box<dyn Fn(Arc<Take>) -> Result<Mic, String> + Send + Sync>,
    /// Questions record from this feed while it is set (`session::App::lend_feed`).
    pub lend: Box<dyn Fn(Option<Arc<Feed>>) + Send + Sync>,
    pub vad: Box<dyn Fn() -> Box<dyn Vad> + Send + Sync>,
    /// Hears every change of the call (`call://state`).
    pub told: Box<dyn Fn(&View) + Send + Sync>,
    /// Hears the level the capsule's line follows, 0 to 1, about 30 times a second: the person's voice while it
    /// listens, its own while it speaks (0 when that is not known).
    pub level: Box<dyn Fn(f32) + Send + Sync>,
}

/// The calls of the app: at most one at a time, on a thread of its own.
pub struct Calls {
    deps: Arc<Deps>,
    view: Arc<Mutex<View>>,
    tx: Arc<Mutex<Option<Sender<Event>>>>,
}

impl Calls {
    pub fn new(deps: Deps) -> Arc<Calls> {
        Arc::new(Calls { deps: Arc::new(deps), view: Arc::default(), tx: Arc::default() })
    }

    pub fn view(&self) -> View {
        self.view.lock().unwrap().clone()
    }

    pub fn active(&self) -> bool {
        self.view.lock().unwrap().active
    }

    /// The Call switch: start a call, or end the one under way.
    pub fn toggle(&self) -> Result<View, String> {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(Event::Toggle);
            return Ok(self.view());
        }
        self.start()
    }

    /// Esc in the panel during a call: once silences the voice, twice ends the call.
    pub fn esc(&self) {
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(Event::Esc);
        }
    }

    fn start(&self) -> Result<View, String> {
        // The microphone opens first: a call that cannot hear never starts.
        let take = Arc::new(Take::default());
        let mic = (self.deps.open)(take.clone())?;
        let feed = Feed::new(KEEP_MS, take.device());
        (self.deps.lend)(Some(feed.clone()));
        let hangup = Arc::new(AtomicBool::new(false));
        let heard = hangup.clone();
        self.deps.ask.hear_words(Some(Arc::new(move |w: &str| {
            let end = is_hangup(w);
            if end {
                heard.store(true, Ordering::SeqCst);
            }
            end
        })));
        let (tx, rx) = mpsc::channel();
        *self.tx.lock().unwrap() = Some(tx);
        let view = View { active: true, side: Some(Side::Listening), ended: None };
        *self.view.lock().unwrap() = view.clone();
        (self.deps.told)(&view);
        let (deps, shared, txs) = (self.deps.clone(), self.view.clone(), self.tx.clone());
        std::thread::Builder::new()
            .name("call".into())
            .spawn(move || run(deps, shared, txs, rx, mic, take, feed, hangup))
            .map_err(|e| e.to_string())?;
        Ok(view)
    }
}

#[allow(clippy::too_many_arguments)]
fn run(deps: Arc<Deps>, view: Arc<Mutex<View>>, txs: Arc<Mutex<Option<Sender<Event>>>>, rx: Receiver<Event>, mic: Mic, take: Arc<Take>, feed: Arc<Feed>, hangup: Arc<AtomicBool>) {
    let t0 = Instant::now();
    let now = || t0.elapsed().as_millis() as u64;
    let mut l = Loop::new(deps.ask.clone(), feed, (deps.vad)(), hangup, now());
    let (mut loud, mut level_at, mut shown) = (audio::Loudness::default(), Instant::now(), Some(Side::Listening));
    while l.over().is_none() {
        std::thread::sleep(POLL);
        let mut events: Vec<Event> = rx.try_iter().collect();
        if take.error.lock().unwrap().is_some() {
            events.push(Event::Failed);
        }
        let x = take.drain();
        l.tick(events, &x, now());
        if l.over().is_none() && shown != Some(l.side()) {
            shown = Some(l.side());
            let v = View { active: true, side: shown, ended: None };
            *view.lock().unwrap() = v.clone();
            (deps.told)(&v);
        }
        let heard = loud.feed(&x);
        if level_at.elapsed() >= LEVEL_EVERY {
            level_at = Instant::now();
            (deps.level)(match l.side() {
                Side::Listening => heard,
                Side::Speaking => audio::loud_scale(deps.ask.played()),
                Side::Thinking => 0.0,
            });
        }
    }
    for (at, what) in &l.log {
        eprintln!("[call] {:>6} ms  {what}", at);
    }
    (deps.lend)(None);
    deps.ask.hear_words(None);
    drop(mic);
    *txs.lock().unwrap() = None;
    let v = View { active: false, side: None, ended: l.over() };
    *view.lock().unwrap() = v.clone();
    (deps.told)(&v);
    (deps.level)(0.0);
}

#[cfg(test)]
mod tests;
