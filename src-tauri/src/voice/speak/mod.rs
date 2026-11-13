//! Answers read aloud. An Ask answer is cut into sentences as it streams in (`split`), and each one is said by a voice
//! of this computer as soon as it is complete, one after another (`Talk`): the Mac's AVSpeechSynthesizer (`mac`),
//! Windows' SpeechSynthesizer (`win`), or the natural voice Kokoro, downloaded once and run here (`natural`, `out`,
//! `voice::kokoro`); `Voices` picks between them. Nothing leaves the computer. Speech stops at once when the person
//! asks again, talks over it, presses Esc or closes the panel (`Reader`).

#[cfg(target_os = "macos")]
mod mac;
pub mod natural;
pub mod out;
pub mod split;
#[cfg(windows)]
mod win;

use crate::live::{Out, State};
use crate::voice::audio;
use std::collections::VecDeque;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

/// A voice installed on this computer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct VoiceInfo {
    /// What the system knows it by; what the settings keep.
    pub id: String,
    pub name: String,
    /// Its language ("en-US").
    pub lang: String,
    /// A Premium or Enhanced voice: one of the natural-sounding ones.
    pub natural: bool,
    /// Bytes to download before it can speak; none when it is on this computer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<u64>,
}

/// How a sentence is said.
#[derive(Debug, Clone, PartialEq)]
pub struct How {
    /// The voice by its id; none is the system's own choice for its language.
    pub voice: Option<String>,
    /// The speaking rate: 1 is normal, 0.5 half as fast, 2 twice as fast.
    pub rate: f32,
}

impl Default for How {
    fn default() -> Self {
        How { voice: None, rate: 1.0 }
    }
}

/// Called once, when a sentence has been said or cut short.
pub type Done = Box<dyn FnOnce() + Send>;

/// Called once, when a voice's download has ended: whether the voice can speak now.
pub type Fetched = Box<dyn FnOnce(bool) + Send>;

/// A voice of the system that says one sentence at a time.
pub trait Speaker: Send + Sync {
    /// Start saying `sentence` and return. `done` is called once, when it has been said or cut short; `Err` means it
    /// never started, and `done` is not called.
    fn speak(&self, sentence: &str, how: &How, done: Done) -> Result<(), String>;
    /// Cut the sentence being said, at once. With nothing being said, nothing happens.
    fn stop(&self);
    /// The voices installed, natural ones first; `Err` says why there are none here.
    fn voices(&self) -> Result<Vec<VoiceInfo>, String>;
    /// `sentence` will be said next: a voice that makes its audio first can start on it now.
    fn prepare(&self, _sentence: &str, _how: &How) {}
    /// Get `how`'s voice ready to speak soon. True when it is on this computer (it loads what it needs now). False
    /// when it has to be downloaded first; with `fetch`, the download starts (once) and `fetch` hears when it ends.
    fn ready(&self, _how: &How, _fetch: Option<Fetched>) -> bool {
        true
    }
}

/// The system's voices and, beside them, the natural voice (Kokoro). A Kokoro voice, or no choice once Kokoro is
/// downloaded, is said by Kokoro; a Kokoro voice not downloaded yet falls back to the system's voice meanwhile.
pub struct Voices {
    system: Arc<dyn Speaker>,
    natural: Option<natural::Natural>,
}

/// Who says a sentence.
enum Pick {
    System(How),
    Natural(String),
}

impl Voices {
    pub fn new(system: Arc<dyn Speaker>, natural: Option<natural::Natural>) -> Voices {
        Voices { system, natural }
    }

    fn pick(&self, how: &How) -> Pick {
        let Some(n) = &self.natural else { return Pick::System(how.clone()) };
        let synth = n.synth();
        match &how.voice {
            Some(v) if synth.owns(v) && synth.installed() => Pick::Natural(v.clone()),
            Some(v) if synth.owns(v) => Pick::System(How { voice: None, rate: how.rate }),
            None if synth.installed() => Pick::Natural(synth.default_voice()),
            _ => Pick::System(how.clone()),
        }
    }
}

impl Speaker for Voices {
    fn speak(&self, sentence: &str, how: &How, done: Done) -> Result<(), String> {
        match (self.pick(how), &self.natural) {
            (Pick::Natural(v), Some(n)) => {
                n.speak(sentence, &v, how.rate, done);
                Ok(())
            }
            (Pick::System(h), _) => self.system.speak(sentence, &h, done),
            (Pick::Natural(_), None) => self.system.speak(sentence, how, done),
        }
    }

    fn stop(&self) {
        if let Some(n) = &self.natural {
            n.stop();
        }
        self.system.stop();
    }

    /// The natural voices first, then the system's. Without a system voice (Linux), its reason.
    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        let system = self.system.voices()?;
        let mut all = self.natural.as_ref().map(|n| n.synth().voices()).unwrap_or_default();
        all.extend(system);
        Ok(all)
    }

    fn prepare(&self, sentence: &str, how: &How) {
        match (self.pick(how), &self.natural) {
            (Pick::Natural(v), Some(n)) => n.prepare(sentence, &v, how.rate),
            (Pick::System(h), _) => self.system.prepare(sentence, &h),
            _ => {}
        }
    }

    fn ready(&self, how: &How, fetch: Option<Fetched>) -> bool {
        let Some(n) = &self.natural else { return self.system.ready(how, fetch) };
        let synth = n.synth();
        match &how.voice {
            Some(v) if synth.owns(v) && !synth.installed() => {
                if let Some(f) = fetch {
                    synth.fetch(f);
                }
                false
            }
            _ => match self.pick(how) {
                Pick::Natural(v) => {
                    n.warm(&v);
                    true
                }
                Pick::System(h) => self.system.ready(&h, fetch),
            },
        }
    }
}

/// This computer's own voice.
pub fn system() -> Arc<dyn Speaker> {
    #[cfg(target_os = "macos")]
    let voice: Arc<dyn Speaker> = Arc::new(mac::Mac::default());
    #[cfg(windows)]
    let voice: Arc<dyn Speaker> = Arc::new(win::Win::default());
    #[cfg(not(any(target_os = "macos", windows)))]
    let voice: Arc<dyn Speaker> = Arc::new(Unsupported);
    voice
}

#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
pub const UNSUPPORTED: &str = "Reading answers aloud is not supported on Linux yet.";

/// Linux has no voice here yet.
#[cfg_attr(any(target_os = "macos", windows), allow(dead_code))]
pub struct Unsupported;

impl Speaker for Unsupported {
    fn speak(&self, _: &str, _: &How, _: Done) -> Result<(), String> {
        Err(UNSUPPORTED.into())
    }
    fn stop(&self) {}
    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        Err(UNSUPPORTED.into())
    }
}

/// Natural voices first, then those in the system's language `here`; within those, the Mac's older robotic voices and
/// its novelty ones (Bells, Bubbles, Zarvox) last, then the system's own region first, then by name.
pub fn ranked(mut voices: Vec<VoiceInfo>, here: &str) -> Vec<VoiceInfo> {
    let base = |l: &str| l.split(['-', '_']).next().unwrap_or("").to_lowercase();
    let mine = base(here);
    let odd = |id: &str| match id {
        _ if id.starts_with("com.apple.speech.synthesis.voice.") => 2,
        _ if id.starts_with("com.apple.eloquence.") => 1,
        _ => 0,
    };
    let key = |v: &VoiceInfo| (!v.natural, base(&v.lang) != mine, odd(&v.id), !v.lang.eq_ignore_ascii_case(here), v.name.to_lowercase());
    voices.sort_by_cached_key(key);
    voices
}

/// After a stop, how long a cut sentence may take to say it ended before the next one starts anyway.
const GRACE: Duration = Duration::from_millis(400);
/// The longest one sentence may take: a voice that never says it is done does not hold the queue.
const LONGEST: Duration = Duration::from_secs(90);

/// Sentences waiting for a speaker, said one after another by a thread of their own. `stop` clears the queue and cuts
/// the sentence being said at once.
pub struct Talk {
    shared: Arc<Shared>,
}

struct Shared {
    speaker: Arc<dyn Speaker>,
    queue: Mutex<Queue>,
    moved: Condvar,
    told: Box<dyn Fn(bool) + Send + Sync>,
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<(String, How)>,
    /// Counts stops: a sentence taken before a stop and started after it is cut.
    stops: u64,
    stopped_at: Option<Instant>,
    speaking: bool,
    /// Something is being said or waits, as last told.
    busy: bool,
    closed: bool,
}

impl Talk {
    /// `told` hears when speech starts (true) and when all of it has ended or stopped (false).
    pub fn new(speaker: Arc<dyn Speaker>, told: impl Fn(bool) + Send + Sync + 'static) -> Talk {
        let shared = Arc::new(Shared { speaker, queue: Mutex::default(), moved: Condvar::new(), told: Box::new(told) });
        let worker = shared.clone();
        std::thread::Builder::new().name("speak".into()).spawn(move || worker.run()).expect("the speech thread starts");
        Talk { shared }
    }

    /// Say `sentence` after those already waiting.
    pub fn say(&self, sentence: String, how: How) {
        self.shared.say(sentence, how);
    }

    /// A handle that says sentences while this queue lives, for a sentence said later (after a download).
    fn sayer(&self) -> Weak<Shared> {
        Arc::downgrade(&self.shared)
    }

    fn speaker(&self) -> &Arc<dyn Speaker> {
        &self.shared.speaker
    }

    /// Clear the queue and cut the sentence being said, at once.
    pub fn stop(&self) {
        {
            let mut q = self.shared.queue.lock().unwrap();
            q.stops += 1;
            q.stopped_at = Some(Instant::now());
            q.waiting.clear();
            self.shared.tell(&mut q);
            self.shared.moved.notify_all();
        }
        self.shared.speaker.stop();
    }

    /// Something is being said or waits to be.
    pub fn busy(&self) -> bool {
        self.shared.queue.lock().unwrap().busy
    }

    /// Wait until nothing is being said or waits; false when that took longer than `limit`.
    #[cfg(test)]
    pub fn wait_idle(&self, limit: Duration) -> bool {
        let q = self.shared.queue.lock().unwrap();
        let (q, _) = self.shared.moved.wait_timeout_while(q, limit, |q| q.busy).unwrap();
        !q.busy
    }

    pub fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        self.shared.speaker.voices()
    }
}

impl Drop for Talk {
    fn drop(&mut self) {
        let mut q = self.shared.queue.lock().unwrap();
        q.closed = true;
        q.waiting.clear();
        self.shared.moved.notify_all();
        drop(q);
        self.shared.speaker.stop();
    }
}

impl Shared {
    fn say(&self, sentence: String, how: How) {
        let next = {
            let mut q = self.queue.lock().unwrap();
            q.waiting.push_back((sentence, how));
            self.tell(&mut q);
            self.moved.notify_all();
            // The one after the sentence being said: the voice can make it now.
            (q.speaking && q.waiting.len() == 1).then(|| q.waiting[0].clone())
        };
        if let Some((s, h)) = next {
            self.speaker.prepare(&s, &h);
        }
    }

    /// Tell whether anything is being said or waits, when that changed.
    fn tell(&self, q: &mut Queue) {
        let busy = q.speaking || !q.waiting.is_empty();
        if busy != q.busy {
            q.busy = busy;
            (self.told)(busy);
        }
    }

    fn run(&self) {
        let (tx, rx) = mpsc::channel::<u64>();
        let mut n = 0u64;
        loop {
            let (sentence, how, stops, next) = {
                let mut q = self.queue.lock().unwrap();
                loop {
                    if q.closed {
                        return;
                    }
                    if let Some((s, h)) = q.waiting.pop_front() {
                        q.speaking = true;
                        break (s, h, q.stops, q.waiting.front().cloned());
                    }
                    q = self.moved.wait(q).unwrap();
                }
            };
            n += 1;
            let (said, id) = (tx.clone(), n);
            match self.speaker.speak(&sentence, &how, Box::new(move || {
                let _ = said.send(id);
            })) {
                Ok(()) => {
                    if let Some((s, h)) = next {
                        self.speaker.prepare(&s, &h);
                    }
                    self.wait(&rx, id, stops)
                }
                Err(why) => eprintln!("[speak] {why}"),
            }
            let mut q = self.queue.lock().unwrap();
            q.speaking = false;
            self.tell(&mut q);
            self.moved.notify_all();
        }
    }

    /// Wait for sentence `id` to end. A stop that came while it started cuts it now, and one it does not answer is
    /// given up on after `GRACE`.
    fn wait(&self, rx: &mpsc::Receiver<u64>, id: u64, stops: u64) {
        if self.queue.lock().unwrap().stops != stops {
            self.speaker.stop();
        }
        let from = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(k) if k == id => return,
                // A sentence given up on before ended late.
                Ok(_) => {}
                Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {
                    let q = self.queue.lock().unwrap();
                    let cut = q.stops != stops && q.stopped_at.is_some_and(|t| t.elapsed() >= GRACE);
                    if q.closed || cut {
                        return;
                    }
                    drop(q);
                    if from.elapsed() >= LONGEST {
                        self.speaker.stop();
                        return;
                    }
                }
            }
        }
    }
}

/// The line "Hear it" in Settings says.
pub const SAMPLE: &str = "Hi, this is how your answers will sound.";

/// The microphone level that counts as the person talking: an rms of 0.01 (about -40 dBFS), well above a quiet room
/// and below speech near the microphone...
pub const BARGE_RMS: f32 = 0.01;
/// ...for this many checks in a row (about 100 ms at 30 a second), so a click or a bump does not count.
pub const BARGE_CHECKS: u32 = 3;

/// Hears the person start talking over an answer, from the microphone's level (`audio::meter`) while it records.
#[derive(Debug, Default)]
pub struct BargeIn {
    over: u32,
}

impl BargeIn {
    /// Hear one level; true once, when it has been over `BARGE_RMS` for `BARGE_CHECKS` checks in a row.
    pub fn hear(&mut self, level: f32) -> bool {
        if level >= audio::meter(BARGE_RMS) {
            self.over += 1;
        } else {
            self.over = 0;
        }
        self.over == BARGE_CHECKS
    }
}

/// Reads Ask's answers aloud as they stream in, when the settings say to, and never the model's thinking.
pub struct Reader {
    talk: Talk,
    /// How answers are said; none when reading aloud is off. Read at each new answer.
    how: Box<dyn Fn() -> Option<How> + Send + Sync>,
    now: Mutex<Answer>,
    /// Counts stops: a sample waiting for its voice's download is said only if nothing stopped it since.
    stops: Arc<AtomicU64>,
}

#[derive(Default)]
struct Answer {
    /// The answer being read, by its turn, and how; both none when nothing more of it is read.
    turn: Option<u64>,
    how: Option<How>,
    split: split::Splitter,
    /// The panel's state last heard: each step is acted on once, when the panel moves to it.
    last: State,
    barge: BargeIn,
}

impl Reader {
    pub fn new(
        speaker: Arc<dyn Speaker>,
        how: impl Fn() -> Option<How> + Send + Sync + 'static,
        told: impl Fn(bool) + Send + Sync + 'static,
    ) -> Reader {
        Reader { talk: Talk::new(speaker, told), how: Box::new(how), now: Mutex::default(), stops: Arc::default() }
    }

    /// What the Ask loop tells the panel. A question sent starts a new answer and stops the old one; its words are
    /// read as they come; its end reads the rest. A new question being said, a discarded one, or the panel closing
    /// stops speech.
    pub fn hear(&self, out: &Out) {
        match out {
            Out::State(s) => {
                let was = std::mem::replace(&mut self.now.lock().unwrap().last, s.state);
                if was == s.state {
                    return;
                }
                match s.state {
                    State::Sending => self.begin(s.turn),
                    State::Shown | State::Error => self.end(),
                    State::Listening | State::Discarded | State::Closed => self.stop(),
                    State::Finishing | State::Answering => {}
                }
            }
            Out::Delta { turn, text } => self.delta(*turn, text),
            Out::Hide => self.stop(),
            Out::Show | Out::MarkOver => {}
        }
    }

    fn begin(&self, turn: u64) {
        self.talk.stop();
        let how = (self.how)();
        // The answer is on its way: a natural voice loads meanwhile.
        if let Some(h) = &how {
            self.talk.speaker().ready(h, None);
        }
        let mut a = self.now.lock().unwrap();
        (a.turn, a.how, a.split) = (Some(turn), how, split::Splitter::default());
    }

    fn delta(&self, turn: u64, text: &str) {
        let mut a = self.now.lock().unwrap();
        if a.turn != Some(turn) {
            return;
        }
        let Some(how) = a.how.clone() else { return };
        for s in a.split.push(text) {
            self.talk.say(s, how.clone());
        }
    }

    fn end(&self) {
        let mut a = self.now.lock().unwrap();
        a.turn = None;
        if let Some(how) = a.how.take() {
            for s in a.split.finish() {
                self.talk.say(s, how.clone());
            }
        }
    }

    /// Stop speaking at once; nothing more of this answer is read.
    pub fn stop(&self) {
        {
            let mut a = self.now.lock().unwrap();
            (a.turn, a.how, a.split) = (None, None, split::Splitter::default());
        }
        self.stops.fetch_add(1, Ordering::SeqCst);
        self.talk.stop();
    }

    /// The microphone's level while something records (`audio::meter`): the person talking over the answer stops it.
    pub fn level(&self, level: f32) {
        let heard = self.now.lock().unwrap().barge.hear(level);
        if heard && self.talk.busy() {
            self.stop();
        }
    }

    /// Something is being said or waits to be.
    pub fn speaking(&self) -> bool {
        self.talk.busy()
    }

    /// Say the sample line the way `how` says, in place of anything being read. A voice that has to be downloaded
    /// first is, and says it once it is (unless something stopped speech meanwhile).
    pub fn sample(&self, how: How) {
        self.stop();
        let (stops, at, talk, later) = (self.stops.clone(), self.stops.load(Ordering::SeqCst), self.talk.sayer(), how.clone());
        let then: Fetched = Box::new(move |ok| {
            if let Some(talk) = talk.upgrade().filter(|_| ok && stops.load(Ordering::SeqCst) == at) {
                talk.say(SAMPLE.into(), later);
            }
        });
        if self.talk.speaker().ready(&how, Some(then)) {
            self.talk.say(SAMPLE.into(), how);
        }
    }

    /// The person chose `how`'s voice: one to download starts downloading, one on this computer loads.
    pub fn ready(&self, how: &How) -> bool {
        self.talk.speaker().ready(how, Some(Box::new(|_| {})))
    }

    pub fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        self.talk.voices()
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;

    /// A speaker for tests. It notes each sentence it starts and each one cut, and a sentence ends only when the test
    /// says so (`finish`) or it is cut; with `auto`, each one ends as it starts.
    #[derive(Default)]
    pub struct Fake {
        pub started: Mutex<Vec<String>>,
        pub cut: Mutex<Vec<String>>,
        now: Mutex<Option<(String, Done)>>,
        pub auto: bool,
        /// How long starting a sentence takes.
        pub delay: Duration,
    }

    impl Speaker for Fake {
        fn speak(&self, sentence: &str, _: &How, done: Done) -> Result<(), String> {
            self.started.lock().unwrap().push(sentence.to_string());
            std::thread::sleep(self.delay);
            if self.auto {
                done();
            } else {
                *self.now.lock().unwrap() = Some((sentence.to_string(), done));
            }
            Ok(())
        }
        fn stop(&self) {
            if let Some((s, done)) = self.now.lock().unwrap().take() {
                self.cut.lock().unwrap().push(s);
                done();
            }
        }
        fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
            Ok(vec![])
        }
    }

    impl Fake {
        /// Each sentence ends as it starts.
        pub fn auto() -> Fake {
            Fake { auto: true, ..Fake::default() }
        }
        /// Starting each sentence takes `delay`.
        pub fn slow(delay: Duration) -> Fake {
            Fake { delay, ..Fake::default() }
        }
        /// End the sentence being said, as if it was said to the end.
        pub fn finish(&self) -> bool {
            let now = self.now.lock().unwrap().take();
            now.map(|(_, done)| done()).is_some()
        }
        /// Wait until `n` sentences have started; false after 2 s.
        pub fn wait_started(&self, n: usize) -> bool {
            let from = Instant::now();
            while from.elapsed() < Duration::from_secs(2) {
                if self.started.lock().unwrap().len() >= n {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            false
        }
        pub fn started(&self) -> Vec<String> {
            self.started.lock().unwrap().clone()
        }
        pub fn cut(&self) -> Vec<String> {
            self.cut.lock().unwrap().clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::Fake;
    use super::*;
    use crate::live::Snapshot;

    const A_WHILE: Duration = Duration::from_millis(80);

    fn state(state: State, turn: u64) -> Out {
        Out::State(Snapshot { state, turn, ..Snapshot::default() })
    }

    fn delta(turn: u64, text: &str) -> Out {
        Out::Delta { turn, text: text.into() }
    }

    fn reader(fake: &Arc<Fake>, on: bool) -> Reader {
        Reader::new(fake.clone(), move || on.then(How::default), |_| {})
    }

    #[test]
    fn sentences_are_said_one_after_another_and_busy_is_told_at_the_start_and_the_end() {
        let fake = Arc::new(Fake::default());
        let told = Arc::new(Mutex::new(Vec::new()));
        let t = told.clone();
        let talk = Talk::new(fake.clone(), move |b| t.lock().unwrap().push(b));
        for s in ["One.", "Two.", "Three."] {
            talk.say(s.into(), How::default());
        }
        assert!(fake.wait_started(1));
        std::thread::sleep(A_WHILE);
        assert_eq!(fake.started(), ["One."], "the next waits for the one before");
        assert!(talk.busy());
        for n in 2..=3 {
            assert!(fake.finish());
            assert!(fake.wait_started(n));
        }
        assert!(fake.finish());
        assert!(talk.wait_idle(Duration::from_secs(1)));
        assert_eq!(fake.started(), ["One.", "Two.", "Three."]);
        assert!(fake.cut().is_empty());
        assert_eq!(*told.lock().unwrap(), [true, false]);
    }

    #[test]
    fn barge_in_a_stop_mid_queue_cuts_the_sentence_at_once_and_speaks_nothing_more() {
        let fake = Arc::new(Fake::default());
        let talk = Talk::new(fake.clone(), |_| {});
        for s in ["One.", "Two.", "Three."] {
            talk.say(s.into(), How::default());
        }
        assert!(fake.wait_started(1));
        talk.stop();
        assert_eq!(fake.cut(), ["One."], "cut at once, not at its end");
        assert!(talk.wait_idle(Duration::from_secs(1)));
        std::thread::sleep(A_WHILE);
        assert_eq!(fake.started(), ["One."], "nothing queued is said after the stop");
        // Speech works again after a stop.
        talk.say("Four.".into(), How::default());
        assert!(fake.wait_started(2));
        assert_eq!(fake.started(), ["One.", "Four."]);
    }

    #[test]
    fn a_stop_while_a_sentence_is_starting_cuts_it_as_soon_as_it_starts() {
        let fake = Arc::new(Fake::slow(Duration::from_millis(150)));
        let talk = Talk::new(fake.clone(), |_| {});
        talk.say("One.".into(), How::default());
        talk.say("Two.".into(), How::default());
        assert!(fake.wait_started(1));
        talk.stop();
        assert!(talk.wait_idle(Duration::from_secs(1)));
        assert_eq!((fake.started(), fake.cut()), (vec!["One.".to_string()], vec!["One.".to_string()]));
    }

    #[test]
    fn a_voice_that_never_answers_a_stop_does_not_hold_the_queue() {
        struct Deaf(Mutex<Vec<String>>);
        impl Speaker for Deaf {
            fn speak(&self, s: &str, _: &How, done: Done) -> Result<(), String> {
                self.0.lock().unwrap().push(s.into());
                std::mem::forget(done);
                Ok(())
            }
            fn stop(&self) {}
            fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
                Ok(vec![])
            }
        }
        let deaf = Arc::new(Deaf(Mutex::default()));
        let talk = Talk::new(deaf.clone(), |_| {});
        talk.say("One.".into(), How::default());
        std::thread::sleep(A_WHILE);
        talk.stop();
        talk.say("Two.".into(), How::default());
        let from = Instant::now();
        while deaf.0.lock().unwrap().len() < 2 && from.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*deaf.0.lock().unwrap(), ["One.", "Two."]);
        assert!(from.elapsed() >= GRACE - A_WHILE, "the cut one had its grace");
    }

    #[test]
    fn a_streamed_answer_is_read_sentence_by_sentence_as_it_comes_and_the_rest_at_its_end() {
        let fake = Arc::new(Fake::auto());
        let r = reader(&fake, true);
        r.hear(&state(State::Sending, 1));
        r.hear(&state(State::Answering, 1));
        r.hear(&delta(1, "The answer is **4**. Here is"));
        assert!(fake.wait_started(1));
        assert_eq!(fake.started(), ["The answer is 4."], "read before the answer ends");
        r.hear(&delta(1, " why: `2 + 2`"));
        r.hear(&state(State::Shown, 1));
        assert!(fake.wait_started(2));
        assert!(r.talk.wait_idle(Duration::from_secs(1)));
        assert_eq!(fake.started(), ["The answer is 4.", "Here is why: 2 + 2"]);
    }

    #[test]
    fn a_new_ask_cancels_the_old_speech_and_late_words_of_the_old_answer_are_not_read() {
        let fake = Arc::new(Fake::default());
        let r = reader(&fake, true);
        r.hear(&state(State::Sending, 1));
        r.hear(&delta(1, "One. Two. Three."));
        assert!(fake.wait_started(1));
        // The Ask key goes down again: a new question is being said.
        r.hear(&state(State::Listening, 1));
        assert_eq!(fake.cut(), ["One."]);
        r.hear(&delta(1, " Four. Five."));
        assert!(r.talk.wait_idle(Duration::from_secs(1)));
        std::thread::sleep(A_WHILE);
        assert_eq!(fake.started(), ["One."]);
        // The new question's answer is read.
        r.hear(&state(State::Finishing, 1));
        r.hear(&state(State::Sending, 2));
        r.hear(&delta(2, "New answer."));
        r.hear(&state(State::Shown, 2));
        assert!(fake.wait_started(2));
        assert_eq!(fake.started(), ["One.", "New answer."]);
        // A typed follow-up sent while it is still read stops it too.
        r.hear(&state(State::Sending, 3));
        assert_eq!(fake.cut(), ["One.", "New answer."]);
    }

    #[test]
    fn thinking_text_is_never_spoken() {
        let fake = Arc::new(Fake::auto());
        let r = reader(&fake, true);
        r.hear(&state(State::Sending, 1));
        // The panel hears the model thinking (no words of it reach the panel), then the answer, which a model on this
        // computer may open with its thoughts between tags.
        r.hear(&Out::State(Snapshot { state: State::Sending, turn: 1, thinking: true, ..Snapshot::default() }));
        r.hear(&delta(1, "<thi"));
        r.hear(&delta(1, "nk>Let me think. The person wants a sum.</th"));
        r.hear(&delta(1, "ink>The sum is 4. Done."));
        r.hear(&state(State::Shown, 1));
        assert!(fake.wait_started(2));
        assert!(r.talk.wait_idle(Duration::from_secs(1)));
        assert_eq!(fake.started(), ["The sum is 4.", "Done."]);
    }

    #[test]
    fn esc_or_a_stop_reads_nothing_more_of_this_answer_but_the_next_one_is_read() {
        let fake = Arc::new(Fake::default());
        let r = reader(&fake, true);
        r.hear(&state(State::Sending, 1));
        r.hear(&delta(1, "One. Two."));
        assert!(fake.wait_started(1));
        r.stop();
        r.hear(&delta(1, " Three. Four."));
        r.hear(&state(State::Shown, 1));
        assert!(r.talk.wait_idle(Duration::from_secs(1)));
        assert_eq!((fake.started(), fake.cut()), (vec!["One.".to_string()], vec!["One.".to_string()]));
        r.hear(&state(State::Sending, 2));
        r.hear(&delta(2, "Next answer."));
        r.hear(&state(State::Shown, 2));
        assert!(fake.wait_started(2));
        assert_eq!(fake.started(), ["One.", "Next answer."]);
    }

    #[test]
    fn the_panel_closing_or_a_discarded_question_stops_speech() {
        for end in [Out::Hide, state(State::Closed, 0), state(State::Discarded, 1)] {
            let fake = Arc::new(Fake::default());
            let r = reader(&fake, true);
            r.hear(&state(State::Sending, 1));
            r.hear(&delta(1, "One. Two."));
            assert!(fake.wait_started(1));
            r.hear(&end);
            assert_eq!(fake.cut(), ["One."], "{end:?}");
            assert!(r.talk.wait_idle(Duration::from_secs(1)));
        }
    }

    #[test]
    fn the_person_talking_over_the_answer_stops_it_but_a_click_or_a_quiet_room_does_not() {
        let fake = Arc::new(Fake::default());
        let r = reader(&fake, true);
        r.hear(&state(State::Sending, 1));
        r.hear(&delta(1, "One. Two. Three."));
        assert!(fake.wait_started(1));
        let (room, voice) = (audio::meter(0.001), audio::meter(0.05));
        for level in [room; 20] {
            r.level(level);
        }
        r.level(voice);
        r.level(room);
        r.level(voice);
        r.level(voice);
        assert!(fake.cut().is_empty(), "a quiet room and short bumps leave it alone");
        r.level(voice);
        assert_eq!(fake.cut(), ["One."]);
        r.hear(&delta(1, " Four."));
        assert!(r.talk.wait_idle(Duration::from_secs(1)));
        assert_eq!(fake.started(), ["One."]);
    }

    #[test]
    fn with_reading_aloud_off_nothing_is_said() {
        let fake = Arc::new(Fake::auto());
        let r = reader(&fake, false);
        r.hear(&state(State::Sending, 1));
        r.hear(&delta(1, "One. Two."));
        r.hear(&state(State::Shown, 1));
        std::thread::sleep(A_WHILE);
        assert!(fake.started().is_empty());
        // Hear it still speaks, the way Settings asks.
        r.sample(How { voice: Some("x".into()), rate: 1.5 });
        assert!(fake.wait_started(1));
        assert_eq!(fake.started(), [SAMPLE]);
    }

    #[test]
    fn barge_in_needs_three_loud_checks_in_a_row_and_tells_once() {
        let mut b = BargeIn::default();
        let loud = audio::meter(BARGE_RMS);
        assert!(!b.hear(loud) && !b.hear(loud));
        assert!(b.hear(loud));
        assert!(!b.hear(loud), "told once per burst");
        assert!(!b.hear(audio::meter(BARGE_RMS * 0.9)));
        assert!(!b.hear(loud) && !b.hear(loud) && b.hear(loud));
    }

    #[test]
    fn voices_list_natural_ones_first_then_the_systems_language_then_by_name() {
        let v = |name: &str, lang: &str, natural: bool| VoiceInfo { id: name.into(), name: name.into(), lang: lang.into(), natural, download: None };
        let got = ranked(vec![v("Zed", "en-GB", false), v("Amélie", "fr-FR", false), v("Ava", "en-US", true), v("Alex", "en-US", false), v("Kyoko", "ja-JP", true)], "en-US");
        let names: Vec<&str> = got.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Ava", "Kyoko", "Alex", "Zed", "Amélie"]);
        // With no natural voice, the Mac's novelty and older robotic voices come after its standard ones.
        let mac = |id: &str, name: &str| VoiceInfo { id: id.into(), name: name.into(), lang: "en-US".into(), natural: false, download: None };
        let got = ranked(
            vec![
                mac("com.apple.speech.synthesis.voice.Albert", "Albert"),
                mac("com.apple.speech.synthesis.voice.Bells", "Bells"),
                mac("com.apple.eloquence.en-US.Eddy", "Eddy"),
                mac("com.apple.voice.compact.en-US.Samantha", "Samantha"),
            ],
            "en-US",
        );
        let names: Vec<&str> = got.iter().map(|v| v.name.as_str()).collect();
        assert_eq!(names, ["Samantha", "Eddy", "Albert", "Bells"]);
    }

    mod natural_voice {
        use super::super::natural::fake::{FakeSink, FakeSynth};
        use super::super::natural::Natural;
        use super::*;

        fn voices(installed: bool) -> (Arc<Fake>, Arc<FakeSynth>, Arc<FakeSink>, Arc<Voices>) {
            let (system, synth, sink) = (Arc::new(Fake::auto()), Arc::new(FakeSynth::new(5, installed)), Arc::new(FakeSink::new(5)));
            let v = Arc::new(Voices::new(system.clone(), Some(Natural::new(synth.clone(), sink.clone()))));
            (system, synth, sink, v)
        }

        fn kokoro() -> How {
            How { voice: Some("kokoro:af_heart".into()), rate: 1.0 }
        }

        fn wait(ok: impl Fn() -> bool) -> bool {
            let t = Instant::now();
            while t.elapsed() < Duration::from_secs(2) {
                if ok() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            false
        }

        #[test]
        fn the_natural_voices_come_first_with_their_download_size_until_downloaded() {
            let (_, synth, _, v) = voices(false);
            let listed = v.voices().unwrap();
            assert_eq!((listed[0].id.as_str(), listed[0].download), ("kokoro:af_heart", Some(1000)));
            synth.fetched(true);
            assert_eq!(v.voices().unwrap()[0].download, None);
        }

        #[test]
        fn a_natural_voice_not_downloaded_yet_reads_answers_in_the_system_voice_meanwhile() {
            let (system, synth, sink, v) = voices(false);
            let r = Reader::new(v, move || Some(kokoro()), |_| {});
            r.hear(&state(State::Sending, 1));
            r.hear(&delta(1, "One. Two."));
            r.hear(&state(State::Shown, 1));
            assert!(system.wait_started(2));
            assert!(r.talk.wait_idle(Duration::from_secs(1)));
            assert!(sink.events("start").is_empty() && synth.events("make").is_empty());
            assert!(synth.fetches.lock().unwrap().is_empty(), "an answer never starts a download");
            // Once downloaded, answers are read in it.
            synth.fetched(true);
            r.hear(&state(State::Sending, 2));
            r.hear(&delta(2, "Three."));
            r.hear(&state(State::Shown, 2));
            assert!(wait(|| sink.events("end").len() == 1));
            assert_eq!(system.started().len(), 2);
        }

        #[test]
        fn hear_it_with_a_voice_to_download_downloads_it_then_says_the_sample_in_it() {
            let (system, synth, sink, v) = voices(false);
            let r = Reader::new(v, || None, |_| {});
            r.sample(kokoro());
            std::thread::sleep(Duration::from_millis(50));
            assert!(system.started().is_empty() && sink.events("start").is_empty(), "nothing said before the download");
            assert_eq!(synth.fetches.lock().unwrap().len(), 1, "the download started");
            synth.fetched(true);
            assert!(wait(|| sink.events("end").len() == 1), "said once it is here");
            assert_eq!(synth.events("make")[0].0, SAMPLE);
        }

        #[test]
        fn a_stop_while_downloading_means_the_sample_is_not_said_after_and_a_failed_download_says_nothing() {
            let (system, synth, sink, v) = voices(false);
            let r = Reader::new(v, || None, |_| {});
            r.sample(kokoro());
            r.stop();
            synth.fetched(true);
            std::thread::sleep(Duration::from_millis(80));
            assert!(sink.events("start").is_empty());
            let (system2, synth2, sink2, v2) = voices(false);
            let r2 = Reader::new(v2, || None, |_| {});
            r2.sample(kokoro());
            synth2.fetched(false);
            std::thread::sleep(Duration::from_millis(80));
            assert!(sink2.events("start").is_empty() && system2.started().is_empty() && system.started().is_empty());
        }

        #[test]
        fn with_no_voice_chosen_the_system_voice_speaks_until_kokoro_is_here_then_kokoro_does() {
            let (system, synth, sink, v) = voices(false);
            let talk = Talk::new(v.clone(), |_| {});
            talk.say("One.".into(), How::default());
            assert!(system.wait_started(1));
            synth.fetched(true);
            talk.say("Two.".into(), How::default());
            assert!(wait(|| sink.events("end").len() == 1));
            assert_eq!(synth.events("make")[0].0, "Two.");
            // A system voice chosen by name stays the system's.
            talk.say("Three.".into(), How { voice: Some("com.apple.voice.premium.en-US.Zoe".into()), rate: 1.0 });
            assert!(system.wait_started(2));
        }

        #[test]
        fn choosing_a_voice_downloads_it_once_and_choosing_one_that_is_here_loads_it() {
            let (_, synth, _, v) = voices(false);
            let r = Reader::new(v, || None, |_| {});
            assert!(!r.ready(&kokoro()));
            assert_eq!(synth.fetches.lock().unwrap().len(), 1);
            synth.fetched(true);
            assert!(r.ready(&kokoro()));
            assert!(wait(|| !synth.events("load").is_empty()));
            assert!(r.ready(&How { voice: Some("com.apple.voice.compact.en-US.Samantha".into()), rate: 1.0 }), "a system voice is always ready");
        }
    }

    #[test]
    fn linux_says_it_is_not_supported() {
        let u = Unsupported;
        assert_eq!(u.speak("Hi.", &How::default(), Box::new(|| {})), Err(UNSUPPORTED.into()));
        assert_eq!(u.voices(), Err(UNSUPPORTED.into()));
    }

    /// Hand-run on a Mac: speaks out loud. `cargo test --lib on_this_mac_say_two_sentences -- --ignored --nocapture`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "speaks out loud on this Mac"]
    fn on_this_mac_say_two_sentences() {
        let mac = Arc::new(mac::Mac::default());
        let voices = mac.voices().expect("the Mac lists its voices");
        let natural = voices.iter().filter(|v| v.natural).count();
        println!("{} voices, {natural} natural. First ten:", voices.len());
        for v in voices.iter().take(10) {
            println!("  {} ({}){}  {}", v.name, v.lang, if v.natural { " (natural)" } else { "" }, v.id);
        }
        let how = How { voice: voices.first().map(|v| v.id.clone()), rate: 1.0 };
        println!("Speaking with {}", voices.first().map_or("the default voice", |v| v.name.as_str()));
        let talk = Talk::new(mac.clone(), |_| {});
        let from = Instant::now();
        talk.say("Hello from Pointcast.".into(), how.clone());
        talk.say("This second sentence is cut off halfway, so you should never hear the end of it.".into(), how);
        let first = wait_for(|| mac.begun().0 >= 1).expect("the first sentence starts to sound");
        println!("time to first audio: {} ms", first.duration_since(from).as_millis());
        let second = wait_for(|| mac.begun().0 >= 2).expect("the second sentence starts to sound");
        let gap = mac.begun().1.map(|t| t.duration_since(first).as_millis());
        println!("second sentence started {} ms after the first ({:?} ms by the voice's own clock)", second.duration_since(first).as_millis(), gap);
        std::thread::sleep(Duration::from_millis(1500));
        let cut = Instant::now();
        talk.stop();
        assert!(talk.wait_idle(Duration::from_secs(2)), "the stop ends speech");
        println!("stopped midway: idle {} ms after stop", cut.elapsed().as_millis());
    }

    #[cfg(target_os = "macos")]
    fn wait_for(ok: impl Fn() -> bool) -> Option<Instant> {
        let from = Instant::now();
        while from.elapsed() < Duration::from_secs(15) {
            if ok() {
                return Some(Instant::now());
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }
}
