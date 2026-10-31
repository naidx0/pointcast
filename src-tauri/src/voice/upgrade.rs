//! The faster GPU engine, set up by itself. When the model recordings use is Whisper Turbo and it runs through sherpa
//! on the processor, the helper and the GPU model are downloaded quietly in the background (resumed, checked against
//! their pins), the helper is started once to see that it finds a graphics card, and from then on recordings use it.
//! A computer where it finds none is noted in `gpu-upgrade.json` in the data folder, so it is not tried again until a
//! new helper is published. It never runs while Pointcast records or transcribes: a recording that starts pauses it,
//! and the next quiet moment picks it up where it stopped.
use super::models::{self, Catalog, Progress};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The model this sets up the GPU version of.
pub const MODEL: &str = "whisper-turbo";

const STATE: &str = "gpu-upgrade.json";

/// Beside the speech models while the GPU files are here and not tried yet: recordings keep to sherpa until the check
/// passed, also after a crash in between.
const UNVERIFIED: &str = "whisper-turbo-gpu.unverified";

/// Free space kept beyond the download.
pub const HEADROOM: u64 = 2_000_000_000;

/// How long a try that failed (offline, say) waits before the next.
const AGAIN_AFTER: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Unusable {
    pub reason: String,
    pub date: String,
    /// The helper it was found unusable with (`stamp`): a new one is tried again.
    pub helper: String,
}

/// What is remembered between runs, in the data folder.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gpu_unusable: Option<Unusable>,
    /// Cancel was pressed on the card: not tried again until Whisper Turbo is picked again.
    #[serde(default)]
    pub declined: bool,
}

pub fn load_state(data: &Path) -> State {
    std::fs::read(data.join(STATE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_state(data: &Path, s: &State) {
    let _ = std::fs::create_dir_all(data);
    let _ = std::fs::write(data.join(STATE), serde_json::to_vec_pretty(s).unwrap_or_default());
}

/// Whether GPU files are here that the background upgrade has not tried yet.
pub fn unverified(root: &Path) -> bool {
    root.join(UNVERIFIED).exists()
}

/// A setup the person asked for counts as tried: its own check runs the helper.
pub fn verified(root: &Path) {
    let _ = std::fs::remove_file(root.join(UNVERIFIED));
}

/// Why the helper published for this computer was found unusable here, if it was.
pub fn unusable_here(data: &Path, cat: &Catalog) -> Option<String> {
    let h = cat.helper_for_host()?;
    load_state(data).gpu_unusable.filter(|u| u.helper == stamp(h)).map(|u| u.reason)
}

/// Which build of the helper: its id and the hash of its archive.
pub fn stamp(h: &models::RuntimeEntry) -> String {
    format!("{} {}", h.id, h.sha256)
}

/// What decides whether the upgrade starts now.
#[derive(Debug, Clone, Default)]
pub struct Facts {
    /// Settings › Speech: get the faster GPU version by itself.
    pub auto_gpu: bool,
    /// The model recordings use is Whisper Turbo.
    pub turbo: bool,
    /// The helper published for this computer (`stamp`); none where there is none.
    pub helper: Option<String>,
    /// The GPU version and the helper are here and tried.
    pub installed: bool,
    /// The helper it was found unusable with, if it was.
    pub unusable: Option<String>,
    pub declined: bool,
    /// A recording, a stop, a History entry transcribed again or a setup the person started.
    pub busy: bool,
    /// On a metered connection (Windows says so).
    pub metered: bool,
    /// Bytes still to download.
    pub missing: u64,
    /// Free space where the speech models are, when known.
    pub free: Option<u64>,
    /// A try failed a short while ago.
    pub wait: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Skip {
    Off,
    NotTurbo,
    Installed,
    NoHelper,
    Unusable,
    Declined,
    Busy,
    Wait,
    Metered,
    LowDisk { need: u64, free: u64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Start,
    Skip(Skip),
}

/// Whether to start now, or why not.
pub fn decide(f: &Facts) -> Decision {
    let skip = if !f.auto_gpu {
        Skip::Off
    } else if !f.turbo {
        Skip::NotTurbo
    } else if f.installed {
        Skip::Installed
    } else if f.helper.is_none() {
        Skip::NoHelper
    } else if f.unusable.is_some() && f.unusable == f.helper {
        Skip::Unusable
    } else if f.declined {
        Skip::Declined
    } else if f.busy {
        Skip::Busy
    } else if f.wait {
        Skip::Wait
    } else if f.metered {
        Skip::Metered
    } else {
        match f.free {
            Some(free) if free < f.missing + HEADROOM => Skip::LowDisk { need: f.missing + HEADROOM, free },
            _ => return Decision::Start,
        }
    };
    Decision::Skip(skip)
}

/// What the check found, as kept: the graphics card it runs on, or why it cannot run here.
#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    Gpu(Option<String>),
    Unusable(String),
}

pub const NO_GPU: &str = "The GPU engine found no graphics card it can use on this computer.";

/// The helper's answer to the check: only one that says it runs on a graphics card is used.
pub fn verdict(checked: Result<(&str, Option<String>), String>) -> Verdict {
    match checked {
        Ok(("gpu", device)) => Verdict::Gpu(device),
        Ok(_) => Verdict::Unusable(NO_GPU.into()),
        Err(e) => Verdict::Unusable(e),
    }
}

/// How the upgrade looks from outside: on the Whisper Turbo card and in the tray's tooltip.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Status {
    /// "idle", "downloading", "checking", "done", "unusable", "low_disk", "metered" or "failed".
    pub state: &'static str,
    pub percent: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// Stops the download under way: a recording started (pause) or Cancel on the card.
static STOP: AtomicBool = AtomicBool::new(false);
static DECLINE: AtomicBool = AtomicBool::new(false);
static FAILED_AT: Mutex<Option<Instant>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().unwrap().clone().unwrap_or(Status { state: "idle", ..Default::default() })
}

fn set(s: Status, report: &dyn Fn(&Status)) {
    *STATUS.lock().unwrap() = Some(s.clone());
    report(&s);
}

pub fn running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

/// A recording starts: the download stops where it is and goes on at the next quiet moment.
pub fn pause() {
    if running() {
        STOP.store(true, Ordering::SeqCst);
    }
}

/// Cancel on the card: stop, throw away what was downloaded, and do not try again until Whisper Turbo is picked again.
pub fn cancel(data: &Path) {
    let mut s = load_state(data);
    s.declined = true;
    save_state(data, &s);
    DECLINE.store(true, Ordering::SeqCst);
    STOP.store(true, Ordering::SeqCst);
}

/// Whisper Turbo was picked: a Cancel from before no longer holds.
pub fn welcome(data: &Path) {
    let mut s = load_state(data);
    if s.declined {
        s.declined = false;
        save_state(data, &s);
    }
}

/// The facts for `decide` on this computer now.
pub fn facts(root: &Path, data: &Path, cat: &Catalog, model: &str, auto_gpu: bool, busy: bool) -> Facts {
    let installed = super::engine::installed(root, cat);
    let turbo = super::machine::route(model, super::machine::recommended(), &installed) == Some(MODEL);
    let helper = cat.helper_for_host().filter(|h| !h.pending);
    let gpu = cat.gpu_variant(MODEL);
    let missing = helper.filter(|h| !models::runtime_installed(root, h)).map_or(0, |h| h.bytes)
        + gpu.filter(|g| !models::model_installed(root, g)).map_or(0, |g| g.bytes);
    let state = load_state(data);
    Facts {
        auto_gpu,
        turbo,
        helper: helper.map(stamp),
        installed: cat.model(MODEL).is_some_and(|m| super::engine::gpu_ready(root, cat, m).is_some()),
        unusable: state.gpu_unusable.map(|u| u.helper),
        declined: state.declined,
        busy,
        metered: metered(),
        missing,
        free: models::free_bytes(root),
        wait: FAILED_AT.lock().unwrap().is_some_and(|t| t.elapsed() < AGAIN_AFTER),
    }
}

/// What `run` needs from the app: whether it is busy now, the check (start the helper, warm it up, say what it runs
/// on), and where to show how it goes.
pub struct Hooks<'a> {
    pub busy: &'a dyn Fn() -> bool,
    pub check: &'a dyn Fn() -> Result<(&'static str, Option<String>), String>,
    pub report: &'a dyn Fn(&Status),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Gpu(Option<String>),
    Unusable(String),
    /// A recording started: it goes on later.
    Paused,
    Cancelled,
    Failed(String),
    /// Another run is under way.
    Running,
}

/// Download what is missing of the GPU version of Whisper Turbo and its helper, check it on this computer, and keep
/// what the check found. One run at a time.
pub fn run(root: &Path, data: &Path, cat: &Catalog, hooks: &Hooks) -> Outcome {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return Outcome::Running;
    }
    STOP.store(false, Ordering::SeqCst);
    DECLINE.store(false, Ordering::SeqCst);
    models::stop_on(Some(&STOP));
    let out = upgrade(root, data, cat, hooks);
    models::stop_on(None);
    if let Outcome::Failed(_) = &out {
        *FAILED_AT.lock().unwrap() = Some(Instant::now());
    }
    RUNNING.store(false, Ordering::SeqCst);
    out
}

fn upgrade(root: &Path, data: &Path, cat: &Catalog, hooks: &Hooks) -> Outcome {
    let (Some(h), Some(g)) = (cat.helper_for_host().filter(|h| !h.pending), cat.gpu_variant(MODEL)) else {
        return Outcome::Failed(models::PENDING.into());
    };
    let _ = std::fs::create_dir_all(root);
    // Written before the model is in place: until the check passed, recordings keep to sherpa.
    let _ = std::fs::write(root.join(UNVERIFIED), stamp(h));
    let had_model = models::model_installed(root, g);
    let need_helper = !models::runtime_installed(root, h);
    let total = if need_helper { h.bytes } else { 0 } + if had_model { 0 } else { g.bytes };
    let step = |offset: u64| {
        move |p: Progress| {
            if let Progress::Downloading { received, .. } = p {
                let percent = ((offset + received) * 100 / total.max(1)).min(99) as u8;
                set(Status { state: "downloading", percent, note: None }, hooks.report);
            }
        }
    };
    set(Status { state: "downloading", percent: 0, note: None }, hooks.report);
    let got = (|| {
        if need_helper {
            models::install_runtime(root, h, &step(0))?;
        }
        if !had_model {
            models::install_model(root, g, &step(if need_helper { h.bytes } else { 0 }))?;
        }
        Ok::<(), String>(())
    })();
    match got {
        Err(e) if e == models::CANCELLED && DECLINE.load(Ordering::SeqCst) => {
            for id in [h.id.as_str(), g.id.as_str()] {
                models::drop_partial(root, id);
            }
            set(Status { state: "idle", ..Default::default() }, hooks.report);
            return Outcome::Cancelled;
        }
        Err(e) if e == models::CANCELLED => {
            set(Status { state: "idle", ..Default::default() }, hooks.report);
            return Outcome::Paused;
        }
        Err(e) => {
            set(Status { state: "failed", percent: 0, note: Some(e.clone()) }, hooks.report);
            return Outcome::Failed(e);
        }
        Ok(()) => {}
    }
    if (hooks.busy)() {
        set(Status { state: "idle", ..Default::default() }, hooks.report);
        return Outcome::Paused;
    }
    set(Status { state: "checking", percent: 100, note: None }, hooks.report);
    match verdict((hooks.check)()) {
        Verdict::Gpu(device) => {
            verified(root);
            let mut s = load_state(data);
            s.gpu_unusable = None;
            save_state(data, &s);
            set(Status { state: "done", percent: 100, note: device.clone() }, hooks.report);
            Outcome::Gpu(device)
        }
        Verdict::Unusable(reason) => {
            let mut s = load_state(data);
            s.gpu_unusable = Some(Unusable { reason: reason.clone(), date: chrono::Local::now().format("%Y-%m-%d").to_string(), helper: stamp(h) });
            save_state(data, &s);
            // The whisper.cpp engine on the processor is no faster than sherpa's, so its model is not kept for that.
            if !had_model {
                let _ = models::remove_model(root, g);
            }
            verified(root);
            set(Status { state: "unusable", percent: 0, note: Some(reason.clone()) }, hooks.report);
            Outcome::Unusable(reason)
        }
    }
}

/// Whether Windows counts the internet connection as metered (or roaming, or near its data limit). Elsewhere it is not
/// known, and counts as not.
#[cfg(windows)]
pub fn metered() -> bool {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let cost = || -> windows::core::Result<bool> {
        let c = NetworkInformation::GetInternetConnectionProfile()?.GetConnectionCost()?;
        let kind = c.NetworkCostType()?;
        Ok(c.Roaming()? || c.OverDataLimit()? || c.ApproachingDataLimit()? || kind == NetworkCostType::Fixed || kind == NetworkCostType::Variable)
    };
    cost().unwrap_or(false)
}

#[cfg(not(windows))]
pub fn metered() -> bool {
    false
}

/// The note the card shows instead of starting, for the reasons the person can do something about.
pub fn note(skip: &Skip) -> Option<Status> {
    match skip {
        Skip::LowDisk { need, free } => Some(Status {
            state: "low_disk",
            percent: 0,
            note: Some(format!(
                "The faster GPU version needs about {:.1} GB free and {:.1} GB are. Free some space and it is set up by itself.",
                *need as f64 / 1e9,
                *free as f64 / 1e9
            )),
        }),
        Skip::Metered => Some(Status { state: "metered", percent: 0, note: Some("The faster GPU version downloads once you are off a metered connection.".into()) }),
        _ => None,
    }
}

/// Show `skip`'s note, or clear one shown before.
pub fn noted(skip: &Skip, report: &dyn Fn(&Status)) {
    let shown = STATUS.lock().unwrap().clone();
    match note(skip) {
        Some(s) if shown.as_ref() != Some(&s) => set(s, report),
        None if shown.is_some_and(|s| matches!(s.state, "low_disk" | "metered")) => set(Status { state: "idle", ..Default::default() }, report),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::models::tests::{pin, serve};
    use sha2::{Digest, Sha256};

    fn ready() -> Facts {
        Facts { auto_gpu: true, turbo: true, helper: Some("h1 abc".into()), free: Some(10_000_000_000), missing: 590_000_000, ..Default::default() }
    }

    #[test]
    fn it_starts_only_for_whisper_turbo_on_the_processor_with_nothing_in_the_way() {
        assert_eq!(decide(&ready()), Decision::Start);
        let skip = |f: Facts| match decide(&f) {
            Decision::Skip(s) => s,
            Decision::Start => panic!("started: {f:?}"),
        };
        assert_eq!(skip(Facts { auto_gpu: false, ..ready() }), Skip::Off);
        assert_eq!(skip(Facts { turbo: false, ..ready() }), Skip::NotTurbo);
        assert_eq!(skip(Facts { installed: true, ..ready() }), Skip::Installed);
        assert_eq!(skip(Facts { helper: None, ..ready() }), Skip::NoHelper);
        assert_eq!(skip(Facts { unusable: Some("h1 abc".into()), ..ready() }), Skip::Unusable);
        assert_eq!(skip(Facts { declined: true, ..ready() }), Skip::Declined);
        assert_eq!(skip(Facts { busy: true, ..ready() }), Skip::Busy);
        assert_eq!(skip(Facts { wait: true, ..ready() }), Skip::Wait);
        assert_eq!(skip(Facts { metered: true, ..ready() }), Skip::Metered);
        // The download and 2 GB beyond it, or a plain note.
        assert_eq!(decide(&Facts { free: Some(2_590_000_000), ..ready() }), Decision::Start);
        assert_eq!(skip(Facts { free: Some(2_589_999_999), ..ready() }), Skip::LowDisk { need: 2_590_000_000, free: 2_589_999_999 });
        assert_eq!(decide(&Facts { free: None, ..ready() }), Decision::Start, "free space not known: tried, and the download says");
        // Found unusable with one helper, a newer one is tried.
        assert_eq!(decide(&Facts { unusable: Some("h1 old".into()), ..ready() }), Decision::Start);
        assert!(note(&Skip::LowDisk { need: 2_590_000_000, free: 1_000_000_000 }).unwrap().note.unwrap().contains("2.6 GB"));
        assert!(note(&Skip::Busy).is_none() && note(&Skip::Metered).is_some());
    }

    #[test]
    fn only_a_helper_that_says_it_runs_on_a_graphics_card_is_used() {
        assert_eq!(verdict(Ok(("gpu", Some("RTX via Vulkan".into())))), Verdict::Gpu(Some("RTX via Vulkan".into())));
        assert_eq!(verdict(Ok(("cpu", None))), Verdict::Unusable(NO_GPU.into()));
        assert_eq!(verdict(Ok(("unknown", None))), Verdict::Unusable(NO_GPU.into()));
        assert_eq!(verdict(Err("did not start".into())), Verdict::Unusable("did not start".into()));
    }

    fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut ar = tar::Builder::new(bzip2::write::BzEncoder::new(vec![], bzip2::Compression::fast()));
        for (name, body) in files {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o755);
            ar.append_data(&mut h, name, *body).unwrap();
        }
        ar.into_inner().unwrap().finish().unwrap()
    }

    /// The catalog with this computer's helper and the GPU model served from loopback servers.
    fn fake_catalog() -> (Catalog, Vec<crate::voice::models::tests::Serve>) {
        let mut c = models::catalog();
        let exe = crate::voice::whisper_server::EXE;
        let (prog, lib) = (b"a helper".to_vec(), b"its library".to_vec());
        let ar = archive(&[(&format!("helper/{exe}"), &prog), ("helper/ggml-base.dll", &lib)]);
        let s1 = serve(ar.clone(), 200, None);
        let h = c.helpers.iter_mut().find(|h| h.target == models::target_triple()).expect("a helper for this computer");
        (h.urls, h.bytes, h.sha256, h.pending) = (vec![s1.url.clone()], ar.len() as u64, format!("{:x}", Sha256::digest(&ar)), false);
        h.lib = exe.into();
        h.files = vec![pin(&format!("helper/{exe}"), &prog), pin("helper/ggml-base.dll", &lib)];
        let model: Vec<u8> = (0..600_000u32).map(|i| (i * 7 % 253) as u8).collect();
        let s2 = serve(model.clone(), 200, None);
        let g = c.models.iter_mut().find(|m| m.gpu_for == MODEL).unwrap();
        let sha = format!("{:x}", Sha256::digest(&model));
        (g.urls, g.bytes, g.sha256) = (vec![s2.url.replace(".tar.bz2", ".bin")], model.len() as u64, sha.clone());
        g.files[0].bytes = model.len() as u64;
        g.files[0].sha256 = sha;
        (c, vec![s1, s2])
    }

    fn scratch(name: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let d = std::env::temp_dir().join(format!("pc-upgrade-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        (d.join("speech"), d.join("data"))
    }

    /// The real thing on this computer, in a data folder of its own: the upgrade downloads the published helper and the
    /// pinned GPU model (about 590 MB), paused once by a recording starting at 20 % and then resumed, checks the helper,
    /// then a History entry made from a real recording (with the words sherpa heard, `<wav>`'s `.items.json`) is
    /// transcribed again on the GPU engine; it prints what History recorded and how the words compare. Then, in a second
    /// folder, a helper made to find no graphics card is noted as unusable and the model it downloaded goes.
    /// `POINTCAST_DATA_DIR=<scratch> POINTCAST_WAV=<recording.wav> cargo test --lib -- --ignored --nocapture real_upgrade`
    #[test]
    #[ignore]
    fn real_upgrade_end_to_end() {
        let data = std::path::PathBuf::from(std::env::var_os("POINTCAST_DATA_DIR").expect("set POINTCAST_DATA_DIR to a scratch folder"));
        let wav = std::path::PathBuf::from(std::env::var_os("POINTCAST_WAV").expect("set POINTCAST_WAV"));
        let (root, cat) = (models::speech_dir(), models::catalog());
        assert!(root.starts_with(&data) && crate::settings::data_dir() == data, "only the scratch folder is written");
        let idle = || false;
        let check = || crate::voice::engine::probe(&root, &cat, MODEL);
        let paused = AtomicBool::new(false);
        let report = |s: &Status| {
            if s.state == "downloading" && s.percent >= 20 && !paused.swap(true, Ordering::SeqCst) {
                eprintln!("PAUSE at {}%", s.percent);
                pause();
            }
        };
        let t = Instant::now();
        let first = run(&root, &data, &cat, &Hooks { busy: &idle, check: &check, report: &report });
        eprintln!("FIRST {first:?} s={:.1}", t.elapsed().as_secs_f64());
        let t = Instant::now();
        let out = run(&root, &data, &cat, &Hooks { busy: &idle, check: &check, report: &|_| {} });
        eprintln!("UPGRADE {out:?} s={:.1} state={:?}", t.elapsed().as_secs_f64(), load_state(&data));
        assert_eq!(first, Outcome::Paused);
        assert!(matches!(out, Outcome::Gpu(_)));
        // A History entry from a real recording, with sherpa's words, transcribed again on the GPU engine.
        let (rate, ch, pcm) = crate::voice::audio::read_wav(&std::fs::read(&wav).unwrap()).unwrap();
        let (mut mono, mut x) = (vec![], vec![]);
        crate::voice::audio::downmix(&pcm, ch, &mut mono);
        crate::voice::audio::Resampler::new(rate).push(&mono, &mut x);
        let old: serde_json::Value = serde_json::from_slice(&std::fs::read(wav.with_extension("items.json")).unwrap()).unwrap();
        let items: Vec<crate::prompt::Item> = serde_json::from_value(old.get("items").cloned().unwrap_or(old)).unwrap();
        let settings = crate::settings::Settings { model: MODEL.into(), onboarded: true, ..Default::default() };
        let a = crate::session::App::new(settings, data.join("settings.json"));
        a.history.append_audio(&x).unwrap();
        let m = a.history.save_recorded(&items, crate::session::now_ms(), x.len() as u64 / 16).unwrap();
        a.history.clear_checkpoint();
        let before = a.history.export(&m.id).unwrap().plain;
        let t = Instant::now();
        a.retranscribe(&m.id, |_, _| {}).unwrap();
        let wall = t.elapsed().as_secs_f64();
        let after = a.history.export(&m.id).unwrap().plain;
        let e = a.history.list().into_iter().find(|e| e.id == m.id).unwrap();
        let words = |s: &str| s.split_whitespace().map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase()).filter(|w| !w.is_empty()).collect::<Vec<_>>();
        let (b, g) = (words(&before), words(&after));
        let shared = b.iter().filter(|w| g.contains(w)).count();
        eprintln!(
            "AGAIN engine={} device={:?} transcribe_ms={} audio_ms={} realtime_x={:.1} wall_s={wall:.1} words_sherpa={} words_gpu={} sherpa_words_in_gpu={:.2}",
            e.engine,
            e.device,
            e.transcribe_ms,
            e.audio_ms,
            e.audio_ms as f64 / e.transcribe_ms.max(1) as f64,
            b.len(),
            g.len(),
            shared as f64 / b.len().max(1) as f64
        );
        eprintln!("SHERPA {before}");
        eprintln!("GPU {after}");
        assert_eq!(e.engine, "gpu");
        a.release_engine();
        // A helper that finds no graphics card, as on a computer without one: noted, and the model it downloaded goes.
        let root2 = data.join("no-gpu");
        let h = cat.helper_for_host().unwrap();
        let copy = |from: &Path, to: &Path| {
            for f in walk(from) {
                let dest = to.join(f.strip_prefix(from).unwrap());
                std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
                std::fs::copy(&f, dest).unwrap();
            }
        };
        copy(&models::install_dir(&root, &h.id), &models::install_dir(&root2, &h.id));
        let mut c2 = cat.clone();
        let local = std::fs::read(models::install_dir(&root, "whisper-turbo-gpu").join(&c2.gpu_variant(MODEL).unwrap().files[0].name)).unwrap();
        let s = serve(local, 200, None);
        c2.models.iter_mut().find(|m| m.gpu_for == MODEL).unwrap().urls = vec![s.url.replace(".tar.bz2", ".bin")];
        std::env::set_var("POINTCAST_HELPER_NO_GPU", "1");
        let data2 = data.join("no-gpu-data");
        let check2 = || crate::voice::engine::probe(&root2, &c2, MODEL);
        let out = run(&root2, &data2, &c2, &Hooks { busy: &idle, check: &check2, report: &|_| {} });
        std::env::remove_var("POINTCAST_HELPER_NO_GPU");
        let g2 = c2.gpu_variant(MODEL).unwrap();
        eprintln!("NO_GPU {out:?} state={:?} model_kept={}", load_state(&data2), models::model_installed(&root2, g2));
        assert_eq!(out, Outcome::Unusable(NO_GPU.into()));
        assert!(load_state(&data2).gpu_unusable.is_some() && !models::model_installed(&root2, g2));
    }

    fn walk(d: &Path) -> Vec<std::path::PathBuf> {
        let mut out = vec![];
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            if e.path().is_dir() {
                out.extend(walk(&e.path()));
            } else {
                out.push(e.path());
            }
        }
        out
    }

    /// The whole run against loopback servers, as one test: the runs share the one upgrade a computer can have.
    #[test]
    fn the_upgrade_downloads_checks_and_keeps_what_it_found_and_a_recording_pauses_it() {
        if crate::voice::engine::dev_helper().is_some() {
            return;
        }
        let (c, servers) = fake_catalog();
        let turbo = c.model(MODEL).unwrap().clone();
        let g = c.gpu_variant(MODEL).unwrap().clone();
        let shown = Mutex::new(vec![]);
        let report = |s: &Status| shown.lock().unwrap().push(s.clone());
        let idle = || false;
        let gpu = || Ok(("gpu", Some("NVIDIA GeForce RTX 2060 SUPER via Vulkan".to_string())));

        // A graphics card: downloaded, checked, and from then on the GPU version is the one recordings load.
        let (root, data) = scratch("gpu");
        let out = run(&root, &data, &c, &Hooks { busy: &idle, check: &gpu, report: &report });
        assert_eq!(out, Outcome::Gpu(Some("NVIDIA GeForce RTX 2060 SUPER via Vulkan".into())));
        assert!(crate::voice::engine::gpu_ready(&root, &c, &turbo).is_some() && !unverified(&root));
        assert_eq!(load_state(&data), State::default());
        let states: Vec<&str> = shown.lock().unwrap().iter().map(|s| s.state).collect();
        assert_eq!((states.first(), states.iter().rev().nth(1), states.last()), (Some(&"downloading"), Some(&"checking"), Some(&"done")));
        assert!(shown.lock().unwrap().iter().map(|s| s.percent).collect::<Vec<_>>().windows(2).all(|w| w[0] <= w[1]), "never back");

        // No graphics card: noted with the helper it was tried with, the model it downloaded goes, sherpa stays.
        let (root, data) = scratch("cpu");
        let no = || Ok(("cpu", None));
        assert_eq!(run(&root, &data, &c, &Hooks { busy: &idle, check: &no, report: &report }), Outcome::Unusable(NO_GPU.into()));
        let u = load_state(&data).gpu_unusable.unwrap();
        assert_eq!((u.reason.as_str(), u.helper), (NO_GPU, stamp(c.helper_for_host().unwrap())));
        assert_eq!(u.date.len(), 10);
        assert!(!models::model_installed(&root, &g) && !unverified(&root) && crate::voice::engine::gpu_ready(&root, &c, &turbo).is_none());
        assert_eq!(status().state, "unusable");
        let f = Facts { unusable: load_state(&data).gpu_unusable.map(|u| u.helper), helper: c.helper_for_host().map(stamp), ..ready() };
        assert_eq!(decide(&f), Decision::Skip(Skip::Unusable), "never tried again with this helper");

        // A recording starts mid-download: it stops with what it has, and the next run goes on from there.
        let (root, data) = scratch("pause");
        let h = c.helper_for_host().unwrap().clone();
        // The first piece of the model in: the helper is set up by then.
        let pausing = |s: &Status| {
            if s.state == "downloading" && models::runtime_installed(&root, &h) {
                pause();
            }
        };
        assert_eq!(run(&root, &data, &c, &Hooks { busy: &idle, check: &gpu, report: &pausing }), Outcome::Paused);
        assert!(!models::model_installed(&root, &g) && crate::voice::engine::gpu_ready(&root, &c, &turbo).is_none());
        assert!(std::fs::read_dir(models::install_dir(&root, &g.id)).unwrap().flatten().any(|e| e.path().extension().is_some_and(|x| x == "partial")));
        let before = servers[1].ranges.lock().unwrap().len();
        assert_eq!(run(&root, &data, &c, &Hooks { busy: &idle, check: &gpu, report: &report }), Outcome::Gpu(Some("NVIDIA GeForce RTX 2060 SUPER via Vulkan".into())));
        let ranges = servers[1].ranges.lock().unwrap()[before..].to_vec();
        assert!(matches!(ranges.as_slice(), [Some(n)] if *n > 0), "resumed, not started over: {ranges:?}");

        // Busy once the files are in: not checked now (and recordings keep to sherpa meanwhile).
        let (root, data) = scratch("busy");
        let busy = || true;
        let never = || -> Result<(&'static str, Option<String>), String> { panic!("no check while busy") };
        assert_eq!(run(&root, &data, &c, &Hooks { busy: &busy, check: &never, report: &report }), Outcome::Paused);
        assert!(models::model_installed(&root, &g) && unverified(&root) && crate::voice::engine::gpu_ready(&root, &c, &turbo).is_none());

        // Cancel on the card: the bytes so far go, and it is not tried again until Whisper Turbo is picked again.
        let (root, data) = scratch("cancel");
        let cancelling = |s: &Status| {
            if s.state == "downloading" && models::runtime_installed(&root, &h) {
                cancel(&data);
            }
        };
        assert_eq!(run(&root, &data, &c, &Hooks { busy: &idle, check: &gpu, report: &cancelling }), Outcome::Cancelled);
        assert!(load_state(&data).declined);
        assert!(std::fs::read_dir(models::install_dir(&root, &g.id)).into_iter().flatten().flatten().all(|e| e.path().extension().is_none_or(|x| x != "partial")));
        welcome(&data);
        assert!(!load_state(&data).declined);
    }
}
