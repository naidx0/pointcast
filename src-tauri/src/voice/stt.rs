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
#[repr(C)]
struct ResultHead {
    text: P,
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

fn open(lib: &Path) -> Result<libloading::Library, String> {
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
    let dll = models::find_lib(root, runtime).ok_or("speech runtime is not installed")?;
    if !models::model_installed(root, model) {
        return Err("Not downloaded".into());
    }
    let lib = open(&dll)?;
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
        })
    }
}

impl Engine {
    pub fn transcribe(&self, samples: &[f32]) -> String {
        if samples.is_empty() {
            return String::new();
        }
        unsafe {
            let s = (self.api.stream)(self.rec);
            if s.is_null() {
                return String::new();
            }
            (self.api.accept)(s, 16000, samples.as_ptr(), samples.len() as i32);
            (self.api.decode)(self.rec, s);
            let r = (self.api.result)(s);
            let text = if r.is_null() || (*r).text.is_null() {
                String::new()
            } else {
                CStr::from_ptr((*r).text)
                    .to_string_lossy()
                    .trim()
                    .to_string()
            };
            if !r.is_null() {
                (self.api.destroy_result)(r);
            }
            (self.api.destroy_stream)(s);
            text
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
    }
}
