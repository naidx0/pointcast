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
use crate::voice::stt::Engine;
use serde::Serialize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

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
    /// After a stop, where the prompt went: pasted, copied, saved or empty.
    pub notice: Option<&'static str>,
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
        let model_ready = {
            let root = models::speech_dir();
            let cat = models::catalog();
            cat.runtime_for_host()
                .map(|r| models::runtime_installed(&root, r))
                .unwrap_or(false)
                && cat
                    .models
                    .iter()
                    .any(|m| models::model_installed(&root, m))
        };
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
            loading: Mutex::default(),
            checked: Mutex::default(),
            finishing: Mutex::default(),
            cancel: AtomicBool::new(false),
            audio_at: AtomicU64::new(0),
            worker: Mutex::default(),
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
        let rt = cat.runtime_for_host().ok_or("Speech isn't available for this computer yet.")?;
        let installed: Vec<&str> = cat
            .models
            .iter()
            .filter(|m| models::model_installed(&root, m))
            .map(|m| m.id.as_str())
            .collect();
        let chosen = self.settings.lock().unwrap().model.clone();
        let id = match machine::route(&chosen, machine::recommended(), &installed) {
            Some(id) if models::runtime_installed(&root, rt) => id,
            _ => return Err(NO_MODEL.into()),
        };
        if self.engine.lock().unwrap().as_ref().is_some_and(|e| e.model == id) {
            self.state.lock().unwrap().model_ready = true;
            return Ok(());
        }
        let model = cat.model(id).ok_or(NO_MODEL)?;
        let threads = std::thread::available_parallelism()
            .map(|n| n.get().min(4) as i32)
            .unwrap_or(2);
        let eng = crate::voice::stt::load(&root, rt, model, threads)?;
        *self.engine.lock().unwrap() = Some(eng);
        self.state.lock().unwrap().model_ready = true;
        Ok(())
    }

    /// Free the model's memory while nothing is recording; the next recording loads it again
    /// (its audio waits in the take meanwhile, so nothing is lost).
    pub fn unload_engine(&self) {
        if matches!(self.state.lock().unwrap().phase, Phase::Recording | Phase::Loading | Phase::Transcribing) {
            return;
        }
        let _one = self.loading.lock().unwrap();
        *self.engine.lock().unwrap() = None;
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
        // The worker loads the model if it is not in memory yet; audio waits in the take meanwhile.
        if self.engine.lock().unwrap().is_some() {
            self.state.lock().unwrap().phase = Phase::Recording;
        }
        let no = self.take_no.fetch_add(1, Ordering::SeqCst) + 1;
        self.trail.lock().unwrap().clear();
        self.tokens.lock().unwrap().clear();
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
        // The frames leave memory with this; only the ones picked below live on, as pictures.
        let frames = self.frames.lock().unwrap().take();
        if let Some(mic) = self.mic.lock().unwrap().take() {
            drop(mic);
        }
        {
            let mut st = self.state.lock().unwrap();
            st.mic = false;
            st.screen = false;
            st.phase = if cancel { Phase::Idle } else { Phase::Transcribing };
        }
        // The worker transcribes what was still being said when stop was pressed; wait for it.
        if let Some(w) = self.worker.lock().unwrap().take() {
            let _ = w.join();
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
        let exp = prompt::assemble_export(&items);
        if self.practice.load(Ordering::SeqCst) {
            self.history.clear_checkpoint();
            self.state.lock().unwrap().phase = Phase::Ready;
            return Ok(Some(exp));
        }
        *self.last_export.lock().unwrap() = Some(exp.clone());
        // The checkpoint goes only once History has the prompt: a failed save leaves it for
        // the next launch to recover.
        let mut saved = None;
        if items.is_empty() {
            self.history.clear_checkpoint();
        } else {
            let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
            if let Ok(m) = self.history.save(&items, t0, now_ms().saturating_sub(t0)) {
                *self.last_id.lock().unwrap() = Some(m.id.clone());
                saved = Some(m.id);
                self.history.clear_checkpoint();
            }
        }
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
        let delivered = crate::clipboard::deliver(&items, mode, settings.also_clipboard);
        let mut st = self.state.lock().unwrap();
        st.notice = Some(notice(&items, mode, delivered.is_ok()));
        st.phase = Phase::Ready;
        Ok(Some(exp))
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
        let mut v = self.items.lock().unwrap();
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
        let exp = self.history.export(id)?;
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

    fn worker(self: Arc<Self>, no: u64, take: Arc<Take>) {
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
        let mut seg = audio::Segmenter::new(900, 12_000);
        let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
        // Only what arrived since the last look is copied out of the take, so a long recording costs no more per
        // look than a short one and the microphone never waits behind a copy of all of it.
        let mut pos = 0;
        loop {
            if self.stop.load(Ordering::SeqCst) || self.take_no.load(Ordering::SeqCst) != no {
                break;
            }
            let new = take.read_from(pos);
            pos += new.len();
            let ranges = seg.feed_new(&new);
            self.decode_ranges(&take, &ranges, t0);
            // A live caption only when one is due and nothing waits to be transcribed: the last few seconds of the open
            // sentence.
            let due = self.engine.lock().unwrap().as_ref().is_some_and(|e| e.wants_partial());
            if let Some(start) = seg.open_start().filter(|_| due && ranges.is_empty() && take.len() - pos < audio::RATE as usize / 2) {
                let samples = take.copy(start.max(pos.saturating_sub(PARTIAL_SPAN))..pos);
                let live = self.engine.lock().unwrap().as_ref().and_then(|e| e.partial(&samples));
                if let Some(t) = live {
                    self.state.lock().unwrap().partial = t;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        if self.cancel.load(Ordering::SeqCst) {
            self.state.lock().unwrap().partial.clear();
            return;
        }
        let mut rest = seg.feed_new(&take.read_from(pos));
        rest.extend(seg.close());
        self.decode_ranges(&take, &rest, t0);
        self.state.lock().unwrap().partial.clear();
    }

    fn decode_ranges(&self, take: &Take, ranges: &[std::ops::Range<usize>], t0: u64) {
        for r in ranges {
            let samples = take.copy(r.clone());
            let heard = {
                let g = self.engine.lock().unwrap();
                match g.as_ref() {
                    Some(eng) => eng.hear(&samples),
                    None => Default::default(),
                }
            };
            let text = heard.text;
            if text.is_empty() {
                continue;
            }
            let at = self.audio_at.load(Ordering::SeqCst);
            let t_ms = at + (r.start as u64) * 1000 / audio::RATE as u64;
            let end_ms = at + (r.end as u64) * 1000 / audio::RATE as u64;
            let _ = t0;
            if let Some(tok) = heard.tokens {
                self.tokens.lock().unwrap().push((t_ms, tok));
            }
            // The words still being transcribed for a stop go in too: the stop waits for them before it copies.
            let _ = self.add(
                Item::Speech {
                    t_ms,
                    end_ms,
                    text,
                },
                &[Phase::Recording, Phase::Loading, Phase::Transcribing],
            );
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

/// What the HUD says after a stop.
pub fn notice(items: &[Item], mode: OutputMode, delivered: bool) -> &'static str {
    match (items.is_empty(), mode, delivered) {
        (true, _, _) => "empty",
        (_, OutputMode::AutoPaste, true) => "pasted",
        (_, OutputMode::Folder, _) => "saved",
        _ => "copied",
    }
}

/// A live caption covers at most this many samples (8 s) of the sentence being spoken.
const PARTIAL_SPAN: usize = audio::RATE as usize * 8;

pub const BUSY: &str = "Pointcast is still finishing the last recording.";

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
        assert_eq!(served(&a).as_deref(), Some("look [image 1: 1=this one]"));
        // Copying the older prompt makes it the last one; agents still read the recording.
        a.set_last(&older.id).unwrap();
        assert_eq!(served(&a).as_deref(), Some("look [image 1: 1=this one]"));
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
        assert_eq!(notice(&[], OutputMode::AutoPaste, true), "empty");
        assert_eq!(notice(&words, OutputMode::AutoPaste, true), "pasted");
        assert_eq!(notice(&words, OutputMode::AutoPaste, false), "copied");
        assert_eq!(notice(&words, OutputMode::Clipboard, true), "copied");
        assert_eq!(notice(&words, OutputMode::Folder, true), "saved");
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

    /// `voice` four times at `gain`, 6, 15 and 9 s apart, over a quiet room or one with noise at rms `noise`.
    fn long_take(voice: &[f32], gain: f32, noise: f32) -> Vec<f32> {
        let sec = |s: f32| (s * audio::RATE as f32) as usize;
        let mut v = vec![0.0f32; sec(1.0)];
        for gap in [6.0, 15.0, 9.0, 2.0] {
            v.extend(voice.iter().map(|x| x * gain));
            v.extend(std::iter::repeat(0.0).take(sec(gap)));
        }
        let mut seed = 0x9e37_79b9u32;
        let amp = noise.max(0.0003) * 3f32.sqrt();
        for x in v.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *x += (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * amp;
        }
        v
    }

    /// The real pipeline over real speech, fed 20 times faster than real time in 10 ms pieces while the worker runs.
    /// Prints word recall against the script, the longest stretch with no text, the longest the microphone waited for
    /// the take, the bytes copied out of it and the wall time. Reads the models only.
    /// `POINTCAST_SPEECH_DIR=<speech dir> POINTCAST_MODEL=<id> POINTCAST_F32=<16 kHz mono f32le> POINTCAST_SCRIPT=<script.md>
    /// [POINTCAST_CASES=1:0,0.25:0.01] cargo test --lib -- --ignored --nocapture real_audio_pipeline`
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
        a.ensure_engine().unwrap();
        let cases = std::env::var("POINTCAST_CASES").unwrap_or_else(|_| "1:0,0.25:0,0.06:0,1:0.01,0.25:0.01,0.06:0.01".into());
        for case in cases.split(',') {
            let (g, n) = case.split_once(':').unwrap();
            let (gain, noise): (f32, f32) = (g.parse().unwrap(), n.parse().unwrap());
            let rec = long_take(&voice, gain, noise);
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
                "RESULT model={model} gain={gain} noise={noise} recall={:.3} gap_s={:.1} block_ms={:.2} copied_mb={:.1} wall_s={wall:.1} audio_s={:.1} items={}",
                recall(&want, &got),
                gap as f64 / 1000.0,
                blocked.as_secs_f64() * 1000.0,
                take.copied.load(Ordering::Relaxed) as f64 / 1e6,
                total as f64 / 1000.0,
                said.len()
            );
            eprintln!("TEXT {}", said.iter().map(|s| format!("{}-{} {}", s.0, s.1, s.2)).collect::<Vec<_>>().join(" | "));
        }
    }
}
