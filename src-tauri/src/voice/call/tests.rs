use super::*;
use std::sync::atomic::AtomicU64;

const MS: usize = audio::RATE as usize / 1000;

#[test]
fn end_call_or_hang_up_said_alone_ends_the_call_and_nothing_else_does() {
    for yes in ["end call", "End call.", "Hang up!", "hang-up", "End the call, please.", "Okay, hang up now.", "Hang up, thank you.", "hangup"] {
        assert!(is_hangup(yes), "{yes}");
    }
    for no in ["", "call", "how do I end a call in Teams?", "don't hang up", "end call and then open the file", "hang up the phone", "end"] {
        assert!(!is_hangup(no), "{no}");
    }
}

#[test]
fn the_preroll_keeps_the_hundred_and_fifty_ms_before_the_first_word() {
    let feed = Feed::new(KEEP_MS, None);
    // Each sample says where it is.
    let all: Vec<f32> = (0..3 * audio::RATE as usize).map(|i| i as f32).collect();
    for c in all.chunks(320) {
        feed.push(c);
    }
    let onset = 2 * audio::RATE as u64;
    feed.start_at(Some(onset - turn::PREROLL_MS as u64 * MS as u64));
    let take = Arc::new(Take::default());
    let mic = Mic::from_feed(feed.clone(), take.clone());
    feed.push(&[-1.0; 160]);
    drop(mic);
    feed.push(&[-2.0; 160]);
    let got = take.all();
    assert_eq!(got[0] as u64, onset - 2400, "it starts 150 ms before the first word");
    assert_eq!(got[2400] as u64, onset);
    assert_eq!(got.len(), audio::RATE as usize + 2400 + 160, "then everything after, until it is closed");
    // Further back than is kept starts where the keeping does; no start given starts with the next sample.
    let feed = Feed::new(100, None);
    feed.push(&all[..16_000]);
    feed.start_at(Some(0));
    let take = Arc::new(Take::default());
    drop(Mic::from_feed(feed.clone(), take.clone()));
    assert_eq!((take.all()[0], take.len()), (14_400.0, 1_600));
}

/// The Ask side, scripted on the simulation's clock: each question is answered `think_ms` after it is asked, all at
/// once, and read aloud for `speak_ms`.
struct Script {
    clock: Arc<AtomicU64>,
    feed: Arc<Feed>,
    think_ms: u64,
    speak_ms: u64,
    st: Mutex<St>,
}

#[derive(Default)]
struct St {
    stage: Option<Stage>,
    asked_at: u64,
    speaking_until: u64,
    mic: Option<(Mic, Arc<Take>)>,
    /// Each turn asked: where its recording began in the call's audio, and what it holds.
    turns: Vec<(u64, Vec<f32>)>,
    cut: Vec<u64>,
    dropped: usize,
}

impl Script {
    fn now(&self) -> u64 {
        self.clock.load(Ordering::SeqCst)
    }
}

impl Asking for Script {
    fn listen(&self) -> bool {
        let mut s = self.st.lock().unwrap();
        if s.stage == Some(Stage::Asking) {
            return false;
        }
        let take = Arc::new(Take::default());
        s.mic = Some((Mic::from_feed(self.feed.clone(), take.clone()), take));
        // The press stops any reading, as the panel's Listening does.
        (s.stage, s.speaking_until) = (Some(Stage::Listening), 0);
        true
    }
    fn ask(&self) {
        let mut s = self.st.lock().unwrap();
        if let Some((mic, take)) = s.mic.take() {
            drop(mic);
            let held = take.all();
            s.turns.push((self.feed.end() - held.len() as u64, held));
        }
        (s.stage, s.asked_at) = (Some(Stage::Asking), self.now());
    }
    fn stop_answer(&self) {
        let mut s = self.st.lock().unwrap();
        if s.stage == Some(Stage::Asking) {
            s.stage = Some(Stage::Done);
        }
    }
    fn drop_turn(&self) {
        let mut s = self.st.lock().unwrap();
        if s.stage == Some(Stage::Listening) {
            s.mic = None;
            s.stage = Some(Stage::Done);
            s.dropped += 1;
        }
    }
    fn stop_speech(&self) {
        let now = self.now();
        let mut s = self.st.lock().unwrap();
        if s.speaking_until > now {
            s.cut.push(now);
        }
        s.speaking_until = 0;
    }
    fn stage(&self) -> Stage {
        let now = self.now();
        let mut s = self.st.lock().unwrap();
        if s.stage == Some(Stage::Asking) && now >= s.asked_at + self.think_ms {
            (s.stage, s.speaking_until) = (Some(Stage::Done), now + self.speak_ms);
        }
        s.stage.unwrap_or(Stage::Done)
    }
    fn speaking(&self) -> bool {
        self.st.lock().unwrap().speaking_until > self.now()
    }
    fn played(&self) -> f32 {
        0.0
    }
    fn hear_words(&self, _: Option<crate::live::Words>) {}
}

/// When the simulated person says something.
#[derive(Clone, Copy, Debug)]
enum When {
    /// At this ms of the call.
    At(u64),
    /// This long after the call starts listening for the `n`th time (from 1).
    Listening(usize, u64),
    /// This long after the `n`th answer starts being read aloud.
    Speaking(usize, u64),
}

struct Sim {
    l: Loop,
    script: Arc<Script>,
    /// Where each thing said began, in samples into the call's audio (none if never said).
    said_at: Vec<Option<u64>>,
}

/// A call with the person saying `says` and Pointcast's voice coming back through the speakers as `echo` (looped)
/// while it reads, every 20 ms until the call ends or `end_ms`, where the switch ends it.
fn simulate(vad: Box<dyn Vad>, says: Vec<(When, Vec<f32>)>, echo: &[f32], speak_ms: u64, end_ms: u64) -> Sim {
    let clock = Arc::new(AtomicU64::new(0));
    let feed = Feed::new(KEEP_MS, None);
    let script = Arc::new(Script { clock: clock.clone(), feed: feed.clone(), think_ms: 900, speak_ms, st: Mutex::default() });
    let mut l = Loop::new(script.clone(), feed, vad, Arc::new(AtomicBool::new(false)), 0);
    let mut said_at = vec![None; says.len()];
    let (mut listens, mut answers, mut listen_ms, mut speak_from) = (1usize, 0usize, 0u64, 0u64);
    let (mut was_side, mut was_speaking) = (l.side(), false);
    // A quiet room (about -60 dBFS), from a fixed seed.
    let mut seed = 7u32;
    let mut room = move || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32 * 0.002 - 0.001
    };
    let (mut pos, mut echo_at) = (0u64, 0usize);
    let mut now = 0u64;
    while l.over().is_none() && now < end_ms {
        now += 20;
        clock.store(now, Ordering::SeqCst);
        let mut chunk: Vec<f32> = (0..20 * MS).map(|_| room()).collect();
        let speaking = script.speaking();
        for (i, (when, clip)) in says.iter().enumerate() {
            let due = match *when {
                When::At(t) => Some(t),
                When::Listening(n, ms) => (listens == n).then_some(listen_ms + ms),
                When::Speaking(n, ms) => (answers == n).then_some(speak_from + ms),
            };
            if said_at[i].is_none() && due.is_some_and(|d| now >= d) {
                said_at[i] = Some(pos);
            }
            if let Some(from) = said_at[i] {
                for (k, c) in chunk.iter_mut().enumerate() {
                    if let Some(v) = clip.get((pos + k as u64 - from) as usize) {
                        *c += v;
                    }
                }
            }
        }
        if speaking && !echo.is_empty() {
            for c in chunk.iter_mut() {
                *c += echo[echo_at % echo.len()];
                echo_at += 1;
            }
        }
        pos += chunk.len() as u64;
        l.tick(vec![], &chunk, now);
        if l.side() != was_side {
            was_side = l.side();
            if was_side == Side::Listening {
                listens += 1;
                listen_ms = now;
            }
        }
        let speaking = script.speaking();
        if speaking && !was_speaking {
            answers += 1;
            speak_from = now;
        }
        was_speaking = speaking;
    }
    if l.over().is_none() {
        l.tick(vec![Event::Toggle], &[], now);
    }
    Sim { l, script, said_at }
}

/// A made-up voice for the level detector: a tone at `rms` in syllables (200 ms on, 60 ms off) for `ms`.
fn voice(rms: f32, ms: usize) -> Vec<f32> {
    (0..ms * MS).map(|i| if i % (260 * MS) < 200 * MS { rms * std::f32::consts::SQRT_2 * (i as f32 * 0.09).sin() } else { 0.0 }).collect()
}

fn whats(l: &Loop) -> Vec<String> {
    l.log.iter().map(|(_, w)| w.split(' ').take(2).collect::<Vec<_>>().join(" ")).collect()
}

#[test]
fn a_three_turn_call_with_a_barge_in_on_synthetic_audio() {
    let rms = |db: f32| 10f32.powf(db / 20.0);
    let says = vec![
        (When::At(1000), voice(rms(-20.0), 1500)),
        (When::Listening(2, 800), voice(rms(-20.0), 1200)),
        // Over the second answer, 1.2 s in, louder than its voice through the speakers.
        (When::Speaking(2, 1200), voice(rms(-14.0), 1400)),
    ];
    // Its voice comes back at about -30 dBFS.
    let echo = voice(rms(-30.0), 2600);
    let sim = simulate(Box::new(vad::Energy::default()), says, &echo, 4000, 30_000);
    let (l, st) = (&sim.l, sim.script.st.lock().unwrap());
    let log = whats(l);
    println!("{:#?}", l.log);
    assert_eq!(l.over(), Some(Why::Toggle), "{log:?}");
    assert_eq!(st.turns.len(), 3, "three turns asked: {log:?}");
    assert_eq!(st.cut.len(), 1, "only the barge-in stopped the voice, never its own echo: {log:?}");
    let order: Vec<&str> = log.iter().map(String::as_str).filter(|w| matches!(*w, "ask" | "speaking" | "stop speaking" | "listening")).collect();
    assert_eq!(order, ["ask", "speaking", "listening", "ask", "speaking", "stop speaking", "listening", "ask", "speaking", "listening"]);
    // The barge-in's recording starts about 150 ms before its first sound (the detector works in 32 ms windows).
    let (began, _) = st.turns[2];
    let lead_ms = (sim.said_at[2].unwrap() as i64 - began as i64) / MS as i64;
    assert!((150 - 32..=150 + 32).contains(&lead_ms), "pre-roll {lead_ms} ms");
    // Every turn's recording holds all of what was said in it.
    for (i, (began, held)) in st.turns.iter().enumerate() {
        let from = sim.said_at[i].unwrap();
        assert!(*began <= from && held.len() as u64 >= from - began + 1200 * MS as u64, "turn {i}");
    }
}

#[test]
fn its_voice_through_the_speakers_at_the_test_level_never_stops_it() {
    let rms = |db: f32| 10f32.powf(db / 20.0);
    // One question, then a long answer coming back loud (-24 dBFS) through the speakers, no one talking over it.
    let says = vec![(When::At(500), voice(rms(-20.0), 1000))];
    let echo = voice(rms(-24.0), 2600);
    let sim = simulate(Box::new(vad::Energy::default()), says, &echo, 8000, 13_000);
    let st = sim.script.st.lock().unwrap();
    assert!(st.cut.is_empty(), "{:#?}", sim.l.log);
    assert_eq!(st.turns.len(), 1);
    assert_eq!(sim.l.side(), Side::Listening, "it listened again once it had said all of it");
}

#[test]
fn forty_five_seconds_of_nobody_talking_ends_the_call_and_drops_the_empty_turn() {
    let sim = simulate(Box::new(vad::Energy::default()), vec![], &[], 1000, 60_000);
    assert_eq!(sim.l.over(), Some(Why::Quiet));
    let ended = sim.l.log.iter().find(|(_, w)| w.starts_with("call ends")).unwrap().0;
    assert!((machine::QUIET_END_MS..machine::QUIET_END_MS + 40).contains(&ended), "{ended}");
    assert_eq!(sim.script.st.lock().unwrap().dropped, 1);
}

/// Hand-run on a Mac, no microphone and no speakers: the real Silero VAD (downloaded once into a temp folder, pinned)
/// in the speech runtime this Mac already has, on voices made with `say -o` (written to files, never played).
/// `cargo test --lib call_through_silero_on_recorded_voices -- --ignored --nocapture`
#[cfg(target_os = "macos")]
#[test]
#[ignore = "downloads Silero VAD (2.3 MB) and needs the speech runtime this Mac downloaded"]
fn call_through_silero_on_recorded_voices() {
    use crate::voice::models;
    let dir = std::env::temp_dir().join("pointcast-call-handrun");
    std::fs::create_dir_all(&dir).unwrap();
    let wav = |name: &str, text: &str| -> Vec<f32> {
        let path = dir.join(format!("{name}.wav"));
        let ok = std::process::Command::new("say").args(["--data-format=LEI16@16000", "-o"]).arg(&path).arg(text).status().unwrap();
        assert!(ok.success());
        let (rate, ch, pcm) = audio::read_wav(&std::fs::read(&path).unwrap()).unwrap();
        let mut mono = vec![];
        audio::downmix(&pcm, ch, &mut mono);
        let mut out = vec![];
        audio::Resampler::new(rate).push(&mono, &mut out);
        out
    };
    let gain = |x: Vec<f32>, db: f32| -> Vec<f32> {
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs())).max(1e-6);
        let g = 10f32.powf(db / 20.0) / peak;
        x.into_iter().map(|v| v * g).collect()
    };
    let one = gain(wav("turn1", "What is the capital of France?"), -6.0);
    let two = gain(wav("turn2", "And how many people live there?"), -6.0);
    let three = gain(wav("barge", "Wait, stop. Tell me about Spain instead."), -3.0);
    // Its own voice reaching the microphone from the speakers, 18 dB under the person.
    let echo = gain(wav("answer", "Paris is the capital of France. It sits on the Seine, in the north of the country, and about two million people live in the city itself."), -24.0);

    let root = std::env::temp_dir().join("pointcast-call-vad");
    models::install_model(&root, &vad::entry(), &|_| {}).expect("Silero VAD downloads and matches its pin");
    // The app's own runtime, only read: every library it loads checked against its pin (`verify_runtime` would unmark
    // the install on a mismatch, and tests never touch the app's data).
    let speech = std::env::var_os("POINTCAST_SPEECH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dirs::data_local_dir().unwrap().join("dev.pointcast.app").join("speech"));
    let rt = models::catalog().runtime_for_host().cloned().unwrap();
    let dir = models::install_dir(&speech, &rt.id);
    for f in &rt.files {
        let p = f.name.split('/').fold(dir.clone(), |p, part| p.join(part));
        models::check(&p, f.bytes, &f.sha256).expect("the speech runtime this Mac downloaded, as pinned");
    }
    let lib = rt.files.iter().find(|f| f.name.ends_with(&rt.lib)).map(|f| f.name.split('/').fold(dir.clone(), |p, part| p.join(part))).unwrap();
    let silero = vad::Silero::open(&lib, &models::install_dir(&root, vad::ID).join(vad::FILE)).expect("Silero loads in sherpa-onnx 2.0");

    let lens = [one.len(), two.len(), three.len()].map(|n| n / MS);
    // `say` starts each file with a little silence: where its first sound is (over -40 dBFS).
    let sound = |x: &[f32]| x.iter().position(|v| v.abs() > 0.01).unwrap_or(0) as u64;
    let first = [sound(&one), sound(&two), sound(&three)];
    let says = vec![(When::At(1000), one), (When::Listening(2, 800), two), (When::Speaking(2, 1500), three)];
    let t = Instant::now();
    let sim = simulate(Box::new(silero), says, &echo, 5000, 40_000);
    println!("simulated {} s of call in {} ms", sim.l.log.last().unwrap().0 / 1000, t.elapsed().as_millis());
    println!("person said: {:?} (ms of audio each)", lens);
    for (i, at) in sim.said_at.iter().enumerate() {
        let at = at.map_or(-1, |a| ((a + first[i]) / MS as u64) as i64);
        println!("  turn {}: first sound at {at} ms of the call", i + 1);
    }
    for (at, what) in &sim.l.log {
        println!("{at:>6} ms  {what}");
    }
    let st = sim.script.st.lock().unwrap();
    for (i, (began, held)) in st.turns.iter().enumerate() {
        let lead = sim.said_at[i].map_or(0, |s| (s + first[i]) as i64 - *began as i64) / MS as i64;
        println!("turn {} recording: starts {lead} ms before its first sound, {} ms long", i + 1, held.len() / MS);
    }
    println!("voice stopped at {:?} ms (barge-ins)", st.cut);
    assert_eq!(st.turns.len(), 3, "three turns asked");
    assert_eq!(st.cut.len(), 1, "the echo never stopped it; the barge-in did");
    let lead = ((sim.said_at[2].unwrap() + first[2]) as i64 - st.turns[2].0 as i64) / MS as i64;
    assert!(lead >= 100, "the barge-in keeps its first word ({lead} ms of pre-roll)");
}
