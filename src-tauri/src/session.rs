//! Recording session: live transcript, captures inserted at the moment they were taken,
//! local STT, and export on stop.
use crate::capture::{self, Shot};
use crate::element::ElementMark;
use crate::keyframes::{Buffer, Still, EVERY_MS};
use crate::pointer::Point;
use crate::prompt::{self, Capture, CaptureKind, Item, Label, SessionExport};
use crate::settings::{MarkPoint, OutputMode, Settings, SpokenAnchors};
use crate::voice::audio::{self, Mic, Take};
use crate::voice::machine;
use crate::voice::models;
use crate::voice::engine::{self, Engine};
use crate::voice::stt::Heard;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Loading,
    Recording,
    Transcribing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize)]
pub struct UiState {
    pub phase: Phase,
    pub mic: bool,
    pub screen: bool,
    pub partial: String,
    pub text: String,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub image_count: u32,
    pub model_ready: bool,
    pub level: f32,
    /// After a stop, where the prompt went: pasted, copied, saved, kept or empty; "partial" when Cancel saved what was
    /// done and kept the audio, "cut" when it did with Keep audio off.
    pub notice: Option<&'static str>,
    /// Audio recorded and not yet transcribed, in ms; 0 when the model has caught up. A stop counts it down.
    pub behind_ms: u64,
    /// "silent" when the microphone gave essentially nothing for 5 s, "quiet" when voice comes through very low.
    pub quiet_mic: Option<String>,
    /// The model transcribes slower than real time on this computer.
    pub slow_engine: bool,
    /// While a stop waits for the last words (Transcribing): how far that is. None in every other phase.
    pub progress: Option<Shown>,
    /// The GPU engine's first start on this computer: the graphics card compiles its shaders (up to a minute or two).
    pub preparing_gpu: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            phase: Phase::Idle,
            mic: false,
            screen: false,
            partial: String::new(),
            text: String::new(),
            error: None,
            duration_ms: 0,
            image_count: 0,
            model_ready: false,
            level: 0.0,
            notice: None,
            behind_ms: 0,
            quiet_mic: None,
            slow_engine: false,
            progress: None,
            preparing_gpu: false,
        }
    }
}

pub struct App {
    pub settings: Mutex<Settings>,
    settings_path: std::path::PathBuf,
    items: Mutex<Vec<Item>>,
    t0: Mutex<Option<u64>>,
    state: Mutex<UiState>,
    take_no: AtomicU64,
    stop: AtomicBool,
    mic: Mutex<Option<Mic>>,
    take: Mutex<Option<Arc<Take>>>,
    engine: Mutex<Option<Engine>>,
    /// Whether the engine in memory was loaded with the GPU version of its model here, so one set up since is loaded.
    engine_gpu: AtomicBool,
    /// Moved on to stop the piece the engine is on, for an engine that can (the GPU helper); reachable while it works.
    aborter: Mutex<Option<Arc<AtomicU64>>>,
    /// Where the engine in memory runs, by model: "gpu", "cpu" or "unknown".
    backend: Mutex<Option<(String, &'static str)>>,
    /// One model load at a time (launch preload, a session, a model switch).
    loading: Mutex<()>,
    /// How the last attempt to load the model and run it went; none before the first.
    checked: Mutex<Option<Result<(), String>>>,
    /// Held for the whole of a stop, so a second stop or a Quit waits for it instead of racing it.
    finishing: Mutex<()>,
    /// Set by Discard: the worker skips transcribing what is left.
    cancel: AtomicBool,
    /// Milliseconds from the session clock's start to the first audio, added to speech times.
    audio_at: AtomicU64,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    /// Samples recorded and not yet transcribed.
    behind: AtomicU64,
    /// Where the worker is with the take: how far it had read when it last cut it into pieces, and the samples before
    /// that still to be transcribed (the pieces of that look not done yet, and the sentence still being said).
    waiting: Mutex<(usize, usize)>,
    /// The piece the worker is transcribing: its length and when it began on the steady clock, both in ms.
    piece: Mutex<Option<(u64, u64)>>,
    /// While a stop waits for the last words: how far that is.
    progress: Mutex<Option<Progress>>,
    /// Set by Cancel while a stop waits: it saves what is transcribed now.
    hurry: AtomicBool,
    /// This recording's audio on its way to the recovery area: what the worker read and has not written yet (it writes a
    /// second at a time), and how far into the take it read. None when the audio is not kept.
    unsaved: Mutex<Option<(Vec<f32>, usize)>>,
    /// This recording's audio transcribed so far and the time that took, both in ms.
    pace: Mutex<(u64, u64)>,
    /// A History entry is being transcribed again; one at a time.
    retranscribing: AtomicBool,
    /// Set by Cancel on a History entry being transcribed again: it stops after the piece under way.
    again_stop: AtomicBool,
    /// Audio transcribed and the time that took, in ms, per model over this run.
    paces: Mutex<std::collections::HashMap<String, (u64, u64)>>,
    pub history: crate::history::Store,
    /// Where the pointer went during this recording (positions only).
    trail: Mutex<crate::pointer::Trail>,
    /// When the model heard each token, by the start of its piece, for the pieces it gave times for.
    tokens: Mutex<Vec<(u64, Vec<crate::anchor::Token>)>>,
    /// The screen once a second during this recording, in memory only; none unless key frames are on.
    frames: Mutex<Option<Buffer>>,
    /// The history entry of the last finished prompt.
    pub last_id: Mutex<Option<String>>,
    /// What coding agents read over MCP: the last recording's prompt, until its History entry goes.
    pub latest: crate::mcp::Latest,
    freeze: Mutex<Option<Shot>>,
    /// When the region frame was frozen: the capture belongs there, not at the moment of Enter.
    freeze_t: Mutex<Option<u64>>,
    last_export: Mutex<Option<SessionExport>>,
    last_dir: Mutex<Option<std::path::PathBuf>>,
    /// This recording is onboarding's practice run: keep the result in the app, deliver it nowhere.
    pub practice: AtomicBool,
    /// Onboarding is on its practice step: the next recording started is a practice run.
    pub practice_next: AtomicBool,
    /// Onboarding shortcut test: hotkeys only report that they fired.
    pub hotkey_test: AtomicBool,
    /// When the session chord went down, for hold-to-talk.
    pub hold_from: Mutex<Option<u64>>,
    /// Chords the OS refused to register (`action`, reason).
    pub hotkey_errors: Mutex<Vec<(String, String)>>,
    /// Mic opened outside a session (onboarding level meter).
    pub monitor: Mutex<Option<(Mic, Arc<Take>)>>,
}

impl App {
    pub fn new(settings: Settings, settings_path: std::path::PathBuf) -> Arc<Self> {
        let model_ready = engine::usable(&models::speech_dir(), &models::catalog());
        Arc::new(App {
            settings: Mutex::new(settings),
            settings_path,
            items: Mutex::default(),
            t0: Mutex::default(),
            state: Mutex::new(UiState {
                model_ready,
                ..UiState::default()
            }),
            take_no: AtomicU64::new(0),
            stop: AtomicBool::new(true),
            mic: Mutex::default(),
            take: Mutex::default(),
            engine: Mutex::default(),
            engine_gpu: AtomicBool::new(false),
            aborter: Mutex::default(),
            backend: Mutex::default(),
            loading: Mutex::default(),
            checked: Mutex::default(),
            finishing: Mutex::default(),
            cancel: AtomicBool::new(false),
            audio_at: AtomicU64::new(0),
            worker: Mutex::default(),
            behind: AtomicU64::new(0),
            waiting: Mutex::default(),
            piece: Mutex::default(),
            progress: Mutex::default(),
            hurry: AtomicBool::new(false),
            unsaved: Mutex::default(),
            pace: Mutex::default(),
            retranscribing: AtomicBool::new(false),
            again_stop: AtomicBool::new(false),
            paces: Mutex::default(),
            freeze: Mutex::default(),
            freeze_t: Mutex::default(),
            last_export: Mutex::default(),
            last_dir: Mutex::default(),
            history: crate::history::Store::new(&crate::settings::data_dir()),
            trail: Mutex::default(),
            tokens: Mutex::default(),
            frames: Mutex::default(),
            last_id: Mutex::default(),
            latest: Default::default(),
            practice: AtomicBool::new(false),
            practice_next: AtomicBool::new(false),
            hotkey_test: AtomicBool::new(false),
            hold_from: Mutex::default(),
            hotkey_errors: Mutex::default(),
            monitor: Mutex::default(),
        })
    }

    pub fn ui(&self) -> UiState {
        let mut s = self.state.lock().unwrap().clone();
        if let Some(t) = self.take.lock().unwrap().as_ref() {
            s.level = t.level();
        }
        if let Some(t0) = *self.t0.lock().unwrap() {
            s.duration_ms = now_ms().saturating_sub(t0);
        }
        s.behind_ms = self.behind.load(Ordering::SeqCst) * 1000 / audio::RATE as u64;
        let (heard, spent) = *self.pace.lock().unwrap();
        s.slow_engine = slow(heard, spent);
        s.progress = match s.phase {
            Phase::Transcribing => self.progress.lock().unwrap().as_mut().map(|p| p.view(steady_ms(), now_ms())),
            _ => None,
        };
        s.text = prompt::compose(&self.items.lock().unwrap()).0;
        s.image_count = self
            .items
            .lock()
            .unwrap()
            .iter()
            .filter(|i| matches!(i, Item::Image { .. }))
            .count() as u32;
        s
    }

    pub fn last_export(&self) -> Option<SessionExport> {
        self.last_export.lock().unwrap().clone()
    }

    pub fn last_dir(&self) -> Option<std::path::PathBuf> {
        self.last_dir.lock().unwrap().clone()
    }

    pub fn save_settings(&self, s: Settings) -> Result<(), String> {
        s.save(&self.settings_path)?;
        *self.settings.lock().unwrap() = s;
        Ok(())
    }

    /// Whether the speech engine last loaded and ran here, or why not.
    pub fn speech_check(&self) -> Option<Result<(), String>> {
        self.checked.lock().unwrap().clone()
    }

    /// Load the chosen model unless it is already the one in memory.
    pub fn ensure_engine(&self) -> Result<(), String> {
        let r = self.load_engine();
        *self.checked.lock().unwrap() = Some(r.clone());
        r
    }

    fn load_engine(&self) -> Result<(), String> {
        let _one = self.loading.lock().unwrap();
        let root = models::speech_dir();
        let cat = models::catalog();
        if cat.runtime_for_host().is_none() && cat.helper_for_host().is_none() {
            return Err("Speech isn't available for this computer yet.".into());
        }
        let installed = engine::installed(&root, &cat);
        let chosen = self.settings.lock().unwrap().model.clone();
        let id = machine::route(&chosen, machine::recommended(), &installed).ok_or(NO_MODEL)?;
        let model = cat.model(id).ok_or(NO_MODEL)?;
        let gpu = engine::gpu_ready(&root, &cat, model).is_some();
        let same = |e: &Engine| e.model() == id && self.engine_gpu.load(Ordering::SeqCst) == gpu && e.failure().is_none();
        if self.engine.lock().unwrap().as_ref().is_some_and(same) {
            self.state.lock().unwrap().model_ready = true;
            return Ok(());
        }
        // The one in memory goes first: a GPU helper frees the graphics card before another starts.
        self.engine.lock().unwrap().take();
        let threads = std::thread::available_parallelism().map(|n| sherpa_threads(n.get())).unwrap_or(2);
        let preparing = |on: bool| self.state.lock().unwrap().preparing_gpu = on;
        let eng = engine::load(&root, &cat, model, threads, &preparing)?;
        self.engine_gpu.store(gpu, Ordering::SeqCst);
        *self.aborter.lock().unwrap() = eng.aborter();
        *self.backend.lock().unwrap() = Some((id.to_string(), eng.backend()));
        *self.engine.lock().unwrap() = Some(eng);
        self.state.lock().unwrap().model_ready = true;
        Ok(())
    }

    /// Where the engine in memory runs, by model, for Settings: "gpu", "cpu" or "unknown".
    pub fn backends(&self) -> std::collections::HashMap<String, &'static str> {
        self.backend.lock().unwrap().iter().cloned().collect()
    }

    /// The engine heard a piece: note where it runs now, and say it when it can hear nothing any more.
    fn after_piece(&self, eng: &Engine) -> Option<String> {
        *self.backend.lock().unwrap() = Some((eng.model().to_string(), eng.backend()));
        let failed = eng.failure();
        if let Some(e) = &failed {
            self.state.lock().unwrap().error = Some(e.clone());
        }
        failed
    }

    /// Stop the piece the engine is on, where it can stop part way (the GPU helper closes its request).
    fn abort_piece(&self) {
        if let Some(a) = self.aborter.lock().unwrap().as_ref() {
            a.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Let the engine go at quit, so a GPU helper ends with Pointcast on every system.
    pub fn release_engine(&self) {
        if let Ok(mut g) = self.engine.try_lock() {
            g.take();
        }
    }

    /// Free the model's memory while nothing is recording or transcribed again; the next recording loads it again
    /// (its audio waits in the take meanwhile, so nothing is lost).
    pub fn unload_engine(&self) {
        if matches!(self.state.lock().unwrap().phase, Phase::Recording | Phase::Loading | Phase::Transcribing) {
            return;
        }
        let _one = self.loading.lock().unwrap();
        if !self.retranscribing.load(Ordering::SeqCst) {
            *self.engine.lock().unwrap() = None;
            *self.aborter.lock().unwrap() = None;
            *self.backend.lock().unwrap() = None;
        }
    }

    /// Decode half a second of silence: proves the runtime and model work on this machine.
    pub fn engine_check(&self) -> Result<(), String> {
        self.ensure_engine()?;
        let g = self.engine.lock().unwrap();
        let eng = g.as_ref().ok_or(NO_MODEL)?;
        let _ = eng.transcribe(&vec![0.0f32; 8_000]);
        *self.checked.lock().unwrap() = Some(Ok(()));
        Ok(())
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        match self.state.lock().unwrap().phase {
            Phase::Recording | Phase::Loading => return Ok(()),
            Phase::Transcribing => return Err(BUSY.into()),
            _ => {}
        }
        self.monitor_off();
        self.bind_practice();
        // A checkpoint still here was never recovered: keep it in History rather than mixing it in.
        if self.history.recover().is_none() {
            self.history.clear_checkpoint();
        }
        self.cancel.store(false, Ordering::SeqCst);
        self.stop.store(false, Ordering::SeqCst);
        self.items.lock().unwrap().clear();
        *self.t0.lock().unwrap() = Some(now_ms());
        {
            let mut st = self.state.lock().unwrap();
            st.phase = Phase::Loading;
            st.error = None;
            st.partial.clear();
            st.notice = None;
            st.text.clear();
            st.mic = false;
            st.screen = true;
        }
        let settings = self.settings.lock().unwrap().clone();
        let take = Arc::new(Take::default());
        match Mic::open(settings.device.clone(), take.clone()) {
            Ok(mic) => {
                let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
                self.audio_at.store(now_ms().saturating_sub(t0), Ordering::SeqCst);
                *self.mic.lock().unwrap() = Some(mic);
                *self.take.lock().unwrap() = Some(take.clone());
                self.state.lock().unwrap().mic = true;
            }
            Err(e) => {
                let mut st = self.state.lock().unwrap();
                st.error = Some(format!("Microphone: {e}"));
            }
        }
        // The worker loads the model if it is not in memory yet; audio waits in the take meanwhile. A model busy with a
        // piece of a History entry transcribed again is in memory: the start never waits for that piece.
        if self.engine.try_lock().map_or(true, |e| e.is_some()) {
            self.state.lock().unwrap().phase = Phase::Recording;
        }
        let no = self.take_no.fetch_add(1, Ordering::SeqCst) + 1;
        self.trail.lock().unwrap().clear();
        self.tokens.lock().unwrap().clear();
        // Nothing of the last recording's audio is written for this one; its worker says what it keeps.
        *self.unsaved.lock().unwrap() = None;
        let me = Arc::clone(self);
        std::thread::spawn(move || me.follow_pointer(no));
        self.keep_frames(&settings, no, capture::grab_key);
        let me = Arc::clone(self);
        *self.worker.lock().unwrap() = Some(std::thread::spawn(move || me.worker(no, take)));
        Ok(())
    }

    /// Only the recording started while onboarding asked for practice is a practice run.
    fn bind_practice(&self) {
        self.practice.store(self.practice_next.swap(false, Ordering::SeqCst), Ordering::SeqCst);
    }

    /// Onboarding left its practice step. True when the practice run is still recording and must be
    /// discarded; one already finishing stays practice until its result is shown.
    pub fn practice_off(&self) -> bool {
        self.practice_next.store(false, Ordering::SeqCst);
        match self.state.lock().unwrap().phase {
            Phase::Recording | Phase::Loading => self.practice.load(Ordering::SeqCst),
            Phase::Transcribing => false,
            _ => {
                self.practice.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    pub fn stop(&self, cancel: bool) -> Result<Option<SessionExport>, String> {
        let _one = self.finishing.lock().unwrap();
        if !matches!(self.state.lock().unwrap().phase, Phase::Recording | Phase::Loading) {
            return Ok(None);
        }
        self.cancel.store(cancel, Ordering::SeqCst);
        self.stop.store(true, Ordering::SeqCst);
        // A Cancel can only be meant for this stop once it waits.
        self.hurry.store(false, Ordering::SeqCst);
        // The frames leave memory with this; only the ones picked below live on, as pictures.
        let frames = self.frames.lock().unwrap().take();
        if let Some(mic) = self.mic.lock().unwrap().take() {
            drop(mic);
        }
        if !cancel {
            self.track();
        }
        {
            let mut st = self.state.lock().unwrap();
            st.mic = false;
            st.screen = false;
            st.phase = if cancel { Phase::Idle } else { Phase::Transcribing };
        }
        // The worker transcribes what was still being said when stop was pressed; wait for it, unless Cancel cuts the
        // wait short. A cut that left nothing undone is a stop like any other.
        let worker = self.worker.lock().unwrap().take();
        let cut = worker.is_some_and(|w| !self.wait_for(w));
        let partial = cut && !self.progress.lock().unwrap().as_ref().is_some_and(Progress::complete);
        if cut {
            self.cut();
        }
        if let Some(p) = self.progress.lock().unwrap().as_mut() {
            p.finish();
        }
        *self.take.lock().unwrap() = None;
        if cancel {
            self.items.lock().unwrap().clear();
            *self.t0.lock().unwrap() = None;
            self.history.clear_checkpoint();
            self.state.lock().unwrap().phase = Phase::Idle;
            return Ok(None);
        }
        let settings = self.settings.lock().unwrap().clone();
        let mut items = prompt::ordered(self.items.lock().unwrap().clone());
        let trail = self.trail.lock().unwrap().since(0);
        let tokens = self.tokens.lock().unwrap().clone();
        if let Some(frames) = frames {
            crate::keyframes::add(&mut items, &frames, &trail, &tokens, settings.mark_point);
        }
        if settings.spoken_anchors == SpokenAnchors::Ring {
            crate::anchor::ring_spoken(&mut items, &trail, &tokens, settings.mark_point != MarkPoint::Off);
        }
        if self.practice.load(Ordering::SeqCst) {
            self.history.clear_checkpoint();
            self.state.lock().unwrap().phase = Phase::Ready;
            return Ok(Some(prompt::assemble_export(&items)));
        }
        // The checkpoint goes only once History has the prompt: a failed save leaves it for
        // the next launch to recover. A recording that came back with no words keeps its audio there, to transcribe
        // again; only one with neither is let go.
        let mut saved = None;
        let mut has_audio = false;
        if items.is_empty() && !self.history.kept_audio() {
            self.history.clear_checkpoint();
        } else {
            let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
            let save = if partial { crate::history::Store::save_partial } else { crate::history::Store::save_recorded };
            if let Ok(m) = save(&self.history, &items, t0, now_ms().saturating_sub(t0)) {
                *self.last_id.lock().unwrap() = Some(m.id.clone());
                has_audio = m.has_audio;
                saved = Some(m.id);
                self.history.clear_checkpoint();
            }
        }
        // Saved, the prompt can name its pictures' files for agents in a terminal; unsaved, it marks them as before.
        let files = match &saved {
            Some(id) if settings.picture_paths => self.history.files(id),
            _ => vec![],
        };
        let exp = prompt::assemble_export_files(&items, &files);
        *self.last_export.lock().unwrap() = Some(exp.clone());
        let audio_kept = saved.is_some();
        self.latest.set(saved, exp.clone());
        // History keeps every prompt; a folder in Documents only when the user chose Save.
        if settings.output == OutputMode::Folder {
            let dir = export_dir(&settings, &chrono::Local::now().format("%Y%m%d-%H%M%S").to_string());
            let _ = std::fs::create_dir_all(&dir);
            if let Ok(p) = prompt::export_folder(&items, &dir) {
                *self.last_dir.lock().unwrap() = Some(p.parent().unwrap_or(&dir).to_path_buf());
            }
        }
        let mode = paste_or_copy(settings.output, crate::permissions::can_paste());
        let delivered = crate::clipboard::deliver(&items, &files, mode, settings.also_clipboard);
        let mut st = self.state.lock().unwrap();
        st.notice = Some(notice(&items, mode, delivered.is_ok(), audio_kept, partial.then_some(has_audio)));
        st.phase = Phase::Ready;
        Ok(Some(exp))
    }

    /// Start counting how far this stop is: the audio the worker had not transcribed yet, what it had not read, and the
    /// piece it is on.
    fn track(&self) {
        let len = self.take.lock().unwrap().as_ref().map_or(0, |t| t.len());
        let (looked, left) = *self.waiting.lock().unwrap();
        let waiting = (left + len.saturating_sub(looked)) as u64 * 1000 / audio::RATE as u64;
        // The model's pace on this recording, or over this run when it has transcribed nothing of it yet.
        let pace = match *self.pace.lock().unwrap() {
            (0, _) => self.paces.lock().unwrap().values().fold((0, 0), |a, p| (a.0 + p.0, a.1 + p.1)),
            p => p,
        };
        let mut g = self.progress.lock().unwrap();
        let mut p = Progress::new(waiting, pace, now_ms());
        if let Some((len, at)) = *self.piece.lock().unwrap() {
            p.begin(len, at);
        }
        *g = Some(p);
    }

    /// Wait for the worker to finish what is left to transcribe. False when Cancel cut the wait short first: the worker
    /// is left to end the piece it is on, as the engine cannot be stopped part way through one.
    fn wait_for(&self, w: std::thread::JoinHandle<()>) -> bool {
        while !w.is_finished() {
            if self.hurry.load(Ordering::SeqCst) {
                return false;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = w.join();
        true
    }

    /// Let this recording's worker go: from here on it adds nothing (checked under the items' lock, so the copy the stop
    /// takes next is final), writes no audio and leaves the HUD alone. The audio it had not written yet goes to the
    /// recovery area now, so the recording keeps all of it.
    fn cut(&self) {
        let mut unsaved = self.unsaved.lock().unwrap();
        {
            let _items = self.items.lock().unwrap();
            self.take_no.fetch_add(1, Ordering::SeqCst);
        }
        self.behind.store(0, Ordering::SeqCst);
        *self.waiting.lock().unwrap() = (0, 0);
        *self.piece.lock().unwrap() = None;
        // Its words would land nowhere: an engine that can stops the piece it is on now.
        self.abort_piece();
        self.state.lock().unwrap().partial.clear();
        if let (Some((mut held, read)), Some(take)) = (unsaved.take(), self.take.lock().unwrap().clone()) {
            held.extend(take.read_from(read));
            let _ = self.history.append_audio(&held);
        }
    }

    /// Cancel while a stop waits for the last words: it saves now with what is transcribed, and keeps the audio so
    /// History can transcribe the rest. Returns once that is saved; with no stop waiting, at once.
    pub fn finish_now(&self) {
        if self.state.lock().unwrap().phase == Phase::Transcribing {
            self.hurry.store(true, Ordering::SeqCst);
        }
        drop(self.finishing.lock().unwrap());
    }

    /// Quitting mid-recording: finish transcribing into the checkpoint and leave it there; the
    /// next launch turns it into a history entry.
    pub fn flush_for_quit(&self) {
        let _one = self.finishing.lock().unwrap();
        if !matches!(self.state.lock().unwrap().phase, Phase::Recording | Phase::Loading) {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
        self.frames.lock().unwrap().take();
        self.mic.lock().unwrap().take();
        if let Some(w) = self.worker.lock().unwrap().take() {
            let _ = w.join();
        }
    }

    /// Open the mic without a session; `level()` reads it.
    pub fn monitor_on(&self) -> Result<(), String> {
        let mut m = self.monitor.lock().unwrap();
        if m.is_some() {
            return Ok(());
        }
        let device = self.settings.lock().unwrap().device.clone();
        let take = Arc::new(Take::default());
        let mic = Mic::open(device, take.clone())?;
        *m = Some((mic, take));
        Ok(())
    }

    pub fn monitor_off(&self) {
        self.monitor.lock().unwrap().take();
    }

    pub fn monitor_level(&self) -> Option<f32> {
        self.monitor.lock().unwrap().as_ref().map(|(_, t)| t.level())
    }

    pub fn inject_speech(&self, text: &str) {
        let Some(t0) = *self.t0.lock().unwrap() else {
            return;
        };
        let t = now_ms().saturating_sub(t0);
        let _ = self.push(Item::Speech {
            t_ms: t.saturating_sub(400),
            end_ms: t,
            text: text.to_string(),
        });
    }

    pub fn capture_full(&self) -> Result<Capture, String> {
        self.capture_shot(capture::grab_full)
    }

    pub fn capture_zoom(&self) -> Result<Capture, String> {
        self.capture_shot(capture::grab_zoom)
    }

    /// A capture the OS refused (Screen Recording off) is said on the HUD, not dropped.
    fn said<T>(&self, r: Result<T, String>) -> Result<T, String> {
        if let Err(e) = &r {
            self.state.lock().unwrap().error = Some(e.clone());
        }
        r
    }

    /// A recording that may not start says why on the HUD, as a stop that kept nothing does. One already running is
    /// left alone.
    pub fn refuse(&self, why: &str) {
        let mut st = self.state.lock().unwrap();
        if !matches!(st.phase, Phase::Recording | Phase::Loading | Phase::Transcribing) {
            st.phase = Phase::Ready;
            st.notice = Some("empty");
            st.error = Some(why.to_string());
        }
    }

    pub fn begin_region(&self) -> Result<Shot, String> {
        let settings = self.settings.lock().unwrap().clone();
        let t = self.t_ms();
        let shot = self.said(capture::grab_freeze(&settings, &self.recent(t), t))?;
        *self.freeze.lock().unwrap() = Some(shot.clone());
        *self.freeze_t.lock().unwrap() = Some(t);
        Ok(shot)
    }

    pub fn freeze(&self) -> Option<Shot> {
        self.freeze.lock().unwrap().clone()
    }

    /// Commit an annotated PNG (already cropped by the overlay) as the next image, with the control the person pointed
    /// at when they kept its outline.
    pub fn commit_region(&self, png: Vec<u8>, labels: Vec<Label>, element: Option<ElementMark>) -> Result<Capture, String> {
        let (width, height) = crate::image_ops::png_size(&png).ok_or("that is not a PNG")?;
        let t = self.freeze_t.lock().unwrap().take().unwrap_or_else(|| self.t_ms());
        let cap = Capture {
            id: 0,
            kind: CaptureKind::Region,
            t_ms: t,
            png,
            width,
            height,
            labels,
            pointer: None,
            trail: vec![],
            element,
            place: None,
            anchors: vec![],
            auto: false,
        };
        self.push(Item::Image {
            t_ms: t,
            capture: cap.clone(),
        })?;
        *self.freeze.lock().unwrap() = None;
        Ok(cap)
    }

    fn capture_shot(
        &self,
        grab: impl FnOnce(&Settings, &[Point], u64) -> Result<Shot, String>,
    ) -> Result<Capture, String> {
        let settings = self.settings.lock().unwrap().clone();
        let asked = self.t_ms();
        let shot = self.said(grab(&settings, &self.recent(asked), asked))?;
        let t = self.t_ms();
        let place = Some(shot.place());
        let cap = Capture {
            id: 0,
            kind: shot.kind,
            t_ms: t,
            png: shot.png,
            width: shot.width,
            height: shot.height,
            labels: vec![],
            pointer: shot.pointer,
            trail: shot.trail,
            element: shot.element,
            place,
            anchors: vec![],
            auto: false,
        };
        self.push(Item::Image {
            t_ms: t,
            capture: cap.clone(),
        })?;
        Ok(cap)
    }

    /// Sample the pointer about 30 times a second for as long as this recording runs.
    fn follow_pointer(self: Arc<Self>, no: u64) {
        while !self.stop.load(Ordering::SeqCst) && self.take_no.load(Ordering::SeqCst) == no {
            if let Ok((x, y)) = capture::cursor_pos() {
                let t_ms = self.t_ms();
                self.trail.lock().unwrap().push(crate::pointer::Point { t_ms, x, y });
            }
            std::thread::sleep(std::time::Duration::from_millis(33));
        }
    }

    /// With key frames on, keep the pointer's monitor once a second in memory for this recording, through `grab`. Off,
    /// no buffer is made and no thread runs. Returns whether it started.
    fn keep_frames(self: &Arc<Self>, settings: &Settings, no: u64, grab: fn(&Settings, u64) -> Result<Still, String>) -> bool {
        *self.frames.lock().unwrap() = None;
        if !settings.key_frames {
            return false;
        }
        *self.frames.lock().unwrap() = Some(Buffer::default());
        let (me, settings) = (Arc::clone(self), settings.clone());
        let live = move |a: &App| !a.stop.load(Ordering::SeqCst) && a.take_no.load(Ordering::SeqCst) == no;
        std::thread::spawn(move || {
            while live(&me) {
                let began = std::time::Instant::now();
                if let Ok(s) = grab(&settings, me.t_ms()) {
                    if let Some(b) = me.frames.lock().unwrap().as_mut().filter(|_| live(&me)) {
                        b.push(s);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(EVERY_MS).saturating_sub(began.elapsed()));
            }
        });
        true
    }

    /// The last few seconds of the pointer's path before `t`, in screen coordinates.
    fn recent(&self, t: u64) -> Vec<Point> {
        self.trail.lock().unwrap().since(t.saturating_sub(crate::pointer::TRAIL_MS))
    }

    fn t_ms(&self) -> u64 {
        let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
        now_ms().saturating_sub(t0)
    }

    /// Add to the recording and checkpoint it, so a crash from here on loses nothing. Only while it records: a picture
    /// that arrived after a stop copied the recording would be checkpointed into a session already saved, and come back
    /// at the next launch as a second, "Recovered" entry.
    fn push(&self, item: Item) -> Result<(), String> {
        self.add(item, &[Phase::Recording, Phase::Loading])
    }

    /// Add to the recording while its phase is one of `live`. The phase is read under the items' lock, so an item is
    /// either in the copy a stop takes or refused.
    fn add(&self, item: Item, live: &[Phase]) -> Result<(), String> {
        self.add_to(self.items.lock().unwrap(), item, live)
    }

    /// What the worker of recording `no` heard, with when the model heard each of its tokens. Refused, tokens and all, once
    /// that recording is no longer the current one: a stop cut short let it go, so a piece it finishes late lands neither
    /// in the saved recording nor in the next one.
    fn add_heard(&self, no: u64, item: Item, tokens: Option<(u64, Vec<crate::anchor::Token>)>) -> Result<(), String> {
        let v = self.items.lock().unwrap();
        if self.take_no.load(Ordering::SeqCst) != no {
            return Err(STOPPED.into());
        }
        if let Some(t) = tokens {
            self.tokens.lock().unwrap().push(t);
        }
        // The words still being transcribed for a stop go in too: the stop waits for them before it copies.
        self.add_to(v, item, &[Phase::Recording, Phase::Loading, Phase::Transcribing])
    }

    fn add_to(&self, mut v: MutexGuard<'_, Vec<Item>>, item: Item, live: &[Phase]) -> Result<(), String> {
        if !live.contains(&self.state.lock().unwrap().phase) {
            return Err(STOPPED.into());
        }
        v.push(item);
        if !self.practice.load(Ordering::SeqCst) {
            let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
            let _ = self.history.checkpoint(&v, t0);
        }
        Ok(())
    }

    /// Make a history entry the one "Paste Last Prompt" uses. When coding agents read that entry, they read it as it
    /// is now (after a picture in it was edited).
    pub fn set_last(&self, id: &str) -> Result<SessionExport, String> {
        let exp = self.history.export_with(id, self.settings.lock().unwrap().picture_paths)?;
        *self.last_export.lock().unwrap() = Some(exp.clone());
        *self.last_id.lock().unwrap() = Some(id.to_string());
        self.latest.changed(id, &exp);
        Ok(exp)
    }

    /// History lost entries (`gone`, or all of them when None). When the last prompt was one, the newest entry left
    /// takes its place, or nothing does; coding agents read nothing rather than an older one. True when the last
    /// prompt changed.
    pub fn forget(&self, gone: Option<&str>) -> bool {
        self.latest.forget(gone);
        let hit = gone.is_none_or(|id| self.last_id.lock().unwrap().as_deref() == Some(id));
        if !hit {
            return false;
        }
        let next = self.history.list().into_iter().next();
        if next.and_then(|m| self.set_last(&m.id).ok()).is_none() {
            *self.last_export.lock().unwrap() = None;
            *self.last_id.lock().unwrap() = None;
        }
        true
    }

    /// Transcribe a History entry again from its audio, on the model chosen now and the way a recording is transcribed.
    /// Its words are replaced and its pictures stay where they were; the last prompt and what coding agents read follow
    /// when it is theirs. `progress(done_ms, total_ms)` as it goes. Refused while a recording runs, and stopped if one
    /// starts; a refusal or failure leaves the entry as it was.
    pub fn retranscribe(&self, id: &str, progress: impl FnMut(u64, u64)) -> Result<(), String> {
        let hear = |s: &[f32], voiced: u64| -> Result<Option<Heard>, String> {
            let g = self.engine.lock().unwrap();
            let eng = g.as_ref().ok_or(NO_MODEL)?;
            let began = std::time::Instant::now();
            let heard = crate::voice::stt::hear_piece(s, voiced, |p| eng.hear(p));
            self.timed(eng.model(), s.len() as u64 * 1000 / audio::RATE as u64, began.elapsed().as_millis() as u64);
            // An engine that stopped working would leave the entry without words: it stays as it was.
            match self.after_piece(eng) {
                Some(e) => Err(e),
                None => Ok(heard),
            }
        };
        self.again(id, || self.ensure_engine(), hear, progress)
    }

    /// `model` transcribed `heard_ms` of audio in `spent_ms`.
    fn timed(&self, model: &str, heard_ms: u64, spent_ms: u64) {
        let mut paces = self.paces.lock().unwrap();
        let p = paces.entry(model.to_string()).or_default();
        (p.0, p.1) = (p.0 + heard_ms, p.1 + spent_ms);
    }

    /// For each model this run has heard enough from, whether it transcribes slower than real time on this computer.
    pub fn slow_models(&self) -> std::collections::HashMap<String, bool> {
        let paces = self.paces.lock().unwrap();
        paces.iter().filter(|(_, p)| p.0 >= JUDGED_MS).map(|(m, p)| (m.clone(), slow(p.0, p.1))).collect()
    }

    /// `retranscribe` with the model loaded by `load` and each piece heard through `hear`.
    fn again(
        &self,
        id: &str,
        load: impl FnOnce() -> Result<(), String>,
        mut hear: impl FnMut(&[f32], u64) -> Result<Option<Heard>, String>,
        progress: impl FnMut(u64, u64),
    ) -> Result<(), String> {
        let busy = || match self.state.lock().unwrap().phase {
            Phase::Recording | Phase::Loading => Err(RECORDING_NOW),
            Phase::Transcribing => Err(BUSY),
            _ => Ok(()),
        };
        busy()?;
        if self.retranscribing.swap(true, Ordering::SeqCst) {
            return Err(AGAIN_BUSY.into());
        }
        self.again_stop.store(false, Ordering::SeqCst);
        let stopped = || self.again_stop.load(Ordering::SeqCst);
        let run = || -> Result<(), String> {
            let mut x = self.history.audio(id)?;
            load()?;
            let piece = |s: &[f32], voiced: u64| match busy() {
                Ok(()) if stopped() => Err(AGAIN_CANCELLED.into()),
                Ok(()) => hear(s, voiced),
                Err(_) => Err(INTERRUPTED.into()),
            };
            let boost = self.settings.lock().unwrap().voice_boost;
            let said = transcribe_all(&mut x, boost, piece, progress)?;
            if stopped() {
                return Err(AGAIN_CANCELLED.into());
            }
            self.history.replace_speech(id, &said)?;
            let exp = self.history.export_with(id, self.settings.lock().unwrap().picture_paths)?;
            if self.last_id.lock().unwrap().as_deref() == Some(id) {
                *self.last_export.lock().unwrap() = Some(exp.clone());
            }
            self.latest.changed(id, &exp);
            Ok(())
        };
        let r = run();
        self.again_stop.store(false, Ordering::SeqCst);
        self.retranscribing.store(false, Ordering::SeqCst);
        r
    }

    /// Cancel on a History entry being transcribed again: it stops after the piece under way and the entry stays as it
    /// was. False when none is being transcribed again.
    pub fn again_cancel(&self) -> bool {
        let on = self.retranscribing.load(Ordering::SeqCst);
        if on {
            self.again_stop.store(true, Ordering::SeqCst);
            self.abort_piece();
        }
        on
    }

    fn worker(self: Arc<Self>, no: u64, take: Arc<Take>) {
        // The audio goes to the recovery area about once a second, from the moment the microphone opened: a crash keeps
        // it, and History can transcribe it again. Never for a practice run, nor with Keep audio off. Held from before
        // the model loads, so a stop cut short meanwhile still keeps all of it.
        let keep = self.settings.lock().unwrap().keep_audio && !self.practice.load(Ordering::SeqCst);
        let lead = self.audio_at.load(Ordering::SeqCst) as usize * audio::RATE as usize / 1000;
        {
            let mut u = self.unsaved.lock().unwrap();
            if self.take_no.load(Ordering::SeqCst) == no {
                *u = keep.then(|| (vec![0.0; lead], 0));
            }
        }
        *self.waiting.lock().unwrap() = (0, 0);
        if let Err(e) = self.ensure_engine() {
            // Screenshots still work; the transcript stays empty.
            self.state.lock().unwrap().error = Some(e);
        }
        {
            let mut st = self.state.lock().unwrap();
            if st.phase == Phase::Loading {
                st.phase = Phase::Recording;
            }
        }
        let mut seg = segmenter();
        // Only what arrived since the last look is copied out of the take, so a long recording costs no more per
        // look than a short one and the microphone never waits behind a copy of all of it.
        let mut pos = 0;
        // The voice detector and the model hear the take through the front end when it is on; the kept audio stays as
        // it came. Only the part the pieces still to come are cut from is held.
        let mut boost = self.settings.lock().unwrap().voice_boost.then(audio::Boost::default);
        let mut heard = audio::Tail::default();
        *self.pace.lock().unwrap() = (0, 0);
        self.state.lock().unwrap().quiet_mic = None;
        loop {
            if self.stop.load(Ordering::SeqCst) || self.take_no.load(Ordering::SeqCst) != no {
                break;
            }
            let mut new = take.read_from(pos);
            pos += new.len();
            self.keep_audio(no, &new, false);
            if let Some(b) = boost.as_mut() {
                b.process(&mut new);
            }
            heard.push(&new);
            let ranges = seg.feed_new(&new);
            let open = seg.open_start().map_or(0, |s| pos.saturating_sub(s));
            self.wait_on(no, Some((pos, open + ranges.iter().map(|r| r.len()).sum::<usize>())), 0);
            let note = match boost.as_ref() {
                Some(b) => audio::boosted_note(&seg.recent(), seg.floor(), b.silent()),
                None => audio::mic_note(&seg.recent(), seg.floor()),
            };
            self.state.lock().unwrap().quiet_mic = note.map(String::from);
            self.decode_ranges(no, &take, &heard, &ranges, seg.floor(), pos);
            // A live caption only when one is due and nothing waits to be transcribed: the last few seconds of the open
            // sentence.
            let due = self.engine.lock().unwrap().as_ref().is_some_and(|e| e.wants_partial());
            if let Some(start) = seg.open_start().filter(|_| due && ranges.is_empty() && take.len() - pos < audio::RATE as usize / 2) {
                let samples = heard.copy(start.max(pos.saturating_sub(PARTIAL_SPAN))..pos);
                let live = self.engine.lock().unwrap().as_ref().and_then(|e| e.partial(&samples));
                if let Some(t) = live.filter(|_| self.take_no.load(Ordering::SeqCst) == no) {
                    self.state.lock().unwrap().partial = t;
                }
            }
            // A stop cut short while the model was busy: nothing of this recording goes on the HUD any more.
            if self.take_no.load(Ordering::SeqCst) != no {
                break;
            }
            heard.drop_before(seg.keep_from());
            self.behind.store(take.len().saturating_sub(pos) as u64, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        // A stop cut short let this recording go: what is left of it waits in History, to transcribe again.
        if self.take_no.load(Ordering::SeqCst) != no {
            return;
        }
        if self.cancel.load(Ordering::SeqCst) {
            self.state.lock().unwrap().partial.clear();
            return;
        }
        let mut new = take.read_from(pos);
        self.keep_audio(no, &new, true);
        if let Some(b) = boost.as_mut() {
            b.process(&mut new);
        }
        heard.push(&new);
        let mut rest = seg.feed_new(&new);
        rest.extend(seg.close());
        if let Some(p) = self.progress.lock().unwrap().as_mut().filter(|_| self.take_no.load(Ordering::SeqCst) == no) {
            p.rest(rest.iter().map(|r| r.len() as u64 * 1000 / audio::RATE as u64).sum());
        }
        self.decode_ranges(no, &take, &heard, &rest, seg.floor(), pos + new.len());
        if self.take_no.load(Ordering::SeqCst) == no {
            self.behind.store(0, Ordering::SeqCst);
            self.state.lock().unwrap().partial.clear();
        }
    }

    /// Hold `new` from the take of recording `no` for the recovery area, and write what is held once there is a second of
    /// it, or at once with `all`. Nothing once that recording is no longer the current one: a stop cut short wrote the rest.
    fn keep_audio(&self, no: u64, new: &[f32], all: bool) {
        let mut u = self.unsaved.lock().unwrap();
        let Some((held, read)) = u.as_mut().filter(|_| self.take_no.load(Ordering::SeqCst) == no) else {
            return;
        };
        held.extend_from_slice(new);
        *read += new.len();
        if all || held.len() >= audio::RATE as usize {
            let _ = self.history.append_audio(&std::mem::take(held));
        }
    }

    /// Transcribe pieces of the take for recording `no`, cut from what the worker `heard` against a room at `floor`; the
    /// worker has read the take up to `pos`. Once that recording is no longer the current one, the pieces left are not.
    fn decode_ranges(&self, no: u64, take: &Take, heard: &audio::Tail, ranges: &[std::ops::Range<usize>], floor: f32, pos: usize) {
        let mine = || self.take_no.load(Ordering::SeqCst) == no;
        for (i, r) in ranges.iter().enumerate() {
            if !mine() {
                return;
            }
            let waiting: usize = ranges[i..].iter().map(|r| r.len()).sum();
            self.behind.store((take.len().saturating_sub(pos) + waiting) as u64, Ordering::SeqCst);
            let samples = heard.copy(r.clone());
            let voiced = audio::voiced_ms(&samples, floor);
            let heard_ms = r.len() as u64 * 1000 / audio::RATE as u64;
            self.under_way(no, Some(heard_ms), 0);
            let began = std::time::Instant::now();
            let mut model = None;
            let heard = {
                let g = self.engine.lock().unwrap();
                g.as_ref().and_then(|eng| {
                    model = Some(eng.model().to_string());
                    let h = crate::voice::stt::hear_piece(&samples, voiced, |s| eng.hear(s));
                    self.after_piece(eng);
                    h
                })
            };
            let spent_ms = began.elapsed().as_millis() as u64;
            if let Some(m) = model {
                self.timed(&m, heard_ms, spent_ms);
            }
            // Cut short while this piece was heard: its words, its pace and its place on the HUD belong to no recording.
            if !mine() {
                return;
            }
            self.under_way(no, None, spent_ms);
            self.wait_on(no, None, r.len());
            {
                let mut p = self.pace.lock().unwrap();
                p.0 += heard_ms;
                p.1 += spent_ms;
            }
            let Some(heard) = heard else { continue };
            let text = heard.text;
            let at = self.audio_at.load(Ordering::SeqCst);
            let t_ms = at + (r.start as u64) * 1000 / audio::RATE as u64;
            let end_ms = at + (r.end as u64) * 1000 / audio::RATE as u64;
            let _ = self.add_heard(no, Item::Speech { t_ms, end_ms, text }, heard.tokens.map(|tok| (t_ms, tok)));
        }
    }

    /// The worker of recording `no` cut what it read up to a point into pieces (`look`: that point, and the samples
    /// before it still to transcribe), or finished `done` samples of them. Only while that recording is the current one.
    fn wait_on(&self, no: u64, look: Option<(usize, usize)>, done: usize) {
        let mut w = self.waiting.lock().unwrap();
        if self.take_no.load(Ordering::SeqCst) == no {
            *w = look.unwrap_or((w.0, w.1.saturating_sub(done)));
        }
    }

    /// The worker of recording `no` starts a piece of `len_ms` (Some), or has finished the one it was on in `spent_ms`
    /// (None): a stop waiting for it counts it.
    fn under_way(&self, no: u64, len_ms: Option<u64>, spent_ms: u64) {
        let mut g = self.progress.lock().unwrap();
        let mut piece = self.piece.lock().unwrap();
        if self.take_no.load(Ordering::SeqCst) != no {
            return;
        }
        let was = std::mem::replace(&mut *piece, len_ms.map(|len| (len, steady_ms())));
        match (g.as_mut(), *piece, was) {
            (Some(p), Some((len, at)), _) => p.begin(len, at),
            (Some(p), None, Some((len, _))) => p.end(len, spent_ms),
            _ => {}
        }
    }
}

/// One new folder per recording, named by when it was saved, in the chosen folder or Documents › Pointcast.
fn export_dir(s: &Settings, stamp: &str) -> std::path::PathBuf {
    let base = match &s.export_dir {
        Some(d) => std::path::PathBuf::from(d),
        None => dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("Pointcast"),
    };
    std::iter::once(base.join(stamp))
        .chain((2..).map(|n| base.join(format!("{stamp}-{n}"))))
        .find(|p| !p.exists())
        .unwrap_or_default()
}

/// Auto-paste needs the OS to accept synthetic keys; where it will not (macOS without
/// Accessibility), the prompt is copied instead of silently going nowhere.
pub fn paste_or_copy(mode: OutputMode, can_paste: bool) -> OutputMode {
    if mode == OutputMode::AutoPaste && !can_paste {
        OutputMode::Clipboard
    } else {
        mode
    }
}

/// What the HUD says after a stop. `cut` when Cancel saved it before every word was in: whether its audio was kept.
pub fn notice(items: &[Item], mode: OutputMode, delivered: bool, audio_kept: bool, cut: Option<bool>) -> &'static str {
    match (items.is_empty(), mode, delivered) {
        (true, _, _) if audio_kept => "kept",
        (true, _, _) => "empty",
        _ if cut == Some(true) => "partial",
        _ if cut == Some(false) => "cut",
        (_, OutputMode::AutoPaste, true) => "pasted",
        (_, OutputMode::Folder, _) => "saved",
        _ => "copied",
    }
}

/// Threads for sherpa on the processor: as many as there are cores, up to 8.
pub fn sherpa_threads(cores: usize) -> i32 {
    cores.clamp(1, 8) as i32
}

/// A model's pace is judged once this much audio (5 s) has been transcribed.
const JUDGED_MS: u64 = 5_000;

/// Slower than real time, judged once 5 s of audio has been transcribed.
pub fn slow(heard_ms: u64, spent_ms: u64) -> bool {
    heard_ms >= JUDGED_MS && spent_ms > heard_ms
}

/// A stop's wait counts as stalled once nothing it shows has moved for this long (20 s).
const STALL_MS: u64 = 20_000;

/// How far a stop is with the last words, as the HUD shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Shown {
    pub done_ms: u64,
    pub total_ms: u64,
    /// 0 to 100, never back, and 100 only once every word is in.
    pub percent: u8,
    /// Nothing moved for 20 s while there is still audio to transcribe.
    pub stalled: bool,
    /// About how long the rest takes at the model's pace, once that pace is measured on 5 s of audio; none before
    /// that, nor while stalled.
    pub left_ms: Option<u64>,
}

/// A stop's wait for the last words: the audio still to transcribe when stop was pressed, the pieces finished since, and
/// the piece under way, credited at the model's measured pace up to nine tenths of its share. The caller gives the
/// clocks: a steady one for that credit, and the wall clock only to tell a stall, so a computer that slept never moves
/// the bar.
#[derive(Debug, Clone, Default)]
pub struct Progress {
    total_ms: u64,
    done_ms: u64,
    /// The piece under way: its length and when it began on the steady clock.
    piece: Option<(u64, u64)>,
    /// Audio the model transcribed and the time that took, both in ms.
    pace: (u64, u64),
    /// The most shown so far: what is shown never goes back.
    shown_ms: u64,
    percent: u8,
    /// When, on the wall clock, what is shown last moved.
    moved_at: u64,
    finished: bool,
}

impl Progress {
    pub fn new(total_ms: u64, pace: (u64, u64), wall_ms: u64) -> Self {
        Progress { total_ms, pace, moved_at: wall_ms, ..Default::default() }
    }

    /// A piece of `len_ms` began at `at_ms` on the steady clock.
    pub fn begin(&mut self, len_ms: u64, at_ms: u64) {
        self.piece = Some((len_ms, at_ms));
    }

    /// A piece of `len_ms` is done; it took `spent_ms`.
    pub fn end(&mut self, len_ms: u64, spent_ms: u64) {
        self.piece = None;
        self.done_ms += len_ms;
        self.pace = (self.pace.0 + len_ms, self.pace.1 + spent_ms);
    }

    /// What is left is now known exactly: `ms` of pieces.
    pub fn rest(&mut self, ms: u64) {
        self.total_ms = self.done_ms + ms;
    }

    /// Every word is in.
    pub fn finish(&mut self) {
        self.finished = true;
    }

    /// Every piece counted so far is done.
    pub fn complete(&self) -> bool {
        self.done_ms >= self.total_ms
    }

    pub fn view(&mut self, steady_ms: u64, wall_ms: u64) -> Shown {
        let credit = match self.piece {
            Some((len, at)) if self.pace.0 > 0 && self.pace.1 > 0 => (steady_ms.saturating_sub(at) * self.pace.0 / self.pace.1).min(len * 9 / 10),
            _ => 0,
        };
        let total = self.total_ms;
        let done = if self.finished { total } else { (self.done_ms + credit).min(total) };
        let percent = match (self.finished, total) {
            (true, _) => 100,
            (_, 0) => 0,
            _ => (done * 100 / total).min(99) as u8,
        };
        // A wall clock set back counts as a move, so a change of time never reads as a stall.
        if done > self.shown_ms || percent > self.percent || wall_ms < self.moved_at {
            self.moved_at = wall_ms;
        }
        self.shown_ms = self.shown_ms.max(done);
        self.percent = self.percent.max(percent);
        let stalled = !self.finished && !self.complete() && wall_ms.saturating_sub(self.moved_at) >= STALL_MS;
        let left = total.saturating_sub(self.shown_ms);
        let left_ms = (self.pace.0 >= JUDGED_MS && !stalled && !self.finished).then(|| left * self.pace.1 / self.pace.0);
        Shown { done_ms: self.shown_ms, total_ms: total.max(self.shown_ms), percent: self.percent, stalled, left_ms }
    }
}

/// Milliseconds on a clock that only goes forward, from the first time it is read.
fn steady_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// A live caption covers at most this many samples (8 s) of the sentence being spoken.
const PARTIAL_SPAN: usize = audio::RATE as usize * 8;

/// Pieces end at a 700 ms pause and stay under Whisper's 30 s window, live and when a recording is transcribed again.
fn segmenter() -> audio::Segmenter {
    audio::Segmenter::new(700, 25_000)
}

/// About what the worker reads at each look (120 ms); a recording transcribed again is fed to the segmenter in these.
const LOOK: usize = audio::RATE as usize * 120 / 1000;

/// A whole recording, 16 kHz mono from the session's start, transcribed as it was live: through the front end when
/// `boost` is on (in place), cut by the same segmenter, fed in the same small steps, each piece with how long it is voice
/// against the room's level then, through `hear` (which may refuse and so stop it). Its words at their times in the
/// session. `progress(done_ms, total_ms)` before each piece, done being where that piece starts, and at the end with done
/// the total.
pub fn transcribe_all(
    x: &mut [f32],
    boost: bool,
    mut hear: impl FnMut(&[f32], u64) -> Result<Option<Heard>, String>,
    mut progress: impl FnMut(u64, u64),
) -> Result<Vec<Item>, String> {
    let ms = |n: usize| n as u64 * 1000 / audio::RATE as u64;
    // The silence kept for the moment before the microphone opened: the worker never heard it.
    let from = x.iter().position(|v| *v != 0.0).unwrap_or(x.len());
    if boost {
        audio::Boost::default().process(&mut x[from..]);
    }
    let x = &*x;
    let mut seg = segmenter();
    let mut pieces = vec![];
    for step in x[from..].chunks(LOOK) {
        let got = seg.feed_new(step);
        pieces.extend(got.into_iter().map(|r| (r.start + from..r.end + from, seg.floor())));
    }
    let last = seg.close();
    pieces.extend(last.into_iter().map(|r| (r.start + from..r.end + from, seg.floor())));
    let total = ms(x.len());
    let mut said = vec![];
    for (r, floor) in pieces {
        progress(ms(r.start), total);
        let samples = &x[r.start.min(x.len())..r.end.min(x.len())];
        if let Some(h) = hear(samples, audio::voiced_ms(samples, floor))? {
            said.push(Item::Speech { t_ms: ms(r.start), end_ms: ms(r.end), text: h.text });
        }
    }
    progress(total, total);
    Ok(said)
}

pub const BUSY: &str = "Pointcast is still finishing the last recording.";

pub const RECORDING_NOW: &str = "Pointcast is recording. Transcribe again once the recording has finished.";

pub const AGAIN_BUSY: &str = "Pointcast is already transcribing a recording again.";

pub const INTERRUPTED: &str = "A recording started, so transcribing again stopped. Try again once it has finished.";

pub const AGAIN_CANCELLED: &str = "Transcribing again was cancelled. The prompt is as it was.";

pub const UPDATING: &str = "Pointcast is updating; it reopens in a moment.";

pub const STOPPED: &str = "That recording has already stopped.";

pub const NO_MODEL: &str = "No speech model yet. Open Settings › Speech to download one.";

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(name: &str) -> Arc<App> {
        let d = std::env::temp_dir().join(format!("pc-session-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let mut a = App::new(Settings::default(), d.join("settings.json"));
        // History and the recovery checkpoint live in the test's own folder, never the real app's.
        Arc::get_mut(&mut a).expect("not shared yet").history = crate::history::Store::new(&d);
        // A practice run checkpoints nothing and delivers nowhere: a stop never pastes into the desktop.
        a.practice.store(true, Ordering::SeqCst);
        a
    }

    fn png() -> Vec<u8> {
        crate::image_ops::encode_png(&[9, 9, 9, 255], 1, 1).unwrap()
    }

    #[test]
    fn a_region_belongs_to_the_moment_the_screen_froze_not_to_enter() {
        let a = app("freeze");
        // Recording started 20 s ago; the frame froze 12 s in; Enter comes 8 s later.
        *a.t0.lock().unwrap() = Some(now_ms() - 20_000);
        a.state.lock().unwrap().phase = Phase::Recording;
        *a.freeze_t.lock().unwrap() = Some(12_000);
        let cap = a.commit_region(png(), vec![], None).unwrap();
        assert_eq!(cap.t_ms, 12_000);
        // The freeze time is used once; a commit without a freeze falls back to now.
        let cap = a.commit_region(png(), vec![], None).unwrap();
        assert!(cap.t_ms >= 20_000, "{}", cap.t_ms);
    }

    #[test]
    fn a_region_keeps_the_control_it_outlined_and_the_prompt_names_it() {
        let a = app("element");
        *a.t0.lock().unwrap() = Some(now_ms());
        a.state.lock().unwrap().phase = Phase::Recording;
        let rect = crate::element::Bounds { x: 4, y: 6, w: 116, h: 41 };
        let mark = ElementMark { role: "button".into(), name: "Place order".into(), rect };
        let pin = vec![Label { n: 1, text: "too far left".into() }];
        let cap = a.commit_region(png(), pin, Some(mark.clone())).unwrap();
        assert_eq!(cap.element, Some(mark));
        assert_eq!(a.ui().text, r#"[image 1: button "Place order"; 1=too far left]"#);
        // Deleted in the overlay: nothing to name.
        a.commit_region(png(), vec![], None).unwrap();
        assert_eq!(a.ui().text, r#"[image 1: button "Place order"; 1=too far left] [image 2]"#);
    }

    /// The colour at `(x, y)` of an exported picture.
    fn pixel(img: &prompt::SessionImage, x: u32, y: u32) -> [u8; 3] {
        let b64 = img.data_url.trim_start_matches("data:image/png;base64,");
        let png = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
        let (buf, w, _) = crate::image_ops::decode_rgba(&png).unwrap();
        let i = ((y * w + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2]]
    }

    #[test]
    fn at_stop_saying_this_rings_where_the_pointer_was_on_the_nearest_picture() {
        let a = app("spoken");
        *a.t0.lock().unwrap() = Some(now_ms());
        let map = capture::Mapping { origin: (0, 0), scale: 1.0, frame: (400, 300), crop: (0, 0), size: (400, 300) };
        let shot = Capture {
            id: 0,
            kind: CaptureKind::Full,
            t_ms: 3_000,
            png: crate::image_ops::encode_png(&vec![255u8; 400 * 300 * 4], 400, 300).unwrap(),
            width: 400,
            height: 300,
            labels: vec![],
            pointer: None,
            trail: vec![],
            element: None,
            place: Some(capture::Place { map, k: 1.0 }),
            anchors: vec![],
            auto: false,
        };
        // "this" is said at about 1.6 s while the pointer sweeps right, passing (100, 80) then.
        a.state.lock().unwrap().phase = Phase::Recording;
        a.push(Item::Speech { t_ms: 1_000, end_ms: 2_700, text: "click this button".into() }).unwrap();
        a.push(Item::Image { t_ms: 3_000, capture: shot }).unwrap();
        for i in 0..15 {
            a.trail.lock().unwrap().push(Point { t_ms: 1_000 + i * 100, x: 40 + i as i32 * 10, y: 80 });
        }
        let on_ring = 100 + crate::image_ops::RING_R as u32;
        let exp = a.stop(false).unwrap().unwrap();
        assert_eq!(pixel(&exp.images[0], on_ring, 80), [245, 165, 36]);
        assert_eq!(pixel(&exp.images[0], 100, 80), [255; 3]);
        assert_eq!(exp.plain, "click this button [image 1]", "the words are untouched");
        // Turned off, the same recording comes out as it was taken.
        a.settings.lock().unwrap().spoken_anchors = SpokenAnchors::Off;
        a.state.lock().unwrap().phase = Phase::Recording;
        let exp = a.stop(false).unwrap().unwrap();
        assert_eq!(pixel(&exp.images[0], on_ring, 80), [255; 3]);
    }

    #[test]
    fn a_region_that_arrives_after_the_stop_is_refused_and_never_recovered_as_a_second_entry() {
        let a = app("late");
        // A real recording, so what goes into it is checkpointed.
        a.practice.store(false, Ordering::SeqCst);
        *a.t0.lock().unwrap() = Some(now_ms());
        let words = || Item::Speech { t_ms: 0, end_ms: 9, text: "last words".into() };
        let shot = |_: &Settings, _: &[Point], _: u64| -> Result<Shot, String> {
            let map = Default::default();
            Ok(Shot { png: png(), width: 1, height: 1, kind: CaptureKind::Full, cursor: (0, 0), pointer: None, trail: vec![], element: None, map, k: 1.0 })
        };
        for phase in [Phase::Transcribing, Phase::Ready, Phase::Idle] {
            a.state.lock().unwrap().phase = phase;
            assert_eq!(a.commit_region(png(), vec![], None).unwrap_err(), STOPPED, "{phase:?}");
            assert_eq!(a.capture_shot(shot).unwrap_err(), STOPPED, "{phase:?}");
            assert_eq!(a.push(words()).unwrap_err(), STOPPED, "{phase:?}");
            a.inject_speech("late");
            assert!(a.history.recover().is_none(), "{phase:?}: nothing was checkpointed");
        }
        assert_eq!(a.ui().image_count, 0);
        // The words still being transcribed for the stop go in.
        a.state.lock().unwrap().phase = Phase::Transcribing;
        a.add(words(), &[Phase::Recording, Phase::Loading, Phase::Transcribing]).unwrap();
        a.history.clear_checkpoint();
        // While it records, a region goes in and is checkpointed.
        a.state.lock().unwrap().phase = Phase::Recording;
        a.commit_region(png(), vec![], None).unwrap();
        assert!(a.history.recover().is_some());
    }

    #[test]
    fn a_stop_with_nothing_recording_saves_nothing() {
        let a = app("idle-stop");
        *a.t0.lock().unwrap() = Some(now_ms());
        a.state.lock().unwrap().phase = Phase::Recording;
        a.push(Item::Speech { t_ms: 0, end_ms: 600, text: "already saved".into() }).unwrap();
        a.state.lock().unwrap().phase = Phase::Ready;
        assert!(a.stop(false).unwrap().is_none(), "a second stop does nothing");
        assert!(a.history.list().is_empty(), "and saves nothing twice");
        a.state.lock().unwrap().phase = Phase::Transcribing;
        assert_eq!(a.start().unwrap_err(), BUSY, "a start while finishing is refused");
    }

    static NEVER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    /// Stands in for the screen: a key frame must never come from the real one in a test.
    fn never(_: &Settings, _: u64) -> Result<Still, String> {
        NEVER.fetch_add(1, Ordering::SeqCst);
        Err("grabbed with key frames off".into())
    }

    fn white(_: &Settings, t: u64) -> Result<Still, String> {
        let map = capture::Mapping { origin: (0, 0), scale: 1.0, frame: (400, 300), crop: (0, 0), size: (400, 300) };
        crate::keyframes::still(vec![255u8; 400 * 300 * 4], 400, 300, map, 1.0, t)
    }

    #[test]
    fn with_key_frames_off_no_buffer_is_made_and_the_screen_is_never_grabbed() {
        let a = app("frames-off");
        *a.t0.lock().unwrap() = Some(now_ms());
        a.stop.store(false, Ordering::SeqCst);
        a.take_no.store(1, Ordering::SeqCst);
        assert!(!a.keep_frames(&Settings::default(), 1, never));
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(a.frames.lock().unwrap().is_none());
        assert_eq!(NEVER.load(Ordering::SeqCst), 0);
        a.state.lock().unwrap().phase = Phase::Recording;
        a.push(Item::Speech { t_ms: 0, end_ms: 600, text: "look at this".into() }).unwrap();
        assert!(a.stop(false).unwrap().unwrap().images.is_empty());
    }

    #[test]
    fn with_key_frames_on_the_screen_is_kept_while_recording_and_dropped_at_stop() {
        let a = app("frames-on");
        *a.t0.lock().unwrap() = Some(now_ms());
        a.stop.store(false, Ordering::SeqCst);
        a.take_no.store(1, Ordering::SeqCst);
        let on = Settings { key_frames: true, ..Settings::default() };
        assert!(a.keep_frames(&on, 1, white));
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while a.frames.lock().unwrap().as_ref().is_some_and(|b| b.frames().is_empty()) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(a.frames.lock().unwrap().as_ref().is_some_and(|b| !b.frames().is_empty()));
        // "this" at 400 ms: the first frame stands for it.
        a.state.lock().unwrap().phase = Phase::Recording;
        a.push(Item::Speech { t_ms: 0, end_ms: 600, text: "look at this".into() }).unwrap();
        let exp = a.stop(false).unwrap().unwrap();
        assert!(a.frames.lock().unwrap().is_none(), "the buffer goes at stop");
        assert_eq!((exp.images.len(), exp.images[0].auto, exp.images[0].width), (1, true, 400));
        assert_eq!(exp.plain, "look at [image 1] this");
        // A new recording starts with an empty buffer; one with the setting off has none at all.
        a.stop.store(false, Ordering::SeqCst);
        a.take_no.store(2, Ordering::SeqCst);
        assert!(!a.keep_frames(&Settings::default(), 2, never));
        assert!(a.frames.lock().unwrap().is_none());
        a.stop.store(true, Ordering::SeqCst);
    }

    #[test]
    fn the_last_engine_check_is_remembered_so_ready_is_not_claimed_after_a_failure() {
        let a = app("check");
        assert!(a.speech_check().is_none(), "nothing tried yet");
        // The test's data folder has no model, so the engine cannot start.
        let r = a.engine_check();
        assert!(r.is_err());
        assert_eq!(a.speech_check(), Some(r));
    }

    #[test]
    fn practice_is_only_the_recording_started_while_onboarding_asked_for_it() {
        let a = app("practice");
        a.practice.store(false, Ordering::SeqCst);
        a.practice_next.store(true, Ordering::SeqCst);
        a.bind_practice();
        assert!(a.practice.load(Ordering::SeqCst) && !a.practice_next.load(Ordering::SeqCst));
        a.bind_practice();
        assert!(!a.practice.load(Ordering::SeqCst), "after one practice run the next recording is a real one");
        // Back or "Download one" mid-run: the run is discarded rather than delivered as a real one.
        a.practice_next.store(true, Ordering::SeqCst);
        a.bind_practice();
        a.state.lock().unwrap().phase = Phase::Recording;
        assert!(a.practice_off());
        // A run already finishing stays practice until its result is shown.
        a.state.lock().unwrap().phase = Phase::Transcribing;
        assert!(!a.practice_off() && a.practice.load(Ordering::SeqCst));
        a.state.lock().unwrap().phase = Phase::Ready;
        assert!(!a.practice_off() && !a.practice.load(Ordering::SeqCst));
        // A real recording is never turned into one, nor discarded.
        a.state.lock().unwrap().phase = Phase::Recording;
        assert!(!a.practice_off());
    }

    #[test]
    fn deleting_the_last_prompt_or_clearing_history_forgets_it() {
        let a = app("forget");
        let said = |t: &str| vec![Item::Speech { t_ms: 0, end_ms: 9, text: t.into() }];
        let old = a.history.save(&said("older"), 1_000, 9).unwrap();
        let new = a.history.save(&said("newest"), 2_000, 9).unwrap();
        a.set_last(&new.id).unwrap();
        // Another entry goes: the last prompt stays.
        a.history.delete(&old.id).unwrap();
        assert!(!a.forget(Some(&old.id)));
        assert_eq!(a.last_export().unwrap().plain, "newest");
        let old = a.history.save(&said("older"), 1_000, 9).unwrap();
        // The last one goes: the newest left takes its place.
        a.history.delete(&new.id).unwrap();
        assert!(a.forget(Some(&new.id)));
        assert_eq!((a.last_export().unwrap().plain.as_str(), a.last_id.lock().unwrap().clone()), ("older", Some(old.id)));
        // Clear: nothing is left to paste.
        a.history.clear().unwrap();
        assert!(a.forget(None));
        assert!(a.last_export().is_none() && a.last_id.lock().unwrap().is_none());
    }

    #[test]
    fn coding_agents_read_their_own_history_entry_until_it_is_deleted_or_edited() {
        let a = app("agents");
        let said = |t: &str| vec![Item::Speech { t_ms: 0, end_ms: 9, text: t.into() }];
        let served = |a: &App| a.latest.get().map(|e| e.plain);
        let pic = Capture {
            id: 0,
            kind: CaptureKind::Region,
            t_ms: 10,
            png: png(),
            width: 1,
            height: 1,
            labels: vec![],
            pointer: None,
            trail: vec![],
            element: None,
            place: None,
            anchors: vec![],
            auto: false,
        };
        // At launch the newest entry is the last prompt, and agents read nothing until a recording.
        let older = a.history.save(&said("older"), 1_000, 9).unwrap();
        let newest = a.history.save(&said("newest"), 2_000, 9).unwrap();
        a.set_last(&newest.id).unwrap();
        a.history.delete(&newest.id).unwrap();
        assert!(a.forget(Some(&newest.id)));
        assert_eq!((a.last_export().unwrap().plain.as_str(), served(&a)), ("older", None), "deleting it serves no older one");

        // A recording: agents read it, as the stop saved it.
        let items = vec![Item::Speech { t_ms: 0, end_ms: 9, text: "look".into() }, Item::Image { t_ms: 10, capture: pic }];
        let rec = a.history.save(&items, 3_000, 9).unwrap();
        a.latest.set(Some(rec.id.clone()), a.set_last(&rec.id).unwrap());
        // A picture in it marked up again: agents read the new one.
        a.history.replace(&rec.id, 1, &png(), vec![Label { n: 1, text: "this one".into() }]).unwrap();
        a.set_last(&rec.id).unwrap();
        // Their plain text names the picture's file in History, and ends with its folder.
        let file = a.history.files(&rec.id)[0].clone();
        let folder = std::path::Path::new(&file).parent().unwrap().to_string_lossy().into_owned();
        let named = format!("look [image 1: {file}; 1=this one]\n\nPictures: {folder}");
        assert_eq!(served(&a), Some(named.clone()));
        // Copying the older prompt makes it the last one; agents still read the recording.
        a.set_last(&older.id).unwrap();
        assert_eq!(served(&a), Some(named));
        // The recording deleted while it is not the last prompt: agents no longer read it.
        a.history.delete(&rec.id).unwrap();
        assert!(!a.forget(Some(&rec.id)));
        assert_eq!(served(&a), None);

        // Clearing History: nothing left to read.
        let again = a.history.save(&said("again"), 4_000, 9).unwrap();
        a.latest.set(Some(again.id.clone()), a.set_last(&again.id).unwrap());
        a.history.delete(&older.id).unwrap();
        a.forget(Some(&older.id));
        assert_eq!(served(&a).as_deref(), Some("again"), "another entry going leaves it");
        a.history.clear().unwrap();
        a.forget(None);
        assert_eq!(served(&a), None);
    }

    #[test]
    fn each_saved_recording_gets_its_own_folder_in_a_chosen_folder_too() {
        let root = std::env::temp_dir().join(format!("pc-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mine = Settings { export_dir: Some(root.to_string_lossy().into_owned()), ..Settings::default() };
        let first = export_dir(&mine, "20261029-101500");
        assert_eq!(first, root.join("20261029-101500"));
        prompt::export_folder(&[Item::Speech { t_ms: 0, end_ms: 9, text: "one".into() }], &first).unwrap();
        // A second recording in the same second never writes over the first one's prompt.md.
        let second = export_dir(&mine, "20261029-101500");
        assert_eq!(second, root.join("20261029-101500-2"));
        assert!(export_dir(&Settings::default(), "20261029-101501").ends_with("Pointcast/20261029-101501"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_capture_the_os_refuses_is_said_on_the_hud() {
        let a = app("refused");
        *a.t0.lock().unwrap() = Some(now_ms());
        let off = |_: &Settings, _: &[Point], _: u64| -> Result<Shot, String> { Err("Screen Recording is off for Pointcast.".into()) };
        assert!(a.capture_shot(off).is_err());
        assert_eq!(a.ui().error.as_deref(), Some("Screen Recording is off for Pointcast."));
        assert_eq!(a.ui().image_count, 0);
    }

    #[test]
    fn a_recording_refused_while_updating_says_why_on_the_hud_and_a_running_one_is_left_alone() {
        let a = app("refuse");
        a.state.lock().unwrap().notice = Some("pasted");
        a.refuse(UPDATING);
        let st = a.ui();
        assert_eq!((st.phase, st.notice, st.error.as_deref()), (Phase::Ready, Some("empty"), Some(UPDATING)));
        a.state.lock().unwrap().phase = Phase::Recording;
        a.state.lock().unwrap().error = None;
        a.refuse(UPDATING);
        assert_eq!((a.ui().phase, a.ui().error), (Phase::Recording, None));
    }

    #[test]
    fn the_hud_says_where_the_prompt_went() {
        let words = vec![Item::Speech { t_ms: 0, end_ms: 10, text: "hi".into() }];
        assert_eq!(notice(&[], OutputMode::AutoPaste, true, false, None), "empty");
        assert_eq!(notice(&[], OutputMode::AutoPaste, true, true, None), "kept", "no words, but the audio waits in History");
        assert_eq!(notice(&words, OutputMode::AutoPaste, true, true, None), "pasted");
        assert_eq!(notice(&words, OutputMode::AutoPaste, false, true, None), "copied");
        assert_eq!(notice(&words, OutputMode::Clipboard, true, true, None), "copied");
        assert_eq!(notice(&words, OutputMode::Folder, true, true, None), "saved");
        // Cancel saved what was done: the rest waits in History when the audio is kept, and is lost when it is not.
        assert_eq!(notice(&words, OutputMode::AutoPaste, true, true, Some(true)), "partial");
        assert_eq!(notice(&words, OutputMode::Folder, true, true, Some(false)), "cut");
        assert_eq!(notice(&[], OutputMode::AutoPaste, true, true, Some(true)), "kept");
    }

    #[test]
    fn without_permission_to_paste_the_prompt_is_copied() {
        assert_eq!(paste_or_copy(OutputMode::AutoPaste, false), OutputMode::Clipboard);
        assert_eq!(paste_or_copy(OutputMode::AutoPaste, true), OutputMode::AutoPaste);
        assert_eq!(paste_or_copy(OutputMode::Folder, false), OutputMode::Folder);
    }

    #[test]
    fn a_long_recording_is_read_once_and_the_microphone_never_waits_behind_the_worker() {
        let a = app("linear");
        *a.t0.lock().unwrap() = Some(now_ms());
        a.state.lock().unwrap().phase = Phase::Recording;
        a.stop.store(false, Ordering::SeqCst);
        a.take_no.store(1, Ordering::SeqCst);
        // Five minutes of 2 s sounds a second apart, fed about 100 times faster than real time in 10 ms pieces.
        let n = audio::RATE as usize * 300;
        let x: Vec<f32> = (0..n).map(|i| if i % 48_000 < 32_000 { 0.2 * (i as f32 * 0.1).sin() } else { 0.0005 }).collect();
        let take = Arc::new(Take::default());
        // Room for all of it from the start: only the worker can make the microphone wait.
        take.samples.lock().unwrap().reserve(n);
        let worker = {
            let (me, take) = (a.clone(), take.clone());
            std::thread::spawn(move || me.worker(1, take))
        };
        let mut blocked = std::time::Duration::ZERO;
        for (i, piece) in x.chunks(160).enumerate() {
            let t = std::time::Instant::now();
            take.samples.lock().unwrap().extend_from_slice(piece);
            blocked = blocked.max(t.elapsed());
            if i % 10 == 9 {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        a.stop.store(true, Ordering::SeqCst);
        worker.join().unwrap();
        let copied = take.copied.load(Ordering::Relaxed);
        assert!(copied <= 3 * n as u64 * 4, "{} MB copied out of {} MB of audio", copied / 1_000_000, n * 4 / 1_000_000);
        assert!(blocked < std::time::Duration::from_millis(20), "the microphone waited {blocked:?}");
    }

    #[test]
    fn the_hud_says_how_far_behind_transcription_is_and_whether_the_model_keeps_up() {
        let a = app("behind");
        let take = Take::default();
        take.samples.lock().unwrap().extend(vec![0.0f32; 48_000]);
        let mut heard = audio::Tail::default();
        heard.push(&[0.0f32; 40_000]);
        // Read to 40 000; two pieces of 8 000 and 4 000 wait (no model here, so each is let go at once).
        a.decode_ranges(0, &take, &heard, &[0..8_000, 8_000..12_000], 0.0, 40_000);
        assert_eq!(a.behind.load(Ordering::SeqCst), 4_000 + 8_000, "the last piece and what is unread");
        assert_eq!(a.ui().behind_ms, 750);
        assert!(!slow(4_000, 9_000), "too little heard to judge");
        assert!(slow(6_000, 6_001) && !slow(6_000, 6_000) && !slow(60_000, 20_000));
        *a.pace.lock().unwrap() = (10_000, 25_000);
        assert!(a.ui().slow_engine);
    }

    #[test]
    fn each_model_is_judged_slow_or_not_over_everything_it_transcribed_this_run() {
        let a = app("paces");
        a.timed("whisper-turbo", 4_000, 7_000);
        a.timed("whisper-turbo", 4_000, 2_000);
        a.timed("moonshine-base-en", 6_000, 900);
        a.timed("moonshine-tiny-en", 4_000, 9_000);
        let want = [("whisper-turbo".to_string(), true), ("moonshine-base-en".to_string(), false)];
        assert_eq!(a.slow_models(), want.into_iter().collect(), "8 s in 9 s is slow; 4 s is too little to judge");
    }

    #[test]
    fn the_recording_audio_is_kept_as_it_comes_unless_turned_off_or_practice() {
        for (keep, practice, kept) in [(true, false, true), (false, false, false), (true, true, false)] {
            let a = app(&format!("keep-{keep}-{practice}"));
            a.practice.store(practice, Ordering::SeqCst);
            a.settings.lock().unwrap().keep_audio = keep;
            *a.t0.lock().unwrap() = Some(now_ms());
            a.state.lock().unwrap().phase = Phase::Recording;
            a.stop.store(false, Ordering::SeqCst);
            a.take_no.store(1, Ordering::SeqCst);
            let take = Arc::new(Take::default());
            let worker = {
                let (me, take) = (a.clone(), take.clone());
                std::thread::spawn(move || me.worker(1, take))
            };
            take.samples.lock().unwrap().extend(vec![0.1f32; 24_000]);
            std::thread::sleep(std::time::Duration::from_millis(400));
            if kept {
                // A crash now: what came in so far is already in the recovery area.
                let m = a.history.recover().expect("the audio so far is kept");
                assert!(m.has_audio && a.history.audio(&m.id).unwrap().len() >= 16_000);
            }
            take.samples.lock().unwrap().extend(vec![0.1f32; 8_000]);
            a.stop.store(true, Ordering::SeqCst);
            worker.join().unwrap();
            assert_eq!(a.history.recover().is_some(), kept, "keep {keep}, practice {practice}");
        }
    }

    /// Words at rms `amp` over a quiet room (-70 dBFS), `n` of them, `secs` long and as far apart, after 1 s of the room.
    fn quiet_words(n: usize, secs: f32, amp: f32) -> Vec<f32> {
        use crate::voice::audio::tests::{noise, voice_like};
        let mut x = noise(1.0, 0.0003, 4, false);
        for _ in 0..n {
            x.extend(voice_like(secs, amp).iter().zip(noise(secs, 0.0003, 5, false)).map(|(a, b)| a + b));
            x.extend(noise(secs, 0.0003, 6, false));
        }
        x
    }

    #[test]
    fn a_recording_is_heard_through_the_front_end_when_it_is_on_and_its_audio_is_kept_as_it_came() {
        // Words at -44 dBFS: a quiet microphone as they come, a usable voice once raised.
        let x = quiet_words(4, 0.6, 0.006);
        for boost in [false, true] {
            let a = app(&format!("boost-{boost}"));
            a.practice.store(false, Ordering::SeqCst);
            a.settings.lock().unwrap().voice_boost = boost;
            *a.t0.lock().unwrap() = Some(now_ms());
            a.state.lock().unwrap().phase = Phase::Recording;
            a.stop.store(false, Ordering::SeqCst);
            a.take_no.store(1, Ordering::SeqCst);
            let take = Arc::new(Take::default());
            take.samples.lock().unwrap().extend_from_slice(&x);
            // The worker's first look reads all of it and says what the HUD shows before it first says how far behind it is.
            a.behind.store(u64::MAX, Ordering::SeqCst);
            let worker = {
                let (me, take) = (a.clone(), take.clone());
                std::thread::spawn(move || me.worker(1, take))
            };
            let t = std::time::Instant::now();
            while a.behind.load(Ordering::SeqCst) == u64::MAX && t.elapsed() < std::time::Duration::from_secs(30) {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let note = a.state.lock().unwrap().quiet_mic.clone();
            a.stop.store(true, Ordering::SeqCst);
            worker.join().unwrap();
            assert_eq!(note.as_deref(), if boost { None } else { Some("quiet") }, "boost {boost}");
            let kept = a.history.audio(&a.history.recover().expect("kept").id).unwrap();
            assert!(kept.len() == x.len() && kept.iter().zip(&x).all(|(k, v)| (k - v).abs() < 1e-4), "boost {boost}: kept as it came");
        }
    }

    #[test]
    fn transcribing_again_hears_through_the_front_end_only_when_it_is_on_and_keeps_the_audio_as_it_was() {
        // 0.3 s from before the microphone opened, then words at -50 dBFS.
        let x = [vec![0.0f32; 4_800], quiet_words(3, 1.0, 0.003)].concat();
        for boost in [true, false] {
            let a = app(&format!("again-boost-{boost}"));
            a.settings.lock().unwrap().voice_boost = boost;
            a.history.append_audio(&x).unwrap();
            let words = [Item::Speech { t_ms: 300, end_ms: 7_000, text: crate::voice::stt::UNCLEAR.into() }];
            let m = a.history.save_recorded(&words, 1_793_000_000_000, 7_300).unwrap();
            a.history.clear_checkpoint();
            let levels = std::cell::RefCell::new(vec![]);
            let hear = |s: &[f32], _: u64| {
                levels.borrow_mut().push(audio::rms(s));
                Ok(Some(Heard { text: "said".into(), tokens: None }))
            };
            a.again(&m.id, || Ok(()), hear, |_, _| {}).unwrap();
            let l = levels.take();
            assert!(!l.is_empty() && l.iter().all(|r| if boost { *r > 0.03 } else { *r < 0.005 }), "boost {boost}: {l:?}");
            let kept = a.history.audio(&m.id).unwrap();
            assert!(kept.len() == x.len() && kept.iter().zip(&x).all(|(k, v)| (k - v).abs() < 1e-4), "boost {boost}: kept as it was");
        }
    }

    /// `secs` of a quiet room with a tone over each of `spans` (in s), after 0.3 s of nothing, as a kept recording starts.
    fn bursts(secs: f32, spans: &[(f32, f32)]) -> Vec<f32> {
        let rate = audio::RATE as f32;
        (0..(secs * rate) as usize)
            .map(|i| match i as f32 / rate {
                t if t < 0.3 => 0.0,
                t if spans.iter().any(|(a, b)| t >= *a && t < *b) => 0.2 * (i as f32 * 0.1).sin(),
                _ => 0.0005,
            })
            .collect()
    }

    /// Two sentences and a short word: under `SURE_VOICE_MS` of voice, enough to be sent (`MIN_VOICED_MS`).
    const SPANS: [(f32, f32); 3] = [(1.0, 3.0), (4.0, 6.0), (6.9, 7.6)];

    /// A model that says "piece 1", "piece 2" and so on, one a call.
    fn counting() -> impl Fn(&[f32]) -> Heard {
        let n = std::cell::Cell::new(0);
        move |_: &[f32]| {
            n.set(n.get() + 1);
            Heard { text: format!("piece {}", n.get()), tokens: None }
        }
    }

    fn texts(items: &[Item]) -> Vec<String> {
        items.iter().filter_map(|i| if let Item::Speech { text, .. } = i { Some(text.clone()) } else { None }).collect()
    }

    #[test]
    fn a_whole_recording_is_cut_and_heard_as_it_was_live_and_says_how_far_it_got() {
        for boost in [false, true] {
            whole_recording(boost);
        }
    }

    fn whole_recording(boost: bool) {
        let x = bursts(8.5, &SPANS);
        let total = 8_500;
        let hear = counting();
        let mut seen = vec![];
        let said = transcribe_all(&mut x.clone(), boost, |s, v| Ok(crate::voice::stt::hear_piece(s, v, &hear)), |d, t| seen.push((d, t))).unwrap();
        assert_eq!(texts(&said), ["piece 1", "piece 2", "piece 3"]);
        let at: Vec<(u64, u64)> = said.iter().map(|i| if let Item::Speech { t_ms, end_ms, .. } = i { (*t_ms, *end_ms) } else { panic!() }).collect();
        for ((t, end), (a, b)) in at.iter().zip(SPANS) {
            // Each piece holds its sound, with a little room on each side, and starts after the silence kept before it.
            assert!(*t >= 300 && *t <= (a * 1000.0) as u64 && *end >= (b * 1000.0) as u64 && end - t < 4_000, "{at:?}");
        }
        assert!(at.windows(2).all(|w| w[0].1 <= w[1].0), "{at:?}");
        assert_eq!(seen.len(), 4, "before each piece, then at the end: {seen:?}");
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0) && seen.iter().all(|p| p.1 == total), "{seen:?}");
        assert_eq!((seen[0].0, seen[3]), (at[0].0, (total, total)));
        // A model that hears nothing: the sentences are kept as [unclear], the short sound is let go.
        let deaf = |s: &[f32], v: u64| Ok(crate::voice::stt::hear_piece(s, v, |_| Heard::default()));
        let said = transcribe_all(&mut x.clone(), boost, deaf, |_, _| {}).unwrap();
        assert_eq!(texts(&said), [crate::voice::stt::UNCLEAR; 2]);
        // Refused part way: it stops there, short of the end.
        let mut seen = vec![];
        let mut calls = 0;
        let refuse = |_: &[f32], _: u64| {
            calls += 1;
            if calls > 1 { Err(INTERRUPTED.to_string()) } else { Ok(Some(Heard { text: "one".into(), tokens: None })) }
        };
        assert_eq!(transcribe_all(&mut x.clone(), boost, refuse, |d, t| seen.push((d, t))).unwrap_err(), INTERRUPTED);
        assert!(seen.iter().all(|p| p.0 < total), "{seen:?}");
        // Nothing but a quiet room: no pieces, and done at once.
        let mut seen = vec![];
        assert!(transcribe_all(&mut bursts(3.0, &[]), boost, |_, _| panic!("nothing to hear"), |d, t| seen.push((d, t))).unwrap().is_empty());
        assert_eq!(seen, [(3_000, 3_000)]);
    }

    /// A saved recording with pictures between its sentences and the audio of `SPANS`; the words came back unclear.
    fn kept(a: &App) -> (String, Vec<Item>) {
        let pic = |t: u64| Item::Image {
            t_ms: t,
            capture: Capture { id: 0, kind: CaptureKind::Region, t_ms: t, png: png(), width: 1, height: 1, labels: vec![], pointer: None, trail: vec![], element: None, place: None, anchors: vec![], auto: false },
        };
        let unclear = |t: u64, end: u64| Item::Speech { t_ms: t, end_ms: end, text: crate::voice::stt::UNCLEAR.into() };
        let items = vec![unclear(600, 3_500), pic(3_800), unclear(3_600, 6_500), pic(8_000)];
        a.history.append_audio(&bursts(8.5, &SPANS)).unwrap();
        let m = a.history.save_recorded(&items, 1_793_000_000_000, 8_500).unwrap();
        a.history.clear_checkpoint();
        let items = a.history.items(&m.id).unwrap();
        (m.id, items)
    }

    fn pictures(items: &[Item]) -> Vec<Item> {
        items.iter().filter(|i| matches!(i, Item::Image { .. })).cloned().collect()
    }

    #[test]
    fn transcribing_again_replaces_the_words_keeps_the_pictures_and_what_agents_read_follows() {
        let a = app("again");
        let (id, before) = kept(&a);
        a.set_last(&id).unwrap();
        a.latest.set(Some(id.clone()), a.last_export().unwrap());
        let hear = counting();
        let mut seen = vec![];
        a.again(&id, || Ok(()), |s, v| Ok(crate::voice::stt::hear_piece(s, v, &hear)), |d, t| seen.push((d, t))).unwrap();
        let after = a.history.items(&id).unwrap();
        assert_eq!(texts(&after), ["piece 1", "piece 2", "piece 3"], "no [unclear] left");
        assert_eq!(pictures(&after), pictures(&before), "the pictures, in order, at their times");
        assert_eq!(seen.last(), Some(&(8_500, 8_500)));
        let plain = "piece 1 [image 1] piece 2 piece 3 [image 2]";
        assert_eq!(a.history.export(&id).unwrap().plain, plain);
        let named = a.history.export_with(&id, true).unwrap().plain;
        assert_eq!((a.last_export().unwrap().plain, a.latest.get().unwrap().plain), (named.clone(), named), "the last prompt and agents read it now");
        assert_eq!(a.history.list()[0].preview, "piece 1 piece 2 piece 3");
        // Another entry is the last prompt: it stays the last prompt, untouched.
        let other = a.history.save(&[Item::Speech { t_ms: 0, end_ms: 9, text: "other".into() }], 1_793_000_100_000, 9).unwrap();
        a.set_last(&other.id).unwrap();
        a.again(&id, || Ok(()), |s, v| Ok(crate::voice::stt::hear_piece(s, v, &hear)), |_, _| {}).unwrap();
        assert_eq!(texts(&a.history.items(&id).unwrap()), ["piece 4", "piece 5", "piece 6"]);
        assert_eq!(a.last_export().unwrap().plain, "other");
    }

    #[test]
    fn transcribing_again_is_refused_while_recording_or_without_audio_and_a_refusal_changes_nothing() {
        let a = app("again-refused");
        let (id, before) = kept(&a);
        let hear = |_: &[f32], _: u64| -> Result<Option<Heard>, String> { panic!("nothing is heard when refused") };
        let ok = || Ok(());
        for (phase, why) in [(Phase::Recording, RECORDING_NOW), (Phase::Loading, RECORDING_NOW), (Phase::Transcribing, BUSY)] {
            a.state.lock().unwrap().phase = phase;
            assert_eq!(a.again(&id, ok, hear, |_, _| {}).unwrap_err(), why, "{phase:?}");
        }
        a.state.lock().unwrap().phase = Phase::Ready;
        let words = a.history.save(&[Item::Speech { t_ms: 0, end_ms: 9, text: "no audio kept".into() }], 1_793_000_100_000, 9).unwrap();
        assert_eq!(a.again(&words.id, ok, hear, |_, _| {}).unwrap_err(), crate::history::NO_AUDIO);
        assert_eq!(a.again("nope", ok, hear, |_, _| {}).unwrap_err(), "no such prompt");
        a.retranscribing.store(true, Ordering::SeqCst);
        assert_eq!(a.again(&id, ok, hear, |_, _| {}).unwrap_err(), AGAIN_BUSY, "one at a time");
        a.retranscribing.store(false, Ordering::SeqCst);
        assert_eq!(a.again(&id, || Err("the speech model did not load".into()), hear, |_, _| {}).unwrap_err(), "the speech model did not load");
        // Without a model here, the real one cannot start either.
        assert!(a.retranscribe(&id, |_, _| {}).is_err());
        // A recording starts after the first piece: it stops there and keeps nothing of it.
        let start = |_: &[f32], _: u64| {
            a.state.lock().unwrap().phase = Phase::Recording;
            Ok(Some(Heard { text: "half".into(), tokens: None }))
        };
        assert_eq!(a.again(&id, ok, start, |_, _| {}).unwrap_err(), INTERRUPTED);
        assert_eq!(a.history.items(&id).unwrap(), before, "every refusal left the entry as it was");
        assert!(!a.retranscribing.load(Ordering::SeqCst), "and the next one may run");
    }

    #[test]
    fn a_recording_with_no_words_keeps_its_audio_in_history_to_transcribe_again() {
        let out = std::env::temp_dir().join(format!("pc-session-no-words-out-{}", std::process::id()));
        for keep in [true, false] {
            let a = app(&format!("no-words-{keep}"));
            a.practice.store(false, Ordering::SeqCst);
            {
                // Saved to a folder only: a stop in a test never touches the clipboard or pastes.
                let mut s = a.settings.lock().unwrap();
                (s.output, s.also_clipboard, s.export_dir, s.keep_audio) = (OutputMode::Folder, false, Some(out.to_string_lossy().into_owned()), keep);
            }
            *a.t0.lock().unwrap() = Some(now_ms());
            a.state.lock().unwrap().phase = Phase::Recording;
            a.stop.store(false, Ordering::SeqCst);
            a.take_no.store(1, Ordering::SeqCst);
            let take = Arc::new(Take::default());
            *a.worker.lock().unwrap() = Some({
                let (me, take) = (a.clone(), take.clone());
                std::thread::spawn(move || me.worker(1, take))
            });
            // Two sentences, and no model here to hear them: no words come back.
            let x = bursts(8.5, &SPANS);
            take.samples.lock().unwrap().extend_from_slice(&x);
            let exp = a.stop(false).unwrap().unwrap();
            assert!(exp.plain.is_empty());
            assert!(a.history.recover().is_none(), "nothing left behind to recover");
            let l = a.history.list();
            if !keep {
                assert!(l.is_empty(), "no words and no audio: nothing to keep");
                continue;
            }
            assert_eq!(l.len(), 1);
            assert!(l[0].has_audio && l[0].preview == crate::history::NO_WORDS, "{:?}", l[0]);
            assert_eq!(a.history.audio(&l[0].id).unwrap().len(), x.len());
            assert_eq!(a.last_id.lock().unwrap().as_deref(), Some(l[0].id.as_str()));
            // Transcribed again with a model that hears: the words arrive.
            let hear = counting();
            a.again(&l[0].id, || Ok(()), |s, v| Ok(crate::voice::stt::hear_piece(s, v, &hear)), |_, _| {}).unwrap();
            assert_eq!(a.last_export().unwrap().plain, "piece 1 piece 2 piece 3");
        }
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn a_stop_shows_how_far_the_last_words_are_and_never_claims_done_early() {
        // 40 s of audio waits; the model took 20 s for 10 s of this recording.
        let mut p = Progress::new(40_000, (10_000, 20_000), 1_000);
        let v = p.view(0, 1_000);
        assert_eq!((v.done_ms, v.total_ms, v.percent, v.stalled, v.left_ms), (0, 40_000, 0, false, Some(80_000)));
        // A 10 s piece under way is credited at that pace, and never past nine tenths of it.
        p.begin(10_000, 100);
        assert_eq!(p.view(10_100, 2_000).done_ms, 5_000);
        assert_eq!(p.view(90_000, 3_000).done_ms, 9_000);
        p.end(10_000, 30_000);
        let v = p.view(90_100, 4_000);
        assert_eq!((v.done_ms, v.percent), (10_000, 25));
        // What is left turns out to be 10 s of pieces: the rest was quiet.
        p.rest(10_000);
        assert_eq!(p.view(90_200, 5_000).percent, 50);
        // The last piece in: 99 until the stop has the words, then 100.
        p.begin(10_000, 90_300);
        p.end(10_000, 9_000);
        let v = p.view(99_300, 6_000);
        assert_eq!((v.done_ms, v.total_ms, v.percent, v.stalled), (20_000, 20_000, 99, false));
        p.finish();
        let v = p.view(99_400, 6_100);
        assert_eq!((v.percent, v.left_ms), (100, None));
        // More left than first counted: what is shown never goes back.
        let mut p = Progress::new(10_000, (0, 0), 0);
        p.begin(5_000, 0);
        p.end(5_000, 5_000);
        assert_eq!(p.view(0, 0).percent, 50);
        p.rest(15_000);
        let v = p.view(10, 10);
        assert_eq!((v.percent, v.done_ms, v.total_ms), (50, 5_000, 20_000));
        // Over a whole wait, the percent only ever rises and is never 100 before the finish.
        let mut p = Progress::new(60_000, (5_000, 5_000), 0);
        let mut seen = vec![];
        for k in 0..6u64 {
            p.begin(10_000, k * 20_000);
            seen.extend((0..20).map(|t| p.view(k * 20_000 + t * 1_000, k * 20_000 + t * 1_000).percent));
            p.end(10_000, 20_000);
        }
        seen.push(p.view(120_000, 120_000).percent);
        assert!(seen.windows(2).all(|w| w[0] <= w[1]) && seen.iter().all(|v| *v < 100), "{seen:?}");
        p.finish();
        assert_eq!(p.view(120_000, 120_000).percent, 100);
    }

    #[test]
    fn a_stop_that_shows_no_progress_for_20_s_is_stalled_by_the_wall_clock_only() {
        // No pace measured yet: the piece under way earns no credit, so only its end moves anything.
        let mut p = Progress::new(30_000, (0, 0), 5_000);
        p.begin(30_000, 0);
        assert!(!p.view(19_999, 24_999).stalled);
        let v = p.view(20_000, 25_000);
        assert!(v.stalled && v.left_ms.is_none(), "{v:?}");
        p.end(30_000, 26_000);
        assert!(!p.view(26_000, 31_000).stalled, "a piece came back");
        // With a pace, the piece is credited as it goes.
        let mut p = Progress::new(30_000, (10_000, 10_000), 0);
        p.begin(30_000, 0);
        assert_eq!(p.view(5_000, 5_000).percent, 16);
        // The computer sleeps for an hour: the steady clock stood still (as on macOS) and the bar with it; the wall
        // clock did not, so Cancel is offered.
        let v = p.view(5_000, 3_605_000);
        assert_eq!((v.done_ms, v.percent, v.stalled), (5_000, 16, true));
        assert!(!p.view(5_100, 3_605_100).stalled, "awake, it moves again");
        // A steady clock that ran on through the sleep credits no more than nine tenths of the piece.
        assert_eq!(p.view(3_605_100, 3_605_200).done_ms, 27_000);
        // Credited as far as it may be and still not back: stalled 20 s after the bar stopped.
        assert!(!p.view(3_615_000, 3_625_199).stalled);
        assert!(p.view(3_625_000, 3_625_200).stalled);
        assert!(!p.view(3_625_100, 25_000).stalled, "a wall clock set back is no stall");
        // Nothing to transcribe: 0 % and never stalled, then 100 % once done.
        let mut p = Progress::new(0, (0, 0), 0);
        let v = p.view(0, 60_000);
        assert_eq!((v.percent, v.stalled, v.done_ms, v.total_ms), (0, false, 0, 0));
        p.finish();
        assert_eq!(p.view(0, 60_000).percent, 100);
    }

    /// Saved to a folder only and keeping its audio: a stop in a test never touches the clipboard or pastes.
    fn real(a: &App, out: &std::path::Path) {
        a.practice.store(false, Ordering::SeqCst);
        let mut s = a.settings.lock().unwrap();
        (s.output, s.also_clipboard, s.export_dir, s.keep_audio) = (OutputMode::Folder, false, Some(out.to_string_lossy().into_owned()), true);
    }

    fn until(what: &str, ok: impl Fn() -> bool) {
        let t = std::time::Instant::now();
        while !ok() {
            assert!(t.elapsed() < std::time::Duration::from_secs(20), "waited too long for {what}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn cancel_at_the_stop_saves_what_is_done_keeps_all_the_audio_and_nothing_late_reaches_the_next_recording() {
        let out = std::env::temp_dir().join(format!("pc-session-cut-out-{}", std::process::id()));
        let a = app("cut");
        real(&a, &out);
        *a.t0.lock().unwrap() = Some(now_ms());
        a.state.lock().unwrap().phase = Phase::Recording;
        a.stop.store(false, Ordering::SeqCst);
        a.take_no.store(1, Ordering::SeqCst);
        a.push(Item::Speech { t_ms: 1_000, end_ms: 3_000, text: "the first sentence".into() }).unwrap();
        // The model is busy, as with a slow piece: the test holds it, so the worker waits with sentences unheard.
        let busy = a.engine.lock().unwrap();
        let x = bursts(8.5, &SPANS);
        let take = Arc::new(Take::default());
        take.samples.lock().unwrap().extend_from_slice(&x[..x.len() - 8_000]);
        *a.take.lock().unwrap() = Some(take.clone());
        let (ended, gone) = std::sync::mpsc::channel();
        *a.worker.lock().unwrap() = Some({
            let (me, take) = (a.clone(), take.clone());
            std::thread::spawn(move || {
                me.worker(1, take);
                let _ = ended.send(());
            })
        });
        // It wrote what it read to the recovery area; the last half second comes after, unread.
        until("the audio so far", || a.history.kept_audio());
        take.samples.lock().unwrap().extend_from_slice(&x[x.len() - 8_000..]);
        let stopping = {
            let a = a.clone();
            std::thread::spawn(move || a.stop(false))
        };
        until("the stop to wait", || a.ui().phase == Phase::Transcribing);
        let v = a.ui().progress.expect("how far it is, while it waits");
        assert!(v.total_ms > 0 && v.percent < 100, "{v:?}");
        a.finish_now();
        let exp = stopping.join().unwrap().unwrap().unwrap();
        assert_eq!(exp.plain, "the first sentence");
        let st = a.ui();
        assert_eq!((st.phase, st.notice, st.progress), (Phase::Ready, Some("partial"), None));
        let l = a.history.list();
        assert_eq!(l.len(), 1);
        assert!(l[0].partial && l[0].has_audio, "{:?}", l[0]);
        assert_eq!(a.history.audio(&l[0].id).unwrap().len(), x.len(), "all of it, the part the worker never read too");
        assert!(a.history.recover().is_none(), "nothing left behind to recover");

        // The next recording starts, as start() begins one, while the old worker is still on its piece.
        let no = a.take_no.fetch_add(1, Ordering::SeqCst) + 1;
        a.items.lock().unwrap().clear();
        a.tokens.lock().unwrap().clear();
        *a.unsaved.lock().unwrap() = None;
        *a.t0.lock().unwrap() = Some(now_ms());
        a.stop.store(false, Ordering::SeqCst);
        a.state.lock().unwrap().phase = Phase::Recording;
        // A piece the old worker finishes now lands nowhere, its tokens neither.
        let late = Item::Speech { t_ms: 4_000, end_ms: 6_000, text: "late".into() };
        assert_eq!(a.add_heard(1, late, Some((4_000, vec![]))).unwrap_err(), STOPPED);
        drop(busy);
        gone.recv_timeout(std::time::Duration::from_secs(20)).expect("the old worker ends");
        assert!(a.items.lock().unwrap().is_empty() && a.tokens.lock().unwrap().is_empty());
        assert!(!a.history.kept_audio(), "it wrote no audio into the next recording's");
        assert_eq!((a.behind.load(Ordering::SeqCst), a.ui().partial.as_str()), (0, ""));
        assert_eq!(a.history.list().len(), 1);
        // The new recording's own worker is heard as before.
        a.add_heard(no, Item::Speech { t_ms: 0, end_ms: 900, text: "new".into() }, None).unwrap();
        assert_eq!(a.ui().text, "new");
        a.state.lock().unwrap().phase = Phase::Idle;
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn cancel_with_nothing_left_to_transcribe_is_a_stop_like_any_other_and_a_second_one_is_harmless() {
        let out = std::env::temp_dir().join(format!("pc-session-cut-none-out-{}", std::process::id()));
        let a = app("cut-none");
        real(&a, &out);
        a.finish_now();
        for (no, said) in [(1, "all of it"), (2, "the next one")] {
            *a.t0.lock().unwrap() = Some(now_ms());
            a.state.lock().unwrap().phase = Phase::Recording;
            a.stop.store(false, Ordering::SeqCst);
            a.take_no.store(no, Ordering::SeqCst);
            a.push(Item::Speech { t_ms: 0, end_ms: 900, text: said.into() }).unwrap();
            *a.worker.lock().unwrap() = Some({
                let me = a.clone();
                std::thread::spawn(move || me.worker(no, Arc::new(Take::default())))
            });
            // A Cancel left over from the first stop never cuts the second.
            a.hurry.store(no == 2, Ordering::SeqCst);
            let stopping = {
                let a = a.clone();
                std::thread::spawn(move || a.stop(false))
            };
            until("the stop", || a.ui().phase != Phase::Recording);
            // Twice, while it finishes or after.
            if no == 1 {
                a.finish_now();
                a.finish_now();
            }
            assert_eq!(stopping.join().unwrap().unwrap().unwrap().plain, said);
            if no == 2 {
                assert_eq!(a.take_no.load(Ordering::SeqCst), 2, "the stop waited for its worker");
            }
            a.finish_now();
            let st = a.ui();
            assert_eq!((st.phase, st.notice), (Phase::Ready, Some("saved")), "{said}");
            assert!(!a.history.list()[0].partial, "{said}");
            a.items.lock().unwrap().clear();
        }
        assert_eq!(a.history.list().len(), 2);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn cancel_on_transcribing_again_stops_after_the_piece_under_way_and_leaves_the_entry_as_it_was() {
        let a = app("again-cancel");
        let (id, before) = kept(&a);
        assert!(!a.again_cancel(), "nothing to cancel");
        let mut calls = 0;
        let hear = |_: &[f32], _: u64| {
            calls += 1;
            // Cancel comes while the first piece is heard.
            assert!(a.again_cancel());
            Ok(Some(Heard { text: "half".into(), tokens: None }))
        };
        assert_eq!(a.again(&id, || Ok(()), hear, |_, _| {}).unwrap_err(), AGAIN_CANCELLED);
        assert_eq!(calls, 1, "the piece under way finished and no other began");
        assert_eq!(a.history.items(&id).unwrap(), before);
        // Cancelled while the model loads: nothing is heard.
        let never = |_: &[f32], _: u64| -> Result<Option<Heard>, String> { panic!("nothing is heard once cancelled") };
        let load = || {
            a.again_cancel();
            Ok(())
        };
        assert_eq!(a.again(&id, load, never, |_, _| {}).unwrap_err(), AGAIN_CANCELLED);
        assert_eq!(a.history.items(&id).unwrap(), before);
        assert!(!a.retranscribing.load(Ordering::SeqCst) && !a.again_cancel(), "and the next one may run");
        let hear = counting();
        a.again(&id, || Ok(()), |s, v| Ok(crate::voice::stt::hear_piece(s, v, &hear)), |_, _| {}).unwrap();
        assert_eq!(texts(&a.history.items(&id).unwrap()), ["piece 1", "piece 2", "piece 3"]);
    }

    #[test]
    fn sherpa_uses_every_core_up_to_8() {
        assert_eq!([0, 1, 4, 6, 8, 16, 32].map(sherpa_threads), [1, 1, 4, 6, 8, 8, 8]);
    }

    #[test]
    fn a_cancel_at_the_stop_or_on_transcribing_again_closes_the_gpu_piece_under_way() {
        use crate::voice::whisper_server::{tests::{fake, Answer}, Server};
        let f = fake("/i-s", vec![Answer::Slow(std::time::Duration::from_secs(10))]);
        let a = app("gpu-cancel");
        let eng = Engine::Gpu(engine::Gpu::new("whisper-turbo", Server::at(f.port, "/i-s"), Box::new(|| Err("none".into()))));
        *a.aborter.lock().unwrap() = eng.aborter();
        *a.engine.lock().unwrap() = Some(eng);
        let piece = |a: &Arc<App>| {
            let a = a.clone();
            std::thread::spawn(move || a.engine.lock().unwrap().as_ref().unwrap().hear(&[0.1; 16_000]))
        };
        for (n, cancel) in [(1, "stop"), (2, "again")] {
            let t = std::time::Instant::now();
            let worker = piece(&a);
            until("the piece to be sent", || f.posts.lock().unwrap().len() == n);
            f.hung_up.store(false, Ordering::SeqCst);
            if cancel == "stop" {
                a.cut();
            } else {
                a.retranscribing.store(true, Ordering::SeqCst);
                assert!(a.again_cancel());
                a.retranscribing.store(false, Ordering::SeqCst);
            }
            assert_eq!(worker.join().unwrap(), Heard::default(), "{cancel}");
            assert!(t.elapsed() < std::time::Duration::from_secs(5), "{cancel}: {:?}", t.elapsed());
            until("the helper to see the request closed", || f.hung_up.load(Ordering::SeqCst));
            let g = a.engine.lock().unwrap();
            assert!(g.as_ref().unwrap().failure().is_none(), "a cancel is no failure");
        }
    }

    /// Words of `s`, lower case, without punctuation.
    fn words(s: &str) -> Vec<String> {
        s.split_whitespace()
            .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase())
            .filter(|w| !w.is_empty())
            .collect()
    }

    /// The share of `want`'s words found in `got`, each counted at most as often as `got` has it.
    fn recall(want: &[String], got: &[String]) -> f64 {
        let mut have: std::collections::HashMap<&str, i64> = Default::default();
        for w in got {
            *have.entry(w.as_str()).or_default() += 1;
        }
        let hit = want
            .iter()
            .filter(|w| {
                have.get_mut(w.as_str()).is_some_and(|n| {
                    *n -= 1;
                    *n >= 0
                })
            })
            .count();
        hit as f64 / want.len().max(1) as f64
    }

    /// `voice` four times at `gain`, 6, 15 and 9 s apart, over a quiet room or one with noise at rms `noise`. Without a
    /// `room` the noise is white; in a room (`Some(rt60)`, s) it is half white and half a low hum and rumble, and the
    /// voice is heard as from across that room (`far`).
    fn long_take(voice: &[f32], gain: f32, noise: f32, room: Option<f32>) -> Vec<f32> {
        let sec = |s: f32| (s * audio::RATE as f32) as usize;
        let voice = match room {
            Some(rt60) if rt60 > 0.0 => far(voice, rt60),
            _ => voice.to_vec(),
        };
        let mut v = vec![0.0f32; sec(1.0)];
        for gap in [6.0, 15.0, 9.0, 2.0] {
            v.extend(voice.iter().map(|x| x * gain));
            v.extend(std::iter::repeat(0.0).take(sec(gap)));
        }
        let mut seed = 0x9e37_79b9u32;
        let mut white = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed as f32 / u32::MAX as f32 * 2.0 - 1.0
        };
        if room.is_none() {
            let amp = noise.max(0.0003) * 3f32.sqrt();
            v.iter_mut().for_each(|x| *x += white() * amp);
            return v;
        }
        // Mains hum at 60 and 120 Hz over rumble (white noise through a one-pole low-pass at about 150 Hz).
        let mut lp = 0.0f32;
        let tau = 2.0 * std::f32::consts::PI / audio::RATE as f32;
        let hum: Vec<f32> = (0..v.len())
            .map(|i| {
                lp += (white() - lp) * 0.057;
                3.0 * lp + 0.5 * (tau * 60.0 * i as f32).sin() + 0.25 * (tau * 120.0 * i as f32).sin()
            })
            .collect();
        let half = noise / 2f32.sqrt();
        let k = half / audio::rms(&hum).max(1e-9);
        let amp = half.max(0.0003) * 3f32.sqrt();
        for (x, h) in v.iter_mut().zip(&hum) {
            *x += h * k + white() * amp;
        }
        v
    }

    /// `x` as heard a few metres away in a room whose sound dies away by 60 dB in `rt60` s: the direct sound and as much
    /// energy again in echoes (sparse random taps under an exponential decay, one every 8 samples from 5 ms on), at the
    /// level of `x`. Longer than `x` by the echoes' tail.
    fn far(x: &[f32], rt60: f32) -> Vec<f32> {
        let rate = audio::RATE as f32;
        let span = (rt60 * rate) as usize;
        let mut seed = 0x2545_f491u32;
        let mut echoes = vec![];
        for k in (audio::RATE as usize / 200..span).step_by(8) {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let at = k + (seed % 8) as usize;
            let sign = if seed & 0x100 == 0 { 1.0 } else { -1.0 };
            echoes.push((at, sign * (-6.9 * at as f32 / rate / rt60).exp()));
        }
        let e = echoes.iter().map(|(_, a)| a * a).sum::<f32>().sqrt();
        let half = 0.5f32.sqrt();
        let mut out = vec![0.0f32; x.len() + span + 8];
        let taps = std::iter::once((0, half)).chain(echoes.into_iter().map(|(at, a)| (at, a / e * half)));
        for (at, a) in taps {
            for (o, v) in out[at..].iter_mut().zip(x) {
                *o += a * v;
            }
        }
        out
    }

    /// The far-field matrix: near and quiet, the old quiet-and-noisy case, then speech 12, 20 and 30 dB down in a quiet
    /// room and under room noise at rms 0.005 and 0.01, heard across a room that rings for 0.4 s.
    const FAR_FIELD: &str = "1:0,0.06:0.01,0.25:0:0.4,0.25:0.005:0.4,0.25:0.01:0.4,0.1:0:0.4,0.1:0.005:0.4,0.1:0.01:0.4,0.03:0:0.4,0.03:0.005:0.4,0.03:0.01:0.4";

    /// The real pipeline over real speech, fed 20 times faster than real time in 10 ms pieces while the worker runs.
    /// Prints word recall against the script, the share of the words heard that are in it (precision: low when noise is
    /// heard as words), the longest stretch with no text, the longest the microphone waited for
    /// the take, the bytes copied out of it and the wall time. Reads the models only. Each case is `gain:noise` (white
    /// noise, dry) or `gain:noise:rt60` (room noise and echoes, see `long_take`); the default is `FAR_FIELD`.
    /// `POINTCAST_SPEECH_DIR=<speech dir> POINTCAST_MODEL=<id> POINTCAST_F32=<16 kHz mono f32le> POINTCAST_SCRIPT=<script.md>
    /// [POINTCAST_CASES=1:0,0.25:0.01:0.4] [POINTCAST_BOOST=0] cargo test --lib -- --ignored --nocapture real_audio_pipeline`
    #[test]
    #[ignore]
    fn real_audio_pipeline() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        assert!(std::env::var_os("POINTCAST_SPEECH_DIR").is_some(), "set POINTCAST_SPEECH_DIR");
        let raw = std::fs::read(var("POINTCAST_F32")).unwrap();
        let voice: Vec<f32> = raw.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let script = std::fs::read_to_string(var("POINTCAST_SCRIPT")).unwrap();
        let lines: Vec<&str> = script
            .lines()
            .filter_map(|l| {
                let c: Vec<&str> = l.split('|').collect();
                let id = c.get(2).map(|s| s.trim()).unwrap_or("");
                (c.len() >= 5 && id.len() == 2 && id.starts_with(['n', 's'])).then(|| c[3])
            })
            .collect();
        let want: Vec<String> = (0..4).flat_map(|_| words(&lines.join(" "))).collect();
        let a = app("real-audio");
        let model = var("POINTCAST_MODEL");
        a.settings.lock().unwrap().model = model.clone();
        // POINTCAST_BOOST=0 hears the takes without the front end.
        let boost = std::env::var("POINTCAST_BOOST").map_or(true, |v| v != "0");
        a.settings.lock().unwrap().voice_boost = boost;
        a.ensure_engine().unwrap();
        let cases = std::env::var("POINTCAST_CASES").unwrap_or_else(|_| FAR_FIELD.into());
        for case in cases.split(',') {
            // `@<file>`: a whole take as made by `long_take`, read from a file (16 kHz mono f32le).
            let rec = match case.strip_prefix('@') {
                Some(file) => std::fs::read(file).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect(),
                None => {
                    let c: Vec<f32> = case.split(':').map(|v| v.parse().unwrap()).collect();
                    long_take(&voice, c[0], c[1], c.get(2).copied())
                }
            };
            // POINTCAST_DUMP=<dir> keeps each case's take, 16 kHz mono f32le, to listen to or measure.
            if let Ok(dir) = std::env::var("POINTCAST_DUMP") {
                let bytes: Vec<u8> = rec.iter().flat_map(|v| v.to_le_bytes()).collect();
                std::fs::write(std::path::Path::new(&dir).join(format!("{}.f32", case.replace([':', '/', '@'], "_"))), bytes).unwrap();
            }
            // A recording as start() begins one, without a microphone.
            a.items.lock().unwrap().clear();
            *a.t0.lock().unwrap() = Some(now_ms());
            a.state.lock().unwrap().phase = Phase::Recording;
            a.stop.store(false, Ordering::SeqCst);
            let no = a.take_no.fetch_add(1, Ordering::SeqCst) + 1;
            let take = Arc::new(Take::default());
            let began = std::time::Instant::now();
            let worker = {
                let (me, take) = (a.clone(), take.clone());
                std::thread::spawn(move || me.worker(no, take))
            };
            let mut blocked = std::time::Duration::ZERO;
            for (i, piece) in rec.chunks(160).enumerate() {
                let due = began + std::time::Duration::from_micros(i as u64 * 500);
                while std::time::Instant::now() < due {
                    std::thread::sleep(std::time::Duration::from_micros(100));
                }
                let t = std::time::Instant::now();
                take.samples.lock().unwrap().extend_from_slice(piece);
                blocked = blocked.max(t.elapsed());
            }
            a.stop.store(true, Ordering::SeqCst);
            a.state.lock().unwrap().phase = Phase::Transcribing;
            worker.join().unwrap();
            let wall = began.elapsed().as_secs_f64();
            let said: Vec<(u64, u64, String)> = prompt::ordered(a.items.lock().unwrap().clone())
                .into_iter()
                .filter_map(|i| match i {
                    Item::Speech { t_ms, end_ms, text } => Some((t_ms, end_ms, text)),
                    _ => None,
                })
                .collect();
            let got = words(&said.iter().map(|s| s.2.as_str()).collect::<Vec<_>>().join(" "));
            let total = rec.len() as u64 * 1000 / audio::RATE as u64;
            let mut edges = vec![0u64];
            for s in &said {
                edges.extend([s.0, s.1]);
            }
            edges.push(total);
            let gap = edges.chunks(2).map(|p| p[1].saturating_sub(p[0])).max().unwrap_or(total);
            eprintln!(
                "RESULT model={model} boost={boost} case={case} recall={:.3} precision={:.3} gap_s={:.1} block_ms={:.2} copied_mb={:.1} wall_s={wall:.1} audio_s={:.1} items={}",
                recall(&want, &got),
                recall(&got, &want),
                gap as f64 / 1000.0,
                blocked.as_secs_f64() * 1000.0,
                take.copied.load(Ordering::Relaxed) as f64 / 1e6,
                total as f64 / 1000.0,
                said.len()
            );
            let (heard, spent) = *a.pace.lock().unwrap();
            eprintln!("PACE model={model} heard_s={:.1} spent_s={:.1} realtime_x={:.2}", heard as f64 / 1e3, spent as f64 / 1e3, heard as f64 / spent.max(1) as f64);
            eprintln!("TEXT {}", said.iter().map(|s| format!("{}-{} {}", s.0, s.1, s.2)).collect::<Vec<_>>().join(" | "));
        }
    }
}
