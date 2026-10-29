mod capture;
mod chord;
mod clipboard;
mod history;
mod image_ops;
mod mcp;
mod overlay;
mod permissions;
mod platform;
mod privacy;
mod prompt;
mod session;
mod settings;
mod tools;
mod tray;
mod voice;

use mcp::Latest;
use prompt::{Label, SessionExport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use session::{now_ms, App, Phase};
use settings::Settings;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use voice::ptt::TalkMode;

/// A hold shorter than this is a tap: the session keeps recording until the next press.
const HOLD_MS: u64 = 350;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Session,
    Full,
    Region,
    Zoom,
}

pub fn show_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn hide_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
    }
}

pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn emit_state(app: &AppHandle, svc: &App) {
    let st = svc.ui();
    tray::refresh(app, &st, svc.last_export().is_some());
    let _ = app.emit("session://state", &st);
}

fn recording(svc: &App) -> bool {
    matches!(svc.ui().phase, Phase::Recording | Phase::Loading)
}

fn begin(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    svc.start()?;
    if svc.settings.lock().unwrap().sounds {
        platform::chime(true);
    }
    platform::show_indicator(app);
    emit_state(app, svc);
    Ok(())
}

fn finish(app: &AppHandle, svc: &Arc<App>) -> Result<Option<SessionExport>, String> {
    hide_window(app, "indicator");
    hide_window(app, "overlay");
    if svc.settings.lock().unwrap().sounds {
        platform::chime(false);
    }
    let exp = svc.stop(false)?;
    if let Some(ref e) = exp {
        if svc.practice.load(Ordering::SeqCst) {
            let _ = app.emit("practice://result", e);
        } else {
            if let Some(latest) = app.try_state::<Latest>() {
                latest.set(e.clone());
            }
            let _ = app.emit("session://result", e);
        }
    }
    emit_state(app, svc);
    Ok(exp)
}

/// Stop without holding the caller (the event loop) while the last words are transcribed.
fn finish_soon(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        let _ = finish(&app, &svc);
    });
}

/// Quit, even mid-recording: what was said is transcribed and kept for the next launch.
pub fn quit(app: &AppHandle) {
    if let Some(svc) = app.try_state::<Arc<App>>() {
        svc.flush_for_quit();
    }
    app.exit(0);
}

pub fn toggle(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    if recording(svc) {
        finish_soon(app, svc);
    } else {
        begin(app, svc)?;
    }
    Ok(())
}

fn frame_payload(shot: &capture::Shot) -> Value {
    json!({
        "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.png),
        "width": shot.width,
        "height": shot.height,
    })
}

fn open_region(app: &AppHandle, svc: &Arc<App>) -> Result<Value, String> {
    let shot = svc.begin_region()?;
    let payload = frame_payload(&shot);
    let _ = app.emit("overlay://frame", &payload);
    platform::show_overlay(app);
    Ok(payload)
}

/// What the session chord's release does: stop only a held (not tapped) push-to-talk.
fn release_stops(mode: TalkMode, held_from: Option<u64>, now: u64) -> bool {
    mode == TalkMode::Hold && held_from.is_some_and(|t| now.saturating_sub(t) >= HOLD_MS)
}

/// Every shortcut, from the keyboard or the tray. A capture chord with no session starts one.
pub fn hotkey(app: &AppHandle, action: Action, pressed: bool) {
    let _ = app.emit("hotkey://fired", json!({ "action": action, "pressed": pressed }));
    let Some(svc) = app.try_state::<Arc<App>>() else {
        return;
    };
    let svc = svc.inner().clone();
    if svc.hotkey_test.load(Ordering::SeqCst) {
        return;
    }
    let rec = recording(&svc);
    match (action, pressed) {
        (Action::Session, true) => {
            if rec {
                *svc.hold_from.lock().unwrap() = None;
                finish_soon(app, &svc);
            } else if begin(app, &svc).is_ok() {
                *svc.hold_from.lock().unwrap() = Some(now_ms());
            }
        }
        (Action::Session, false) => {
            let from = svc.hold_from.lock().unwrap().take();
            let mode = svc.settings.lock().unwrap().talk_mode;
            if rec && release_stops(mode, from, now_ms()) {
                finish_soon(app, &svc);
            }
        }
        (_, false) => {}
        (action, true) => {
            if !rec && begin(app, &svc).is_err() {
                return;
            }
            let _ = match action {
                Action::Full => svc.capture_full().map(|_| Value::Null),
                Action::Zoom => svc.capture_zoom().map(|_| Value::Null),
                _ => open_region(app, &svc),
            };
            emit_state(app, &svc);
        }
    }
}

fn settings_view(svc: &App) -> Value {
    let s = svc.settings.lock().unwrap().clone();
    let errors: Vec<Value> = svc
        .hotkey_errors
        .lock()
        .unwrap()
        .iter()
        .map(|(a, e)| json!({ "action": a, "error": e }))
        .collect();
    let chord = |c: chord::Chord, d: chord::Chord| {
        json!({ "label": c.label(), "keys": c.keys(), "default": d.label() })
    };
    json!({
        "platform": std::env::consts::OS,
        "materials": platform::materials(),
        "version": env!("CARGO_PKG_VERSION"),
        "session_hotkey": s.session_chord().label(),
        "full_hotkey": s.full_chord().label(),
        "region_hotkey": s.region_chord().label(),
        "zoom_hotkey": s.zoom_chord().label(),
        "hotkeys": {
            "session": chord(s.session_chord(), chord::default_session()),
            "full": chord(s.full_chord(), chord::default_full()),
            "region": chord(s.region_chord(), chord::default_region()),
            "zoom": chord(s.zoom_chord(), chord::default_zoom()),
        },
        "hotkey_errors": errors,
        "talk_mode": s.talk_mode,
        "model": s.model,
        "device": s.device,
        "output": s.output,
        "also_clipboard": s.also_clipboard,
        "export_dir": s.export_dir,
        "zoom_px": s.zoom_px(),
        "start_at_login": s.start_at_login,
        "include_app": s.include_app,
        "hidden": s.hidden_list(),
        "mcp_bind": s.mcp_bind,
        "mcp_enabled": s.mcp_enabled,
        "onboarded": s.onboarded,
        "sounds": s.sounds,
        "reserved": chord::reserved().into_iter().map(|(k, why)| json!({"keys": k, "why": why})).collect::<Vec<_>>(),
        "speech": voice::models::view(&voice::models::speech_dir(), &voice::models::catalog()),
        "devices": voice::audio::devices(),
        "permissions": permissions::probe(),
        "recommended": voice::machine::recommended(),
    })
}

#[tauri::command]
fn settings_get(svc: tauri::State<'_, Arc<App>>) -> Value {
    settings_view(&svc)
}

/// Merge a partial change into the saved settings. Shortcuts are checked before anything is kept.
#[tauri::command]
fn settings_patch(
    app: AppHandle,
    svc: tauri::State<'_, Arc<App>>,
    patch: serde_json::Map<String, Value>,
) -> Result<Value, String> {
    let cur = svc.settings.lock().unwrap().clone();
    let next = merge(&cur, patch)?;
    svc.save_settings(next.clone())?;
    if keys_changed(&cur, &next) {
        rebind(&app, &svc);
    }
    if cur.include_app != next.include_app {
        platform::protect(&app, !next.include_app);
    }
    if cur.model != next.model {
        let svc = svc.inner().clone();
        std::thread::spawn(move || {
            let _ = svc.ensure_engine();
        });
    }
    Ok(settings_view(&svc))
}

fn merge(cur: &Settings, patch: serde_json::Map<String, Value>) -> Result<Settings, String> {
    let mut v = serde_json::to_value(cur).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().ok_or("settings are not an object")?;
    for (k, val) in patch {
        if val.is_null() {
            obj.remove(&k);
        } else {
            obj.insert(k, val);
        }
    }
    let next: Settings = serde_json::from_value(v).map_err(|e| e.to_string())?;
    let chords = [
        ("Start / stop", &next.session_hotkey),
        ("Capture screen", &next.full_hotkey),
        ("Capture region", &next.region_hotkey),
        ("Capture around pointer", &next.zoom_hotkey),
    ];
    let mut seen: Vec<(&str, chord::Chord)> = vec![];
    for (name, raw) in chords {
        let Some(raw) = raw else { continue };
        let c = chord::Chord::parse(raw)?;
        if let Some(why) = chord::refusal(&c) {
            return Err(format!("{}: {why}", c.label()));
        }
        if let Some((other, _)) = seen.iter().find(|(_, x)| *x == c) {
            return Err(format!("{} is already {other}", c.label()));
        }
        seen.push((name, c));
    }
    Ok(next)
}

fn keys_changed(a: &Settings, b: &Settings) -> bool {
    (a.session_chord(), a.full_chord(), a.region_chord(), a.zoom_chord())
        != (b.session_chord(), b.full_chord(), b.region_chord(), b.zoom_chord())
}

fn rebind(app: &AppHandle, svc: &Arc<App>) {
    let s = svc.settings.lock().unwrap().clone();
    *svc.hotkey_errors.lock().unwrap() = register_shortcuts(app, &s);
    tray::rekey(app, svc);
}

#[tauri::command]
fn session_state(svc: tauri::State<'_, Arc<App>>) -> session::UiState {
    svc.ui()
}

#[tauri::command]
fn session_toggle(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<session::UiState, String> {
    toggle(&app, &svc)?;
    Ok(svc.ui())
}

#[tauri::command]
fn session_cancel(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let _ = svc.stop(true)?;
    hide_window(&app, "indicator");
    hide_window(&app, "overlay");
    emit_state(&app, &svc);
    Ok(())
}

#[tauri::command]
fn capture_full(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let cap = svc.capture_full()?;
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
fn capture_zoom(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let cap = svc.capture_zoom()?;
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
fn region_begin(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    open_region(&app, &svc)
}

#[derive(Deserialize)]
struct RegionCommit {
    png_b64: String,
    #[serde(default)]
    labels: Vec<Label>,
}

#[tauri::command]
fn region_commit(
    app: AppHandle,
    svc: tauri::State<'_, Arc<App>>,
    body: RegionCommit,
) -> Result<Value, String> {
    use base64::Engine;
    let png = base64::engine::general_purpose::STANDARD
        .decode(body.png_b64.trim())
        .map_err(|e| e.to_string())?;
    let cap = svc.commit_region(png, body.labels)?;
    hide_window(&app, "overlay");
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height, "labels": cap.labels}))
}

#[tauri::command]
fn region_cancel(app: AppHandle) {
    hide_window(&app, "overlay");
}

#[tauri::command]
fn last_prompt(svc: tauri::State<'_, Arc<App>>) -> Option<SessionExport> {
    svc.last_export()
}

#[tauri::command]
fn last_folder(svc: tauri::State<'_, Arc<App>>) -> Option<String> {
    svc.last_dir().map(|p| p.to_string_lossy().into_owned())
}

#[tauri::command]
fn open_folder(path: String) -> Result<(), String> {
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| e.to_string())
}

fn copy_last(svc: &App) -> Result<(), String> {
    let exp = svc.last_export().ok_or("no session yet")?;
    copy_export(&exp)
}

fn copy_export(exp: &SessionExport) -> Result<(), String> {
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    if exp.images.is_empty() {
        return cb.set_text(&exp.plain).map_err(|e| e.to_string());
    }
    cb.set_html(&exp.html, Some(&exp.plain)).map_err(|e| e.to_string())
}

#[tauri::command]
fn history_list(svc: tauri::State<'_, Arc<App>>) -> Value {
    json!({ "entries": svc.history.list(), "last": *svc.last_id.lock().unwrap() })
}

#[tauri::command]
fn history_get(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<SessionExport, String> {
    svc.history.export(&id)
}

/// Copy a past prompt; it also becomes the one "Paste Last Prompt" uses.
#[tauri::command]
fn history_copy(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_export(&svc.set_last(&id)?)
}

#[tauri::command]
fn history_paste(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_export(&svc.set_last(&id)?)?;
    hide_window(&app, "result");
    std::thread::sleep(std::time::Duration::from_millis(220));
    clipboard::paste_now()
}

#[tauri::command]
fn history_delete(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    svc.history.delete(&id)
}

#[tauri::command]
fn history_clear(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    *svc.last_id.lock().unwrap() = None;
    svc.history.clear()
}

pub fn deliver_last(svc: &App) -> Result<(), String> {
    copy_last(svc)?;
    clipboard::paste_now()
}

#[tauri::command]
fn copy_again(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)
}

/// Hide our window first so the paste lands in the app underneath it.
#[tauri::command]
fn paste_again(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)?;
    hide_window(&app, "result");
    std::thread::sleep(std::time::Duration::from_millis(220));
    clipboard::paste_now()
}

/// The latest setup progress, so a window opened mid-download picks it up.
static SPEECH: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);
static SPEECH_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn speech_emit(app: &AppHandle, v: Value) {
    *SPEECH.lock().unwrap() = Some(v.clone());
    let _ = app.emit("speech://progress", v);
}

/// Download what is missing for `id` (engine, then model) as one job with one progress bar, then
/// load it and decode silence so "ready" means it runs here. A second call while one runs is a no-op.
#[tauri::command]
fn speech_setup(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    if SPEECH_BUSY.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    voice::models::CANCEL.store(false, Ordering::SeqCst);
    let svc = svc.inner().clone();
    std::thread::spawn(move || {
        let r = speech_job(&app, &svc, &id);
        *SPEECH.lock().unwrap() = None;
        SPEECH_BUSY.store(false, Ordering::SeqCst);
        let _ = app.emit("speech://done", json!({ "id": id, "ok": r.is_ok(), "error": r.err() }));
    });
    Ok(())
}

fn speech_job(app: &AppHandle, svc: &Arc<App>, id: &str) -> Result<(), String> {
    use voice::models::{self as m, Progress};
    let root = m::speech_dir();
    let cat = m::catalog();
    let rt = cat.runtime_for_host().ok_or("Speech isn't available for this computer yet.")?;
    let model = cat.model(id).ok_or("That model isn't in the catalog.")?;
    let need_rt = !m::runtime_installed(&root, rt);
    let need_model = !m::model_installed(&root, model);
    let total = if need_rt { rt.bytes } else { 0 } + if need_model { model.bytes } else { 0 };
    if let Some(free) = m::free_bytes(&root) {
        let need = m::space_needed(total);
        if total > 0 && free < need {
            return Err(format!(
                "Not enough free disk space: setup needs about {} MB and {} MB are free.",
                need / 1_000_000,
                free / 1_000_000
            ));
        }
    }
    let step = |offset: u64, part: &'static str| {
        let app = app.clone();
        move |p: Progress| {
            let v = match p {
                Progress::Downloading { received, .. } => json!({
                    "phase": "download", "part": part, "received": offset + received, "total": total,
                }),
                Progress::Unpacking | Progress::Verifying => json!({ "phase": "unpack", "part": part }),
                _ => return,
            };
            speech_emit(&app, v);
        }
    };
    speech_emit(app, json!({ "phase": "download", "part": "runtime", "received": 0, "total": total }));
    if need_rt {
        m::install_runtime(&root, rt, &step(0, "runtime"))?;
    }
    if need_model {
        let done = if need_rt { rt.bytes } else { 0 };
        m::install_model(&root, model, &step(done, "model"))?;
    }
    speech_emit(app, json!({ "phase": "check" }));
    let mut s = svc.settings.lock().unwrap().clone();
    if s.model != id {
        s.model = id.to_string();
        svc.save_settings(s)?;
    }
    svc.engine_check()
        .map_err(|e| format!("The speech engine didn't start on this computer: {e}"))
}

#[tauri::command]
fn speech_cancel() {
    voice::models::CANCEL.store(true, Ordering::SeqCst);
}

#[tauri::command]
fn speech_status() -> Option<Value> {
    SPEECH.lock().unwrap().clone()
}

#[tauri::command]
fn permissions_get() -> permissions::PermissionStatus {
    permissions::probe()
}

#[tauri::command]
fn permission_request(kind: String) -> permissions::PermissionStatus {
    permissions::request(&kind);
    permissions::probe()
}

#[tauri::command]
fn permission_open(kind: String) -> Result<(), String> {
    let url = permissions::settings_url(&kind).ok_or("no settings page for this")?;
    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// macOS applies a new Screen Recording grant only to a fresh process.
#[tauri::command]
fn relaunch(app: AppHandle) {
    app.restart();
}

#[tauri::command]
fn detect_tools() -> Vec<tools::DetectedTool> {
    tools::detect()
}

#[tauri::command]
fn open_tool(id: String) -> Result<(), String> {
    tools::open(&id)
}

#[tauri::command]
fn send_to_tool(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    copy_last(&svc)?;
    tools::open(&id)?;
    std::thread::sleep(std::time::Duration::from_millis(700));
    clipboard::paste_now()
}

#[tauri::command]
fn overlay_esc(layer: overlay::OverlayLayer) -> overlay::OverlayLayer {
    overlay::esc(layer)
}

#[tauri::command]
fn overlay_hint(layer: overlay::OverlayLayer, tool: overlay::Tool) -> String {
    overlay::hint(layer, tool).into()
}

#[tauri::command]
fn inject_speech(svc: tauri::State<'_, Arc<App>>, text: String) {
    svc.inject_speech(&text);
}

#[tauri::command]
fn parse_chord(text: String) -> Result<Value, String> {
    let c = chord::Chord::parse(&text)?;
    if let Some(why) = chord::refusal(&c) {
        return Err(why);
    }
    Ok(json!({ "label": c.label(), "accelerator": c.accelerator(), "keys": c.keys() }))
}

/// While a shortcut field is listening, the OS must not swallow the chord being typed.
#[tauri::command]
fn hotkeys_suspend(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool) -> Value {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    if on {
        let _ = app.global_shortcut().unregister_all();
    } else {
        rebind(&app, &svc);
    }
    settings_view(&svc)
}

/// Onboarding's "press it now": shortcuts report that they fired and do nothing else.
#[tauri::command]
fn hotkey_test(svc: tauri::State<'_, Arc<App>>, on: bool) {
    svc.hotkey_test.store(on, Ordering::SeqCst);
}

/// Onboarding's practice run: a real session whose prompt stays in the onboarding window.
#[tauri::command]
fn practice(svc: tauri::State<'_, Arc<App>>, on: bool) {
    svc.practice.store(on, Ordering::SeqCst);
}

/// Live input level outside a session, for the microphone check.
#[tauri::command]
fn mic_monitor(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool) -> Result<(), String> {
    if !on {
        svc.monitor_off();
        return Ok(());
    }
    let fresh = svc.monitor_level().is_none();
    svc.monitor_on()?;
    if fresh {
        let svc = svc.inner().clone();
        std::thread::spawn(move || {
            while let Some(level) = svc.monitor_level() {
                let _ = app.emit("mic://level", level);
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
    Ok(())
}

/// Whatever step the assistant was on, shortcuts and sessions go back to normal.
fn leave_onboarding(svc: &App) {
    svc.practice.store(false, Ordering::SeqCst);
    svc.hotkey_test.store(false, Ordering::SeqCst);
    svc.monitor_off();
}

#[tauri::command]
fn onboarding_finish(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    leave_onboarding(&svc);
    let mut s = svc.settings.lock().unwrap().clone();
    s.onboarded = true;
    svc.save_settings(s)?;
    hide_window(&app, "onboarding");
    Ok(())
}

#[tauri::command]
fn platform_info() -> Value {
    json!({ "os": std::env::consts::OS, "materials": platform::materials() })
}

#[tauri::command]
fn window_open(app: AppHandle, label: String) {
    show_window(&app, &label);
}

/// Register the four chords. Each one the OS refuses (another app owns it) is reported, not fatal.
fn register_shortcuts(app: &AppHandle, s: &Settings) -> Vec<(String, String)> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut errors = vec![];
    let chords = [
        (Action::Session, "session", s.session_chord()),
        (Action::Full, "full", s.full_chord()),
        (Action::Region, "region", s.region_chord()),
        (Action::Zoom, "zoom", s.zoom_chord()),
    ];
    for (action, name, chord) in chords {
        let res = Shortcut::from_str(&chord.accelerator())
            .map_err(|e| e.to_string())
            .and_then(|sc| {
                gs.on_shortcut(sc, move |app, _s, ev| {
                    hotkey(app, action, ev.state == ShortcutState::Pressed);
                })
                .map_err(|_| format!("{} is taken by another app", chord.label()))
            });
        if let Err(e) = res {
            errors.push((name.to_string(), e));
        }
    }
    errors
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings_path = settings::settings_path();
    let settings = Settings::load(&settings_path);
    let svc = App::new(settings.clone(), settings_path);
    let latest = Latest::default();
    if settings.mcp_enabled {
        match mcp::spawn(&settings.mcp_bind, latest.clone()) {
            Ok(addr) => eprintln!("Pointcast MCP on http://{addr}"),
            Err(e) => eprintln!("Pointcast MCP did not start: {e}"),
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app, "settings");
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .manage(svc.clone())
        .manage(latest)
        .invoke_handler(tauri::generate_handler![
            settings_get,
            settings_patch,
            session_state,
            session_toggle,
            session_cancel,
            capture_full,
            capture_zoom,
            region_begin,
            region_commit,
            region_cancel,
            last_prompt,
            last_folder,
            open_folder,
            copy_again,
            paste_again,
            speech_setup,
            history_list,
            history_get,
            history_copy,
            history_paste,
            history_delete,
            history_clear,
            speech_cancel,
            speech_status,
            permissions_get,
            permission_request,
            permission_open,
            relaunch,
            detect_tools,
            open_tool,
            send_to_tool,
            overlay_esc,
            overlay_hint,
            inject_speech,
            parse_chord,
            hotkeys_suspend,
            hotkey_test,
            practice,
            mic_monitor,
            onboarding_finish,
            window_open,
            platform_info,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            tray::setup(&handle, &svc)?;
            *svc.hotkey_errors.lock().unwrap() = register_shortcuts(&handle, &settings);
            for label in ["settings", "indicator", "overlay", "result", "onboarding"] {
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.hide();
                }
            }
            platform::protect(&handle, !settings.include_app);
            if let Some(m) = svc.history.recover() {
                let _ = svc.set_last(&m.id);
                show_window(&handle, "result");
            } else if let Some(m) = svc.history.list().first() {
                let _ = svc.set_last(&m.id);
            }
            let svc3 = svc.clone();
            std::thread::spawn(move || {
                let _ = svc3.ensure_engine();
            });
            if !settings.onboarded {
                show_window(&handle, "onboarding");
            }
            // Pulse live state to the HUD and the menu bar clock while a session runs.
            let svc2 = svc.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                if matches!(svc2.ui().phase, Phase::Recording | Phase::Loading | Phase::Transcribing) {
                    emit_state(&handle, &svc2);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Windows close to the tray; only Quit ends Pointcast.
                if matches!(window.label(), "settings" | "result" | "onboarding") {
                    api.prevent_close();
                    let _ = window.hide();
                }
                if window.label() == "onboarding" {
                    if let Some(svc) = window.app_handle().try_state::<Arc<App>>() {
                        leave_onboarding(&svc);
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Pointcast");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tap_keeps_recording_and_a_hold_stops_on_release() {
        assert!(!release_stops(TalkMode::Tap, Some(0), 5_000));
        assert!(!release_stops(TalkMode::Hold, Some(1_000), 1_000 + HOLD_MS - 1));
        assert!(release_stops(TalkMode::Hold, Some(1_000), 1_000 + HOLD_MS));
        assert!(!release_stops(TalkMode::Hold, None, 9_000));
    }

    #[test]
    fn a_patch_keeps_what_it_does_not_name_and_refuses_bad_chords() {
        let mut cur = Settings::default();
        cur.onboarded = true;
        cur.zoom_px = 640;
        let p = |v: Value| v.as_object().unwrap().clone();
        let next = merge(&cur, p(json!({ "sounds": false }))).unwrap();
        assert!(next.onboarded && !next.sounds && next.zoom_px == 640);
        let next = merge(&cur, p(json!({ "session_hotkey": "Ctrl+Alt+F9" })));
        if cfg!(target_os = "macos") {
            assert!(next.is_ok());
        } else {
            assert!(next.unwrap_err().contains("AltGr"));
        }
        let dup = merge(&cur, p(json!({ "full_hotkey": "Ctrl+Shift+F8", "zoom_hotkey": "Ctrl+Shift+F8" })));
        assert!(dup.unwrap_err().contains("already"));
        assert!(merge(&cur, p(json!({ "region_hotkey": "Shift" }))).is_err());
        let back = merge(&cur, p(json!({ "export_dir": "/tmp/x" }))).unwrap();
        let cleared = merge(&back, p(json!({ "export_dir": null }))).unwrap();
        assert_eq!(cleared.export_dir, None);
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
    }
}
