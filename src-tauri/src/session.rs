//! Recording session: live transcript, captures inserted at the moment they were taken,
//! local STT, and export on stop.
use crate::capture::{self, Shot};
use crate::element::ElementMark;
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
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub history: crate::history::Store,
    /// Where the pointer went during this recording (positions only).
    trail: Mutex<crate::pointer::Trail>,
    /// When the model heard each token, by the start of its piece, for the pieces it gave times for.
    tokens: Mutex<Vec<(u64, Vec<crate::anchor::Token>)>>,
    /// The history entry of the last finished prompt.
    pub last_id: Mutex<Option<String>>,
    freeze: Mutex<Option<Shot>>,
    /// When the region frame was frozen: the capture belongs there, not at the moment of Enter.
    freeze_t: Mutex<Option<u64>>,
    last_export: Mutex<Option<SessionExport>>,
    last_dir: Mutex<Option<std::path::PathBuf>>,
    /// Onboarding practice run: keep the result in the app, deliver it nowhere.
    pub practice: AtomicBool,
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
            worker: Mutex::default(),
            freeze: Mutex::default(),
            freeze_t: Mutex::default(),
            last_export: Mutex::default(),
            last_dir: Mutex::default(),
            history: crate::history::Store::new(&crate::settings::data_dir()),
            trail: Mutex::default(),
            tokens: Mutex::default(),
            last_id: Mutex::default(),
            practice: AtomicBool::new(false),
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

    /// Load the chosen model unless it is already the one in memory.
    pub fn ensure_engine(&self) -> Result<(), String> {
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
        *self.engine.lock().unwrap() = None;
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
        Ok(())
    }

    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        if matches!(
            self.state.lock().unwrap().phase,
            Phase::Recording | Phase::Loading | Phase::Transcribing
        ) {
            return Ok(());
        }
        self.monitor_off();
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
        let me = Arc::clone(self);
        *self.worker.lock().unwrap() = Some(std::thread::spawn(move || me.worker(no, take)));
        Ok(())
    }

    pub fn stop(&self, cancel: bool) -> Result<Option<SessionExport>, String> {
        self.stop.store(true, Ordering::SeqCst);
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
        if settings.spoken_anchors == SpokenAnchors::Ring {
            let trail = self.trail.lock().unwrap().since(0);
            let tokens = self.tokens.lock().unwrap().clone();
            crate::anchor::ring_spoken(&mut items, &trail, &tokens, settings.mark_point != MarkPoint::Off);
        }
        let exp = prompt::assemble_export(&items);
        self.history.clear_checkpoint();
        if self.practice.load(Ordering::SeqCst) {
            self.state.lock().unwrap().phase = Phase::Ready;
            return Ok(Some(exp));
        }
        *self.last_export.lock().unwrap() = Some(exp.clone());
        if !items.is_empty() {
            let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
            if let Ok(m) = self.history.save(&items, t0, now_ms().saturating_sub(t0)) {
                *self.last_id.lock().unwrap() = Some(m.id);
            }
        }
        // History keeps every prompt; a folder in Documents only when the user chose Save.
        if settings.output == OutputMode::Folder {
            let dir = export_dir(&settings);
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
        if !matches!(self.state.lock().unwrap().phase, Phase::Recording | Phase::Loading) {
            return;
        }
        self.stop.store(true, Ordering::SeqCst);
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
        self.push(Item::Speech {
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

    pub fn begin_region(&self) -> Result<Shot, String> {
        let settings = self.settings.lock().unwrap().clone();
        let t = self.t_ms();
        let shot = capture::grab_freeze(&settings, &self.recent(t), t)?;
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
        };
        self.push(Item::Image {
            t_ms: t,
            capture: cap.clone(),
        });
        *self.freeze.lock().unwrap() = None;
        Ok(cap)
    }

    fn capture_shot(
        &self,
        grab: impl FnOnce(&Settings, &[Point], u64) -> Result<Shot, String>,
    ) -> Result<Capture, String> {
        let settings = self.settings.lock().unwrap().clone();
        let asked = self.t_ms();
        let shot = grab(&settings, &self.recent(asked), asked)?;
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
        };
        self.push(Item::Image {
            t_ms: t,
            capture: cap.clone(),
        });
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

    /// The last few seconds of the pointer's path before `t`, in screen coordinates.
    fn recent(&self, t: u64) -> Vec<Point> {
        self.trail.lock().unwrap().since(t.saturating_sub(crate::pointer::TRAIL_MS))
    }

    fn t_ms(&self) -> u64 {
        let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
        now_ms().saturating_sub(t0)
    }

    /// Add to the recording and checkpoint it, so a crash from here on loses nothing.
    fn push(&self, item: Item) {
        let mut v = self.items.lock().unwrap();
        v.push(item);
        if !self.practice.load(Ordering::SeqCst) {
            let t0 = self.t0.lock().unwrap().unwrap_or_else(now_ms);
            let _ = self.history.checkpoint(&v, t0);
        }
    }

    /// Make a history entry the one "Paste Last Prompt" uses.
    pub fn set_last(&self, id: &str) -> Result<SessionExport, String> {
        let exp = self.history.export(id)?;
        *self.last_export.lock().unwrap() = Some(exp.clone());
        *self.last_id.lock().unwrap() = Some(id.to_string());
        Ok(exp)
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
        loop {
            if self.stop.load(Ordering::SeqCst) || self.take_no.load(Ordering::SeqCst) != no {
                break;
            }
            let all = take.all();
            let ranges = seg.feed(&all);
            self.decode_ranges(&take, &ranges, t0);
            if let Some(start) = seg.open_start() {
                let samples = take.copy(start..all.len());
                let live = self.engine.lock().unwrap().as_ref().and_then(|e| e.partial(&samples));
                if let Some(t) = live {
                    self.state.lock().unwrap().partial = t;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let all = take.all();
        let rest = {
            let mut s = seg;
            s.finish(&all)
        };
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
            let t_ms = (r.start as u64) * 1000 / audio::RATE as u64;
            let end_ms = (r.end as u64) * 1000 / audio::RATE as u64;
            let _ = t0;
            if let Some(tok) = heard.tokens {
                self.tokens.lock().unwrap().push((t_ms, tok));
            }
            self.push(Item::Speech {
                t_ms,
                end_ms,
                text,
            });
        }
    }
}

fn export_dir(s: &Settings) -> std::path::PathBuf {
    if let Some(d) = &s.export_dir {
        return std::path::PathBuf::from(d);
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Pointcast")
        .join(stamp)
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
        };
        // "this" is said at about 1.6 s while the pointer sweeps right, passing (100, 80) then.
        a.push(Item::Speech { t_ms: 1_000, end_ms: 2_700, text: "click this button".into() });
        a.push(Item::Image { t_ms: 3_000, capture: shot });
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
        let exp = a.stop(false).unwrap().unwrap();
        assert_eq!(pixel(&exp.images[0], on_ring, 80), [255; 3]);
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
}
