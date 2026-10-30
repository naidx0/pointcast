//! The speech engine a recording uses: sherpa-onnx in this process, or Whisper on the graphics card through the
//! whisper.cpp helper (`whisper_server`), which falls back to sherpa's Whisper on the processor when it cannot run.
use super::models::{self, Catalog, ModelEntry};
use super::stt::{self, Heard};
use super::whisper_server::{self as ws, Fail, Launch, Server};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub enum Engine {
    Sherpa(stt::Engine),
    Gpu(Gpu),
}

/// What hears a piece on the processor once the helper gave up: sherpa's engine, or a stand-in in tests.
pub trait Backup: Send {
    fn hear(&self, samples: &[f32]) -> Heard;
}

impl Backup for stt::Engine {
    fn hear(&self, samples: &[f32]) -> Heard {
        stt::Engine::hear(self, samples)
    }
}

pub type LoadBackup = Box<dyn Fn() -> Result<Box<dyn Backup>, String> + Send>;

/// Said when the helper stopped working and no engine on the processor is set up.
pub const GPU_FAILED: &str =
    "The GPU speech engine stopped working. Your audio is kept: open Settings › Speech and download Whisper Turbo again.";

/// How long the helper may take to load the model and answer (a slow disk, a first start of the graphics driver).
const READY_WITHIN: Duration = Duration::from_secs(180);
/// The first request compiles the graphics card's shaders: about 30 s on Vulkan and 90 s on CUDA the first time ever.
const WARM_WITHIN: Duration = Duration::from_secs(600);

/// Whisper through the helper. Each piece is sent once and, unanswered, once more (after starting the helper again if
/// it died); twice without an answer, the processor engine takes over for good, or the failure is said.
pub struct Gpu {
    pub model: String,
    server: Mutex<Option<Server>>,
    backup: Mutex<Option<Box<dyn Backup>>>,
    load_backup: LoadBackup,
    failure: Mutex<Option<String>>,
    /// Moved on by a cancel: the piece under way stops. Shared by every helper this engine starts.
    pub abort: Arc<AtomicU64>,
    pub language: String,
}

impl Gpu {
    pub fn new(model: &str, mut server: Server, load_backup: LoadBackup) -> Gpu {
        let abort = Arc::new(AtomicU64::new(0));
        server.abort = abort.clone();
        Gpu {
            model: model.into(),
            server: Mutex::new(Some(server)),
            backup: Mutex::default(),
            load_backup,
            failure: Mutex::default(),
            abort,
            language: "auto".into(),
        }
    }

    pub fn hear(&self, samples: &[f32]) -> Heard {
        if let Some(b) = self.backup.lock().unwrap().as_ref() {
            return b.hear(samples);
        }
        let ms = samples.len() as u64 * 1000 / super::audio::RATE as u64;
        for attempt in 0..2 {
            let mut g = self.server.lock().unwrap();
            let Some(srv) = g.as_mut() else { break };
            if attempt > 0 && srv.exited() {
                match srv.launch.clone().map(|l| Server::start(&l, READY_WITHIN)) {
                    Some(Ok(mut again)) => {
                        again.abort = self.abort.clone();
                        *srv = again;
                    }
                    Some(Err(_)) => break,
                    None => {}
                }
            }
            match srv.transcribe(samples, &self.language, ws::piece_timeout(ms)) {
                Ok(text) => return Heard { text, tokens: None },
                Err(Fail::Aborted) => return Heard::default(),
                Err(Fail::Broken(_)) => {}
            }
        }
        // The helper's memory on the graphics card is freed either way.
        if let Some(mut srv) = self.server.lock().unwrap().take() {
            srv.kill();
        }
        match (self.load_backup)() {
            Ok(b) => {
                let h = b.hear(samples);
                *self.backup.lock().unwrap() = Some(b);
                h
            }
            Err(_) => {
                *self.failure.lock().unwrap() = Some(GPU_FAILED.into());
                Heard::default()
            }
        }
    }

    pub fn backend(&self) -> &'static str {
        if self.backup.lock().unwrap().is_some() {
            return "cpu";
        }
        self.server.lock().unwrap().as_ref().map_or("unknown", Server::backend)
    }
}

impl Engine {
    pub fn model(&self) -> &str {
        match self {
            Engine::Sherpa(e) => &e.model,
            Engine::Gpu(g) => &g.model,
        }
    }

    pub fn hear(&self, samples: &[f32]) -> Heard {
        match self {
            Engine::Sherpa(e) => e.hear(samples),
            Engine::Gpu(g) => g.hear(samples),
        }
    }

    pub fn transcribe(&self, samples: &[f32]) -> String {
        self.hear(samples).text
    }

    /// Live captions come from sherpa's quick models only; the helper is sent finished pieces.
    pub fn wants_partial(&self) -> bool {
        matches!(self, Engine::Sherpa(e) if e.wants_partial())
    }

    pub fn partial(&self, samples: &[f32]) -> Option<String> {
        match self {
            Engine::Sherpa(e) => e.partial(samples),
            Engine::Gpu(_) => None,
        }
    }

    /// Where it runs: "gpu", "cpu" or "unknown" (the helper has not said yet).
    pub fn backend(&self) -> &'static str {
        match self {
            Engine::Sherpa(_) => "cpu",
            Engine::Gpu(g) => g.backend(),
        }
    }

    /// Why it can hear nothing any more, once the helper failed with nothing to fall back on.
    pub fn failure(&self) -> Option<String> {
        match self {
            Engine::Gpu(g) => g.failure.lock().unwrap().clone(),
            Engine::Sherpa(_) => None,
        }
    }

    /// What a cancel moves on to stop the piece under way, for an engine that can stop part way through one.
    pub fn aborter(&self) -> Option<Arc<AtomicU64>> {
        match self {
            Engine::Gpu(g) => Some(g.abort.clone()),
            Engine::Sherpa(_) => None,
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, Engine::Gpu(_))
    }
}

/// A development build may run a helper from `POINTCAST_WHISPER_HELPER=<dir>`, unpinned; a release never does.
pub fn dev_helper() -> Option<PathBuf> {
    if !cfg!(debug_assertions) {
        return None;
    }
    let d = PathBuf::from(std::env::var_os("POINTCAST_WHISPER_HELPER")?);
    d.join(ws::EXE).is_file().then_some(d)
}

/// Whether the GPU version can be set up on this computer: its helper is published (or a development one is given).
pub fn helper_available(cat: &Catalog) -> bool {
    dev_helper().is_some() || cat.helper_for_host().is_some_and(|h| !h.pending)
}

/// Whether the helper is here, by its files (checked against their pins only when it is started).
pub fn helper_installed(root: &Path, cat: &Catalog) -> bool {
    dev_helper().is_some() || cat.helper_for_host().is_some_and(|h| !h.pending && models::runtime_installed(root, h))
}

/// The GPU version of `m`, when it and the helper are both here.
pub fn gpu_ready<'a>(root: &Path, cat: &'a Catalog, m: &ModelEntry) -> Option<&'a ModelEntry> {
    cat.gpu_variant(&m.id).filter(|v| models::model_installed(root, v) && helper_installed(root, cat))
}

/// The models a recording can use: those whose files are here, a model with a GPU version also when that is.
pub fn installed<'a>(root: &Path, cat: &'a Catalog) -> Vec<&'a str> {
    cat.models
        .iter()
        .filter(|m| m.gpu_for.is_empty() && (models::model_installed(root, m) || gpu_ready(root, cat, m).is_some()))
        .map(|m| m.id.as_str())
        .collect()
}

/// Whether anything can transcribe here: a sherpa model with its runtime, or a GPU version with its helper.
pub fn usable(root: &Path, cat: &Catalog) -> bool {
    let sherpa = cat.runtime_for_host().is_some_and(|r| models::runtime_installed(root, r))
        && cat.models.iter().any(|m| m.gpu_for.is_empty() && models::model_installed(root, m));
    sherpa || cat.models.iter().any(|m| gpu_ready(root, cat, m).is_some())
}

/// The speech models as Settings shows them: a GPU version is no card of its own but its model's `gpu`, and that card
/// counts as downloaded when either is.
pub fn view(root: &Path, cat: &Catalog) -> Value {
    let mut v = models::view(root, cat);
    let available = helper_available(cat);
    let helper = installed_helper_bytes(root, cat);
    let list = v["models"].as_array().cloned().unwrap_or_default();
    let cards: Vec<Value> = list
        .into_iter()
        .filter(|m| m["gpu_for"].as_str().is_none_or(str::is_empty))
        .map(|mut m| {
            let id = m["id"].as_str().unwrap_or_default().to_string();
            if let (Some(e), Some(g)) = (cat.model(&id), cat.gpu_variant(&id)) {
                let ready = gpu_ready(root, cat, e).is_some();
                m["installed"] = Value::Bool(ready || models::model_installed(root, e));
                m["gpu"] = serde_json::json!({
                    "available": available,
                    "installed": ready,
                    "bytes": g.bytes + if ready { 0 } else { helper },
                    "cpu_installed": models::model_installed(root, e),
                });
            }
            m
        })
        .collect();
    v["models"] = Value::Array(cards);
    v
}

/// The helper's download, or nothing when it is here or none is published.
fn installed_helper_bytes(root: &Path, cat: &Catalog) -> u64 {
    match cat.helper_for_host() {
        Some(h) if !helper_installed(root, cat) && !h.pending => h.bytes,
        _ => 0,
    }
}

/// Load `model` for this computer: on the graphics card when its GPU version and the helper are here, else (or when
/// the helper cannot start, or finds no graphics card and sherpa's files are here) through sherpa on the processor.
/// `preparing(true)` while the first start ever compiles the graphics card's shaders.
pub fn load(root: &Path, cat: &Catalog, model: &ModelEntry, threads: i32, preparing: &dyn Fn(bool)) -> Result<Engine, String> {
    let rt = cat.runtime_for_host().cloned();
    let sherpa_here = rt.as_ref().is_some_and(|r| models::runtime_installed(root, r)) && models::model_installed(root, model);
    let sherpa = || match &rt {
        Some(r) if sherpa_here => stt::load(root, r, model, threads),
        _ => Err(crate::session::NO_MODEL.to_string()),
    };
    let Some(variant) = gpu_ready(root, cat, model) else {
        return sherpa().map(Engine::Sherpa);
    };
    match start_gpu(root, cat, variant, preparing) {
        Ok(srv) if srv.backend() == "cpu" && sherpa_here => {
            drop(srv);
            sherpa().map(Engine::Sherpa)
        }
        Ok(srv) => {
            let (root, m, rt) = (root.to_path_buf(), model.clone(), rt);
            let backup: LoadBackup = Box::new(move || match &rt {
                Some(r) if models::runtime_installed(&root, r) && models::model_installed(&root, &m) => {
                    stt::load(&root, r, &m, threads).map(|e| Box::new(e) as Box<dyn Backup>)
                }
                _ => Err(crate::session::NO_MODEL.to_string()),
            });
            Ok(Engine::Gpu(Gpu::new(&model.id, srv, backup)))
        }
        Err(e) if sherpa_here => sherpa().map(Engine::Sherpa).map_err(|_| e),
        Err(e) => Err(e),
    }
}

/// Marks that this helper has compiled its shaders on this computer once.
const WARMED: &str = "whisper-helper.warmed";

/// Start the helper on `variant`'s model, checked against its pins, and send it a second of silence so the first
/// piece is not the one that waits for the graphics card.
fn start_gpu(root: &Path, cat: &Catalog, variant: &ModelEntry, preparing: &dyn Fn(bool)) -> Result<Server, String> {
    let (dir, stamp) = match dev_helper() {
        Some(d) => (d.clone(), d.display().to_string()),
        None => {
            let h = cat.helper_for_host().ok_or(crate::session::NO_MODEL)?;
            let exe = models::verify_runtime(root, h)?;
            (exe.parent().ok_or(models::CHANGED)?.to_path_buf(), format!("{} {}", h.id, h.sha256))
        }
    };
    let file = variant.files.first().ok_or(crate::session::NO_MODEL)?;
    let launch = Launch { dir, model: models::install_dir(root, &variant.id).join(&file.name), state: root.to_path_buf() };
    let srv = Server::start(&launch, READY_WITHIN)?;
    let marker = root.join(WARMED);
    let first = std::fs::read_to_string(&marker).ok().as_deref() != Some(stamp.as_str());
    if first {
        preparing(true);
    }
    let warm = srv.transcribe(&vec![0.0; super::audio::RATE as usize], "en", WARM_WITHIN);
    if first {
        preparing(false);
    }
    match warm {
        Ok(_) => {
            let _ = std::fs::write(&marker, &stamp);
            Ok(srv)
        }
        Err(_) => Err("The GPU speech engine started but could not transcribe.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::whisper_server::tests::{fake, Answer};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const PATH: &str = "/i-engine";

    struct Cpu(&'static str);
    impl Backup for Cpu {
        fn hear(&self, _: &[f32]) -> Heard {
            Heard { text: self.0.into(), tokens: None }
        }
    }

    fn gpu(port: u16, backup: Option<&'static str>, loads: Arc<AtomicUsize>) -> Engine {
        let load: LoadBackup = Box::new(move || {
            loads.fetch_add(1, Ordering::SeqCst);
            backup.map(|t| Box::new(Cpu(t)) as Box<dyn Backup>).ok_or_else(|| "none".to_string())
        });
        Engine::Gpu(Gpu::new("whisper-turbo", Server::at(port, PATH), load))
    }

    #[test]
    fn the_gpu_engine_hears_through_the_helper_with_no_live_captions() {
        let f = fake(PATH, vec![Answer::Json(r#"{"text":" Make the header sticky."}"#.into())]);
        let e = gpu(f.port, None, Arc::default());
        assert_eq!(e.hear(&vec![0.2; 16_000]).text, "Make the header sticky.");
        assert_eq!((e.model(), e.wants_partial(), e.partial(&[0.0; 16_000])), ("whisper-turbo", false, None));
        assert!(e.aborter().is_some() && e.is_gpu() && e.failure().is_none());
        assert_eq!(e.backend(), "unknown", "a stand-in says nothing about its device");
    }

    #[test]
    fn a_blank_piece_from_the_helper_is_dropped_like_any_empty_one() {
        let f = fake(PATH, vec![Answer::Json(r#"{"text":"[BLANK_AUDIO]"}"#.into())]);
        let e = gpu(f.port, None, Arc::default());
        assert_eq!(stt::hear_piece(&vec![0.2; 16_000], 900, |s| e.hear(s)), None);
        assert_eq!(f.posts.lock().unwrap().len(), 1);
    }

    #[test]
    fn a_helper_that_dies_mid_run_hands_over_to_the_processor_engine() {
        let ok = r#"{"text":"first piece"}"#;
        let f = fake(PATH, vec![Answer::Json(ok.into()), Answer::Die, Answer::Die, Answer::Json(ok.into())]);
        let loads = Arc::new(AtomicUsize::new(0));
        let e = gpu(f.port, Some("from the processor"), loads.clone());
        assert_eq!(e.hear(&[0.1; 8_000]).text, "first piece");
        assert_eq!(e.hear(&[0.1; 8_000]).text, "from the processor", "tried twice, then the processor");
        assert_eq!(e.hear(&[0.1; 8_000]).text, "from the processor", "for good: the helper is not asked again");
        assert_eq!((f.posts.lock().unwrap().len(), loads.load(Ordering::SeqCst)), (3, 1));
        assert_eq!((e.backend(), e.failure()), ("cpu", None));
    }

    #[test]
    fn one_bad_answer_is_tried_again_and_two_with_nothing_to_fall_back_on_are_said() {
        let f = fake(PATH, vec![Answer::Json("{oops".into()), Answer::Json(r#"{"text":"second try"}"#.into())]);
        let e = gpu(f.port, None, Arc::default());
        assert_eq!(e.hear(&[0.1; 8_000]).text, "second try");
        assert!(e.failure().is_none());
        let f = fake(PATH, vec![Answer::Json("{oops".into())]);
        let e = gpu(f.port, None, Arc::default());
        assert_eq!(e.hear(&[0.1; 8_000]), Heard::default());
        assert_eq!(e.failure().as_deref(), Some(GPU_FAILED));
    }

    #[test]
    fn a_cancelled_piece_is_not_a_failure() {
        let f = fake(PATH, vec![Answer::Slow(Duration::from_secs(10))]);
        let loads = Arc::new(AtomicUsize::new(0));
        let e = gpu(f.port, Some("cpu"), loads.clone());
        let abort = e.aborter().unwrap();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            abort.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        });
        assert_eq!(e.hear(&[0.1; 16_000]), Heard::default());
        t.join().unwrap();
        assert_eq!((loads.load(Ordering::SeqCst), f.posts.lock().unwrap().len(), e.failure()), (0, 1, None));
    }

    fn catalog_with_helper(pending: bool) -> Catalog {
        let mut c = models::catalog();
        for h in &mut c.helpers {
            h.pending = pending;
        }
        c
    }

    #[test]
    fn whisper_turbo_is_one_card_that_counts_as_downloaded_with_either_engine() {
        let root = std::env::temp_dir().join(format!("pc-engine-view-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let c = catalog_with_helper(true);
        let ids = |v: &Value| v["models"].as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        let v = view(&root, &c);
        assert!(ids(&v).contains(&"whisper-turbo".to_string()) && !ids(&v).contains(&"whisper-turbo-gpu".to_string()));
        let card = |v: &Value| v["models"].as_array().unwrap().iter().find(|m| m["id"] == "whisper-turbo").unwrap().clone();
        let gpu = c.model("whisper-turbo-gpu").unwrap();
        assert_eq!(card(&v)["gpu"]["bytes"], gpu.bytes);
        if dev_helper().is_none() {
            assert_eq!((card(&v)["gpu"]["available"].clone(), card(&v)["installed"].clone()), (Value::Bool(false), Value::Bool(false)));
        }
        // The sherpa files of an earlier version: downloaded, and the GPU version not yet.
        let sherpa = c.model("whisper-turbo").unwrap();
        std::fs::create_dir_all(models::install_dir(&root, &sherpa.id)).unwrap();
        for f in &sherpa.files {
            let file = std::fs::File::create(models::install_dir(&root, &sherpa.id).join(&f.name)).unwrap();
            file.set_len(f.bytes).unwrap();
        }
        let v = view(&root, &c);
        assert_eq!((card(&v)["installed"].clone(), card(&v)["gpu"]["cpu_installed"].clone()), (Value::Bool(true), Value::Bool(true)));
        assert_eq!(installed(&root, &c), vec!["whisper-turbo"]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_helper_without_pins_is_never_installed_or_offered() {
        let c = models::catalog();
        for h in c.helpers.iter().filter(|h| h.pending) {
            assert!(h.files.is_empty() || h.sha256.is_empty(), "{}", h.id);
        }
        let root = std::env::temp_dir().join(format!("pc-engine-pending-{}", std::process::id()));
        let c = catalog_with_helper(true);
        if let Some(h) = c.helper_for_host() {
            assert_eq!(models::install_runtime(&root, h, &|_| {}).unwrap_err(), models::PENDING);
            if dev_helper().is_none() {
                assert!(!helper_available(&c) && !helper_installed(&root, &c));
            }
        }
    }
}
