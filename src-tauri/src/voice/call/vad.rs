//! Is this window of audio speech? Silero VAD (MIT), run on this computer by the sherpa-onnx runtime speech already
//! downloads (its voice activity detector C API), from a 2.3 MB model downloaded once and pinned by URL, size and
//! SHA-256; or, until those are here, a detector on the level alone (`Energy`).

use crate::voice::audio;
use crate::voice::models::{self, FileEntry, ModelEntry, RuntimeEntry};
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::Path;

/// Samples per window: Silero's own at 16 kHz (32 ms).
pub const WINDOW: usize = 512;

/// Says whether each window of `WINDOW` samples (16 kHz mono) is speech.
pub trait Vad: Send {
    fn speech(&mut self, window: &[f32]) -> bool;
    /// What it is, for the log.
    fn name(&self) -> &'static str;
}

/// Silero VAD v5 as k2-fsa publishes it for sherpa-onnx.
pub const ID: &str = "silero-vad-v5";
pub const URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/asr-models/silero_vad_v5.onnx";
pub const FILE: &str = "silero_vad.onnx";
pub const BYTES: u64 = 2_313_101;
pub const SHA256: &str = "6b99cbfd39246b6706f98ec13c7c50c6b299181f2474fa05cbc8046acc274396";
/// A window is speech above this probability (Silero's own default); once speech, down to 0.15 under it.
const THRESHOLD: f32 = 0.5;

/// The model's entry, downloaded by the speech models' own download (`models::install_model`).
pub fn entry() -> ModelEntry {
    ModelEntry {
        id: ID.into(),
        name: "Silero VAD v5 (end of turn)".into(),
        family: "silero-vad".into(),
        license: "MIT".into(),
        urls: vec![URL.into()],
        bytes: BYTES,
        sha256: SHA256.into(),
        root: ID.into(),
        files: vec![FileEntry { name: FILE.into(), role: Some("model".into()), bytes: BYTES, sha256: SHA256.into() }],
        bench: serde_json::Value::Null,
        gpu_for: String::new(),
    }
}

// —— sherpa-onnx 2.0's VAD C API (c-api.h), field for field. ——

type P = *const c_char;

#[repr(C)]
struct WindowedConfig {
    model: P,
    threshold: f32,
    min_silence_duration: f32,
    min_speech_duration: f32,
    window_size: i32,
    max_speech_duration: f32,
}
#[repr(C)]
struct VadConfig {
    silero_vad: WindowedConfig,
    sample_rate: i32,
    num_threads: i32,
    provider: P,
    debug: i32,
    ten_vad: WindowedConfig,
}
#[repr(C)]
struct Segment {
    start: i64,
    samples: *mut f32,
    n: i32,
}

type Ptr = *const c_void;

/// Silero, loaded in the runtime. Its own smoothing is set to one window, so each window's answer is the model's (with
/// its hysteresis); what makes a turn is `turn::Turn`'s.
pub struct Silero {
    _lib: libloading::Library,
    vad: Ptr,
    destroy: unsafe extern "C" fn(Ptr),
    accept: unsafe extern "C" fn(Ptr, *const f32, i32),
    detected: unsafe extern "C" fn(Ptr) -> i32,
    empty: unsafe extern "C" fn(Ptr) -> i32,
    front: unsafe extern "C" fn(Ptr) -> *const Segment,
    pop: unsafe extern "C" fn(Ptr),
    free_segment: unsafe extern "C" fn(*const Segment),
}

// Used by the call's thread only.
unsafe impl Send for Silero {}

impl Drop for Silero {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.vad) }
    }
}

impl Silero {
    /// Load the model under the speech folder `root` in the pinned runtime. Err when either is not here (or not the
    /// pinned files).
    pub fn load(root: &Path, runtime: &RuntimeEntry) -> Result<Silero, String> {
        let model = entry();
        if !models::model_installed(root, &model) {
            return Err("The end-of-turn model is not downloaded yet.".into());
        }
        let path = models::install_dir(root, ID).join(FILE);
        models::check(&path, BYTES, SHA256)?;
        Silero::open(&models::verify_runtime(root, runtime)?, &path)
    }

    /// Load the model at `model` in the runtime library at `lib`.
    pub fn open(lib: &Path, model: &Path) -> Result<Silero, String> {
        let lib = crate::voice::stt::open(lib)?;
        let model = CString::new(model.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let cpu = CString::new("cpu").unwrap();
        unsafe {
            let version: libloading::Symbol<unsafe extern "C" fn() -> P> = lib.get(b"SherpaOnnxGetVersionStr\0").map_err(|e| e.to_string())?;
            let v = CStr::from_ptr(version()).to_string_lossy().into_owned();
            if !v.starts_with("2.") {
                return Err(format!("The speech runtime {v} is not the one Pointcast pins."));
            }
            macro_rules! sym {
                ($n:literal) => {
                    *lib.get($n).map_err(|e| e.to_string())?
                };
            }
            let create: unsafe extern "C" fn(*const VadConfig, f32) -> Ptr = sym!(b"SherpaOnnxCreateVoiceActivityDetector\0");
            let mut cfg: VadConfig = std::mem::zeroed();
            cfg.silero_vad.model = model.as_ptr();
            cfg.silero_vad.threshold = THRESHOLD;
            // One window of each: the answer per window, not sherpa's segments.
            cfg.silero_vad.min_silence_duration = WINDOW as f32 / audio::RATE as f32;
            cfg.silero_vad.min_speech_duration = WINDOW as f32 / audio::RATE as f32;
            cfg.silero_vad.window_size = WINDOW as i32;
            cfg.silero_vad.max_speech_duration = 3600.0;
            cfg.sample_rate = audio::RATE as i32;
            cfg.num_threads = 1;
            cfg.provider = cpu.as_ptr();
            let vad = create(&cfg, 30.0);
            if vad.is_null() {
                return Err("The end-of-turn model would not load.".into());
            }
            Ok(Silero {
                destroy: sym!(b"SherpaOnnxDestroyVoiceActivityDetector\0"),
                accept: sym!(b"SherpaOnnxVoiceActivityDetectorAcceptWaveform\0"),
                detected: sym!(b"SherpaOnnxVoiceActivityDetectorDetected\0"),
                empty: sym!(b"SherpaOnnxVoiceActivityDetectorEmpty\0"),
                front: sym!(b"SherpaOnnxVoiceActivityDetectorFront\0"),
                pop: sym!(b"SherpaOnnxVoiceActivityDetectorPop\0"),
                free_segment: sym!(b"SherpaOnnxDestroySpeechSegment\0"),
                vad,
                _lib: lib,
            })
        }
    }
}

impl Vad for Silero {
    fn speech(&mut self, window: &[f32]) -> bool {
        unsafe {
            (self.accept)(self.vad, window.as_ptr(), window.len() as i32);
            let speech = (self.detected)(self.vad) != 0;
            // The segments it keeps are not used: let them go as they come.
            while (self.empty)(self.vad) == 0 {
                (self.free_segment)((self.front)(self.vad));
                (self.pop)(self.vad);
            }
            speech
        }
    }

    fn name(&self) -> &'static str {
        "silero"
    }
}

/// Speech by level alone: a window is speech when it stands `ON` times over the room's noise floor and above
/// `audio::MIN_VOICE`. The floor follows the room down at once and up slowly (about 1 dB a second), so a steady fan or
/// hum becomes the floor and a voice never does. The hangover that keeps a pause inside a turn is `turn::Turn`'s.
#[derive(Debug, Clone)]
pub struct Energy {
    floor: f32,
}

const ON: f32 = 3.0;
/// The floor rises by this share a window (32 ms): about 1 dB a second.
const FLOOR_RISE: f32 = 1.0037;

impl Default for Energy {
    fn default() -> Self {
        Energy { floor: audio::MIN_VOICE / ON }
    }
}

impl Vad for Energy {
    fn speech(&mut self, window: &[f32]) -> bool {
        let rms = audio::rms(window);
        let speech = rms >= audio::MIN_VOICE.max(self.floor * ON);
        self.floor = if rms < self.floor { rms.max(1e-5) } else { (self.floor * FLOOR_RISE).min(rms) };
        speech
    }

    fn name(&self) -> &'static str {
        "energy"
    }
}

/// The detector to use: Silero when it and the runtime are here, else `Energy`.
pub fn best(root: &Path) -> Box<dyn Vad> {
    let rt = models::catalog().runtime_for_host().cloned();
    match rt.map(|r| Silero::load(root, &r)) {
        Some(Ok(s)) => Box::new(s),
        Some(Err(why)) => {
            eprintln!("[call] level-only end of turn: {why}");
            Box::new(Energy::default())
        }
        None => Box::new(Energy::default()),
    }
}

/// Download Silero once, in the background, when the runtime it needs is here; `done` hears whether it is.
pub fn fetch(root: std::path::PathBuf) {
    let Some(rt) = models::catalog().runtime_for_host().cloned() else { return };
    if !models::runtime_installed(&root, &rt) || models::model_installed(&root, &entry()) {
        return;
    }
    std::thread::spawn(move || {
        if let Err(why) = models::install_model(&root, &entry(), &|_| {}) {
            eprintln!("[call] the end-of-turn model did not download: {why}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rms: f32) -> Vec<f32> {
        (0..WINDOW).map(|i| rms * std::f32::consts::SQRT_2 * (i as f32 * 0.07).sin()).collect()
    }

    #[test]
    fn the_level_detector_hears_a_voice_over_the_room_and_learns_a_steady_hum_as_the_room() {
        let mut e = Energy::default();
        for _ in 0..30 {
            assert!(!e.speech(&tone(0.0005)), "a quiet room");
        }
        assert!(e.speech(&tone(0.03)), "a voice");
        assert!(!e.speech(&tone(0.001)), "under MIN_VOICE is never a voice");
        // A fan at -40 dBFS for a minute becomes the room...
        let fan = tone(0.01);
        let mut heard = 0;
        for _ in 0..1900 {
            heard += e.speech(&fan) as u32;
        }
        assert!(!e.speech(&fan), "the hum is the room after a minute ({heard} windows heard as speech on the way)");
        // ...and a voice over it is still heard.
        assert!(e.speech(&tone(0.06)));
    }

    #[test]
    fn the_model_is_pinned_by_url_size_and_sha256() {
        let m = entry();
        assert!(m.urls.iter().all(|u| u.starts_with("https://github.com/k2-fsa/sherpa-onnx/releases/download/")));
        assert_eq!((m.files.len(), m.files[0].bytes, m.bytes), (1, BYTES, BYTES));
        assert_eq!(m.files[0].sha256.len(), 64);
        assert_eq!(m.sha256, m.files[0].sha256);
        assert_eq!(std::mem::size_of::<WindowedConfig>(), 32, "c-api.h's SherpaOnnxSileroVadModelConfig");
        assert_eq!(std::mem::size_of::<VadConfig>(), 32 + 8 + 8 + 8 + 32, "c-api.h's SherpaOnnxVadModelConfig");
    }
}
