//! A natural voice that runs on this computer (Kokoro, `voice::kokoro`): each sentence is made into audio by a thread
//! of its own and played through the speakers (`out`). The next sentence is made while this one plays, so one follows
//! the other without a wait, and a stop cuts the sound at once.

use super::{Done, Fetched, VoiceInfo};
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// What makes a natural voice's audio.
pub trait Synth: Send + Sync {
    /// Its voices, those it has to download first with their size.
    fn voices(&self) -> Vec<VoiceInfo>;
    /// Whether `voice` is one of its own.
    fn owns(&self, voice: &str) -> bool;
    /// Whether its files are on this computer.
    fn installed(&self) -> bool;
    /// The voice to use when none is chosen, once it is installed.
    fn default_voice(&self) -> String;
    /// Download its files; `then` hears whether it worked. A second call while one runs only adds `then`.
    fn fetch(&self, then: Fetched);
    /// Load what speaking in `voice` needs (slow the first time, then nothing).
    fn load(&self, voice: &str) -> Result<(), String>;
    /// Say `text` in `voice` at `rate` (1 is normal) as mono samples, handed to `part` a piece at a time as each is
    /// made (with their sample rate), so the first can play while the rest is made. `part` returning false stops it.
    fn make(&self, text: &str, voice: &str, rate: f32, part: &mut dyn FnMut(Vec<f32>, u32) -> bool) -> Result<(), String>;
    /// Let go of what `load` holds, after a while unused.
    fn unload(&self);
}

/// Where the audio is heard.
pub trait Sink: Send + Sync {
    /// Play `samples` (mono, `rate` Hz) after anything playing; `ended` is called once, when they played to the end or
    /// were cut, or at once with `Err` when they could not be played.
    fn play(&self, samples: Vec<f32>, rate: u32, ended: Done) -> Result<(), String>;
    /// Cut what plays, at once.
    fn stop(&self);
    /// Get ready to play soon (open the device now, so the first sound is not kept waiting for it).
    fn wake(&self) {}
}

/// After this long with nothing said, the model is let go of, giving its memory back.
const IDLE: Duration = Duration::from_secs(10 * 60);
/// Sentences made ahead and kept for when they are asked for.
const KEEP: usize = 3;

/// A sentence as made: its words, voice and rate.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    text: String,
    voice: String,
    rate: u32,
}

impl Key {
    fn new(text: &str, voice: &str, rate: f32) -> Key {
        Key { text: text.into(), voice: voice.into(), rate: rate.to_bits() }
    }
}

/// A sentence's audio, in the pieces it was made in, and their sample rate.
type Parts = (Vec<Vec<f32>>, u32);

/// The sentence being made.
struct Making {
    key: Key,
    /// The epoch it was asked in.
    epoch: u64,
    /// Pieces made and not played yet.
    parts: Vec<Vec<f32>>,
    rate: u32,
    /// Once it is the sentence to say, its pieces go to the speakers as they are made, and this hears its end.
    live: Option<Done>,
}

#[derive(Default)]
struct St {
    /// Counts stops: audio made for a sentence asked before a stop is thrown away.
    epoch: u64,
    /// Sentences to make, in order.
    wanted: VecDeque<Key>,
    making: Option<Making>,
    made: VecDeque<(Key, Result<Parts, String>)>,
    /// The sentence to say now, waiting for its audio.
    now: Option<(Key, Done)>,
    /// A voice to load before anything is said.
    warm: Option<String>,
    loaded: bool,
    closed: bool,
}

struct Inner {
    synth: Arc<dyn Synth>,
    sink: Arc<dyn Sink>,
    st: Mutex<St>,
    moved: Condvar,
}

/// A natural voice as a `Speaker` (through `Voices`).
pub struct Natural {
    inner: Arc<Inner>,
}

fn nothing() -> Done {
    Box::new(|| {})
}

impl Natural {
    pub fn new(synth: Arc<dyn Synth>, sink: Arc<dyn Sink>) -> Natural {
        let inner = Arc::new(Inner { synth, sink, st: Mutex::default(), moved: Condvar::new() });
        let worker = inner.clone();
        std::thread::Builder::new().name("speak-natural".into()).spawn(move || worker.run()).expect("the voice thread starts");
        Natural { inner }
    }

    pub fn synth(&self) -> &Arc<dyn Synth> {
        &self.inner.synth
    }

    /// Say `text`: its audio is made (unless it was made ahead) and played; `done` is called when it has been heard or
    /// was cut.
    pub fn speak(&self, text: &str, voice: &str, rate: f32, done: Done) {
        let key = Key::new(text, voice, rate);
        let mut st = self.inner.st.lock().unwrap();
        if let Some((_, old)) = st.now.take() {
            old();
        }
        if !st.queued(&key) {
            st.wanted.push_front(key.clone());
        }
        st.now = Some((key, done));
        self.inner.moved.notify_all();
    }

    /// Start making `text` now, to be said next.
    pub fn prepare(&self, text: &str, voice: &str, rate: f32) {
        let key = Key::new(text, voice, rate);
        let mut st = self.inner.st.lock().unwrap();
        if !st.queued(&key) {
            st.wanted.push_back(key);
            self.inner.moved.notify_all();
        }
    }

    /// Load `voice` and open the speakers now, so the first sentence is quick.
    pub fn warm(&self, voice: &str) {
        let mut st = self.inner.st.lock().unwrap();
        st.warm = Some(voice.into());
        self.inner.moved.notify_all();
        drop(st);
        self.inner.sink.wake();
    }

    /// Cut the sentence being said and forget those made ahead, at once; one being made stops at its next piece.
    pub fn stop(&self) {
        let (now, live) = {
            let mut st = self.inner.st.lock().unwrap();
            st.epoch += 1;
            st.wanted.clear();
            st.made.clear();
            (st.now.take(), st.making.as_mut().and_then(|m| m.live.take()))
        };
        self.inner.sink.stop();
        if let Some((_, done)) = now {
            done();
        }
        if let Some(done) = live {
            done();
        }
    }
}

impl Drop for Natural {
    fn drop(&mut self) {
        self.stop();
        self.inner.st.lock().unwrap().closed = true;
        self.inner.moved.notify_all();
    }
}

impl St {
    /// Made, being made in this epoch, or waiting to be.
    fn queued(&self, key: &Key) -> bool {
        self.made.iter().any(|(k, _)| k == key) || self.making.as_ref().is_some_and(|m| m.key == *key && m.epoch == self.epoch) || self.wanted.contains(key)
    }
}

impl Inner {
    /// Play a sentence's pieces, then hear its end.
    fn hand(&self, parts: Vec<Vec<f32>>, rate: u32, done: Done) {
        for p in parts {
            if let Err(why) = self.sink.play(p, rate, nothing()) {
                eprintln!("[speak] {why}");
            }
        }
        // An empty sound after the last piece: it ends when they all have.
        if let Err(why) = self.sink.play(Vec::new(), rate, done) {
            eprintln!("[speak] {why}");
        }
    }

    /// A piece of the sentence being made: kept, or played at once when it is the sentence to say. False after a stop.
    fn part(&self, samples: Vec<f32>, rate: u32) -> bool {
        let mut st = self.st.lock().unwrap();
        let epoch = st.epoch;
        let St { making, now, .. } = &mut *st;
        let Some(m) = making.as_mut().filter(|m| m.epoch == epoch) else { return false };
        m.rate = rate;
        if m.live.is_none() && now.as_ref().is_some_and(|(k, _)| *k == m.key) {
            m.live = now.take().map(|(_, d)| d);
            for p in m.parts.drain(..) {
                let _ = self.sink.play(p, rate, nothing());
            }
        }
        if m.live.is_some() {
            let _ = self.sink.play(samples, rate, nothing());
        } else {
            m.parts.push(samples);
        }
        true
    }

    fn run(&self) {
        let mut st = self.st.lock().unwrap();
        loop {
            if st.closed {
                return;
            }
            // The sentence to say has its audio: play it.
            let ready = st.now.as_ref().and_then(|(k, _)| st.made.iter().position(|(m, _)| m == k));
            if let Some(i) = ready {
                let (_, made) = st.made.remove(i).unwrap();
                let (_, done) = st.now.take().unwrap();
                drop(st);
                match made {
                    Ok((parts, rate)) => self.hand(parts, rate, done),
                    Err(why) => {
                        eprintln!("[speak] {why}");
                        done();
                    }
                }
                st = self.st.lock().unwrap();
                continue;
            }
            if let Some(key) = st.wanted.pop_front() {
                let epoch = st.epoch;
                st.making = Some(Making { key: key.clone(), epoch, parts: Vec::new(), rate: 0, live: None });
                drop(st);
                // A panic in the voice (a bug in its text or model) costs this sentence only: the thread goes on to the
                // next, rather than leaving every later sentence to wait out `LONGEST`.
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.synth.make(&key.text, &key.voice, f32::from_bits(key.rate), &mut |samples, rate| self.part(samples, rate))
                }))
                .unwrap_or_else(|_| Err("the natural voice failed on a sentence and skipped it".into()));
                st = self.st.lock().unwrap_or_else(|e| e.into_inner());
                st.loaded = true;
                let Some(m) = st.making.take() else { continue };
                if let Err(why) = &r {
                    eprintln!("[speak] {why}");
                }
                if m.epoch != st.epoch {
                    continue;
                }
                if let Some(done) = m.live {
                    drop(st);
                    self.hand(m.parts, m.rate, done);
                    st = self.st.lock().unwrap();
                } else {
                    st.made.push_back((key, r.map(|()| (m.parts, m.rate))));
                    while st.made.len() > KEEP {
                        st.made.pop_front();
                    }
                }
                continue;
            }
            if let Some(voice) = st.warm.take() {
                drop(st);
                if let Err(why) = self.synth.load(&voice) {
                    eprintln!("[speak] {why}");
                }
                st = self.st.lock().unwrap();
                st.loaded = true;
                continue;
            }
            let (next, waited) = self.moved.wait_timeout(st, IDLE).unwrap();
            st = next;
            if waited.timed_out() && st.loaded && st.now.is_none() && st.wanted.is_empty() {
                st.loaded = false;
                self.synth.unload();
            }
        }
    }
}

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::time::Instant;

    /// A synth for tests: making a sentence takes `make_ms`, in `pieces` pieces, and each start and end is noted with
    /// its time ("make", "part", "made").
    pub struct FakeSynth {
        pub make_ms: u64,
        pub pieces: usize,
        /// Making this sentence panics, as a bug in the voice would.
        pub panics_on: Option<&'static str>,
        pub installed: Mutex<bool>,
        pub log: Mutex<Vec<(String, &'static str, Instant)>>,
        pub fetches: Mutex<Vec<Fetched>>,
    }

    impl FakeSynth {
        pub fn new(make_ms: u64, installed: bool) -> FakeSynth {
            FakeSynth { make_ms, pieces: 1, panics_on: None, installed: Mutex::new(installed), log: Mutex::default(), fetches: Mutex::default() }
        }
        pub fn events(&self, what: &str) -> Vec<(String, Instant)> {
            self.log.lock().unwrap().iter().filter(|e| e.1 == what).map(|e| (e.0.clone(), e.2)).collect()
        }
        /// End the download as if it finished, or failed.
        pub fn fetched(&self, ok: bool) {
            *self.installed.lock().unwrap() = ok;
            let waiting: Vec<Fetched> = self.fetches.lock().unwrap().drain(..).collect();
            for then in waiting {
                then(ok);
            }
        }
    }

    impl Synth for FakeSynth {
        fn voices(&self) -> Vec<VoiceInfo> {
            let download = (!*self.installed.lock().unwrap()).then_some(1000);
            vec![VoiceInfo { id: "kokoro:af_heart".into(), name: "Heart".into(), lang: "en-US".into(), natural: true, download }]
        }
        fn owns(&self, voice: &str) -> bool {
            voice.starts_with("kokoro:")
        }
        fn installed(&self) -> bool {
            *self.installed.lock().unwrap()
        }
        fn default_voice(&self) -> String {
            "kokoro:af_heart".into()
        }
        fn fetch(&self, then: Fetched) {
            self.fetches.lock().unwrap().push(then);
        }
        fn load(&self, voice: &str) -> Result<(), String> {
            self.log.lock().unwrap().push((voice.into(), "load", Instant::now()));
            Ok(())
        }
        fn make(&self, text: &str, _: &str, _: f32, part: &mut dyn FnMut(Vec<f32>, u32) -> bool) -> Result<(), String> {
            self.log.lock().unwrap().push((text.into(), "make", Instant::now()));
            if self.panics_on == Some(text) {
                panic!("a test's bug in the voice, on {text}");
            }
            for _ in 0..self.pieces {
                std::thread::sleep(Duration::from_millis(self.make_ms / self.pieces as u64));
                self.log.lock().unwrap().push((text.into(), "part", Instant::now()));
                if !part(vec![0.1; 10], 24_000) {
                    break;
                }
            }
            self.log.lock().unwrap().push((text.into(), "made", Instant::now()));
            Ok(())
        }
        fn unload(&self) {}
    }

    #[derive(Default)]
    struct Queue {
        /// Sounds to play: their number, whether they have samples, who hears their end.
        waiting: VecDeque<(usize, bool, Done)>,
        playing: Option<(usize, Done)>,
        n: usize,
    }

    /// Speakers for tests: sounds play one after another, each with samples for `play_ms` (an empty one at once),
    /// noted when they start and end or are cut.
    type Log = Arc<Mutex<Vec<(usize, &'static str, Instant)>>>;

    pub struct FakeSink {
        pub log: Log,
        q: Arc<(Mutex<Queue>, Condvar)>,
    }

    impl FakeSink {
        pub fn new(play_ms: u64) -> FakeSink {
            let (log, q) = (Arc::new(Mutex::new(Vec::new())), Arc::new((Mutex::new(Queue::default()), Condvar::new())));
            let (log2, q2) = (log.clone(), q.clone());
            std::thread::spawn(move || loop {
                let (id, sound) = {
                    let (m, cv) = &*q2;
                    let mut g = m.lock().unwrap();
                    while g.waiting.is_empty() {
                        g = cv.wait(g).unwrap();
                    }
                    let (id, sound, done) = g.waiting.pop_front().unwrap();
                    if sound {
                        log2.lock().unwrap().push((id, "start", Instant::now()));
                    }
                    g.playing = Some((id, done));
                    (id, sound)
                };
                if sound {
                    std::thread::sleep(Duration::from_millis(play_ms));
                }
                let mine = {
                    let mut g = q2.0.lock().unwrap();
                    if g.playing.as_ref().is_some_and(|(i, _)| *i == id) {
                        g.playing.take()
                    } else {
                        None
                    }
                };
                if let Some((_, done)) = mine {
                    if sound {
                        log2.lock().unwrap().push((id, "end", Instant::now()));
                    }
                    done();
                }
            });
            FakeSink { log, q }
        }
        pub fn events(&self, what: &str) -> Vec<Instant> {
            self.log.lock().unwrap().iter().filter(|e| e.1 == what).map(|e| e.2).collect()
        }
    }

    impl Sink for FakeSink {
        fn play(&self, samples: Vec<f32>, _: u32, ended: Done) -> Result<(), String> {
            let (m, cv) = &*self.q;
            let mut g = m.lock().unwrap();
            g.n += 1;
            let n = g.n;
            g.waiting.push_back((n, !samples.is_empty(), ended));
            cv.notify_all();
            Ok(())
        }
        fn stop(&self) {
            let ended: Vec<(usize, Done)> = {
                let mut g = self.q.0.lock().unwrap();
                let mut all: Vec<(usize, Done)> = g.playing.take().into_iter().collect();
                all.extend(g.waiting.drain(..).map(|(i, _, d)| (i, d)));
                all
            };
            if let Some((id, _)) = ended.first() {
                self.log.lock().unwrap().push((*id, "cut", Instant::now()));
            }
            for (_, done) in ended {
                done();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeSink, FakeSynth};
    use super::*;
    use crate::voice::speak::{How, Speaker, Talk, Voices};
    use std::time::Instant;

    fn natural(make_ms: u64, play_ms: u64) -> (Arc<FakeSynth>, Arc<FakeSink>, Arc<Voices>) {
        let synth = Arc::new(FakeSynth::new(make_ms, true));
        let sink = Arc::new(FakeSink::new(play_ms));
        let fake_system = Arc::new(crate::voice::speak::fake::Fake::auto());
        let voices = Arc::new(Voices::new(fake_system, Some(Natural::new(synth.clone(), sink.clone()))));
        (synth, sink, voices)
    }

    fn kokoro() -> How {
        How { voice: Some("kokoro:af_heart".into()), rate: 1.0 }
    }

    #[test]
    fn the_next_sentence_is_made_while_this_one_plays_so_they_follow_without_a_gap() {
        let (synth, sink, voices) = natural(60, 200);
        let talk = Talk::new(voices, |_| {});
        for s in ["One.", "Two.", "Three."] {
            talk.say(s.into(), kokoro());
        }
        assert!(talk.wait_idle(Duration::from_secs(5)));
        let (made, starts, ends) = (synth.events("made"), sink.events("start"), sink.events("end"));
        assert_eq!(made.iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["One.", "Two.", "Three."], "each made once, in order");
        assert_eq!((starts.len(), ends.len()), (3, 3));
        for i in 1..3 {
            assert!(made[i].1 < ends[i - 1], "sentence {} was made before sentence {} ended", i + 1, i);
            let gap = starts[i].duration_since(ends[i - 1]);
            assert!(gap < Duration::from_millis(40), "a {gap:?} wait between sentences {i} and {}", i + 1);
        }
    }

    #[test]
    fn a_sentence_that_arrives_while_another_plays_is_made_at_once() {
        let (synth, sink, voices) = natural(50, 300);
        let talk = Talk::new(voices, |_| {});
        talk.say("One.".into(), kokoro());
        let t = Instant::now();
        while sink.events("start").is_empty() && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(2));
        }
        // The answer streams on: the next sentence comes while the first is heard.
        talk.say("Two.".into(), kokoro());
        assert!(talk.wait_idle(Duration::from_secs(5)));
        let (made, ends) = (synth.events("made"), sink.events("end"));
        assert!(made[1].1 < ends[0], "made while the first played");
        assert!(sink.events("start")[1].duration_since(ends[0]) < Duration::from_millis(40));
    }

    #[test]
    fn a_stop_during_playback_cuts_it_at_once_and_says_nothing_more() {
        let (synth, sink, voices) = natural(20, 2_000);
        let talk = Talk::new(voices, |_| {});
        for s in ["One.", "Two.", "Three."] {
            talk.say(s.into(), kokoro());
        }
        let t = Instant::now();
        while sink.events("start").is_empty() && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(2));
        }
        let cut = Instant::now();
        talk.stop();
        assert_eq!(sink.events("cut").len(), 1, "the sound is cut");
        assert!(sink.events("cut")[0].duration_since(cut) < Duration::from_millis(20), "at once");
        assert!(talk.wait_idle(Duration::from_millis(500)), "the queue is free at once, not after the sentence");
        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(sink.events("start").len(), 1, "nothing made ahead is played after the stop");
        // It speaks again after a stop.
        talk.say("Four.".into(), kokoro());
        assert!(talk.wait_idle(Duration::from_secs(5)));
        assert_eq!(sink.events("start").len(), 2);
        assert_eq!(synth.events("make").last().unwrap().0, "Four.");
    }

    #[test]
    fn the_same_line_asked_again_right_after_a_stop_is_still_said() {
        let (_, sink, voices) = natural(80, 30);
        let talk = Talk::new(voices, |_| {});
        talk.say("Hi.".into(), kokoro());
        std::thread::sleep(Duration::from_millis(20));
        // Hear it pressed twice: the first is still being made.
        talk.stop();
        talk.say("Hi.".into(), kokoro());
        assert!(talk.wait_idle(Duration::from_secs(5)));
        assert_eq!(sink.events("end").len(), 1);
    }

    #[test]
    fn a_long_sentence_starts_playing_when_its_first_piece_is_made_not_when_all_of_it_is() {
        let mut synth = FakeSynth::new(300, true);
        synth.pieces = 3;
        let synth = Arc::new(synth);
        let sink = Arc::new(FakeSink::new(150));
        let voices = Arc::new(Voices::new(Arc::new(crate::voice::speak::fake::Fake::auto()), Some(Natural::new(synth.clone(), sink.clone()))));
        let talk = Talk::new(voices, |_| {});
        talk.say("A long sentence, in three pieces.".into(), kokoro());
        assert!(talk.wait_idle(Duration::from_secs(5)));
        let (made, starts, ends) = (synth.events("made"), sink.events("start"), sink.events("end"));
        assert_eq!((starts.len(), ends.len()), (3, 3), "each piece played once");
        assert!(starts[0] < made[0].1, "the first piece played while the rest was made");
        assert!(talk.wait_idle(Duration::from_millis(10)) && *ends.last().unwrap() <= Instant::now(), "the sentence ended after its last piece");
    }

    #[test]
    fn a_sentence_whose_making_panics_is_skipped_and_the_ones_after_it_are_still_said() {
        let mut synth = FakeSynth::new(10, true);
        synth.panics_on = Some("Boom.");
        let synth = Arc::new(synth);
        let sink = Arc::new(FakeSink::new(20));
        let voices = Arc::new(Voices::new(Arc::new(crate::voice::speak::fake::Fake::auto()), Some(Natural::new(synth.clone(), sink.clone()))));
        let talk = Talk::new(voices, |_| {});
        for s in ["One.", "Boom.", "Two."] {
            talk.say(s.into(), kokoro());
        }
        assert!(talk.wait_idle(Duration::from_secs(5)), "the voice's thread died with the sentence that panicked");
        assert_eq!(synth.events("made").iter().map(|m| m.0.as_str()).collect::<Vec<_>>(), ["One.", "Two."]);
        assert_eq!(sink.events("end").len(), 2, "the sentences either side are heard");
        // And it speaks after.
        talk.say("Three.".into(), kokoro());
        assert!(talk.wait_idle(Duration::from_secs(5)));
        assert_eq!(sink.events("end").len(), 3);
    }

    #[test]
    fn warming_a_voice_loads_it_before_anything_is_said() {
        let (synth, _, voices) = natural(10, 10);
        assert!(voices.ready(&kokoro(), None));
        let t = Instant::now();
        while synth.events("load").is_empty() && t.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(synth.events("load")[0].0, "kokoro:af_heart");
        assert!(synth.events("make").is_empty());
    }
}
