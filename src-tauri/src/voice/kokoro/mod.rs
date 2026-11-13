//! Kokoro-82M (Apache-2.0), the natural voice. Downloaded once, only when the person picks one of its voices, from
//! k2-fsa's official sherpa-onnx release, pinned by size and SHA-256 (archive and every file kept). It runs on this
//! computer through the sherpa-onnx runtime Pointcast already downloads for speech, by its offline TTS C API; the words
//! are turned into phonemes here (`g2p`), since sherpa-onnx 2.0 carries no espeak-ng.

pub mod g2p;

use crate::voice::models::{self, FileEntry, ModelEntry, Progress, RuntimeEntry};
use crate::voice::speak::natural::Synth;
use crate::voice::speak::{Fetched, VoiceInfo};
use std::collections::HashSet;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// The model as released: Kokoro v1.0 at full precision, with the US and UK English lexicons. (Its int8 version is a
/// third of the size but, on an Apple M5, made audio more slowly than it plays.)
pub const ID: &str = "kokoro-multi-lang-v1_0";
pub const URL: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2";
pub const BYTES: u64 = 349_886_476;
pub const SHA256: &str = "60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4";
/// The files kept from the archive (the rest, Chinese dictionaries and espeak-ng data, is left out): about 366 MB.
const FILES: [(&str, &str, u64, &str); 6] = [
    ("model.onnx", "model", 325_560_556, "b40f62b166ac8164b0627ef48a0b358eda0985e272fb03ef5252e7206305da11"),
    ("voices.bin", "voices", 28_200_960, "1c5a5b983d3d50d8586d437a51f3faa2da7919ce76a013c081e65671a3447c29"),
    ("tokens.txt", "tokens", 687, "6ebb6bb288f20f3ae8d004d3c2ca27697da27c037d75e81a60e2a6a663f95425"),
    ("lexicon-us-en.txt", "lexicon_us", 5_956_885, "7daaab53a181be9885b853a8582bf1838186317e5dadacbcef9c426d6fa0da14"),
    ("lexicon-gb-en.txt", "lexicon_gb", 6_366_635, "c4cbb37316f62210dff52718a7afcaae24f50c032cc75ab47ae67b831d1049e7"),
    ("LICENSE", "license", 11_358, "cfc7749b96f63bd31c3c42b5c471bf756814053e847c10f3eb003417bc523d30"),
];

/// The model's entry, installed by the speech models' own download (`models::install_model`).
pub fn entry() -> ModelEntry {
    ModelEntry {
        id: ID.into(),
        name: "Kokoro v1.0 (natural voice)".into(),
        family: "kokoro".into(),
        license: "Apache-2.0".into(),
        urls: vec![URL.into()],
        bytes: BYTES,
        sha256: SHA256.into(),
        root: ID.into(),
        files: FILES.iter().map(|(name, role, bytes, sha)| FileEntry { name: (*name).into(), role: Some((*role).into()), bytes: *bytes, sha256: (*sha).into() }).collect(),
        bench: serde_json::Value::Null,
        gpu_for: String::new(),
    }
}

/// Voice ids as the settings keep them: "kokoro:af_heart".
pub const PREFIX: &str = "kokoro:";
/// The recommended voice once Kokoro is here.
pub const DEFAULT: &str = "kokoro:af_heart";

/// A voice offered: its name in the model, its speaker number there, its name for people, and its English.
pub struct Voice {
    pub key: &'static str,
    pub sid: i32,
    pub name: &'static str,
    pub lang: &'static str,
}

/// The voices offered: Kokoro's American and British English voices graded C or better by its authors, the best
/// first. The speaker numbers are the model's own (its `speaker2id`).
pub const VOICES: [Voice; 15] = [
    Voice { key: "af_heart", sid: 3, name: "Heart", lang: "en-US" },
    Voice { key: "af_bella", sid: 2, name: "Bella", lang: "en-US" },
    Voice { key: "af_nicole", sid: 6, name: "Nicole", lang: "en-US" },
    Voice { key: "af_aoede", sid: 1, name: "Aoede", lang: "en-US" },
    Voice { key: "af_kore", sid: 5, name: "Kore", lang: "en-US" },
    Voice { key: "af_sarah", sid: 9, name: "Sarah", lang: "en-US" },
    Voice { key: "af_nova", sid: 7, name: "Nova", lang: "en-US" },
    Voice { key: "af_alloy", sid: 0, name: "Alloy", lang: "en-US" },
    Voice { key: "am_michael", sid: 16, name: "Michael", lang: "en-US" },
    Voice { key: "am_fenrir", sid: 14, name: "Fenrir", lang: "en-US" },
    Voice { key: "am_puck", sid: 18, name: "Puck", lang: "en-US" },
    Voice { key: "bf_emma", sid: 21, name: "Emma", lang: "en-GB" },
    Voice { key: "bf_isabella", sid: 22, name: "Isabella", lang: "en-GB" },
    Voice { key: "bm_george", sid: 26, name: "George", lang: "en-GB" },
    Voice { key: "bm_fable", sid: 25, name: "Fable", lang: "en-GB" },
];

pub fn voice(id: &str) -> Option<&'static Voice> {
    let key = id.strip_prefix(PREFIX)?;
    VOICES.iter().find(|v| v.key == key)
}

/// Phonemes the model reads at once, at most (it cuts at 400 by itself, inside a word).
const CHUNK: usize = 380;
/// The first piece of a sentence, at most (about six words), so its sound starts soon.
const FIRST: usize = 48;

/// The runtime's threads for one sentence: two thirds of the cores, two to eight (on an Apple M5's ten, six made
/// audio fastest; all ten were slower).
fn threads() -> i32 {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    (cores * 2 / 3).clamp(2, 8) as i32
}

// —— sherpa-onnx 2.0's offline TTS C API (c-api.h), field for field. ——

type P = *const c_char;

#[repr(C)]
struct Vits {
    model: P,
    lexicon: P,
    tokens: P,
    data_dir: P,
    noise_scale: f32,
    noise_scale_w: f32,
    length_scale: f32,
    dict_dir: P,
}
#[repr(C)]
struct Matcha {
    acoustic_model: P,
    vocoder: P,
    lexicon: P,
    tokens: P,
    data_dir: P,
    noise_scale: f32,
    length_scale: f32,
    dict_dir: P,
}
#[repr(C)]
struct KokoroConfig {
    model: P,
    voices: P,
    tokens: P,
    data_dir: P,
    length_scale: f32,
    dict_dir: P,
    lexicon: P,
    lang: P,
}
#[repr(C)]
struct Kitten {
    model: P,
    voices: P,
    tokens: P,
    data_dir: P,
    length_scale: f32,
    lexicon: P,
}
#[repr(C)]
struct Zipvoice {
    tokens: P,
    encoder: P,
    decoder: P,
    vocoder: P,
    data_dir: P,
    lexicon: P,
    feat_scale: f32,
    t_shift: f32,
    target_rms: f32,
    guidance_scale: f32,
}
#[repr(C)]
struct Pocket {
    lm_flow: P,
    lm_main: P,
    encoder: P,
    decoder: P,
    text_conditioner: P,
    vocab_json: P,
    token_scores_json: P,
    voice_embedding_cache_capacity: i32,
}
#[repr(C)]
struct Supertonic {
    duration_predictor: P,
    text_encoder: P,
    vector_estimator: P,
    vocoder: P,
    tts_json: P,
    unicode_indexer: P,
    voice_style: P,
}
#[repr(C)]
struct ModelConfig {
    vits: Vits,
    num_threads: i32,
    debug: i32,
    provider: P,
    matcha: Matcha,
    kokoro: KokoroConfig,
    kitten: Kitten,
    zipvoice: Zipvoice,
    pocket: Pocket,
    supertonic: Supertonic,
}
#[repr(C)]
struct TtsConfig {
    model: ModelConfig,
    rule_fsts: P,
    max_num_sentences: i32,
    rule_fars: P,
    silence_scale: f32,
}
#[repr(C)]
struct GenerationConfig {
    silence_scale: f32,
    speed: f32,
    sid: i32,
    reference_audio: *const f32,
    reference_audio_len: i32,
    reference_sample_rate: i32,
    reference_text: P,
    num_steps: i32,
    extra: P,
    phoneme_codepoints: *const i32,
    phoneme_codepoints_lens: *const i32,
    phoneme_codepoints_num_sentences: i32,
    tokens: *const P,
    tokens_lens: *const i32,
    tokens_num_sentences: i32,
}
#[repr(C)]
struct GeneratedAudio {
    samples: *const f32,
    n: i32,
    sample_rate: i32,
}

type Progressed = Option<unsafe extern "C" fn(*const f32, i32, f32, *mut c_void) -> i32>;

/// The model loaded in the runtime.
struct Engine {
    _lib: libloading::Library,
    tts: *const c_void,
    destroy: unsafe extern "C" fn(*const c_void),
    generate: unsafe extern "C" fn(*const c_void, P, *const GenerationConfig, Progressed, *mut c_void) -> *const GeneratedAudio,
    free: unsafe extern "C" fn(*const GeneratedAudio),
    rate: u32,
    /// The phonemes the model has a token for.
    tokens: HashSet<char>,
}

// The engine is used by one thread at a time, behind a lock.
unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { (self.destroy)(self.tts) }
    }
}

impl Engine {
    fn load(root: &Path, runtime: &RuntimeEntry, model: &ModelEntry) -> Result<Engine, String> {
        let lib = crate::voice::stt::open(&models::verify_runtime(root, runtime)?)?;
        let dir = models::install_dir(root, &model.id);
        let path = |role: &str| -> Result<CString, String> {
            let f = model.file(role).ok_or_else(|| format!("{} has no {role}", model.id))?;
            CString::new(dir.join(&f.name).to_string_lossy().as_bytes()).map_err(|e| e.to_string())
        };
        let (onnx, voices, tokens) = (path("model")?, path("voices")?, path("tokens")?);
        let cpu = CString::new("cpu").unwrap();
        unsafe {
            let version: libloading::Symbol<unsafe extern "C" fn() -> P> = lib.get(b"SherpaOnnxGetVersionStr\0").map_err(|e| e.to_string())?;
            let v = CStr::from_ptr(version()).to_string_lossy().into_owned();
            if !v.starts_with("2.") {
                return Err(format!("The speech runtime {v} has no natural voice; set up speech again in Settings."));
            }
            macro_rules! sym {
                ($n:literal) => {
                    *lib.get($n).map_err(|e| e.to_string())?
                };
            }
            let create: unsafe extern "C" fn(*const TtsConfig) -> *const c_void = sym!(b"SherpaOnnxCreateOfflineTts\0");
            let rate: unsafe extern "C" fn(*const c_void) -> i32 = sym!(b"SherpaOnnxOfflineTtsSampleRate\0");
            let mut cfg: TtsConfig = std::mem::zeroed();
            cfg.model.kokoro.model = onnx.as_ptr();
            cfg.model.kokoro.voices = voices.as_ptr();
            cfg.model.kokoro.tokens = tokens.as_ptr();
            cfg.model.kokoro.length_scale = 1.0;
            cfg.model.num_threads = threads();
            cfg.model.provider = cpu.as_ptr();
            cfg.max_num_sentences = 1;
            cfg.silence_scale = 0.2;
            let tts = create(&cfg);
            if tts.is_null() {
                return Err("The natural voice's files would not load. Pick the voice again to download them again.".into());
            }
            let mut e = Engine {
                destroy: sym!(b"SherpaOnnxDestroyOfflineTts\0"),
                generate: sym!(b"SherpaOnnxOfflineTtsGenerateWithConfig\0"),
                free: sym!(b"SherpaOnnxDestroyOfflineTtsGeneratedAudio\0"),
                rate: 0,
                tokens: HashSet::new(),
                tts,
                _lib: lib,
            };
            e.rate = rate(tts).max(1) as u32;
            let listed = std::fs::read_to_string(dir.join("tokens.txt")).map_err(|x| models::io_text(&x, &dir))?;
            e.tokens = token_chars(&listed);
            Ok(e)
        }
    }

    /// Phoneme pieces as audio in speaker `sid` at `speed`, each handed to `each` as soon as it is made; `each`
    /// returning false stops the rest.
    fn say(&self, pieces: &[String], sid: i32, speed: f32, each: &mut dyn FnMut(Vec<f32>) -> bool) -> Result<(), String> {
        let mut points = Vec::new();
        let mut lens = Vec::new();
        for p in pieces {
            let before = points.len();
            points.extend(p.chars().filter(|c| self.tokens.contains(c)).map(|c| c as i32));
            if points.len() > before {
                lens.push((points.len() - before) as i32);
            }
        }
        if lens.is_empty() {
            return Ok(());
        }
        /// sherpa calls this after each piece; `arg` is `each`.
        unsafe extern "C" fn piece(samples: *const f32, n: i32, _: f32, arg: *mut c_void) -> i32 {
            let each = &mut *(arg as *mut &mut dyn FnMut(Vec<f32>) -> bool);
            let v = if samples.is_null() || n <= 0 { Vec::new() } else { std::slice::from_raw_parts(samples, n as usize).to_vec() };
            each(v) as i32
        }
        let empty = CString::default();
        let mut each = each;
        unsafe {
            let mut g: GenerationConfig = std::mem::zeroed();
            g.silence_scale = 0.2;
            g.speed = speed.clamp(0.5, 2.0);
            g.sid = sid;
            g.phoneme_codepoints = points.as_ptr();
            g.phoneme_codepoints_lens = lens.as_ptr();
            g.phoneme_codepoints_num_sentences = lens.len() as i32;
            let arg = &mut each as *mut &mut dyn FnMut(Vec<f32>) -> bool as *mut c_void;
            let audio = (self.generate)(self.tts, empty.as_ptr(), &g, Some(piece), arg);
            if audio.is_null() {
                return Err("The natural voice could not say that.".into());
            }
            // The whole of it, which the pieces already carried.
            (self.free)(audio);
            Ok(())
        }
    }
}

/// The single-character tokens in the model's tokens.txt ("ə 83"; a line of just a number is the space).
fn token_chars(listed: &str) -> HashSet<char> {
    listed
        .lines()
        .filter_map(|l| {
            let mut it = l.split(' ');
            let (a, b) = (it.next()?, it.next());
            let sym = if b.is_none() || a.is_empty() { " " } else { a };
            let mut cs = sym.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => Some(c),
                _ => None,
            }
        })
        .collect()
}

/// Where a download stands, for Settings.
#[derive(Default)]
struct Fetch {
    running: bool,
    waiters: Vec<Fetched>,
    last: Option<Progress>,
}

struct Inner {
    root: PathBuf,
    model: ModelEntry,
    runtime: Option<RuntimeEntry>,
    told: Box<dyn Fn(&Progress) + Send + Sync>,
    fetch: Mutex<Fetch>,
    /// Known to be all here (checked once, then after each download or failed load).
    here: Mutex<Option<bool>>,
    engine: Mutex<Option<Engine>>,
    /// The lexicon loaded, for "us" or "gb".
    lexicon: Mutex<Option<(&'static str, Arc<g2p::Lexicon>)>>,
}

/// The natural voice, as `natural::Synth`.
#[derive(Clone)]
pub struct Kokoro(Arc<Inner>);

/// A download stops on this, not on speech setup's Cancel.
static STOP: AtomicBool = AtomicBool::new(false);

impl Kokoro {
    /// Kokoro under the speech folder `root`; `told` hears each step of a download.
    pub fn new(root: PathBuf, told: impl Fn(&Progress) + Send + Sync + 'static) -> Kokoro {
        Kokoro::with(root, entry(), models::catalog().runtime_for_host().cloned(), told)
    }

    fn with(root: PathBuf, model: ModelEntry, runtime: Option<RuntimeEntry>, told: impl Fn(&Progress) + Send + Sync + 'static) -> Kokoro {
        Kokoro(Arc::new(Inner {
            root,
            model,
            runtime,
            told: Box::new(told),
            fetch: Mutex::default(),
            here: Mutex::new(None),
            engine: Mutex::new(None),
            lexicon: Mutex::new(None),
        }))
    }

    /// The latest step of a download under way, for a Settings window opened in the middle of it.
    pub fn progress(&self) -> Option<Progress> {
        let f = self.0.fetch.lock().unwrap();
        f.running.then(|| f.last.clone()).flatten()
    }

    /// What a download would fetch now: the model, and the runtime when speech has not downloaded it.
    pub fn download_bytes(&self) -> u64 {
        let i = &self.0;
        let rt = i.runtime.as_ref().filter(|r| !models::runtime_installed(&i.root, r)).map_or(0, |r| r.bytes);
        let model = if models::model_installed(&i.root, &i.model) { 0 } else { i.model.bytes };
        rt + model
    }

    fn tell(&self, p: Progress) {
        self.0.fetch.lock().unwrap().last = Some(p.clone());
        (self.0.told)(&p);
    }

    /// Download what is missing, the runtime then the model, as one progress from 0 to their total.
    fn install(&self) -> Result<(), String> {
        let i = &self.0;
        let rt = i.runtime.as_ref().ok_or("The natural voice is not available for this computer yet.")?;
        let rt = (!models::runtime_installed(&i.root, rt)).then_some(rt);
        let need_model = !models::model_installed(&i.root, &i.model);
        let total = rt.map_or(0, |r| r.bytes) + if need_model { i.model.bytes } else { 0 };
        if let Some(free) = models::free_bytes(&i.root) {
            let need = models::space_needed(total);
            if total > 0 && free < need {
                return Err(format!("Not enough free disk space: the natural voice needs about {} MB and {} MB are free.", need / 1_000_000, free / 1_000_000));
            }
        }
        let step = |offset: u64| {
            move |p: Progress| match p {
                Progress::Downloading { received, .. } => self.tell(Progress::Downloading { received: offset + received, total: Some(total) }),
                Progress::Unpacking | Progress::Verifying => self.tell(Progress::Unpacking),
                _ => {}
            }
        };
        self.tell(Progress::Downloading { received: 0, total: Some(total) });
        let r = (|| {
            if let Some(rt) = rt {
                models::install_runtime(&i.root, rt, &step(0))?;
            }
            if need_model {
                models::install_model(&i.root, &i.model, &step(rt.map_or(0, |r| r.bytes)))?;
            }
            Ok(())
        })();
        if r.is_err() {
            // Nothing half-downloaded stays behind: the next try starts clean.
            models::drop_partial(&i.root, &i.model.id);
            if let Some(rt) = rt {
                models::drop_partial(&i.root, &rt.id);
            }
        }
        r
    }

    fn here(&self) -> bool {
        let mut h = self.0.here.lock().unwrap();
        *h.get_or_insert_with(|| {
            let i = &self.0;
            models::model_installed(&i.root, &i.model) && i.runtime.as_ref().is_some_and(|r| models::runtime_installed(&i.root, r))
        })
    }

    fn forget_here(&self) {
        *self.0.here.lock().unwrap() = None;
    }

    fn lexicon(&self, lang: &str) -> Result<Arc<g2p::Lexicon>, String> {
        let which = if lang == "en-GB" { "gb" } else { "us" };
        let mut l = self.0.lexicon.lock().unwrap();
        if let Some((w, lex)) = l.as_ref().filter(|(w, _)| *w == which) {
            let _ = w;
            return Ok(lex.clone());
        }
        let f = self.0.model.file(&format!("lexicon_{which}")).ok_or("no lexicon")?;
        let path = models::install_dir(&self.0.root, &self.0.model.id).join(&f.name);
        let text = std::fs::read_to_string(&path).map_err(|e| models::io_text(&e, &path))?;
        let lex = Arc::new(g2p::Lexicon::parse(text));
        *l = Some((which, lex.clone()));
        Ok(lex)
    }
}

impl Synth for Kokoro {
    fn voices(&self) -> Vec<VoiceInfo> {
        let download = (!self.here()).then(|| self.download_bytes());
        VOICES
            .iter()
            .map(|v| VoiceInfo { id: format!("{PREFIX}{}", v.key), name: v.name.into(), lang: v.lang.into(), natural: true, download })
            .collect()
    }

    fn owns(&self, voice: &str) -> bool {
        voice.starts_with(PREFIX)
    }

    fn installed(&self) -> bool {
        self.here()
    }

    fn default_voice(&self) -> String {
        DEFAULT.into()
    }

    fn fetch(&self, then: Fetched) {
        {
            let mut f = self.0.fetch.lock().unwrap();
            f.waiters.push(then);
            if f.running {
                return;
            }
            f.running = true;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("kokoro-fetch".into())
            .spawn(move || {
                models::stop_on(Some(&STOP));
                let r = me.install();
                me.forget_here();
                let ok = r.is_ok() && me.here();
                me.tell(match r {
                    Ok(()) if ok => Progress::Ready,
                    Ok(()) => Progress::Failed { reason: models::DAMAGED.into() },
                    Err(reason) => Progress::Failed { reason },
                });
                let waiters = {
                    let mut f = me.0.fetch.lock().unwrap();
                    f.running = false;
                    std::mem::take(&mut f.waiters)
                };
                for then in waiters {
                    then(ok);
                }
            })
            .expect("the download thread starts");
    }

    fn load(&self, voice: &str) -> Result<(), String> {
        let v = self::voice(voice).ok_or_else(|| format!("No natural voice {voice}"))?;
        self.lexicon(v.lang)?;
        let mut e = self.0.engine.lock().unwrap();
        if e.is_none() {
            let rt = self.0.runtime.as_ref().ok_or("The natural voice is not available for this computer yet.")?;
            match Engine::load(&self.0.root, rt, &self.0.model) {
                Ok(engine) => *e = Some(engine),
                Err(why) => {
                    self.forget_here();
                    return Err(why);
                }
            }
        }
        Ok(())
    }

    fn make(&self, text: &str, voice: &str, rate: f32, part: &mut dyn FnMut(Vec<f32>, u32) -> bool) -> Result<(), String> {
        self.load(voice)?;
        let v = self::voice(voice).ok_or_else(|| format!("No natural voice {voice}"))?;
        let lex = self.lexicon(v.lang)?;
        let pieces = g2p::parts(&g2p::phonemes(&lex, text), FIRST, CHUNK);
        let e = self.0.engine.lock().unwrap();
        let e = e.as_ref().ok_or("The natural voice is not loaded.")?;
        let hz = e.rate;
        e.say(&pieces, v.sid, rate, &mut |samples| part(samples, hz))
    }

    fn unload(&self) {
        *self.0.engine.lock().unwrap() = None;
        *self.0.lexicon.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::speak::natural::Sink;
    use crate::voice::speak::Speaker;
    use sha2::{Digest, Sha256};
    use std::time::{Duration, Instant};

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-kokoro-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn the_model_is_pinned_to_one_official_release_file_by_size_and_hash() {
        assert_eq!(URL, "https://github.com/k2-fsa/sherpa-onnx/releases/download/tts-models/kokoro-multi-lang-v1_0.tar.bz2");
        assert_eq!((BYTES, SHA256), (349_886_476, "60703707bc05ccd48d60e1eb723f38b07d6e474795a0b6a588d7220400e0f7a4"));
        let e = entry();
        assert_eq!((e.urls.clone(), e.license.as_str()), (vec![URL.to_string()], "Apache-2.0"));
        assert!(!models::bare(&e), "an archive, unpacked and checked file by file");
        for f in &e.files {
            assert!(f.bytes > 0 && f.sha256.len() == 64 && f.sha256.chars().all(|c| c.is_ascii_hexdigit()), "{}", f.name);
        }
        for role in ["model", "voices", "tokens", "lexicon_us", "lexicon_gb"] {
            assert!(e.file(role).is_some(), "{role}");
        }
        let kept: u64 = e.files.iter().map(|f| f.bytes).sum();
        assert!(kept < 370_000_000, "{kept} bytes kept on disk");
    }

    #[test]
    fn the_voices_are_english_unique_and_the_recommended_one_comes_first() {
        assert_eq!(VOICES[0].key, "af_heart");
        assert_eq!(voice(DEFAULT).map(|v| v.sid), Some(3));
        let keys: HashSet<_> = VOICES.iter().map(|v| v.key).collect();
        let sids: HashSet<_> = VOICES.iter().map(|v| v.sid).collect();
        assert_eq!((keys.len(), sids.len()), (VOICES.len(), VOICES.len()));
        for v in &VOICES {
            let accent = if v.key.starts_with('b') { "en-GB" } else { "en-US" };
            assert_eq!(v.lang, accent, "{}", v.key);
            assert!((0..53).contains(&v.sid));
        }
        assert!(voice("kokoro:nope").is_none() && voice("af_heart").is_none());
    }

    #[test]
    fn its_tokens_are_read_one_character_each_with_the_space() {
        let t = token_chars("; 1\n: 2\n  16\nə 83\nab 99\n");
        assert_eq!(t, [';', ':', ' ', 'ə'].into_iter().collect());
    }

    /// A Kokoro whose model downloads from a loopback server.
    fn served(name: &str, body: Vec<u8>, sha: String) -> (Kokoro, PathBuf, Arc<Mutex<Vec<Progress>>>) {
        let root = scratch(name);
        let s = models::tests::serve(body.clone(), 200, None);
        let mut m = entry();
        (m.urls, m.bytes, m.sha256) = (vec![s.url.clone()], body.len() as u64, sha);
        // The runtime is "installed": an empty one with its marker, so only the model downloads.
        let rt = RuntimeEntry { id: "rt".into(), target: "x".into(), urls: vec![], bytes: 0, sha256: "s".into(), lib: "lib".into(), files: vec![], pending: false, version: String::new() };
        std::fs::create_dir_all(root.join("rt")).unwrap();
        std::fs::write(root.join("rt").join("lib"), b"x").unwrap();
        std::fs::write(root.join("rt").join(".pointcast-installed"), "s").unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let k = Kokoro::with(root.clone(), m, Some(rt), move |p| log.lock().unwrap().push(p.clone()));
        (k, root, seen)
    }

    fn fetch_and_wait(k: &Kokoro) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        k.fetch(Box::new(move |ok| tx.send(ok).unwrap()));
        rx.recv_timeout(Duration::from_secs(20)).expect("the download ends")
    }

    #[test]
    fn a_download_with_the_wrong_checksum_is_refused_and_leaves_nothing_behind() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 253) as u8).collect();
        let (k, root, seen) = served("badsum", body, "0".repeat(64));
        assert!(!k.installed());
        assert!(!fetch_and_wait(&k), "refused");
        assert!(!k.installed());
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "rt").collect();
        assert!(left.is_empty(), "nothing left but the runtime: {left:?}");
        match seen.lock().unwrap().last() {
            Some(Progress::Failed { reason }) => assert_eq!(reason, models::DAMAGED),
            other => panic!("{other:?}"),
        }
        assert!(k.progress().is_none(), "no download under way");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_archive_with_the_right_checksum_is_cleaned_up_too() {
        let junk = b"not a bzip2 archive at all".to_vec();
        let sha = format!("{:x}", Sha256::digest(&junk));
        let (k, root, seen) = served("junk", junk, sha);
        assert!(!fetch_and_wait(&k));
        let left: Vec<_> = std::fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n != "rt").collect();
        assert!(left.is_empty(), "{left:?}");
        assert!(seen.lock().unwrap().iter().any(|p| matches!(p, Progress::Downloading { total: Some(26), .. })));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn voices_show_their_download_size_until_the_model_is_here_then_none() {
        let (k, root, _) = served("listing", vec![1, 2, 3], String::new());
        let before = k.voices();
        assert_eq!(before.len(), VOICES.len());
        assert_eq!(before[0].id, DEFAULT);
        assert!(before.iter().all(|v| v.natural && v.download == Some(3)), "the model's size, the runtime being here");
        // The model's files arrive, as a download leaves them.
        let dir = models::install_dir(&root, ID);
        std::fs::create_dir_all(&dir).unwrap();
        for f in &k.0.model.files {
            std::fs::write(dir.join(&f.name), vec![0u8; f.bytes as usize]).unwrap();
        }
        k.forget_here();
        assert!(k.installed());
        assert!(k.voices().iter().all(|v| v.download.is_none()));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_second_pick_while_downloading_joins_the_same_download() {
        let body: Vec<u8> = vec![7; 50_000];
        let (k, root, _) = served("join", body, "1".repeat(64));
        let (tx, rx) = std::sync::mpsc::channel();
        for _ in 0..2 {
            let tx = tx.clone();
            k.fetch(Box::new(move |ok| tx.send(ok).unwrap()));
        }
        let t = Instant::now();
        let got: Vec<bool> = (0..2).map(|_| rx.recv_timeout(Duration::from_secs(20)).unwrap()).collect();
        assert_eq!(got, [false, false], "both told, once each");
        assert!(t.elapsed() < Duration::from_secs(20));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Hand-run on this Mac: uses the real install if Kokoro is in Pointcast's speech folder; otherwise downloads it
    /// (about 350 MB, and the 9 MB runtime) into a scratch folder under Pointcast's data folder and deletes that at
    /// the end. Says two sentences in af_heart out loud and prints the time from text to first audio.
    /// `cargo test --lib kokoro_probe -- --ignored --nocapture`
    #[test]
    #[ignore = "downloads the natural voice and speaks out loud"]
    fn kokoro_probe() {
        use crate::voice::speak::natural::Natural;
        use crate::voice::speak::out::Speakers;
        use crate::voice::speak::{How, Talk, Voices};
        let data = dirs::data_local_dir().unwrap().join("dev.pointcast.app");
        let real = data.join("speech");
        let rt = models::catalog().runtime_for_host().cloned();
        let scratch = !(models::model_installed(&real, &entry()) && rt.as_ref().is_some_and(|r| models::runtime_installed(&real, r)));
        let root = if scratch { data.join("kokoro-probe") } else { real };
        println!("speech folder: {}{}", root.display(), if scratch { " (scratch, deleted at the end)" } else { "" });
        let k = Kokoro::with(root.clone(), entry(), rt, |p| {
            if let Progress::Downloading { received, total: Some(t) } = p {
                if received % 20_000_000 < 300_000 {
                    println!("  {} / {} MB", received / 1_000_000, t / 1_000_000);
                }
            }
        });
        if !k.installed() {
            println!("downloading {} MB…", k.download_bytes() / 1_000_000);
            let t = Instant::now();
            assert!(fetch_and_wait_long(&k), "the download works");
            println!("downloaded and checked in {:.1} s", t.elapsed().as_secs_f32());
        }
        let t = Instant::now();
        k.load(DEFAULT).expect("the model loads");
        println!("model loaded in {} ms ({} threads)", t.elapsed().as_millis(), threads());
        let lex = k.lexicon("en-US").unwrap();
        println!("lexicon: {} words", lex.len());
        println!("phonemes: {}", g2p::phonemes(&lex, "Pointcast reads 42 answers on GitHub, at 3.5x speed."));
        for s in ["Hello from Pointcast.", "This is the natural voice, made on this Mac in a moment."] {
            let t = Instant::now();
            let (mut first, mut n, mut hz) = (None, 0usize, 1u32);
            k.make(s, DEFAULT, 1.0, &mut |a, rate| {
                first.get_or_insert(t.elapsed());
                (n, hz) = (n + a.len(), rate);
                true
            })
            .unwrap();
            println!("made {:.2} s of audio in {} ms, its first piece in {} ms: {s}", n as f32 / hz as f32, t.elapsed().as_millis(), first.unwrap().as_millis());
        }
        let sink = Arc::new(Speakers::default());
        // POINTCAST_PROBE_MUTE: the same path, playing zeros, to time it without a sound.
        let muted = std::env::var_os("POINTCAST_PROBE_MUTE").is_some();
        let out: Arc<dyn Sink> = if muted { Arc::new(Muted(sink.clone())) } else { sink.clone() };
        let voices = Arc::new(Voices::new(Arc::new(crate::voice::speak::Unsupported), Some(Natural::new(Arc::new(k.clone()), out))));
        let how = How { voice: Some(DEFAULT.into()), rate: 1.0 };
        // As the app does when a question is sent: the voice loads and the speakers wake while the answer is on its way.
        voices.ready(&how, None);
        std::thread::sleep(Duration::from_millis(300));
        println!("{}", if muted { "playing muted" } else { "playing out loud" });
        let heard = Arc::new(Mutex::new(Vec::<Instant>::new()));
        let h = heard.clone();
        let talk = Talk::new(voices, move |speaking| {
            if speaking {
                h.lock().unwrap().push(Instant::now());
            }
        });
        let from = Instant::now();
        talk.say("Hello from Pointcast.".into(), how.clone());
        talk.say("This is the natural voice, made on this Mac while the first sentence plays.".into(), how);
        let first = sink.first_sound_after(from, Duration::from_secs(10)).expect("sound starts");
        println!("time from text to first audio: {} ms", first.duration_since(from).as_millis());
        assert!(talk.wait_idle(Duration::from_secs(30)), "both sentences end");
        println!("both sentences said in {:.1} s", from.elapsed().as_secs_f32());
        if let Some(gap) = sink.gaps().into_iter().next() {
            println!("silence between the sentences, beyond the voice's own: {} ms", gap.as_millis());
        }
        let size = |d: &Path| walk(d).iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum::<u64>();
        println!("on disk: model {} MB, whole folder {} MB", size(&models::install_dir(&root, ID)) / 1_000_000, size(&root) / 1_000_000);
        drop(talk);
        if scratch && std::env::var_os("POINTCAST_KEEP_PROBE").is_none() {
            std::fs::remove_dir_all(&root).unwrap();
            println!("deleted {}", root.display());
        }
    }

    /// Speakers that play zeros in place of the voice, for timing without a sound.
    struct Muted(Arc<crate::voice::speak::out::Speakers>);

    impl Sink for Muted {
        fn play(&self, samples: Vec<f32>, rate: u32, ended: crate::voice::speak::Done) -> Result<(), String> {
            self.0.play(vec![0.0; samples.len()], rate, ended)
        }
        fn stop(&self) {
            self.0.stop()
        }
        fn wake(&self) {
            self.0.wake()
        }
    }

    fn walk(d: &Path) -> Vec<PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
        out
    }

    fn fetch_and_wait_long(k: &Kokoro) -> bool {
        let (tx, rx) = std::sync::mpsc::channel();
        k.fetch(Box::new(move |ok| tx.send(ok).unwrap()));
        rx.recv_timeout(Duration::from_secs(900)).expect("the download ends")
    }
}
