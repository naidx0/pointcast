//! Ask about this: press the Ask key, say a question about what is on the screen, and the answer streams into the
//! panel. One frame is taken at the press, before the panel shows; the words come through the recording path; the
//! turn is kept in a session (`sessions`) before anything is sent, so no question is lost to a failure after that.
//! The panel's states are one small reducer (`next`); everything around it is told what to do through `Deps`, so the
//! whole loop runs in a test on a scripted provider with no microphone, screen or window.

use crate::assist::{self, Chat, Event, Failure, Kind, Part, Provider, Role, Turn};
use crate::capture::{Asked, Look};
use crate::keyframes::{self, Thumb};
use crate::prompt::{self, Item};
use crate::session::{Phase, Purpose, Said};
use crate::voice::stt;
use crate::sessions::{Answer, Question, Sent, Sessions, Times, Used, Wire};
use base64::Engine as _;
use serde::Serialize;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a question waits for the first sign of its answer.
pub const FIRST_WAIT: Duration = Duration::from_secs(20);

/// The panel hears of new words at most 30 times a second.
const FRAME: Duration = Duration::from_millis(34);

/// The long side of the picture the panel shows beside the question.
const THUMB: u32 = 480;

/// While a question is said, where the person looks is read every this many words checks (100 ms each): twice a second.
const LOOK_EVERY: u64 = 5;

pub const NO_PROVIDER: &str = assist::catalog::NONE_CHOSEN;
pub const NO_ANSWER: &str = "No answer after 20 seconds. Ask again.";
pub const NOTHING_TO_RETRY: &str = "There is no question to ask again.";

const SYSTEM: &str = "You are the study assistant in Pointcast. The person asks a question out loud about what is on \
their screen, and a picture of it may be attached. Answer plainly and briefly, in markdown. You cannot run tools, \
open files or see anything but the pictures you are sent.";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    #[default]
    Closed,
    Listening,
    Finishing,
    Sending,
    Answering,
    Shown,
    Error,
    /// Stop while the question was said or finished: nothing was kept or sent. The panel says so briefly, then closes.
    Discarded,
}

/// What moves the panel on. Each state is entered by exactly one of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The Ask key went down and the microphone is on.
    Press,
    /// The key came up, or was pressed again: the last words are being finished.
    Release,
    /// The question is kept and on its way.
    Send,
    /// The first words of the answer came.
    First,
    /// The question was empty, or its answer ended: the panel shows what there is.
    Settle,
    /// Stop while the question was said or finished: it is thrown away.
    Discard,
    Fail,
    Close,
}

/// The state after `step`, or none when that step means nothing where the panel is.
pub fn next(from: State, step: Step) -> Option<State> {
    use State::*;
    match (from, step) {
        (Closed | Shown | Error | Discarded, Step::Press) => Some(Listening),
        (Listening, Step::Release) => Some(Finishing),
        (Finishing | Error, Step::Send) => Some(Sending),
        (Sending, Step::First) => Some(Answering),
        (Listening | Finishing | Sending | Answering, Step::Settle) => Some(Shown),
        (Listening | Finishing, Step::Discard) => Some(Discarded),
        (Closed | Shown | Error | Discarded | Finishing | Sending | Answering, Step::Fail) => Some(Error),
        (_, Step::Close) => Some(Closed),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct SessionRef {
    pub id: String,
    pub title: String,
}

/// The picture of the question, small, as something an `<img>` can load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Picture {
    pub src: String,
    pub w: u32,
    pub h: u32,
    /// The window it shows, by its title (`label`); empty for a whole screen or a window that is never named.
    pub label: String,
}

/// Everything the panel draws from, sent whole on every change (`live://state`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct Snapshot {
    pub state: State,
    /// The session the panel is on; both empty until its first question is kept.
    pub session: SessionRef,
    /// Which question of this panel opening the words and the answer belong to, counted from 1; 0 before the first.
    pub turn: u64,
    pub words: String,
    pub picture: Option<Picture>,
    pub dropped: bool,
    /// The AI being asked, as the person knows it ("ChatGPT"); empty when none is.
    pub provider: String,
    pub error: Option<String>,
    /// The model is working and has said nothing yet.
    pub thinking: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    State(Snapshot),
    Delta { turn: u64, text: String },
    Show,
    Hide,
}

/// The recording path, as the loop needs it.
pub trait Voice: Send + Sync {
    /// Something records or is finishing.
    fn busy(&self) -> bool;
    /// Start listening for a question; false when a recording is already under way.
    fn start(&self) -> Result<bool, String>;
    /// The question so far: the pieces heard, and the piece being heard.
    fn heard(&self) -> (Vec<Item>, String);
    /// Stop, waiting for the last words. None when discarded or when nothing was listening.
    fn stop(&self, discard: bool) -> Option<Said>;
}

/// A question's pieces without the strays it starts with: a piece that says only one of `stt::STRAY` ("you", "thank
/// you") before any other words is what the speech model made of the quiet before the person spoke, as dictation drops
/// it. A question of nothing else has nothing said.
pub fn unstrayed(mut items: Vec<Item>) -> Vec<Item> {
    let mut lead = true;
    items.retain(|i| match i {
        Item::Speech { text, .. } if lead && (text.trim().is_empty() || stt::is_stray(text)) => false,
        Item::Speech { .. } => {
            lead = false;
            true
        }
        Item::Image { .. } => true,
    });
    items
}

/// The question so far as the panel shows it: the pieces heard, then the piece being heard, with no stray at the start.
pub fn said_so_far(items: Vec<Item>, partial: &str) -> String {
    let items = unstrayed(items);
    let lead = !items.iter().any(|i| matches!(i, Item::Speech { .. }));
    let partial = if lead && stt::is_stray(partial) { "" } else { partial.trim() };
    [prompt::compose(&items).0, partial.to_string()].into_iter().filter(|s| !s.trim().is_empty()).collect::<Vec<_>>().join(" ")
}

/// The recording path of the app.
pub struct AppVoice(pub Arc<crate::session::App>);

impl Voice for AppVoice {
    fn busy(&self) -> bool {
        !matches!(self.0.ui().phase, Phase::Idle | Phase::Ready | Phase::Error)
    }

    fn start(&self) -> Result<bool, String> {
        self.0.start_for(Purpose::Ask)
    }

    fn heard(&self) -> (Vec<Item>, String) {
        (self.0.heard(), self.0.ui().partial)
    }

    fn stop(&self, discard: bool) -> Option<Said> {
        let _ = self.0.stop(discard);
        self.0.take_said()
    }
}

/// A frame of the screen as it was taken, not yet cut or encoded.
pub trait Taken: Send {
    /// Where the person looked when it was taken.
    fn look(&self) -> Look;
    /// The whole frame as a 64 px thumbnail, to tell how much the screen changed.
    fn thumb(&self) -> Thumb;
    /// Cut, marked and encoded: the question's picture.
    fn cut(self: Box<Self>) -> Result<Asked, String>;
}

pub struct Deps {
    pub voice: Arc<dyn Voice>,
    /// One frame of the screen as it is now.
    pub grab: Box<dyn Fn() -> Result<Box<dyn Taken>, String> + Send + Sync>,
    /// Where the person looks now, from one read of the window list and no picture.
    pub look: Box<dyn Fn() -> Look + Send + Sync>,
    /// The AI the person chose, as the settings say at this Ask; else the line the panel shows instead.
    pub provider: Box<dyn Fn() -> Result<Arc<dyn Provider>, String> + Send + Sync>,
    pub out: Box<dyn Fn(Out) + Send + Sync>,
    /// Unix ms.
    pub now: Box<dyn Fn() -> u64 + Send + Sync>,
    pub tidy: Box<dyn Fn() -> bool + Send + Sync>,
}

/// A question that is kept and on its way, or that failed and can be asked again: what is sent, and its answer so far.
#[derive(Clone)]
struct Kept {
    session: String,
    turn: String,
    parts: Vec<Part>,
    answer: Answer,
}

#[derive(Default)]
struct Inner {
    snap: Snapshot,
    /// Moved on by every press, discard and close: work begun under an older number leaves the panel alone.
    run: u64,
    /// The picture of the question being said, on its way from the frame.
    picture: Option<Receiver<Option<Shot>>>,
    /// Where the person looked when that frame was taken.
    look: Look,
    /// Moved on by every frame taken for a question: a picture cut from an older one never reaches the panel.
    shot: u64,
    provider: Option<Arc<dyn Provider>>,
    chat: Option<Box<dyn Chat>>,
    stopper: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Stop was asked while the question was on its way.
    halt: bool,
    session: Option<String>,
    press_ms: u64,
    release_ms: u64,
    /// The question that failed, until it is answered, another is asked or the panel closes.
    kept: Option<Kept>,
    /// When the AI's helper was last used: from here the idle scheduler counts the time before it is let go.
    used: Option<Instant>,
}

pub struct Live {
    deps: Deps,
    sessions: Sessions,
    wait: Duration,
    inner: Mutex<Inner>,
}

/// The line the panel shows for a failure.
pub fn line(f: &Failure) -> String {
    match f {
        Failure::NotInstalled => "The AI's app is not installed.".into(),
        Failure::SignedOut => "Sign in to your AI, then ask again.".into(),
        Failure::Limit { .. } => "Your plan's limit is used up for now.".into(),
        Failure::Offline => "No connection to the AI.".into(),
        Failure::Refused => "The AI tried to do more than answer, so the answer was stopped.".into(),
        Failure::NoPictures => "This AI does not take pictures.".into(),
        Failure::ContextFull => "This conversation no longer fits the AI. Start a new one.".into(),
        Failure::Stopped => "Stopped.".into(),
        Failure::Broken(why) => why.clone(),
    }
}

/// What the person calls the AI behind provider `id`: the panel never shows an id, so one with no name here shows
/// nothing.
pub fn name_of(id: &str) -> &'static str {
    assist::catalog::name_of(id)
}

/// One line about the picture, sent with the question: its size, the window it shows, the control pointed at when the
/// OS names one, and how much of it is painted over.
pub fn caption(a: &Asked) -> String {
    let mut c = format!("[Picture: {} x {} pixels", a.wire_w, a.wire_h);
    if let Some(t) = &a.window {
        c.push_str(&format!(", the window \"{t}\""));
    }
    if let Some(e) = &a.element {
        c.push_str(&format!(", pointing at {} \"{}\"", e.role, e.name));
    }
    match a.hidden {
        0 => {}
        1 => c.push_str(", 1 area hidden"),
        n => c.push_str(&format!(", {n} areas hidden")),
    }
    c.push(']');
    c
}

/// What the panel names a picture by: the title of the window it shows. The capture never reads out the title of a
/// window on the never-capture list, so such a window is never named; nor is a whole screen.
pub fn label(a: &Asked) -> String {
    a.window.clone().unwrap_or_default()
}

/// While a question is said, its picture is taken again when the window in front or the one under the pointer is
/// another than when it was taken (or the same one with another title or place).
pub fn look_moved(was: &Look, now: &Look) -> bool {
    was != now
}

/// At the release, the frame taken then goes instead of the picture so far when the person looks at another window, or
/// the screen changed by more than `keyframes::CHANGE` on its 64 px thumbnail (another page in the same window).
pub fn moved_on(was: &Look, now: &Look, change: f32) -> bool {
    look_moved(was, now) || change > keyframes::CHANGE
}

/// A frame cut and encoded: what is kept and sent, the panel's small copy, and the thumbnail its change is measured on.
struct Shot {
    asked: Asked,
    small: Option<Picture>,
    thumb: Thumb,
}

/// Exactly the picture that is sent, made small for the panel.
fn thumbnail(a: &Asked) -> Option<Picture> {
    let small = image::load_from_memory(&a.wire).ok()?.thumbnail(THUMB, THUMB).to_rgb8();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 80)
        .encode(small.as_raw(), small.width(), small.height(), image::ExtendedColorType::Rgb8)
        .ok()?;
    let src = format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(&jpeg));
    Some(Picture { src, w: a.wire_w, h: a.wire_h, label: label(a) })
}

fn shot_from(asked: Asked, thumb: Thumb) -> Shot {
    Shot { small: thumbnail(&asked), asked, thumb }
}

fn shot_of(taken: Box<dyn Taken>) -> Option<Shot> {
    let thumb = taken.thumb();
    taken.cut().ok().map(|a| shot_from(a, thumb))
}

impl Live {
    pub fn new(deps: Deps, data_dir: &std::path::Path) -> Arc<Live> {
        Live::with_wait(deps, data_dir, FIRST_WAIT)
    }

    pub fn with_wait(deps: Deps, data_dir: &std::path::Path, wait: Duration) -> Arc<Live> {
        Arc::new(Live { deps, sessions: Sessions::new(data_dir), wait, inner: Mutex::default() })
    }

    pub fn snapshot(&self) -> Snapshot {
        self.inner.lock().unwrap().snap.clone()
    }

    pub fn listening(&self) -> bool {
        self.inner.lock().unwrap().snap.state == State::Listening
    }

    fn step(&self, g: &mut Inner, step: Step) -> bool {
        match next(g.snap.state, step) {
            Some(s) => {
                g.snap.state = s;
                true
            }
            None => false,
        }
    }

    fn tell(&self, g: &Inner) {
        (self.deps.out)(Out::State(g.snap.clone()));
    }

    /// The panel opens on `why` and nothing else happens.
    fn refuse(&self, mut g: std::sync::MutexGuard<'_, Inner>, why: String) {
        g.snap.error = Some(why);
        g.snap.thinking = false;
        self.step(&mut g, Step::Fail);
        self.tell(&g);
        drop(g);
        (self.deps.out)(Out::Show);
    }

    /// The Ask key went down. True when a question is now being listened to.
    pub fn press(self: &Arc<Self>) -> bool {
        let mut g = self.inner.lock().unwrap();
        // A dictation under way is left alone: it never opens the panel.
        if next(g.snap.state, Step::Press).is_none() || self.deps.voice.busy() {
            return false;
        }
        let provider = match (self.deps.provider)() {
            Ok(p) => p,
            Err(why) => {
                // Nothing is grabbed, kept or sent without an AI the person chose and allowed.
                self.refuse(g, why);
                return false;
            }
        };
        // The screen as it is now, before the panel is on it.
        let frame = (self.deps.grab)().ok();
        provider.warm();
        match self.deps.voice.start() {
            Ok(true) => {}
            Ok(false) => return false,
            Err(e) => {
                self.refuse(g, e);
                return false;
            }
        }
        g.run += 1;
        let run = g.run;
        g.press_ms = (self.deps.now)();
        g.halt = false;
        g.kept = None;
        g.used = Some(Instant::now());
        g.snap.provider = name_of(provider.id()).into();
        g.provider = Some(provider);
        g.snap.error = None;
        g.snap.words.clear();
        g.snap.picture = None;
        g.snap.dropped = false;
        g.snap.thinking = false;
        self.step(&mut g, Step::Press);
        g.picture = None;
        if let Some(frame) = frame {
            self.cut(&mut g, frame, run);
        }
        self.tell(&g);
        drop(g);
        (self.deps.out)(Out::Show);
        let me = self.clone();
        std::thread::spawn(move || me.follow(run));
        true
    }

    /// `frame` is the question's picture from now on. Cutting and encoding it happens off the caller's thread; the panel
    /// gets its small copy when it is done, unless a later frame took its place or the picture was dropped meanwhile.
    fn cut(self: &Arc<Self>, g: &mut Inner, frame: Box<dyn Taken>, run: u64) {
        g.shot += 1;
        g.look = frame.look();
        let shot = g.shot;
        let (tx, rx) = mpsc::channel();
        g.picture = Some(rx);
        let me = self.clone();
        std::thread::spawn(move || {
            let made = shot_of(frame);
            {
                let mut g = me.inner.lock().unwrap();
                let waiting = matches!(g.snap.state, State::Listening | State::Finishing);
                if g.run == run && g.shot == shot && waiting && !g.snap.dropped {
                    g.snap.picture = made.as_ref().and_then(|s| s.small.clone());
                    me.tell(&g);
                }
            }
            let _ = tx.send(made);
        });
    }

    /// While the question is said, its words reach the panel as they are heard, and twice a second the picture follows
    /// the window the person looks at, so the one the panel shows is the one that would go.
    fn follow(self: &Arc<Self>, run: u64) {
        for n in 1u64.. {
            std::thread::sleep(Duration::from_millis(100));
            let look = (n % LOOK_EVERY == 0).then(|| (self.deps.look)());
            let mut g = self.inner.lock().unwrap();
            if g.run != run || g.snap.state != State::Listening {
                return;
            }
            // The recording was ended somewhere else: the question is discarded.
            if !self.deps.voice.busy() {
                drop(g);
                return self.stop();
            }
            let (items, partial) = self.deps.voice.heard();
            let words = said_so_far(items, &partial);
            if words != g.snap.words {
                g.snap.words = words;
                self.tell(&g);
            }
            let Some(look) = look.filter(|l| !g.snap.dropped && look_moved(&g.look, l)) else { continue };
            // Not asked again for this window if the screen cannot be read now.
            g.look = look;
            drop(g);
            // Reading the screen takes a moment: never under the lock.
            let Ok(frame) = (self.deps.grab)() else { continue };
            let mut g = self.inner.lock().unwrap();
            if g.run == run && g.snap.state == State::Listening && !g.snap.dropped {
                self.cut(&mut g, frame, run);
            }
        }
    }

    /// The key came up, or was pressed again: finish the words, then ask.
    pub fn release(self: &Arc<Self>) {
        let mut g = self.inner.lock().unwrap();
        if !self.step(&mut g, Step::Release) {
            return;
        }
        g.release_ms = (self.deps.now)();
        let run = g.run;
        self.tell(&g);
        drop(g);
        let me = self.clone();
        std::thread::spawn(move || me.answer(run));
    }

    /// The picture is taken off the question being said: it is neither kept nor sent.
    pub fn drop_picture(&self) {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.snap.state, State::Listening | State::Finishing) && !g.snap.dropped {
            g.picture = None;
            g.snap.picture = None;
            g.snap.dropped = true;
            self.tell(&g);
        }
    }

    /// Esc: a question being said is discarded with nothing kept or sent; an answer on its way is stopped.
    pub fn stop(&self) {
        let mut g = self.inner.lock().unwrap();
        match g.snap.state {
            State::Listening | State::Finishing => {
                let listening = g.snap.state == State::Listening;
                // The worker finishing the words, if there is one, finds its question discarded.
                g.run += 1;
                g.picture = None;
                // Nothing of the discarded question is left for the panel to show as sent: no words, no picture, and no
                // AI being asked. The turn count is of kept questions, so this one never moved it.
                g.snap.picture = None;
                g.snap.words.clear();
                g.snap.dropped = false;
                g.snap.provider.clear();
                g.snap.error = None;
                g.snap.thinking = false;
                self.step(&mut g, Step::Discard);
                self.tell(&g);
                drop(g);
                if listening {
                    let _ = self.deps.voice.stop(true);
                }
            }
            State::Sending | State::Answering => {
                g.halt = true;
                let stopper = g.stopper.clone();
                drop(g);
                if let Some(s) = stopper {
                    s();
                }
            }
            _ => {}
        }
    }

    /// The panel closes: whatever is under way stops, and the next Ask starts a new session.
    pub fn close(&self) {
        let mut g = self.inner.lock().unwrap();
        let listening = g.snap.state == State::Listening;
        g.run += 1;
        let (stopper, chat) = (g.stopper.take(), g.chat.take());
        g.picture = None;
        g.provider = None;
        g.session = None;
        g.kept = None;
        g.halt = false;
        g.snap = Snapshot::default();
        self.tell(&g);
        drop(g);
        (self.deps.out)(Out::Hide);
        if listening {
            let _ = self.deps.voice.stop(true);
        }
        if let Some(s) = stopper {
            s();
        }
        if let Some(mut c) = chat {
            c.close();
        }
    }

    fn fail(&self, run: u64, why: String) {
        let mut g = self.inner.lock().unwrap();
        if g.run != run {
            return;
        }
        g.snap.error = Some(why);
        g.snap.thinking = false;
        self.step(&mut g, Step::Fail);
        self.tell(&g);
    }

    /// What a session already said, as turns, and the ids of those turns, oldest first; never the turn `except`.
    fn earlier(&self, session: &str, except: Option<&str>) -> (Vec<Turn>, Vec<String>) {
        let Ok((_, kept)) = self.sessions.read(session) else {
            return Default::default();
        };
        let (mut turns, mut ids) = (vec![], vec![]);
        for k in kept.into_iter().filter(|k| Some(k.meta.id.as_str()) != except) {
            let Some(a) = k.answer else { continue };
            let mut parts = vec![Part::Text(a.sent.text.clone())];
            for p in &a.sent.pictures {
                if let Ok((path, _)) = self.sessions.sent_picture(session, &k.meta.id, p.n) {
                    parts.push(Part::Picture { path, w: p.w, h: p.h, mime: p.mime });
                }
            }
            turns.push(Turn { role: Role::Person, parts });
            if !a.text.is_empty() {
                turns.push(Turn { role: Role::Assistant, parts: vec![Part::Text(a.text.clone())] });
            }
            ids.push(k.meta.id);
        }
        (turns, ids)
    }

    /// From the release to the end of the answer, off the key's thread.
    fn answer(&self, run: u64) {
        let now = || (self.deps.now)();
        // The screen at the release is read while the last words are finished, unless the picture was taken off.
        let dropped = self.inner.lock().unwrap().snap.dropped;
        let (said, later) = std::thread::scope(|s| {
            let later = (!dropped).then(|| s.spawn(|| (self.deps.grab)().ok()));
            let said = self.deps.voice.stop(false).map(|mut said| {
                said.items = unstrayed(said.items);
                said
            });
            (said, later.and_then(|l| l.join().ok().flatten()))
        });
        let words_ms = now();
        let text = said.as_ref().map(|s| prompt::compose_with(&s.items, (self.deps.tidy)()).0).unwrap_or_default().trim().to_string();
        let picture = {
            let mut g = self.inner.lock().unwrap();
            // Discarded or closed while the words were finished: nothing is kept.
            if g.run != run || g.snap.state != State::Finishing {
                return;
            }
            let picture = g.picture.take();
            if text.is_empty() {
                // Nothing was said: the frame goes, nothing is kept or sent, and the panel is simply open.
                g.snap.picture = None;
                g.snap.words.clear();
                self.step(&mut g, Step::Settle);
                return self.tell(&g);
            }
            g.snap.words = text.clone();
            self.tell(&g);
            picture
        };
        let Some(said) = said else { return };
        let shot = self.pick(run, picture.and_then(|rx| rx.recv().ok().flatten()), later);
        let (provider, session, fresh, press_ms, release_ms, asked) = {
            let mut g = self.inner.lock().unwrap();
            if g.run != run || g.snap.state != State::Finishing {
                return;
            }
            // From here the picture can no longer be dropped: it is what is kept and sent, or it is neither.
            let asked = shot.map(|s| s.asked).filter(|_| !g.snap.dropped);
            self.step(&mut g, Step::Send);
            self.tell(&g);
            (g.provider.clone(), g.session.clone(), g.chat.is_none(), g.press_ms, g.release_ms, asked)
        };
        let Some(provider) = provider else { return };
        let sent_text = match asked.as_ref().map(caption) {
            Some(c) => format!("{text}\n\n{c}"),
            None => text,
        };
        // A conversation opened on a session that already has turns starts from them.
        let (before, ids) = match (&session, fresh) {
            (Some(id), true) => self.earlier(id, None),
            _ => Default::default(),
        };
        let wires: Vec<Wire> = asked.iter().map(|a| Wire { bytes: &a.wire, w: a.wire_w, h: a.wire_h, mime: a.mime }).collect();
        let first = Answer {
            provider: provider.id().into(),
            times: Times { press_ms, release_ms, words_ms, ..Times::default() },
            sent: Sent { text: sent_text.clone(), pictures: vec![], earlier: ids, adds: vec![] },
            ..Answer::default()
        };
        let q = Question { items: &said.items, created_ms: said.created_ms, duration_ms: said.duration_ms, audio: said.audio.clone(), pictures: &wires };
        let saved = match self.sessions.save_turn(session.as_deref(), q, first, "", now()) {
            Ok(s) => s,
            Err(e) => return self.fail(run, format!("The question could not be kept: {e}")),
        };
        let mut parts = vec![Part::Text(sent_text)];
        if let (Some(a), Some(path)) = (&asked, saved.pictures.first()) {
            parts.push(Part::Picture { path: path.clone(), w: a.wire_w, h: a.wire_h, mime: a.mime });
        }
        let kept = Kept { session: saved.session.id.clone(), turn: saved.turn.clone(), parts, answer: saved.answer.clone() };
        let (turn_no, chat) = {
            let mut g = self.inner.lock().unwrap();
            if g.run != run {
                // The panel closed while the question was kept: it stays on disk, unanswered.
                let mut answer = kept.answer;
                answer.stopped = true;
                let _ = self.sessions.write_answer(&kept.session, &kept.turn, &answer, now());
                return;
            }
            g.session = Some(kept.session.clone());
            g.snap.session = SessionRef { id: kept.session.clone(), title: saved.session.title.clone() };
            g.snap.turn += 1;
            g.kept = Some(kept.clone());
            self.tell(&g);
            (g.snap.turn, g.chat.take())
        };
        self.send(run, &provider, kept, turn_no, chat, before);
    }

    /// The one picture that goes with the question: the one so far (`was`), or the frame taken at the release (`later`)
    /// when the person moved on since to another window or another page. The panel is shown the one that goes.
    fn pick(&self, run: u64, was: Option<Shot>, later: Option<Box<dyn Taken>>) -> Option<Shot> {
        let Some(later) = later else { return was };
        let thumb = later.thumb();
        let change = was.as_ref().map_or(1.0, |s| keyframes::change(&s.thumb, &thumb));
        let look = later.look();
        if !moved_on(&self.inner.lock().unwrap().look, &look, change) {
            return was;
        }
        let Ok(asked) = later.cut() else { return was };
        let shot = shot_from(asked, thumb);
        let mut g = self.inner.lock().unwrap();
        if g.run == run && g.snap.state == State::Finishing && !g.snap.dropped {
            g.shot += 1;
            g.look = look;
            g.snap.picture = shot.small.clone();
            self.tell(&g);
        }
        Some(shot)
    }

    /// Ask the question that failed again, as it was kept: the same words and the same picture bytes, in the same
    /// session, as the panel's next turn. Nothing happens when no question failed.
    pub fn retry(self: &Arc<Self>) -> Result<(), String> {
        let mut g = self.inner.lock().unwrap();
        let Some(mut kept) = g.kept.clone().filter(|_| g.snap.state == State::Error) else {
            return Err(NOTHING_TO_RETRY.into());
        };
        let provider = (self.deps.provider)()?;
        g.run += 1;
        let run = g.run;
        g.halt = false;
        g.used = Some(Instant::now());
        g.snap.error = None;
        g.snap.thinking = false;
        g.snap.provider = name_of(provider.id()).into();
        g.provider = Some(provider.clone());
        g.snap.turn += 1;
        let turn_no = g.snap.turn;
        self.step(&mut g, Step::Send);
        self.tell(&g);
        let chat = g.chat.take();
        drop(g);
        // The conversation that failed was closed: the new one starts from what the session kept before this question.
        let (before, ids) = if chat.is_none() { self.earlier(&kept.session, Some(&kept.turn)) } else { Default::default() };
        let was = kept.answer;
        kept.answer = Answer {
            provider: provider.id().into(),
            times: Times { sent_ms: 0, first_ms: 0, done_ms: 0, ..was.times },
            sent: Sent { earlier: ids, ..was.sent },
            ..Answer::default()
        };
        let me = self.clone();
        std::thread::spawn(move || me.send(run, &provider, kept, turn_no, chat, before));
        Ok(())
    }

    /// Ask the kept question and follow its answer to the end, keeping it as it comes.
    fn send(&self, run: u64, provider: &Arc<dyn Provider>, kept: Kept, turn_no: u64, chat: Option<Box<dyn Chat>>, before: Vec<Turn>) {
        let now = || (self.deps.now)();
        let Kept { session: sid, turn: tid, parts, mut answer } = kept;
        let keep = |a: &Answer| {
            let _ = self.sessions.write_answer(&sid, &tid, a, now());
        };
        // Opening can take a while (a helper starts): never under the lock.
        let opened = match chat {
            Some(c) => Ok(c),
            None if before.is_empty() => provider.open(SYSTEM, &[]),
            None => provider.open(SYSTEM, &[Turn { role: Role::Person, parts: assist::replay(&before, 2) }]),
        };
        let mut chat = match opened {
            Ok(c) => c,
            Err(f) => {
                answer.error = Some(line(&f));
                keep(&answer);
                return self.fail(run, line(&f));
            }
        };
        // What the AI's own app adds to the question goes along too, and is kept as sent.
        answer.sent.adds = chat.adds();
        let stopper = chat.stopper();
        let (tx, rx) = mpsc::channel::<Event>();
        answer.times.sent_ms = now();
        let id = chat.ask(Turn { role: Role::Person, parts }, tx);
        {
            let mut g = self.inner.lock().unwrap();
            if g.run != run {
                chat.close();
            } else {
                g.stopper = Some(stopper.clone());
                g.chat = Some(chat);
            }
            // A stop asked before there was anything to stop, or a panel closed meanwhile.
            if g.halt || g.run != run {
                g.halt = false;
                drop(g);
                stopper();
            }
        }
        let id = match id {
            Ok(id) => id,
            Err(f) => {
                answer.error = Some(line(&f));
                keep(&answer);
                return self.fail(run, line(&f));
            }
        };
        // The answer: only this turn's events count, and the first Done or Failed is its end.
        let deadline = Instant::now() + self.wait;
        let (mut seen, mut pending, mut told) = (false, String::new(), Instant::now() - FRAME);
        let end = loop {
            let wait = if !seen {
                deadline.saturating_duration_since(Instant::now())
            } else if pending.is_empty() {
                Duration::from_secs(3600)
            } else {
                FRAME.saturating_sub(told.elapsed())
            };
            match rx.recv_timeout(wait) {
                Ok(ev) if ev.turn != id => {}
                Ok(ev) => {
                    seen = true;
                    match ev.kind {
                        Kind::Thinking => {
                            let mut g = self.inner.lock().unwrap();
                            if g.run == run && g.snap.state == State::Sending && !g.snap.thinking {
                                g.snap.thinking = true;
                                self.tell(&g);
                            }
                        }
                        Kind::Delta(words) => {
                            if answer.times.first_ms == 0 {
                                answer.times.first_ms = now();
                                let mut g = self.inner.lock().unwrap();
                                if g.run == run {
                                    g.snap.thinking = false;
                                    self.step(&mut g, Step::First);
                                    self.tell(&g);
                                }
                            }
                            answer.text.push_str(&words);
                            pending.push_str(&words);
                        }
                        Kind::Usage(u) => answer.usage = Some(Used { input: u.input, cached: u.cached, output: u.output }),
                        Kind::Limits(_) => {}
                        Kind::Done => break None,
                        Kind::Failed(f) => break Some(line(&f)).filter(|_| f != Failure::Stopped).or(Some(String::new())),
                    }
                }
                Err(RecvTimeoutError::Timeout) if !seen => {
                    stopper();
                    break Some(NO_ANSWER.into());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break Some("The answer ended before it was finished.".into()),
            }
            if !pending.is_empty() && told.elapsed() >= FRAME {
                (self.deps.out)(Out::Delta { turn: turn_no, text: std::mem::take(&mut pending) });
                told = Instant::now();
            }
        };
        if !pending.is_empty() {
            (self.deps.out)(Out::Delta { turn: turn_no, text: pending });
        }
        answer.times.done_ms = now();
        // An empty line is the person's own stop: the answer is kept as far as it came, with no error.
        match end {
            None => {}
            Some(why) if why.is_empty() => answer.stopped = true,
            Some(why) => answer.error = Some(why),
        }
        keep(&answer);
        let mut g = self.inner.lock().unwrap();
        if g.run != run {
            return;
        }
        g.halt = false;
        g.snap.thinking = false;
        g.used = Some(Instant::now());
        match &answer.error {
            Some(why) => {
                // A conversation that failed is not asked again: the next question opens one from what is kept.
                g.stopper = None;
                if let Some(mut c) = g.chat.take() {
                    c.close();
                }
                g.snap.error = Some(why.clone());
                self.step(&mut g, Step::Fail);
            }
            None => {
                g.kept = None;
                self.step(&mut g, Step::Settle);
            }
        }
        self.tell(&g);
        drop(g);
        // The idle scheduler counts five minutes from here.
        crate::wake();
    }

    /// When the AI's helper was last used, while the panel may still hold one.
    pub fn idle_since(&self) -> Option<Instant> {
        self.inner.lock().unwrap().used
    }

    /// Let the AI's helper go after it sat idle: the conversation is closed, and the next question opens one from
    /// what the session kept. An answer under way is left alone.
    pub fn let_go(&self) {
        let mut g = self.inner.lock().unwrap();
        if matches!(g.snap.state, State::Sending | State::Answering) {
            g.used = Some(Instant::now());
            return;
        }
        g.used = None;
        g.stopper = None;
        let chat = g.chat.take();
        drop(g);
        if let Some(mut c) = chat {
            c.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::{Caps, Health, Mime, Scripted, TurnId};
    use crate::prompt::Item;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
    use std::sync::mpsc::Sender;

    #[derive(Default)]
    struct FakeVoice {
        on: AtomicBool,
        say: Mutex<String>,
        /// When set, what is heard piece by piece instead of `say` as one piece.
        pieces: Mutex<Vec<String>>,
        /// Every stop asked of it: whether it was a discard.
        stops: Mutex<Vec<bool>>,
        nth: AtomicU64,
    }

    impl FakeVoice {
        fn items(&self) -> Vec<Item> {
            let pieces = self.pieces.lock().unwrap().clone();
            let pieces = if pieces.is_empty() { vec![self.say.lock().unwrap().clone()] } else { pieces };
            let piece = |(n, text): (usize, String)| Item::Speech { t_ms: n as u64 * 1_000, end_ms: n as u64 * 1_000 + 900, text };
            pieces.into_iter().filter(|t| !t.is_empty()).enumerate().map(piece).collect()
        }
    }

    impl Voice for FakeVoice {
        fn busy(&self) -> bool {
            self.on.load(Ordering::SeqCst)
        }

        fn start(&self) -> Result<bool, String> {
            Ok(!self.on.swap(true, Ordering::SeqCst))
        }

        fn heard(&self) -> (Vec<Item>, String) {
            (self.items(), String::new())
        }

        fn stop(&self, discard: bool) -> Option<Said> {
            let was = self.on.swap(false, Ordering::SeqCst);
            self.stops.lock().unwrap().push(discard);
            if !was || discard {
                return None;
            }
            let items = self.items();
            // Each question begins at its own second, as real ones do.
            let created_ms = 1_700_000_000_000 + self.nth.fetch_add(1, Ordering::SeqCst) * 1_000;
            Some(Said { items, created_ms, duration_ms: 900, audio: None })
        }
    }

    /// A provider that says exactly these events, under these turn ids, the moment it is asked; `hold` keeps the line
    /// open after them.
    struct Canned {
        events: Vec<(u64, Kind)>,
        hold: bool,
        held: Arc<Mutex<Vec<Sender<Event>>>>,
    }

    struct CannedChat(Vec<(u64, Kind)>, bool, Arc<Mutex<Vec<Sender<Event>>>>);

    impl Provider for Canned {
        fn id(&self) -> &'static str {
            "canned"
        }
        fn caps(&self, _: Option<&str>) -> Caps {
            Caps { images: true, max_images: 4, max_edge: 2048, offline: true }
        }
        fn health(&self) -> Health {
            Health { state: assist::State::Ready, line: "Canned.".into(), goes_to: "nowhere" }
        }
        fn warm(&self) {}
        fn open(&self, _: &str, _: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
            Ok(Box::new(CannedChat(self.events.clone(), self.hold, self.held.clone())))
        }
    }

    impl Chat for CannedChat {
        fn ask(&mut self, _: Turn, out: Sender<Event>) -> Result<TurnId, Failure> {
            for (turn, kind) in self.0.clone() {
                let _ = out.send(Event { turn: TurnId(turn), kind });
            }
            if self.1 {
                self.2.lock().unwrap().push(out);
            }
            Ok(TurnId(1))
        }
        fn stopper(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn close(&mut self) {}
    }

    struct Rig {
        live: Arc<Live>,
        voice: Arc<FakeVoice>,
        outs: Arc<Mutex<Vec<Out>>>,
        grabs: Arc<AtomicUsize>,
        /// Held by a test, the frame taken at the press is not cut until it lets go.
        gate: Arc<Mutex<()>>,
        /// What is on the screen: where the person looks, the window a frame shows and its shade.
        screen: Arc<Mutex<Screen>>,
        dir: PathBuf,
    }

    #[derive(Clone)]
    struct Screen {
        look: Look,
        window: &'static str,
        shade: u8,
    }

    impl Default for Screen {
        fn default() -> Self {
            Screen { look: Look { front: Some(1), under: Some(1) }, window: "Notes", shade: 200 }
        }
    }

    fn shot() -> Asked {
        shot_on(&Screen::default())
    }

    fn shot_on(s: &Screen) -> Asked {
        let wire = crate::image_ops::encode_png(&[s.shade; 4 * 2 * 4], 4, 2).unwrap();
        Asked { png: wire.clone(), width: 4, height: 2, wire, wire_w: 4, wire_h: 2, mime: Mime::Png, window: Some(s.window.into()), element: None, hidden: 1 }
    }

    struct FakeFrame(Screen, Arc<Mutex<()>>);

    impl Taken for FakeFrame {
        fn look(&self) -> Look {
            self.0.look
        }
        fn thumb(&self) -> Thumb {
            keyframes::thumb(&[self.0.shade; 64 * 36 * 4], 64, 36)
        }
        fn cut(self: Box<Self>) -> Result<Asked, String> {
            let _open = self.1.lock().unwrap();
            Ok(shot_on(&self.0))
        }
    }

    fn rig(name: &str, provider: Option<Arc<dyn Provider>>, wait: Duration) -> Rig {
        rig_with(name, Box::new(move || provider.clone().ok_or_else(|| NO_PROVIDER.to_owned())), wait)
    }

    fn rig_with(name: &str, provider: Box<dyn Fn() -> Result<Arc<dyn Provider>, String> + Send + Sync>, wait: Duration) -> Rig {
        let dir = std::env::temp_dir().join(format!("pc-live-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (voice, outs, grabs) = (Arc::new(FakeVoice::default()), Arc::new(Mutex::new(vec![])), Arc::new(AtomicUsize::new(0)));
        let (o, g, clock) = (outs.clone(), grabs.clone(), AtomicU64::new(1_700_000_000_000));
        let gate = Arc::new(Mutex::new(()));
        let held = gate.clone();
        let screen = Arc::new(Mutex::new(Screen::default()));
        let (on, looked) = (screen.clone(), screen.clone());
        let deps = Deps {
            voice: voice.clone(),
            grab: Box::new(move || {
                g.fetch_add(1, Ordering::SeqCst);
                let frame: Box<dyn Taken> = Box::new(FakeFrame(on.lock().unwrap().clone(), held.clone()));
                Ok(frame)
            }),
            look: Box::new(move || looked.lock().unwrap().look),
            provider,
            out: Box::new(move |out| o.lock().unwrap().push(out)),
            // Every reading is a millisecond after the last, so the order of the stages is the order of their times.
            now: Box::new(move || clock.fetch_add(1, Ordering::SeqCst)),
            tidy: Box::new(|| false),
        };
        Rig { live: Live::with_wait(deps, &dir, wait), voice, outs, grabs, gate, screen, dir }
    }

    impl Rig {
        fn until(&self, what: State) {
            let t = Instant::now();
            while self.live.snapshot().state != what {
                assert!(t.elapsed() < Duration::from_secs(10), "never {what:?}: {:?}", self.live.snapshot());
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        fn wait(&self, what: &str, ok: impl Fn() -> bool) {
            let t = Instant::now();
            while !ok() {
                assert!(t.elapsed() < Duration::from_secs(10), "never {what}: {:?}", self.live.snapshot());
                std::thread::sleep(Duration::from_millis(2));
            }
        }

        /// Press, say `words`, release, and wait for `end`.
        fn ask(&self, words: &str, end: State) {
            assert!(self.live.press(), "the press listens");
            *self.voice.say.lock().unwrap() = words.into();
            self.live.release();
            self.until(end);
        }

        /// The states the panel was told of, in order, each once.
        fn states(&self) -> Vec<State> {
            let mut v: Vec<State> = self.outs.lock().unwrap().iter().filter_map(|o| if let Out::State(s) = o { Some(s.state) } else { None }).collect();
            v.dedup();
            v
        }

        fn deltas(&self) -> Vec<(u64, String)> {
            self.outs.lock().unwrap().iter().filter_map(|o| if let Out::Delta { turn, text } = o { Some((*turn, text.clone())) } else { None }).collect()
        }

        fn kept(&self) -> Vec<(crate::sessions::Session, Vec<crate::sessions::Kept>)> {
            let s = Sessions::new(&self.dir);
            s.list().iter().map(|x| s.read(&x.id).unwrap()).collect()
        }

        fn end(self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn scripted(steps: &[(u64, &str)]) -> (Scripted, Option<Arc<dyn Provider>>) {
        let s = Scripted::new(steps);
        (s.clone(), Some(Arc::new(s)))
    }

    #[test]
    fn each_state_is_reached_by_one_step_and_a_question_walks_them_in_order() {
        use State::*;
        let states = [Closed, Listening, Finishing, Sending, Answering, Shown, Error, Discarded];
        let steps = [Step::Press, Step::Release, Step::Send, Step::First, Step::Settle, Step::Discard, Step::Fail, Step::Close];
        for to in states {
            let ways: Vec<Step> = steps.iter().copied().filter(|s| states.iter().any(|from| next(*from, *s) == Some(to))).collect();
            assert_eq!(ways.len(), 1, "{to:?} is entered by {ways:?}");
        }
        let walk = |from: State, path: &[Step]| path.iter().try_fold(from, |s, step| next(s, *step));
        assert_eq!(walk(Closed, &[Step::Press, Step::Release, Step::Send, Step::First, Step::Settle]), Some(Shown));
        assert_eq!(walk(Shown, &[Step::Press, Step::Release, Step::Send, Step::Fail, Step::Press, Step::Close]), Some(Closed));
        // A step out of place is refused: no answer without a question, no second release, no press while one runs.
        assert_eq!((next(Closed, Step::Release), next(Finishing, Step::Release), next(Listening, Step::Send)), (None, None, None));
        assert_eq!((next(Closed, Step::First), next(Shown, Step::Settle), next(Listening, Step::Fail)), (None, None, None));
        assert_eq!([Listening, Finishing, Sending, Answering].map(|s| next(s, Step::Press)), [None; 4]);
        // Only a question still being said or finished is discarded; after a discard the next press listens.
        assert_eq!([Listening, Finishing].map(|s| next(s, Step::Discard)), [Some(Discarded); 2]);
        assert_eq!([Closed, Sending, Answering, Shown, Error, Discarded].map(|s| next(s, Step::Discard)), [None; 6]);
        assert_eq!((next(Discarded, Step::Press), next(Discarded, Step::Close)), (Some(Listening), Some(Closed)));
        // Lower-case on the wire, as the panel reads it.
        assert_eq!(serde_json::to_value(Discarded).unwrap(), "discarded");
    }

    #[test]
    fn a_question_is_answered_in_the_panel_and_kept_with_its_picture_what_was_sent_and_six_times() {
        let script = Scripted { id: assist::codex::ID, ..Scripted::new(&[(5, "Hel"), (5, "lo")]) };
        let r = rig("asked", Some(Arc::new(script.clone())), FIRST_WAIT);
        r.ask("what is this", State::Shown);
        // A frame at the press and one at the release, to see whether the screen moved on; it did not, so the press frame
        // went (rule changed: the release frame is new).
        assert_eq!(r.grabs.load(Ordering::SeqCst), 2, "a frame at the press and one at the release");
        assert_eq!(r.outs.lock().unwrap().iter().filter(|o| **o == Out::Show).count(), 1);
        assert_eq!(r.states(), [State::Listening, State::Finishing, State::Sending, State::Answering, State::Shown]);
        assert_eq!(r.deltas().iter().map(|d| d.1.as_str()).collect::<String>(), "Hello");
        assert!(r.deltas().iter().all(|d| d.0 == 1), "{:?}", r.deltas());
        let kept = r.kept();
        let (session, turns) = &kept[0];
        assert_eq!((kept.len(), turns.len(), session.turns), (1, 1, 1));
        assert!(session.title.contains("what is this") && r.live.snapshot().session.title == session.title, "{session:?}");
        let snap = r.live.snapshot();
        // The panel names the AI as the person knows it; what is kept names the provider.
        assert_eq!((snap.session.id.as_str(), snap.turn, snap.words.as_str(), snap.provider.as_str()), (session.id.as_str(), 1, "what is this", "ChatGPT"));
        assert_eq!(turns[0].answer.as_ref().unwrap().provider, assist::codex::ID);
        assert!(snap.picture.as_ref().is_some_and(|p| p.src.starts_with("data:image/jpeg;base64,") && (p.w, p.h) == (4, 2)), "{snap:?}");
        let a = turns[0].answer.clone().unwrap();
        assert_eq!((a.text.as_str(), a.error.clone(), a.stopped), ("Hello", None, false));
        let t = a.times;
        assert!(0 < t.press_ms && t.press_ms < t.release_ms && t.release_ms < t.words_ms, "{t:?}");
        assert!(t.words_ms < t.sent_ms && t.sent_ms < t.first_ms && t.first_ms < t.done_ms, "{t:?}");
        // What was sent is what the provider got: the words, one caption line, and the picture as kept.
        assert_eq!(a.sent.text, "what is this\n\n[Picture: 4 x 2 pixels, the window \"Notes\", 1 area hidden]");
        let got = script.asked.lock().unwrap().clone();
        assert_eq!(got[0].parts[0], Part::Text(a.sent.text.clone()));
        let Part::Picture { path, w, h, mime } = &got[0].parts[1] else { panic!("no picture sent: {got:?}") };
        assert_eq!((std::fs::read(path).unwrap(), *w, *h, *mime), (shot().wire, 4, 2, Mime::Png));
        assert_eq!((a.sent.pictures.len(), script.opened.lock().unwrap().len()), (1, 1));
        // A second question while the panel is open is the next turn of the same session and conversation.
        r.ask("and this", State::Shown);
        let kept = r.kept();
        assert_eq!((kept.len(), kept[0].1.len(), script.opened.lock().unwrap().len(), r.live.snapshot().turn), (1, 2, 1, 2));
        r.end();
    }

    #[test]
    fn stopping_while_listening_discards_the_question_and_nothing_is_kept_or_sent() {
        let (script, provider) = scripted(&[(0, "never")]);
        let r = rig("discard", provider, FIRST_WAIT);
        assert!(r.live.press());
        *r.voice.say.lock().unwrap() = "what is".into();
        r.wait("the words and the picture in the panel", || {
            let s = r.live.snapshot();
            s.picture.is_some() && s.words == "what is"
        });
        r.live.stop();
        // Rule changed: a discard was `shown`, which the panel drew as an answer sent; it is its own state now.
        let snap = r.live.snapshot();
        assert_eq!(snap.state, State::Discarded);
        assert_eq!(*r.voice.stops.lock().unwrap(), [true], "the recording was discarded, not finished");
        std::thread::sleep(Duration::from_millis(150));
        assert!(r.kept().is_empty() && script.asked.lock().unwrap().is_empty() && script.opened.lock().unwrap().is_empty());
        assert!(r.live.inner.lock().unwrap().picture.is_none() && r.live.snapshot().picture.is_none());
        // Nothing in what the panel draws from can say the question went: no words, picture, AI or turn.
        let s = r.live.snapshot();
        assert_eq!((s.words.as_str(), s.dropped, s.provider.as_str(), s.turn, s.thinking, s.error), ("", false, "", 0, false, None), "{s:?}");
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!((json["state"].as_str(), json["picture"].is_null()), (Some("discarded"), true));
        // The same for a question stopped while its last words were finished.
        let (script, provider) = scripted(&[(0, "never")]);
        let r2 = rig("discard-late", provider, FIRST_WAIT);
        let slow = r2.gate.lock().unwrap();
        assert!(r2.live.press());
        *r2.voice.say.lock().unwrap() = "what is this".into();
        r2.live.release();
        r2.wait("the question waiting for its picture", || r2.live.inner.lock().unwrap().picture.is_none());
        r2.live.stop();
        drop(slow);
        std::thread::sleep(Duration::from_millis(150));
        let s = r2.live.snapshot();
        assert_eq!((s.state, s.words.as_str(), s.picture.is_none(), s.provider.as_str()), (State::Discarded, "", true, ""), "{s:?}");
        assert!(r2.kept().is_empty() && script.asked.lock().unwrap().is_empty());
        r.end();
        r2.end();
    }

    #[test]
    fn a_stray_piece_before_the_question_never_shows_or_goes_and_a_question_of_only_strays_is_nothing_said() {
        let speech = |texts: &[&str]| -> Vec<Item> {
            texts.iter().enumerate().map(|(n, t)| Item::Speech { t_ms: n as u64 * 1_000, end_ms: n as u64 * 1_000 + 900, text: t.to_string() }).collect()
        };
        let text = |items: Vec<Item>| prompt::compose(&items).0;
        // The model's word for the quiet before the person spoke goes, however many pieces of it there are.
        assert_eq!(text(unstrayed(speech(&["you", "Thank you.", "What is this graph?"]))), "What is this graph?");
        // A stray after the first real words is the person's own: it stays, as do words that only begin like one.
        assert_eq!(text(unstrayed(speech(&["What is this?", "Okay."]))), "What is this? Okay.");
        assert_eq!(text(unstrayed(speech(&["So what is this?"]))), "So what is this?");
        // Nothing but strays is nothing said.
        assert!(unstrayed(speech(&["you", "Um."])).is_empty());
        for s in stt::STRAY {
            assert!(unstrayed(speech(&[*s])).is_empty(), "{s}");
        }
        // Live: neither a heard piece nor the piece being heard starts the words with one.
        assert_eq!(said_so_far(speech(&[]), "you"), "");
        assert_eq!(said_so_far(speech(&["You."]), "what is"), "what is");
        assert_eq!(said_so_far(speech(&["What is"]), "you"), "What is you");
        // Through the loop: the panel never shows the stray, and the question kept and sent has none.
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("stray", provider, FIRST_WAIT);
        assert!(r.live.press());
        *r.voice.pieces.lock().unwrap() = vec!["you".into(), "What is this?".into()];
        r.wait("the words in the panel", || !r.live.snapshot().words.is_empty());
        assert_eq!(r.live.snapshot().words, "What is this?");
        r.live.release();
        r.until(State::Shown);
        let kept = r.kept();
        let a = kept[0].1[0].answer.clone().unwrap();
        assert!(a.sent.text.starts_with("What is this?\n\n[Picture"), "{}", a.sent.text);
        assert_eq!(script.asked.lock().unwrap()[0].parts[0], Part::Text(a.sent.text.clone()));
        assert!(r.outs.lock().unwrap().iter().all(|o| !matches!(o, Out::State(s) if s.words.to_lowercase().starts_with("you"))));
        // A question of only strays keeps and sends nothing: the panel is simply open.
        assert!(r.live.press());
        *r.voice.pieces.lock().unwrap() = vec!["you".into(), "Thank you.".into()];
        r.live.release();
        r.wait("the second question settled", || r.live.snapshot().state == State::Shown && r.live.inner.lock().unwrap().picture.is_none());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!((r.kept()[0].1.len(), script.asked.lock().unwrap().len(), r.live.snapshot().words.as_str()), (1, 1, ""));
        r.end();
    }

    #[test]
    fn the_picture_follows_the_window_the_person_looks_at_and_the_one_the_panel_shows_is_the_one_sent() {
        let (a, b) = (Look { front: Some(1), under: Some(1) }, Look { front: Some(2), under: Some(1) });
        // The decision, alone: another window in front or under the pointer takes the picture again; at the release a
        // screen that changed on its thumbnail (another page in the same window) does too, and a small change does not.
        assert!(!look_moved(&a, &a) && look_moved(&a, &b) && look_moved(&a, &Look { under: Some(3), ..a }));
        assert!(moved_on(&a, &b, 0.0) && moved_on(&a, &a, keyframes::CHANGE + 0.01));
        assert!(!moved_on(&a, &a, 0.0) && !moved_on(&a, &a, keyframes::CHANGE));
        // The panel's picture is named by the window it shows; a whole screen or an unnamed window is not.
        assert_eq!((label(&shot()), label(&Asked { window: None, ..shot() })), ("Notes".to_string(), String::new()));

        // Another app in front while the question is said: the panel's picture is taken again, and that one goes.
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("follows", provider, FIRST_WAIT);
        assert!(r.live.press());
        r.wait("the press picture", || r.live.snapshot().picture.is_some_and(|p| p.label == "Notes"));
        *r.screen.lock().unwrap() = Screen { look: b, window: "Lecture 4.pdf", shade: 30 };
        r.wait("the picture following the window in front", || r.live.snapshot().picture.is_some_and(|p| p.label == "Lecture 4.pdf"));
        *r.voice.say.lock().unwrap() = "what is this".into();
        r.live.release();
        r.until(State::Shown);
        let sent = &script.asked.lock().unwrap()[0];
        assert_eq!(sent.parts.len(), 2, "one picture, not two");
        let Part::Picture { path, .. } = &sent.parts[1] else { panic!("no picture sent") };
        assert_eq!(std::fs::read(path).unwrap(), shot_on(&r.screen.lock().unwrap()).wire);
        assert_eq!(sent.parts[0], Part::Text("what is this\n\n[Picture: 4 x 2 pixels, the window \"Lecture 4.pdf\", 1 area hidden]".into()));
        assert_eq!(r.live.snapshot().picture.map(|p| p.label), Some("Lecture 4.pdf".into()));

        // Another page in the same window between the last look and the release: the release frame goes, and the panel
        // shows it before it is sent.
        let (script, provider) = scripted(&[(0, "ok")]);
        let r2 = rig("moved-on", provider, FIRST_WAIT);
        assert!(r2.live.press());
        r2.wait("the press picture", || r2.live.snapshot().picture.is_some());
        *r2.screen.lock().unwrap() = Screen { window: "Notes, page 2", shade: 30, ..Screen::default() };
        *r2.voice.say.lock().unwrap() = "and this".into();
        r2.live.release();
        r2.until(State::Shown);
        assert!(matches!(&script.asked.lock().unwrap()[0].parts[0], Part::Text(t) if t.contains("Notes, page 2")));
        assert_eq!(r2.live.snapshot().picture.map(|p| p.label), Some("Notes, page 2".into()));
        // A picture taken off before the release is not taken again.
        let (script, provider) = scripted(&[(0, "ok")]);
        let r3 = rig("dropped-stays", provider, FIRST_WAIT);
        assert!(r3.live.press());
        r3.live.drop_picture();
        *r3.screen.lock().unwrap() = Screen { look: b, window: "Lecture 4.pdf", shade: 30 };
        std::thread::sleep(Duration::from_millis(700));
        *r3.voice.say.lock().unwrap() = "what is this".into();
        r3.live.release();
        r3.until(State::Shown);
        assert_eq!((r3.grabs.load(Ordering::SeqCst), script.asked.lock().unwrap()[0].parts.len()), (1, 1));
        r.end();
        r2.end();
        r3.end();
    }

    #[test]
    fn an_ask_with_nothing_said_keeps_and_sends_nothing_drops_its_frame_and_just_opens_the_panel() {
        let (script, provider) = scripted(&[(0, "never")]);
        let r = rig("empty", provider, FIRST_WAIT);
        assert!(r.live.press());
        // The picture is in the panel by the time the key comes up: it is there to be taken away.
        r.wait("the picture in the panel", || r.live.snapshot().picture.is_some());
        r.live.release();
        r.until(State::Shown);
        assert_eq!(r.states(), [State::Listening, State::Finishing, State::Shown]);
        assert!(r.outs.lock().unwrap().contains(&Out::Show), "the panel opened");
        assert!(r.kept().is_empty() && script.asked.lock().unwrap().is_empty() && script.opened.lock().unwrap().is_empty());
        assert!(r.live.inner.lock().unwrap().picture.is_none() && r.live.snapshot().picture.is_none(), "the frame is gone");
        r.end();
    }

    #[test]
    fn a_picture_dropped_before_release_is_neither_kept_nor_sent() {
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("dropped", provider, FIRST_WAIT);
        assert!(r.live.press());
        r.live.drop_picture();
        assert!(r.live.snapshot().dropped && r.live.snapshot().picture.is_none());
        *r.voice.say.lock().unwrap() = "what is this".into();
        r.live.release();
        r.until(State::Shown);
        let kept = r.kept();
        let a = kept[0].1[0].answer.clone().unwrap();
        assert_eq!((a.sent.text.as_str(), a.sent.pictures.len()), ("what is this", 0));
        assert!(Sessions::new(&r.dir).sent_picture(&kept[0].0.id, &kept[0].1[0].meta.id, 1).is_err(), "no picture file");
        assert_eq!(script.asked.lock().unwrap()[0].parts, [Part::Text("what is this".into())]);
        r.end();
    }

    #[test]
    fn a_picture_dropped_after_release_while_it_is_still_being_cut_is_neither_kept_nor_sent() {
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("dropped-late", provider, FIRST_WAIT);
        // The frame is slow to cut: the key comes up, the words are finished, and the question waits for its picture.
        let slow = r.gate.lock().unwrap();
        assert!(r.live.press());
        *r.voice.say.lock().unwrap() = "what is this".into();
        r.live.release();
        r.wait("the question waiting for its picture", || {
            let g = r.live.inner.lock().unwrap();
            g.snap.state == State::Finishing && g.picture.is_none()
        });
        r.live.drop_picture();
        assert!(r.live.snapshot().dropped);
        drop(slow);
        r.until(State::Shown);
        assert!(r.live.snapshot().dropped && r.live.snapshot().picture.is_none(), "{:?}", r.live.snapshot());
        let kept = r.kept();
        let a = kept[0].1[0].answer.clone().unwrap();
        assert_eq!((a.sent.text.as_str(), a.sent.pictures.len()), ("what is this", 0));
        assert!(Sessions::new(&r.dir).sent_picture(&kept[0].0.id, &kept[0].1[0].meta.id, 1).is_err(), "no picture file");
        assert_eq!(script.asked.lock().unwrap()[0].parts, [Part::Text("what is this".into())]);
        r.end();
    }

    #[test]
    fn a_provider_failure_keeps_the_question_with_the_error_line_and_the_next_question_starts_from_what_is_kept() {
        let (script, provider) = scripted(&[(1, "par")]);
        let script = script.failing(Failure::Offline);
        let provider = provider.map(|_| Arc::new(script.clone()) as Arc<dyn Provider>);
        let r = rig("failed", provider, FIRST_WAIT);
        r.ask("what is this", State::Error);
        let kept = r.kept();
        let turn = &kept[0].1[0];
        let a = turn.answer.clone().unwrap();
        assert_eq!((a.text.as_str(), a.error.clone()), ("par", Some(line(&Failure::Offline))));
        assert_eq!(a.sent.text.lines().next(), Some("what is this"), "the question is kept");
        assert_eq!(r.live.snapshot().error, Some(line(&Failure::Offline)));
        // The failed conversation is not asked again: the next question opens one from the turns on disk.
        r.ask("and this", State::Error);
        let opened = script.opened.lock().unwrap().clone();
        assert_eq!(opened.len(), 2);
        assert!(opened[0].1.is_empty());
        let Part::Text(context) = &opened[1].1[0].parts[0] else { panic!("no context: {opened:?}") };
        assert!(context.contains("what is this") && context.contains("par"), "{context}");
        assert_eq!(r.kept()[0].1[1].answer.clone().unwrap().sent.earlier, [turn.meta.id.clone()]);
        r.end();
    }

    /// Fails the answer of its first conversation as `first` does, then answers as `then` does.
    struct FailsOnce {
        first: Scripted,
        then: Scripted,
        opened: AtomicUsize,
    }

    impl Provider for FailsOnce {
        fn id(&self) -> &'static str {
            "scripted"
        }
        fn caps(&self, m: Option<&str>) -> Caps {
            self.then.caps(m)
        }
        fn health(&self) -> Health {
            self.then.health()
        }
        fn warm(&self) {}
        fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure> {
            let p = if self.opened.fetch_add(1, Ordering::SeqCst) == 0 { &self.first } else { &self.then };
            p.open(system, earlier)
        }
    }

    #[test]
    fn a_question_that_failed_is_asked_again_with_the_same_words_and_picture_as_the_next_turn() {
        let (first, then) = (Scripted::new(&[]).failing(Failure::Offline), Scripted::new(&[(1, "Four.")]));
        let r = rig("retry", Some(Arc::new(FailsOnce { first: first.clone(), then: then.clone(), opened: AtomicUsize::new(0) })), FIRST_WAIT);
        assert_eq!(r.live.retry(), Err(NOTHING_TO_RETRY.to_owned()), "nothing failed yet");
        r.ask("what is two plus two", State::Error);
        assert_eq!(r.live.snapshot().error, Some(line(&Failure::Offline)));
        let failed = r.kept()[0].1[0].answer.clone().unwrap();
        assert_eq!(failed.error, Some(line(&Failure::Offline)));
        assert_eq!(r.live.retry(), Ok(()));
        r.until(State::Shown);
        let snap = r.live.snapshot();
        assert_eq!((snap.turn, snap.error.clone(), snap.words.as_str()), (2, None, "what is two plus two"));
        assert_eq!(r.deltas(), [(2, "Four.".to_owned())], "the answer comes under the new turn number");
        // Still one question on disk, now answered, and what was sent is what was sent the first time.
        let kept = r.kept();
        assert_eq!((kept.len(), kept[0].1.len()), (1, 1));
        let a = kept[0].1[0].answer.clone().unwrap();
        assert_eq!((a.text.as_str(), a.error.clone(), &a.sent), ("Four.", None, &failed.sent));
        assert!(a.times.sent_ms > failed.times.sent_ms && a.times.press_ms == failed.times.press_ms, "{:?}", a.times);
        // The provider got the same words and the same picture bytes both times.
        let (one, two) = (first.asked.lock().unwrap().clone(), then.asked.lock().unwrap().clone());
        assert_eq!((one.len(), two.len()), (1, 1));
        assert_eq!(one[0], two[0]);
        let Part::Picture { path, .. } = &two[0].parts[1] else { panic!("no picture: {two:?}") };
        assert_eq!(std::fs::read(path).unwrap(), shot().wire);
        assert_eq!(r.live.retry(), Err(NOTHING_TO_RETRY.to_owned()), "answered: nothing is left to ask again");
        r.end();
        // With no AI chosen nothing was kept, so there is nothing to ask again either.
        let none = rig("retry-none", None, FIRST_WAIT);
        assert!(!none.live.press());
        assert_eq!(none.live.retry(), Err(NOTHING_TO_RETRY.to_owned()));
        assert_eq!((none.grabs.load(Ordering::SeqCst), none.kept().len()), (0, 0));
        none.end();
    }

    #[test]
    fn a_question_that_waited_too_long_is_asked_again_on_the_same_kept_turn() {
        let silent = Canned { events: vec![], hold: true, held: Arc::default() };
        let r = rig("retry-silent", Some(Arc::new(silent)), Duration::from_millis(80));
        r.ask("anyone there", State::Error);
        assert_eq!(r.live.snapshot().error.as_deref(), Some(NO_ANSWER));
        assert_eq!(r.live.retry(), Ok(()));
        r.until(State::Error);
        r.wait("the second wait to end", || r.live.snapshot().turn == 2 && r.live.snapshot().error.as_deref() == Some(NO_ANSWER));
        assert_eq!((r.kept().len(), r.kept()[0].1.len()), (1, 1));
        r.end();
    }

    #[test]
    fn a_conversation_left_idle_is_closed_and_the_next_question_opens_one_from_what_is_kept() {
        let (script, provider) = scripted(&[(1, "Yes.")]);
        let r = rig("idle", provider, FIRST_WAIT);
        assert!(r.live.idle_since().is_none(), "nothing used yet");
        r.ask("is this a cell", State::Shown);
        let used = r.live.idle_since().expect("the helper was used");
        assert!(r.live.inner.lock().unwrap().chat.is_some());
        r.live.let_go();
        assert!(r.live.idle_since().is_none() && r.live.inner.lock().unwrap().chat.is_none());
        r.ask("and this", State::Shown);
        let opened = script.opened.lock().unwrap().clone();
        assert_eq!(opened.len(), 2, "a new conversation");
        assert!(!opened[1].1.is_empty(), "it starts from what was kept");
        assert!(r.live.idle_since().unwrap() > used);
        r.end();
    }

    #[test]
    fn an_ask_through_the_settings_makes_nothing_without_a_chosen_and_allowed_ai_and_takes_a_new_choice_at_once() {
        use crate::assist::catalog::{self, Held};
        use crate::settings::Settings;
        let settings = Arc::new(Mutex::new(Settings::default()));
        let made: Arc<Mutex<Vec<Scripted>>> = Arc::default();
        let m = made.clone();
        let held = Arc::new(Held::with(Box::new(move |p| {
            let s = Scripted { id: p.id, ..Scripted::new(&[(1, "Fine.")]) };
            m.lock().unwrap().push(s.clone());
            Ok(Arc::new(s) as Arc<dyn Provider>)
        })));
        let (s2, h2) = (settings.clone(), held.clone());
        let r = rig_with("chosen", Box::new(move || h2.provider(&s2.lock().unwrap())), FIRST_WAIT);
        let refused = |why: &str| {
            assert!(!r.live.press());
            let snap = r.live.snapshot();
            assert_eq!((snap.state, snap.error.as_deref()), (State::Error, Some(why)));
            assert_eq!((made.lock().unwrap().len(), r.grabs.load(Ordering::SeqCst), r.kept().len()), (0, 0, 0), "nothing made, grabbed or kept");
        };
        refused(NO_PROVIDER);
        settings.lock().unwrap().assist_provider = Some("ollama".into());
        settings.lock().unwrap().assist_model = Some("llava".into());
        refused("Allow Ollama on this computer in Settings first");
        catalog::choose(&mut settings.lock().unwrap(), "openai", Some("m".into()), true, 1).unwrap();
        settings.lock().unwrap().assist_offline_only = true;
        refused(catalog::OFFLINE_ONLY);
        // Allowed: asked through what was chosen, no restart between.
        catalog::choose(&mut settings.lock().unwrap(), "ollama", Some("llava".into()), true, 2).unwrap();
        r.ask("what is this", State::Shown);
        assert_eq!(r.live.snapshot().provider, "Ollama on this computer");
        let first = made.lock().unwrap().clone();
        assert_eq!((first.len(), first[0].asked.lock().unwrap().len()), (1, 1));
        catalog::choose(&mut settings.lock().unwrap(), "lmstudio", Some("q".into()), true, 3).unwrap();
        r.live.close();
        r.ask("and now", State::Shown);
        assert_eq!(r.live.snapshot().provider, "LM Studio on this computer");
        assert_eq!(made.lock().unwrap().len(), 2);
        // No key reaches the panel: it never holds one, so a planted one cannot show in what it is told.
        let told = format!("{:?}", r.outs.lock().unwrap());
        assert!(!told.contains("test-key-not-real") && told.contains("LM Studio"));
        r.end();
    }

    #[test]
    fn the_first_ending_ends_a_turn_and_words_for_another_turn_or_after_the_end_are_ignored() {
        let events = vec![
            (99, Kind::Delta("stray".into())),
            (1, Kind::Delta("ok".into())),
            (99, Kind::Failed(Failure::Offline)),
            (1, Kind::Done),
            (1, Kind::Delta(" late".into())),
            (1, Kind::Failed(Failure::Offline)),
        ];
        let r = rig("one-end", Some(Arc::new(Canned { events, hold: true, held: Arc::default() })), FIRST_WAIT);
        r.ask("what is this", State::Shown);
        std::thread::sleep(Duration::from_millis(100));
        let a = r.kept()[0].1[0].answer.clone().unwrap();
        assert_eq!((a.text.as_str(), a.error, a.stopped), ("ok", None, false));
        assert_eq!(r.deltas(), [(1, "ok".to_string())]);
        assert_eq!((r.live.snapshot().state, r.live.snapshot().error), (State::Shown, None));
        r.end();
    }

    #[test]
    fn no_first_event_in_time_is_an_error_and_the_question_is_kept() {
        let silent = Canned { events: vec![(99, Kind::Delta("not this turn".into()))], hold: true, held: Arc::default() };
        let r = rig("silent", Some(Arc::new(silent)), Duration::from_millis(80));
        r.ask("what is this", State::Error);
        assert_eq!(r.live.snapshot().error.as_deref(), Some(NO_ANSWER));
        let a = r.kept()[0].1[0].answer.clone().unwrap();
        assert_eq!((a.text.as_str(), a.error.as_deref(), a.times.first_ms), ("", Some(NO_ANSWER), 0));
        r.end();
    }

    #[test]
    fn many_small_pieces_reach_the_panel_in_order_in_a_few_batches() {
        let steps: Vec<(u64, &str)> = (0..60).map(|_| (0, "ab ")).collect();
        let (_, provider) = scripted(&steps);
        let r = rig("batches", provider, FIRST_WAIT);
        r.ask("what is this", State::Shown);
        let d = r.deltas();
        assert_eq!(d.iter().map(|x| x.1.as_str()).collect::<String>(), "ab ".repeat(60));
        assert!(d.len() <= 10, "60 pieces sent at once came as {} events", d.len());
        r.end();
    }

    #[test]
    fn with_no_ai_chosen_an_ask_ends_in_an_error_without_grabbing_keeping_or_sending_anything() {
        let r = rig("none", None, FIRST_WAIT);
        assert!(!r.live.press());
        let snap = r.live.snapshot();
        assert_eq!((snap.state, snap.error.as_deref(), snap.provider.as_str()), (State::Error, Some(NO_PROVIDER), ""));
        assert_eq!(NO_PROVIDER, "Choose an AI in Settings first");
        assert_eq!(r.grabs.load(Ordering::SeqCst), 0, "the screen was not grabbed");
        assert!(!r.voice.busy() && r.voice.stops.lock().unwrap().is_empty(), "the microphone never opened");
        assert!(r.kept().is_empty() && r.outs.lock().unwrap().contains(&Out::Show));
        r.end();
    }

    #[test]
    fn the_panel_names_the_ai_as_the_person_knows_it_and_never_shows_a_provider_id() {
        assert_eq!(name_of(assist::codex::ID), "ChatGPT");
        // A provider with no name for the person shows nothing in every state the panel is told of, never its id.
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("unnamed", provider, FIRST_WAIT);
        r.ask("what is this", State::Shown);
        let told: Vec<String> = r.outs.lock().unwrap().iter().filter_map(|o| if let Out::State(s) = o { Some(s.provider.clone()) } else { None }).collect();
        assert!(told.len() >= 5 && told.iter().all(String::is_empty), "{told:?}");
        assert_eq!(r.kept()[0].1[0].answer.clone().unwrap().provider, script.id);
        r.end();
    }

    #[test]
    fn a_dictation_under_way_never_opens_the_panel_or_touches_a_session() {
        let (script, provider) = scripted(&[(0, "never")]);
        let r = rig("dictating", provider, FIRST_WAIT);
        // A dictation records.
        r.voice.on.store(true, Ordering::SeqCst);
        assert!(!r.live.press());
        r.live.release();
        r.live.stop();
        r.live.drop_picture();
        assert_eq!(r.live.snapshot(), Snapshot::default());
        assert!(r.outs.lock().unwrap().is_empty(), "the panel heard nothing and was not shown");
        assert_eq!(r.grabs.load(Ordering::SeqCst), 0);
        assert!(r.voice.busy() && r.voice.stops.lock().unwrap().is_empty(), "the dictation goes on");
        assert!(r.kept().is_empty() && script.opened.lock().unwrap().is_empty());
        r.end();
    }

    #[test]
    fn closing_the_panel_ends_its_session_and_the_next_ask_starts_another() {
        let (script, provider) = scripted(&[(0, "ok")]);
        let r = rig("closed", provider, FIRST_WAIT);
        r.ask("what is this", State::Shown);
        r.live.close();
        assert_eq!(r.live.snapshot(), Snapshot::default());
        assert_eq!(r.outs.lock().unwrap().last(), Some(&Out::Hide));
        r.ask("something else", State::Shown);
        assert_eq!((r.kept().len(), script.opened.lock().unwrap().len(), r.live.snapshot().turn), (2, 2, 1));
        r.end();
    }
}
