//! Local speech to text: Moonshine v2 through sherpa-onnx's C API, loaded at run time
//! from the downloaded runtime, never linked. Structs mirror sherpa-onnx 2.0.0 c-api.h.
#![allow(non_snake_case, dead_code)]
use super::models::{self, ModelEntry, RuntimeEntry};
use std::ffi::{c_char, c_float, CStr, CString};
use std::path::Path;

pub const PINNED_VERSION: &str = "2.0.0";

type P = *const c_char;

#[repr(C)]
struct FeatureConfig {
    sample_rate: i32,
    feature_dim: i32,
}
#[repr(C)]
struct Transducer {
    encoder: P,
    decoder: P,
    joiner: P,
}
#[repr(C)]
struct One {
    model: P,
}
#[repr(C)]
struct Whisper {
    encoder: P,
    decoder: P,
    language: P,
    task: P,
    tail_paddings: i32,
    enable_token_timestamps: i32,
    enable_segment_timestamps: i32,
}
#[repr(C)]
struct SenseVoice {
    model: P,
    language: P,
    use_itn: i32,
}
#[repr(C)]
struct Moonshine {
    preprocessor: P,
    encoder: P,
    uncached_decoder: P,
    cached_decoder: P,
    merged_decoder: P,
}
#[repr(C)]
struct Two {
    encoder: P,
    decoder: P,
}
#[repr(C)]
struct Canary {
    encoder: P,
    decoder: P,
    src_lang: P,
    tgt_lang: P,
    use_pnc: i32,
}
#[repr(C)]
struct FunAsrNano {
    encoder_adaptor: P,
    llm: P,
    embedding: P,
    tokenizer: P,
    system_prompt: P,
    user_prompt: P,
    max_new_tokens: i32,
    temperature: c_float,
    top_p: c_float,
    seed: i32,
    language: P,
    itn: i32,
    hotwords: P,
}
#[repr(C)]
struct Qwen3 {
    conv_frontend: P,
    encoder: P,
    decoder: P,
    tokenizer: P,
    max_total_len: i32,
    max_new_tokens: i32,
    temperature: c_float,
    top_p: c_float,
    seed: i32,
    hotwords: P,
}
#[repr(C)]
struct Cohere {
    encoder: P,
    decoder: P,
    language: P,
    use_punct: i32,
    use_itn: i32,
}
#[repr(C)]
struct ModelConfig {
    transducer: Transducer,
    paraformer: One,
    nemo_ctc: One,
    whisper: Whisper,
    tdnn: One,
    tokens: P,
    num_threads: i32,
    debug: i32,
    provider: P,
    model_type: P,
    modeling_unit: P,
    bpe_vocab: P,
    telespeech_ctc: P,
    sense_voice: SenseVoice,
    moonshine: Moonshine,
    fire_red_asr: Two,
    dolphin: One,
    zipformer_ctc: One,
    canary: Canary,
    wenet_ctc: One,
    omnilingual: One,
    medasr: One,
    funasr_nano: FunAsrNano,
    fire_red_asr_ctc: One,
    qwen3_asr: Qwen3,
    cohere_transcribe: Cohere,
}
#[repr(C)]
struct LmConfig {
    model: P,
    scale: c_float,
}
#[repr(C)]
struct Homophone {
    dict_dir: P,
    lexicon: P,
    rule_fsts: P,
}
#[repr(C)]
struct CtcFst {
    graph: P,
    max_active: i32,
}
#[repr(C)]
struct RecognizerConfig {
    feat_config: FeatureConfig,
    model_config: ModelConfig,
    lm_config: LmConfig,
    decoding_method: P,
    max_active_paths: i32,
    hotwords_file: P,
    hotwords_score: c_float,
    rule_fsts: P,
    rule_fars: P,
    blank_penalty: c_float,
    hr: Homophone,
    ctc_fst_decoder_config: CtcFst,
}
/// The leading fields of `SherpaOnnxOfflineRecognizerResult`; sherpa owns the rest, which is never read.
#[repr(C)]
struct ResultHead {
    text: P,
    /// Token start times in seconds from the start of the audio, `count` of them, parallel to `tokens_arr`; null when
    /// the model gives none.
    timestamps: *const c_float,
    count: i32,
    tokens: P,
    tokens_arr: *const P,
}

/// What a model heard: the text and, when the model gives them, its tokens with their start in ms.
#[derive(Debug, Default, PartialEq)]
pub struct Heard {
    pub text: String,
    pub tokens: Option<Vec<(String, u64)>>,
}

/// The tokens of a result with their start times, or None when the model gave no times.
unsafe fn token_times(r: &ResultHead) -> Option<Vec<(String, u64)>> {
    if r.timestamps.is_null() || r.tokens_arr.is_null() || r.count <= 0 {
        return None;
    }
    let n = r.count as usize;
    let (toks, secs) = (std::slice::from_raw_parts(r.tokens_arr, n), std::slice::from_raw_parts(r.timestamps, n));
    toks.iter()
        .zip(secs)
        .map(|(t, s)| (!t.is_null()).then(|| (CStr::from_ptr(*t).to_string_lossy().into_owned(), (s.max(0.0) * 1000.0).round() as u64)))
        .collect()
}

type Opaque = std::ffi::c_void;

struct Api {
    _lib: libloading::Library,
    create: unsafe extern "C" fn(*const RecognizerConfig) -> *const Opaque,
    destroy: unsafe extern "C" fn(*const Opaque),
    stream: unsafe extern "C" fn(*const Opaque) -> *const Opaque,
    destroy_stream: unsafe extern "C" fn(*const Opaque),
    accept: unsafe extern "C" fn(*const Opaque, i32, *const c_float, i32),
    decode: unsafe extern "C" fn(*const Opaque, *const Opaque),
    result: unsafe extern "C" fn(*const Opaque) -> *const ResultHead,
    destroy_result: unsafe extern "C" fn(*const ResultHead),
}

pub struct Engine {
    api: Api,
    rec: *const Opaque,
    pub model: String,
    pub load_ms: u64,
    /// Live captions while someone is still talking: only for models quick enough to redo the
    /// sentence often (Whisper is not), and at most every `PARTIAL_EVERY`.
    live: bool,
    last_partial: std::sync::Mutex<Option<std::time::Instant>>,
}

pub const PARTIAL_EVERY: std::time::Duration = std::time::Duration::from_millis(500);

/// A piece with this much voice is never dropped for decoding to nothing.
pub const SURE_VOICE_MS: u64 = 1_500;

/// The text kept for a piece of clear voice the model could not make out, so the loss shows.
pub const UNCLEAR: &str = "[unclear]";

/// A piece with less voice than this (0.6 s) is never sent: every model, given a breath or a click at the end of a
/// take, makes up words for it ("Thank you.", "I'm going to go.", "you").
pub const MIN_VOICED_MS: u64 = 600;

/// What a piece of a take says, through `hear`. One with under `MIN_VOICED_MS` of voice is dropped unheard. A quiet
/// piece is raised first. One with `SURE_VOICE_MS` of voice that comes back empty is heard once more at full level and,
/// if still empty, kept as `UNCLEAR`; a shorter one is dropped.
pub fn hear_piece(samples: &[f32], voiced_ms: u64, hear: impl Fn(&[f32]) -> Heard) -> Option<Heard> {
    if voiced_ms < MIN_VOICED_MS {
        return None;
    }
    let h = hear(&super::audio::louder(samples, false));
    if !h.text.is_empty() {
        return Some(h);
    }
    if voiced_ms < SURE_VOICE_MS {
        return None;
    }
    let h = hear(&super::audio::louder(samples, true));
    if !h.text.is_empty() {
        return Some(h);
    }
    Some(Heard { text: UNCLEAR.into(), tokens: None })
}

/// Families fast enough on a CPU to re-decode the open sentence for a live caption.
pub fn live_captions(family: &str) -> bool {
    family != "whisper"
}

unsafe impl Send for Engine {}

impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { (self.api.destroy)(self.rec) }
    }
}

fn cs(p: &Path) -> Result<CString, String> {
    CString::new(p.to_string_lossy().as_bytes()).map_err(|e| e.to_string())
}

/// The path handed to the loader, in this system's separators. On Windows a forward slash makes
/// LOAD_WITH_ALTERED_SEARCH_PATH search the usual places for the libraries it pulls in, and System32 has an
/// onnxruntime.dll of another version.
fn loader_path(lib: &Path) -> std::path::PathBuf {
    lib.components().collect()
}

fn open(lib: &Path) -> Result<libloading::Library, String> {
    let lib = &loader_path(lib);
    #[cfg(windows)]
    unsafe {
        const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
        libloading::os::windows::Library::load_with_flags(lib, LOAD_WITH_ALTERED_SEARCH_PATH)
            .map(Into::into)
            .map_err(|e| e.to_string())
    }
    #[cfg(not(windows))]
    unsafe {
        libloading::Library::new(lib).map_err(|e| e.to_string())
    }
}

pub fn load(
    root: &Path,
    runtime: &RuntimeEntry,
    model: &ModelEntry,
    threads: i32,
) -> Result<Engine, String> {
    let t0 = std::time::Instant::now();
    models::find_lib(root, runtime).ok_or("speech runtime is not installed")?;
    if !models::model_installed(root, model) {
        return Err("Not downloaded".into());
    }
    let lib = open(&models::verify_runtime(root, runtime)?)?;
    unsafe {
        let version: libloading::Symbol<unsafe extern "C" fn() -> P> = lib
            .get(b"SherpaOnnxGetVersionStr\0")
            .map_err(|e| e.to_string())?;
        let v = CStr::from_ptr(version()).to_string_lossy().into_owned();
        if v != PINNED_VERSION && !v.starts_with("2.0") {
            return Err(format!("speech runtime {v} is not {PINNED_VERSION}"));
        }
        macro_rules! sym {
            ($n:literal) => {
                *lib.get($n).map_err(|e| e.to_string())?
            };
        }
        let api = Api {
            create: sym!(b"SherpaOnnxCreateOfflineRecognizer\0"),
            destroy: sym!(b"SherpaOnnxDestroyOfflineRecognizer\0"),
            stream: sym!(b"SherpaOnnxCreateOfflineStream\0"),
            destroy_stream: sym!(b"SherpaOnnxDestroyOfflineStream\0"),
            accept: sym!(b"SherpaOnnxAcceptWaveformOffline\0"),
            decode: sym!(b"SherpaOnnxDecodeOfflineStream\0"),
            result: sym!(b"SherpaOnnxGetOfflineStreamResult\0"),
            destroy_result: sym!(b"SherpaOnnxDestroyOfflineRecognizerResult\0"),
            _lib: lib,
        };
        let dir = models::install_dir(root, &model.id);
        let path = |role: &str| -> Result<CString, String> {
            let f = model
                .file(role)
                .ok_or_else(|| format!("{} has no {role}", model.id))?;
            cs(&dir.join(&f.name))
        };
        let (enc, tok) = (path("encoder")?, path("tokens")?);
        let cpu = CString::new("cpu").unwrap();
        let greedy = CString::new("greedy_search").unwrap();
        let mut cfg: RecognizerConfig = std::mem::zeroed();
        cfg.feat_config = FeatureConfig {
            sample_rate: 16000,
            feature_dim: 80,
        };
        let keep: Vec<CString> = match model.family.as_str() {
            "moonshine" => {
                let dec = path("merged_decoder")?;
                cfg.model_config.moonshine.encoder = enc.as_ptr();
                cfg.model_config.moonshine.merged_decoder = dec.as_ptr();
                vec![dec]
            }
            "whisper" => {
                // An empty language lets Whisper detect it; "transcribe" keeps the speaker's language.
                let (dec, lang, task) = (path("decoder")?, CString::new("").unwrap(), CString::new("transcribe").unwrap());
                cfg.model_config.whisper.encoder = enc.as_ptr();
                cfg.model_config.whisper.decoder = dec.as_ptr();
                cfg.model_config.whisper.language = lang.as_ptr();
                cfg.model_config.whisper.task = task.as_ptr();
                cfg.model_config.whisper.tail_paddings = -1;
                vec![dec, lang, task]
            }
            "transducer" => {
                let (dec, join) = (path("decoder")?, path("joiner")?);
                let kind = CString::new("nemo_transducer").unwrap();
                cfg.model_config.transducer.encoder = enc.as_ptr();
                cfg.model_config.transducer.decoder = dec.as_ptr();
                cfg.model_config.transducer.joiner = join.as_ptr();
                cfg.model_config.model_type = kind.as_ptr();
                vec![dec, join, kind]
            }
            other => return Err(format!("{other} models are not supported")),
        };
        cfg.model_config.tokens = tok.as_ptr();
        cfg.model_config.num_threads = threads.max(1);
        cfg.model_config.provider = cpu.as_ptr();
        cfg.decoding_method = greedy.as_ptr();
        let rec = (api.create)(&cfg);
        drop(keep);
        if rec.is_null() {
            return Err("the speech model did not load".into());
        }
        Ok(Engine {
            api,
            rec,
            model: model.id.clone(),
            load_ms: t0.elapsed().as_millis() as u64,
            live: live_captions(&model.family),
            last_partial: std::sync::Mutex::new(None),
        })
    }
}

impl Engine {
    /// A live caption for the sentence still being spoken, or None when it is not time yet or
    /// this model is too slow to redo it while the person talks.
    pub fn partial(&self, samples: &[f32]) -> Option<String> {
        if !self.wants_partial() {
            return None;
        }
        let mut last = self.last_partial.lock().unwrap();
        if last.is_some_and(|t| t.elapsed() < PARTIAL_EVERY) {
            return None;
        }
        *last = Some(std::time::Instant::now());
        drop(last);
        Some(self.transcribe(samples))
    }

    /// Whether a live caption is due, before the open sentence is copied out for one.
    pub fn wants_partial(&self) -> bool {
        self.live && !self.last_partial.lock().unwrap().is_some_and(|t| t.elapsed() < PARTIAL_EVERY)
    }

    pub fn transcribe(&self, samples: &[f32]) -> String {
        self.hear(samples).text
    }

    /// The text and, where the model gives them, when each token started. Moonshine gives none, so its words are timed
    /// by estimate.
    pub fn hear(&self, samples: &[f32]) -> Heard {
        if samples.is_empty() {
            return Heard::default();
        }
        unsafe {
            let s = (self.api.stream)(self.rec);
            if s.is_null() {
                return Heard::default();
            }
            (self.api.accept)(s, 16000, samples.as_ptr(), samples.len() as i32);
            (self.api.decode)(self.rec, s);
            let r = (self.api.result)(s);
            let heard = if r.is_null() {
                Heard::default()
            } else {
                Heard {
                    text: if (*r).text.is_null() {
                        String::new()
                    } else {
                        CStr::from_ptr((*r).text).to_string_lossy().trim().to_string()
                    },
                    tokens: token_times(&*r),
                }
            };
            if !r.is_null() {
                (self.api.destroy_result)(r);
            }
            (self.api.destroy_stream)(s);
            heard
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn the_mirrored_structs_have_the_headers_layout() {
        if size_of::<usize>() != 8 {
            return;
        }
        assert_eq!(size_of::<Whisper>(), 48);
        assert_eq!(size_of::<FunAsrNano>(), 88);
        assert_eq!(size_of::<Qwen3>(), 64);
        assert_eq!(offset_of!(ModelConfig, tokens), 24 + 8 + 8 + 48 + 8);
        assert_eq!(offset_of!(ModelConfig, moonshine), 96 + 8 + 8 + 8 * 5 + 24);
        assert_eq!(
            size_of::<ModelConfig>(),
            176 + 40 + 16 + 8 + 8 + 40 + 8 * 3 + 88 + 8 + 64 + 32
        );
        assert_eq!(offset_of!(RecognizerConfig, model_config), 8);
        assert_eq!(
            offset_of!(RecognizerConfig, decoding_method),
            8 + size_of::<ModelConfig>() + 16
        );
        // SherpaOnnxOfflineRecognizerResult: text, timestamps, count, tokens, tokens_arr, then fields never read.
        assert_eq!(
            (offset_of!(ResultHead, timestamps), offset_of!(ResultHead, count), offset_of!(ResultHead, tokens), offset_of!(ResultHead, tokens_arr)),
            (8, 16, 24, 32)
        );
    }

    #[test]
    fn a_runtime_library_that_changed_since_setup_is_not_loaded() {
        use sha2::{Digest, Sha256};
        let root = std::env::temp_dir().join(format!("pc-stt-changed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let model: ModelEntry = serde_json::from_value(serde_json::json!({
            "id": "m", "family": "moonshine", "urls": [],
            "files": [
                { "name": "e.ort", "role": "encoder", "bytes": 1 },
                { "name": "d.ort", "role": "merged_decoder", "bytes": 1 },
                { "name": "t.txt", "role": "tokens", "bytes": 1 }
            ]
        }))
        .unwrap();
        std::fs::create_dir_all(models::install_dir(&root, "m")).unwrap();
        for f in &model.files {
            std::fs::write(models::install_dir(&root, "m").join(&f.name), b"x").unwrap();
        }
        // The library as set up, then another program's of the same size in its place.
        let pinned = b"the speech library as downloaded".to_vec();
        let rt: RuntimeEntry = serde_json::from_value(serde_json::json!({
            "id": "rt", "target": "any", "urls": [], "lib": "speech.lib",
            "files": [{ "name": "pkg/lib/speech.lib", "bytes": pinned.len(), "sha256": format!("{:x}", Sha256::digest(&pinned)) }]
        }))
        .unwrap();
        let dir = models::install_dir(&root, "rt").join("pkg").join("lib");
        std::fs::create_dir_all(&dir).unwrap();
        let mut swapped = pinned.clone();
        swapped[3] ^= 1;
        std::fs::write(dir.join("speech.lib"), &swapped).unwrap();
        match load(&root, &rt, &model, 1) {
            Err(e) => assert_eq!(e, models::CHANGED),
            Ok(_) => panic!("a library that changed was loaded"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_piece_of_clear_voice_is_never_dropped_for_decoding_to_nothing() {
        let tries = std::cell::RefCell::new(vec![]);
        let deaf = |s: &[f32]| {
            tries.borrow_mut().push(s.iter().fold(0f32, |m, v| m.max(v.abs())));
            Heard::default()
        };
        let quiet = vec![0.05f32; 16_000];
        assert_eq!(hear_piece(&quiet, 1_000, &deaf), None, "a short sound that says nothing is dropped");
        assert_eq!(tries.take(), vec![0.5], "heard once, raised");
        assert_eq!(hear_piece(&quiet, SURE_VOICE_MS, &deaf).unwrap().text, UNCLEAR);
        assert_eq!(tries.take().len(), 2, "heard twice before it is kept as unclear");
        let loud = vec![0.4f32; 16_000];
        let second = |s: &[f32]| Heard { text: if s[0] > 0.45 { "got it".into() } else { String::new() }, tokens: None };
        assert_eq!(hear_piece(&loud, SURE_VOICE_MS, second).unwrap().text, "got it", "the retry is at full level");
        let fine = |_: &[f32]| Heard { text: "hello".into(), tokens: None };
        assert_eq!(hear_piece(&loud, MIN_VOICED_MS, fine).unwrap().text, "hello");
    }

    #[test]
    fn a_piece_with_almost_no_voice_is_never_sent_to_a_model() {
        let made_up = |_: &[f32]| -> Heard { panic!("a breath at the end of a take was sent") };
        let tail = vec![0.3f32; 17_600];
        for voiced in [0, 200, MIN_VOICED_MS - 1] {
            assert_eq!(hear_piece(&tail, voiced, made_up), None, "{voiced} ms of voice");
        }
        let said = |_: &[f32]| Heard { text: "Thank you.".into(), tokens: None };
        assert!(hear_piece(&tail, MIN_VOICED_MS, said).is_some(), "a short word with enough voice is heard");
    }

    #[test]
    fn the_library_handed_to_the_loader_has_no_forward_slashes_on_windows() {
        let dir = std::env::temp_dir().to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        let p = loader_path(Path::new(&format!("{dir}/speech/runtime/pkg/lib/sherpa-onnx-c-api.dll")));
        if cfg!(windows) {
            assert!(!p.to_string_lossy().contains('/'), "{}", p.display());
        }
        assert!(p.ends_with("pkg/lib/sherpa-onnx-c-api.dll") && p.is_absolute(), "{}", p.display());
    }

    #[test]
    fn token_times_are_read_when_the_model_gives_them() {
        let names = [CString::new("▁click").unwrap(), CString::new("▁th").unwrap(), CString::new("is").unwrap()];
        let arr: Vec<P> = names.iter().map(|c| c.as_ptr()).collect();
        let secs: [c_float; 3] = [0.08, 0.52, 0.6004];
        let text = CString::new("click this").unwrap();
        let mut r = ResultHead {
            text: text.as_ptr(),
            timestamps: secs.as_ptr(),
            count: 3,
            tokens: std::ptr::null(),
            tokens_arr: arr.as_ptr(),
        };
        let want = vec![("▁click".to_string(), 80), ("▁th".to_string(), 520), ("is".to_string(), 600)];
        assert_eq!(unsafe { token_times(&r) }, Some(want));
        r.timestamps = std::ptr::null();
        assert_eq!(unsafe { token_times(&r) }, None, "a model without times");
        r.timestamps = secs.as_ptr();
        r.count = 0;
        assert_eq!(unsafe { token_times(&r) }, None);
    }

    /// Whether an installed model gives token times on real speech. Reads only.
    /// `POINTCAST_SPEECH_DIR=<speech dir> POINTCAST_MODEL=<id> POINTCAST_WAV=<wav> cargo test -- --ignored real_token_times`
    #[test]
    #[ignore]
    fn real_token_times() {
        let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("set {k}"));
        let root = std::path::PathBuf::from(var("POINTCAST_SPEECH_DIR"));
        let c = models::catalog();
        let m = c.model(&var("POINTCAST_MODEL")).unwrap();
        let eng = load(&root, c.runtime_for_host().unwrap(), m, 2).unwrap();
        let (rate, ch, pcm) = crate::voice::audio::read_wav(&std::fs::read(var("POINTCAST_WAV")).unwrap()).unwrap();
        let mut mono = vec![];
        crate::voice::audio::downmix(&pcm, ch, &mut mono);
        let mut at16 = vec![];
        crate::voice::audio::Resampler::new(rate).push(&mono, &mut at16);
        let h = eng.hear(&at16);
        eprintln!("{}: {:?}; token times: {:?}", m.id, h.text, h.tokens);
        assert!(h.text.split_whitespace().count() >= 3, "{h:?}");
    }
}
