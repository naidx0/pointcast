mod anchor;
mod capture;
mod chord;
mod clipboard;
mod element;
mod history;
mod image_ops;
mod keyframes;
mod login;
mod mcp;
mod overlay;
mod permissions;
mod platform;
mod pointer;
mod privacy;
mod prompt;
mod session;
mod settings;
mod tools;
mod tray;
mod update;
mod voice;

use prompt::{Label, SessionExport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use session::{now_ms, App, Phase};
use settings::Settings;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};
use voice::ptt::TalkMode;

/// Minutes without recording before the HUD, overlay and speech model are released.
const IDLE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

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
    if let Some(w) = platform::ensure(app, label) {
        // History's Paste into App goes back to the app that was in front when it opened.
        if label == "result" && !w.is_visible().unwrap_or(false) {
            platform::remember_front();
        }
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        if platform::DESK.contains(&label) {
            platform::dock(app, None);
        }
    }
}

fn hide_window(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.hide();
        if platform::DESK.contains(&label) {
            platform::dock(app, Some(label));
        }
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

fn emit_state_now(app: &AppHandle, svc: &App) {
    let _ = app.emit("session://state", &svc.ui());
}

fn recording(svc: &App) -> bool {
    matches!(svc.ui().phase, Phase::Recording | Phase::Loading)
}

/// No recording runs or is finishing.
fn idle(svc: &App) -> bool {
    matches!(svc.ui().phase, Phase::Idle | Phase::Ready | Phase::Error)
}

fn begin(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    // The installer closes Pointcast: a recording started now would stop mid-sentence.
    if update::installing() {
        svc.refuse(session::UPDATING);
        platform::show_indicator(app);
        emit_state(app, svc);
        hide_hud_soon(app, svc);
        return Err(session::UPDATING.into());
    }
    platform::forget_front();
    svc.start()?;
    if svc.settings.lock().unwrap().sounds {
        platform::chime(true);
    }
    platform::show_indicator(app);
    emit_state(app, svc);
    Ok(())
}

/// The HUD's last word stays a moment, then it goes unless a new recording took it.
fn hide_hud_soon(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(1600));
        if svc.ui().phase == Phase::Ready {
            hide_window(&app, "indicator");
        }
    });
}

/// Every way the overlay goes away (save, cancel, stop, discard, Alt+F4): hide it, give focus
/// back to the app it was over, and forget its frame and any History edit.
fn close_overlay(app: &AppHandle) {
    EDIT.cancel();
    hide_window(app, "overlay");
    platform::refocus_front();
    FRAME.lock().unwrap().take();
}

fn finish(app: &AppHandle, svc: &Arc<App>) -> Result<Option<SessionExport>, String> {
    close_overlay(app);
    if svc.settings.lock().unwrap().sounds {
        platform::chime(false);
    }
    // The HUD reads "Finishing…" while the last words are transcribed, then where the prompt went.
    emit_state_now(app, svc);
    let practice = svc.practice.load(Ordering::SeqCst);
    let exp = svc.stop(false);
    platform::forget_front();
    if practice || exp.is_err() {
        hide_window(app, "indicator");
    } else {
        emit_state_now(app, svc);
        hide_hud_soon(app, svc);
    }
    if practice {
        svc.practice.store(false, Ordering::SeqCst);
    }
    let exp = exp?;
    if let Some(ref e) = exp {
        if practice {
            let _ = app.emit("practice://result", e);
        } else {
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

/// The frozen frame for the overlay, with where the person pointed (frame pixels) when they want it marked and the
/// control there when they want outlines; the overlay places a ring and an outline as shapes they can move or remove.
fn frame_payload(shot: &capture::Shot, mark: settings::MarkPoint) -> Value {
    let pointer = shot.pointer.filter(|_| mark != settings::MarkPoint::Off);
    json!({
        "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.png),
        "width": shot.width,
        "height": shot.height,
        "pointer": pointer.map(|p| json!({ "x": p.x, "y": p.y })),
        "element": shot.element,
    })
}

fn overlay_open(app: &AppHandle) -> bool {
    app.get_webview_window("overlay")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// Show a frame in the overlay, remembering the app in front unless the overlay already is.
fn show_frame(app: &AppHandle, payload: &Value) {
    if !overlay_open(app) {
        platform::remember_front();
    }
    *FRAME.lock().unwrap() = Some(payload.clone());
    let _ = app.emit("overlay://frame", payload);
    platform::show_overlay(app);
}

fn open_region(app: &AppHandle, svc: &Arc<App>) -> Result<Value, String> {
    let shot = svc.begin_region()?;
    EDIT.cancel();
    let mark = svc.settings.lock().unwrap().mark_point;
    let payload = frame_payload(&shot, mark);
    show_frame(app, &payload);
    Ok(payload)
}

/// The frozen frame the overlay is showing; an overlay built just now asks for it on load,
/// since the event may have fired before its page was listening.
static FRAME: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

#[tauri::command]
fn region_frame() -> Option<Value> {
    FRAME.lock().unwrap().clone()
}

/// A picture from History reopened in the overlay: (entry id, picture n). Committing writes back into it.
struct Editing(std::sync::Mutex<Option<(String, usize)>>);

static EDIT: Editing = Editing(std::sync::Mutex::new(None));

impl Editing {
    /// The overlay frame for picture `n` of entry `id`; the next commit goes back into that picture.
    fn begin(&self, store: &history::Store, id: &str, n: usize) -> Result<Value, String> {
        let cap = store.picture(id, n)?;
        let (width, height) = image_ops::png_size(&cap.png).unwrap_or((cap.width, cap.height));
        *self.0.lock().unwrap() = Some((id.to_string(), n));
        Ok(json!({
            "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &cap.png),
            "width": width,
            "height": height,
            "pointer": null,
            "element": null,
            "edit": true,
            // New pins are numbered on from the labels already drawn into the picture.
            "base": cap.labels.iter().map(|l| l.n).max().unwrap_or(0),
        }))
    }

    /// Write a commit back into the picture being edited; None when the overlay is not editing one. A failed write
    /// keeps the edit open, so a second try never falls through into a live recording.
    fn commit(&self, store: &history::Store, png: &[u8], labels: Vec<Label>) -> Option<Result<(String, prompt::Capture), String>> {
        let mut g = self.0.lock().unwrap();
        let (id, n) = g.clone()?;
        let r = store.replace(&id, n, png, labels);
        if r.is_ok() {
            *g = None;
        }
        Some(r.map(|c| (id, c)))
    }

    fn cancel(&self) {
        self.0.lock().unwrap().take();
    }
}

/// Reopen picture `n` of a past prompt in the overlay to mark it up again.
#[tauri::command]
async fn history_edit(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String, n: usize) -> Result<(), String> {
    let payload = EDIT.begin(&svc.history, &id, n)?;
    show_frame(&app, &payload);
    Ok(())
}

/// What the session chord's release does: stop only a held (not tapped) push-to-talk.
fn release_stops(mode: TalkMode, held_from: Option<u64>, now: u64) -> bool {
    mode == TalkMode::Hold && held_from.is_some_and(|t| now.saturating_sub(t) >= HOLD_MS)
}

#[derive(Debug, PartialEq, Eq)]
enum Press {
    Start,
    Stop,
    /// The overlay is open: what is marked there goes in first, then the recording stops.
    CommitThenStop,
}

/// What pressing the session chord does.
fn session_press(rec: bool, overlay: bool) -> Press {
    match (rec, overlay) {
        (false, _) => Press::Start,
        (true, false) => Press::Stop,
        (true, true) => Press::CommitThenStop,
    }
}

/// Every shortcut, from the keyboard or the tray. A capture chord with no session starts one.
pub fn hotkey(app: &AppHandle, action: Action, pressed: bool) {
    let _ = app.emit("hotkey://fired", json!({ "action": action, "pressed": pressed }));
    let Some(svc) = app.try_state::<Arc<App>>() else {
        return;
    };
    let svc = svc.inner().clone();
    let rec = recording(&svc);
    // While a recording runs, its shortcuts still work: the test must not trap it.
    if svc.hotkey_test.load(Ordering::SeqCst) && !rec {
        return;
    }
    match (action, pressed) {
        (Action::Session, true) => match session_press(rec, rec && overlay_open(app)) {
            Press::CommitThenStop => {
                *svc.hold_from.lock().unwrap() = None;
                let _ = app.emit_to("overlay", "overlay://finish", ());
            }
            Press::Stop => {
                *svc.hold_from.lock().unwrap() = None;
                finish_soon(app, &svc);
            }
            Press::Start => {
                if begin(app, &svc).is_ok() {
                    *svc.hold_from.lock().unwrap() = Some(now_ms());
                }
            }
        },
        (Action::Session, false) => {
            let from = svc.hold_from.lock().unwrap().take();
            let mode = svc.settings.lock().unwrap().talk_mode;
            // A release while the overlay is open keeps recording, so the box being drawn is not lost.
            if rec && release_stops(mode, from, now_ms()) && !overlay_open(app) {
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

fn installed_models<'a>(root: &std::path::Path, cat: &'a voice::models::Catalog) -> Vec<&'a str> {
    cat.models
        .iter()
        .filter(|x| voice::models::model_installed(root, x))
        .map(|x| x.id.as_str())
        .collect()
}

fn settings_view(svc: &App) -> Value {
    let s = svc.settings.lock().unwrap().clone();
    let speech = (voice::models::speech_dir(), voice::models::catalog());
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
    let mut v = json!({
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
        "mark_point": s.mark_point,
        "zoom_target": s.zoom_target,
        "zoom_sharpen": s.zoom_sharpen,
        "outline_element": s.outline_element,
        "spoken_anchors": s.spoken_anchors,
        "key_frames": s.key_frames,
        "start_at_login": s.start_at_login,
        "check_updates": s.check_updates,
        "update": update::available(),
        "update_installing": update::installing(),
        "update_failure": update::last_failure(),
        "include_app": s.include_app,
        "hidden": s.hidden_list(),
        "mcp_bind": s.mcp_bind,
        "mcp_enabled": s.mcp_enabled,
        "mcp_error": MCP_ERROR.get(),
        "onboarded": s.onboarded,
        "sounds": s.sounds,
        "reserved": chord::reserved().into_iter().map(|(k, why)| json!({"keys": k, "why": why})).collect::<Vec<_>>(),
        "speech": voice::models::view(&speech.0, &speech.1),
        "speech_check": svc.speech_check().map(|r| json!({ "ok": r.is_ok(), "error": r.err() })),
        // The model a recording loads, which may not be the one picked (that one is not downloaded).
        "speech_in_use": voice::machine::route(&s.model, voice::machine::recommended(), &installed_models(&speech.0, &speech.1)),
        "devices": voice::audio::devices(),
        "permissions": permissions::probe(),
        "recommended": voice::machine::recommended(),
    });
    // Set apart: one more key in the literal above passes json!'s recursion limit.
    v["update_checked"] = update::checked();
    v
}

/// Async, like permissions_get: off the event loop, since it reads the registry and lists devices.
#[tauri::command]
async fn settings_get(svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    Ok(settings_view(&svc))
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
    if cur.model != next.model && !recording(&svc) {
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
        ("Start / stop", &cur.session_hotkey, &next.session_hotkey),
        ("Capture screen", &cur.full_hotkey, &next.full_hotkey),
        ("Capture region", &cur.region_hotkey, &next.region_hotkey),
        ("Capture around pointer", &cur.zoom_hotkey, &next.zoom_hotkey),
    ];
    let mut seen: Vec<(&str, chord::Chord)> = vec![];
    for (name, was, raw) in chords {
        let Some(raw) = raw else { continue };
        // A saved chord this patch leaves alone and this version refuses is not in use (the default is), so it never
        // blocks changing something else.
        let changed = was.as_deref() != Some(raw.as_str());
        let c = match chord::Chord::parse(raw) {
            Ok(c) => c,
            Err(e) if changed => return Err(e),
            Err(_) => continue,
        };
        if let Some(why) = chord::refusal(&c) {
            if !changed {
                continue;
            }
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
async fn session_toggle(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<session::UiState, String> {
    toggle(&app, &svc)?;
    Ok(svc.ui())
}

/// Async: discarding waits for the speech worker, which must not hold the event loop.
#[tauri::command]
async fn session_cancel(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let _ = svc.stop(true)?;
    hide_window(&app, "indicator");
    close_overlay(&app);
    platform::forget_front();
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
async fn region_begin(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    open_region(&app, &svc)
}

#[derive(Deserialize)]
struct RegionCommit {
    png_b64: String,
    #[serde(default)]
    labels: Vec<Label>,
    /// The outlined control, in the cropped picture's pixels, when the person kept it.
    #[serde(default)]
    element: Option<element::ElementMark>,
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
    // A picture from History goes back into its entry, which becomes the last prompt and is copied again.
    let (cap, edited) = match EDIT.commit(&svc.history, &png, body.labels.clone()) {
        Some(r) => {
            let (id, cap) = r?;
            (cap, Some(id))
        }
        None => (svc.commit_region(png, body.labels, body.element)?, None),
    };
    close_overlay(&app);
    let last = match edited {
        Some(id) => {
            let exp = svc.set_last(&id)?;
            let _ = app.emit("history://changed", json!({ "id": id }));
            Some(exp)
        }
        None => None,
    };
    emit_state(&app, &svc);
    if let Some(exp) = last {
        copy_export(&exp)?;
    }
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height, "labels": cap.labels}))
}

#[tauri::command]
fn region_cancel(app: AppHandle) {
    close_overlay(&app);
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
    paste_from_history(&app)
}

/// Hide History so the paste lands in the app underneath it.
fn paste_from_history(app: &AppHandle) -> Result<(), String> {
    paste_out_of(
        permissions::can_paste(),
        || {
            hide_window(app, "result");
            platform::restore_front();
            std::thread::sleep(std::time::Duration::from_millis(220));
        },
        clipboard::paste_now,
        || show_window(app, "result"),
    )
}

/// Paste from a window that must get out of the way first. Without permission to paste it stays, and a paste that
/// fails once it is hidden brings it back, so the person reads that the prompt was only copied.
fn paste_out_of(can_paste: bool, hide: impl FnOnce(), paste: impl FnOnce() -> Result<(), String>, back: impl FnOnce()) -> Result<(), String> {
    if !can_paste {
        return Err(PASTE_YOURSELF.into());
    }
    hide();
    let r = paste_or_say(true, paste);
    if r.is_err() {
        back();
    }
    r
}

pub const PASTE_YOURSELF: &str = "Copied, paste it yourself.";

/// Paste what was just copied, or say it is only on the clipboard: without permission to paste (macOS
/// Accessibility), or when the app in front refuses it (an app run as administrator on Windows).
fn paste_or_say(can_paste: bool, paste: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    if !can_paste {
        return Err(PASTE_YOURSELF.into());
    }
    paste().map_err(|_| PASTE_YOURSELF.into())
}

fn paste() -> Result<(), String> {
    paste_or_say(permissions::can_paste(), clipboard::paste_now)
}

/// Take a key frame Pointcast picked out of a past prompt, which becomes the last prompt and is copied again.
#[tauri::command]
fn history_remove(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String, n: usize) -> Result<SessionExport, String> {
    svc.history.remove(&id, n)?;
    let exp = svc.set_last(&id)?;
    let _ = app.emit("history://changed", json!({ "id": id }));
    copy_export(&exp)?;
    Ok(exp)
}

#[tauri::command]
fn history_delete(app: AppHandle, svc: tauri::State<'_, Arc<App>>, id: String) -> Result<(), String> {
    svc.history.delete(&id)?;
    forgot(&app, &svc, Some(&id));
    Ok(())
}

#[tauri::command]
fn history_clear(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let r = svc.history.clear();
    forgot(&app, &svc, None);
    r
}

/// A deleted prompt leaves Paste Last Prompt and what coding agents read too.
fn forgot(app: &AppHandle, svc: &App, gone: Option<&str>) {
    if svc.forget(gone) {
        emit_state(app, svc);
    }
}

pub fn deliver_last(svc: &App) -> Result<(), String> {
    copy_last(svc)?;
    paste()
}

#[tauri::command]
fn copy_again(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)
}

/// Copy the MCP setup for coding agents, token included. The token never passes through a window.
#[tauri::command]
fn mcp_copy_setup(svc: tauri::State<'_, Arc<App>>, agent: String) -> Result<(), String> {
    let bind = svc.settings.lock().unwrap().mcp_bind.clone();
    let token = mcp::token(&mcp::token_path())?;
    let text = agent_setup(&agent, &bind, &token)?;
    clipboard::copy_secret(&text)
}

/// For the user's own settings only, never a project file people commit: the key is this computer's. Claude Code gets
/// its user-scope command, Cursor the one server entry for `~/.cursor/mcp.json`.
fn agent_setup(agent: &str, bind: &str, token: &str) -> Result<String, String> {
    let all = mcp::client_config(bind, token);
    let server = &all["mcpServers"]["pointcast"];
    let url = server["url"].as_str().ok_or("no MCP address")?;
    let auth = server["headers"]["Authorization"].as_str().ok_or("no MCP key")?;
    match agent {
        "claude" => Ok(format!("claude mcp add --transport http --scope user pointcast {url} --header {q}Authorization: {auth}{q}", q = '"')),
        "cursor" => Ok(format!("{q}pointcast{q}: {}", serde_json::to_string_pretty(server).map_err(|e| e.to_string())?, q = '"')),
        _ => Err("Unknown agent.".into()),
    }
}

#[tauri::command]
fn paste_again(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    copy_last(&svc)?;
    paste_from_history(&app)
}

/// Why sharing with coding agents did not start at launch (the address is taken, say), for Settings.
static MCP_ERROR: std::sync::OnceLock<String> = std::sync::OnceLock::new();

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
        if r.as_ref().err().map(String::as_str) == Some(voice::models::CANCELLED) {
            let root = voice::models::speech_dir();
            let cat = voice::models::catalog();
            for part in cat.runtime_for_host().map(|r| r.id.as_str()).into_iter().chain([id.as_str()]) {
                voice::models::drop_partial(&root, part);
            }
        }
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

/// The model the next recording would load, by the same rule the engine uses.
fn in_use(chosen: &str, recommended: &str, installed: &[&str], id: &str) -> bool {
    voice::machine::route(chosen, recommended, installed) == Some(id)
}

/// Delete a downloaded model to free its disk space. The model in use stays.
#[tauri::command]
fn speech_remove(svc: tauri::State<'_, Arc<App>>, id: String) -> Result<Value, String> {
    let cat = voice::models::catalog();
    let m = cat.model(&id).ok_or("That model isn't in the catalog.")?;
    let root = voice::models::speech_dir();
    let installed = installed_models(&root, &cat);
    let active = in_use(&svc.settings.lock().unwrap().model, voice::machine::recommended(), &installed, &m.id);
    if active || SPEECH_BUSY.load(Ordering::SeqCst) {
        return Err("That model is in use. Pick another one first.".into());
    }
    voice::models::remove_model(&root, m)?;
    Ok(settings_view(&svc))
}

/// Async: the setup assistant asks every second and a half, and each probe reads the registry and lists devices.
#[tauri::command]
async fn permissions_get(svc: tauri::State<'_, Arc<App>>) -> Result<permissions::PermissionStatus, String> {
    Ok(noted(&svc, permissions::probe()))
}

#[tauri::command]
fn permission_request(svc: tauri::State<'_, Arc<App>>, kind: String) -> permissions::PermissionStatus {
    permissions::request(&kind);
    noted(&svc, permissions::probe())
}

/// Remember what was ever granted, so launch can tell a permission an update took from one never given.
fn noted(svc: &App, p: permissions::PermissionStatus) -> permissions::PermissionStatus {
    let mut s = svc.settings.lock().unwrap().clone();
    if s.granted.note(p.screen == permissions::Access::Granted, p.accessibility == permissions::Access::Granted) {
        let _ = svc.save_settings(s);
    }
    p
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
    paste()
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
    PAUSED.store(on, Ordering::SeqCst);
    if on {
        let _ = app.global_shortcut().unregister_all();
    } else {
        rebind(&app, &svc);
    }
    settings_view(&svc)
}

/// Shortcuts are unregistered while a field listens for new keys. A window closed mid-listen
/// never runs its own cleanup, so closing it brings them back here.
static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Onboarding's "press it now": shortcuts report that they fired and do nothing else.
#[tauri::command]
fn hotkey_test(svc: tauri::State<'_, Arc<App>>, on: bool) {
    svc.hotkey_test.store(on, Ordering::SeqCst);
}

/// Onboarding's practice run: the next session started is a real one whose prompt stays in the onboarding window.
/// Off mid-run discards it (async: that waits for the speech worker).
#[tauri::command]
async fn practice(app: AppHandle, svc: tauri::State<'_, Arc<App>>, on: bool) -> Result<(), String> {
    if on {
        svc.practice_next.store(true, Ordering::SeqCst);
    } else {
        end_practice(&app, &svc);
    }
    Ok(())
}

fn end_practice(app: &AppHandle, svc: &App) {
    if svc.practice_off() {
        let _ = svc.stop(true);
        svc.practice.store(false, Ordering::SeqCst);
        hide_window(app, "indicator");
        close_overlay(app);
        platform::forget_front();
        emit_state(app, svc);
    }
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

/// Whatever step the assistant was on, shortcuts and sessions go back to normal. On a thread of its own: discarding a
/// practice run waits for the speech worker, which must not hold the event loop.
fn leave_onboarding(app: &AppHandle, svc: &Arc<App>) {
    let (app, svc) = (app.clone(), svc.clone());
    std::thread::spawn(move || {
        end_practice(&app, &svc);
        svc.hotkey_test.store(false, Ordering::SeqCst);
        svc.monitor_off();
    });
}

#[tauri::command]
fn onboarding_finish(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    leave_onboarding(&app, &svc);
    let mut s = svc.settings.lock().unwrap().clone();
    // Open at login is on unless the person switched it off: register it now, so Pointcast is
    // back after a restart without anyone running the install command again.
    if s.start_at_login {
        if let Err(e) = login::set(&app, true) {
            eprintln!("Pointcast could not set itself to open at login: {e}");
        }
    }
    s.onboarded = true;
    svc.save_settings(s)?;
    // Closed rather than hidden, so opening it again starts over at the first step that needs attention.
    if let Some(w) = app.get_webview_window("onboarding") {
        let _ = w.close();
    }
    Ok(())
}

#[tauri::command]
fn login_get(app: AppHandle) -> Result<login::Login, String> {
    login::get(&app)
}

#[tauri::command]
fn login_set(app: AppHandle, on: bool) -> Result<login::Login, String> {
    login::set(&app, on)
}

/// Ask GitHub now. Returns the newer version, or null when up to date.
#[tauri::command]
async fn update_check(app: AppHandle) -> Result<Option<String>, String> {
    let found = tauri::async_runtime::spawn_blocking(update::check)
        .await
        .map_err(|e| e.to_string())??;
    tray::offer_update(&app, found.as_deref());
    Ok(found)
}

/// Run the installer: it closes Pointcast, installs the new version and opens it again.
#[tauri::command]
fn update_install(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    if !idle(&svc) {
        return Err("Finish the recording first, then update.".into());
    }
    let svc = svc.inner().clone();
    update::install(
        move || idle(&svc),
        move |why| {
            let _ = app.emit("update://failed", why);
        },
    )
}

#[tauri::command]
fn platform_info() -> Value {
    json!({ "os": std::env::consts::OS, "materials": platform::materials() })
}

#[tauri::command]
async fn window_open(app: AppHandle, label: String) {
    show_window(&app, &label);
}

/// Register the four chords. Each one the OS refuses (another app owns it) is reported, not fatal.
fn register_shortcuts(app: &AppHandle, s: &Settings) -> Vec<(String, String)> {
    use std::str::FromStr;
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut errors = refused(s);
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
            errors.retain(|(n, _)| n != name);
            errors.push((name.to_string(), e));
        }
    }
    errors
}

/// Saved chords this version refuses (saved before the rule existed): they fall back to the default, and the
/// person is told why rather than that another app took the key.
fn refused(s: &Settings) -> Vec<(String, String)> {
    let saved = [
        ("session", &s.session_hotkey, s.session_chord()),
        ("full", &s.full_hotkey, s.full_chord()),
        ("region", &s.region_hotkey, s.region_chord()),
        ("zoom", &s.zoom_hotkey, s.zoom_chord()),
    ];
    saved
        .into_iter()
        .filter_map(|(name, raw, used)| {
            let c = chord::Chord::parse(raw.as_deref()?).ok()?;
            let why = chord::refusal(&c)?;
            Some((name.to_string(), format!("{}: {why}. Using {} instead.", c.label(), used.label())))
        })
        .collect()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let settings_path = settings::settings_path();
    let settings = Settings::load(&settings_path);
    let svc = App::new(settings.clone(), settings_path);
    if settings.mcp_enabled {
        // Errors name the problem, never the token.
        match mcp::token(&mcp::token_path()).and_then(|t| mcp::spawn(&settings.mcp_bind, svc.latest.clone(), t)) {
            Ok(addr) => eprintln!("Pointcast MCP on http://{addr}"),
            Err(e) => {
                eprintln!("Pointcast MCP did not start: {e}");
                let _ = MCP_ERROR.set(e);
            }
        }
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // Started again at login (macOS session restore had already opened it): nothing to show.
            if login::at_login(&args) {
                return;
            }
            let onboarded = app
                .try_state::<Arc<App>>()
                .is_some_and(|svc| svc.settings.lock().unwrap().onboarded);
            show_window(app, if onboarded { "settings" } else { "onboarding" });
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(svc.clone())
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
            region_frame,
            last_prompt,
            last_folder,
            open_folder,
            copy_again,
            mcp_copy_setup,
            paste_again,
            speech_setup,
            history_list,
            history_get,
            history_edit,
            history_remove,
            history_copy,
            history_paste,
            history_delete,
            history_clear,
            speech_cancel,
            speech_status,
            speech_remove,
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
            login_get,
            login_set,
            update_check,
            update_install,
            window_open,
            platform_info,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let handle = app.handle().clone();
            clipboard::bind(&handle);
            tray::setup(&handle, &svc)?;
            *svc.hotkey_errors.lock().unwrap() = register_shortcuts(&handle, &settings);
            platform::protect(&handle, !settings.include_app);
            if let Some(m) = svc.history.recover() {
                let _ = svc.set_last(&m.id);
                show_window(&handle, "result");
            } else if let Some(m) = svc.history.list().first() {
                let _ = svc.set_last(&m.id);
            }
            if !settings.onboarded {
                show_window(&handle, "onboarding");
            }
            #[cfg(target_os = "macos")]
            if settings.onboarded {
                let p = noted(&svc, permissions::probe());
                let paste = settings.output == settings::OutputMode::AutoPaste;
                let on = |a: permissions::Access| a == permissions::Access::Granted;
                if settings.granted.lost(on(p.screen), on(p.accessibility), paste) {
                    show_window(&handle, "onboarding");
                }
            }
            // An entry an earlier version made is written again in this one's form (with --login).
            if settings.onboarded && settings.start_at_login {
                let handle = handle.clone();
                std::thread::spawn(move || {
                    if login::get(&handle).is_ok_and(|l| l.enabled && !l.machine) {
                        let _ = login::set(&handle, true);
                    }
                });
            }
            // A minute after launch and then once a day, look for a newer release (if allowed).
            {
                let (handle, svc) = (handle.clone(), svc.clone());
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    loop {
                        if svc.settings.lock().unwrap().check_updates {
                            if let Ok(found) = update::check() {
                                tray::offer_update(&handle, found.as_deref());
                                let _ = handle.emit("update://checked", &found);
                            }
                        }
                        std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
                    }
                });
            }
            // Pulse live state to the HUD and the menu bar clock while a session runs. After
            // IDLE without one, the floating windows and the speech model are let go.
            let svc2 = svc.clone();
            std::thread::spawn(move || {
                let mut busy_at = std::time::Instant::now();
                loop {
                    let busy = matches!(svc2.ui().phase, Phase::Recording | Phase::Loading | Phase::Transcribing);
                    if busy {
                        emit_state(&handle, &svc2);
                        busy_at = std::time::Instant::now();
                        // The app the prompt goes to at stop is the last one in front that is not Pointcast: a
                        // click on the HUD's Stop can bring Pointcast forward on macOS.
                        #[cfg(target_os = "macos")]
                        if recording(&svc2) && !handle.webview_windows().values().any(|w| w.is_focused().unwrap_or(false)) {
                            platform::remember_front();
                        }
                    } else if busy_at.elapsed() >= IDLE && !SPEECH_BUSY.load(Ordering::SeqCst) {
                        platform::reap(&handle);
                        svc2.unload_engine();
                        busy_at = std::time::Instant::now();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(if busy { 100 } else { 400 }));
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Closing a window frees it; Pointcast itself stays in the tray (see ExitRequested).
                let _ = api;
                if platform::DESK.contains(&window.label()) {
                    platform::dock(window.app_handle(), Some(window.label()));
                }
                if window.label() == "overlay" {
                    close_overlay(window.app_handle());
                }
                // History closed without pasting: the app it opened over is not where a later paste goes.
                if window.label() == "result" && !window.app_handle().try_state::<Arc<App>>().is_some_and(|svc| recording(&svc)) {
                    platform::forget_front();
                }
                if PAUSED.swap(false, Ordering::SeqCst) {
                    if let Some(svc) = window.app_handle().try_state::<Arc<App>>() {
                        rebind(window.app_handle(), &svc);
                    }
                }
                if window.label() == "onboarding" {
                    let app = window.app_handle();
                    if let Some(svc) = app.try_state::<Arc<App>>() {
                        leave_onboarding(app, &svc);
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Pointcast")
        .run(|_app, event| match event {
            // The last window closing is not quitting: only Quit (an explicit exit code) ends it.
            tauri::RunEvent::ExitRequested { api, code: None, .. } => api.prevent_exit(),
            // Any way out (the app menu, logout, restart, the updater): what was said is kept for the next launch.
            tauri::RunEvent::Exit => {
                if let Some(svc) = _app.try_state::<Arc<App>>() {
                    svc.flush_for_quit();
                }
            }
            // Clicking the Dock icon (shown while a window is open) brings Settings forward.
            #[cfg(target_os = "macos")]
            tauri::RunEvent::Reopen { .. } => show_window(_app, "settings"),
            _ => {}
        });
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
    fn the_session_chord_over_the_open_overlay_keeps_the_box_then_stops() {
        assert_eq!(session_press(false, false), Press::Start);
        assert_eq!(session_press(true, false), Press::Stop);
        assert_eq!(session_press(true, true), Press::CommitThenStop);
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
    fn a_saved_chord_this_version_refuses_blocks_no_other_change() {
        let p = |v: Value| v.as_object().unwrap().clone();
        let old = Settings { zoom_hotkey: Some("F5".into()), ..Settings::default() };
        let next = merge(&old, p(json!({ "sounds": false }))).unwrap();
        assert!(!next.sounds && next.zoom_hotkey.as_deref() == Some("F5"), "kept as saved");
        assert!(merge(&old, p(json!({ "full_hotkey": "Ctrl+Shift+F8" }))).is_ok());
        let set = merge(&old, p(json!({ "zoom_hotkey": "F6" })));
        assert!(set.unwrap_err().contains("Add Ctrl, Alt or Shift"), "a chord being set is still checked");
        assert!(merge(&old, p(json!({ "zoom_hotkey": null }))).is_ok());
        let garbled = Settings { region_hotkey: Some("Ctrl+Shift+Nope+Nope".into()), ..Settings::default() };
        assert!(merge(&garbled, p(json!({ "model": "auto" }))).is_ok());
    }

    #[test]
    fn the_overlay_frame_says_where_the_person_pointed_unless_marking_is_off() {
        let shot = capture::Shot {
            png: vec![1, 2],
            width: 4,
            height: 3,
            kind: prompt::CaptureKind::Full,
            cursor: (0, 0),
            pointer: Some(pointer::Point { t_ms: 9, x: 3, y: 2 }),
            trail: vec![],
            element: None,
            map: Default::default(),
            k: 1.0,
        };
        let v = frame_payload(&shot, settings::MarkPoint::Ring);
        assert_eq!((v["width"].clone(), v["pointer"].clone()), (json!(4), json!({ "x": 3, "y": 2 })));
        assert!(v["element"].is_null());
        assert!(frame_payload(&shot, settings::MarkPoint::Off)["pointer"].is_null());
        let none = capture::Shot { pointer: None, ..shot.clone() };
        assert!(frame_payload(&none, settings::MarkPoint::Trail)["pointer"].is_null());
        let rect = element::Bounds { x: 1, y: 0, w: 2, h: 2 };
        let button = element::ElementMark { role: "button".into(), name: "Place order".into(), rect };
        let with = capture::Shot { element: Some(button), ..shot };
        let v = frame_payload(&with, settings::MarkPoint::Off);
        assert_eq!(v["element"], json!({ "role": "button", "name": "Place order", "rect": { "x": 1, "y": 0, "w": 2, "h": 2 } }));
        assert!(v["pointer"].is_null(), "the outline does not need the ring");
    }

    #[test]
    fn a_region_commit_takes_the_outlined_control_or_none() {
        let body: RegionCommit = serde_json::from_value(json!({
            "png_b64": "",
            "labels": [{ "n": 1, "text": "too far left" }],
            "element": { "role": "button", "name": "Place order", "rect": { "x": 4, "y": 6, "w": 116, "h": 41 } },
        }))
        .unwrap();
        let e = body.element.unwrap();
        assert_eq!((e.role.as_str(), e.name.as_str(), e.rect.w), ("button", "Place order", 116));
        let old: RegionCommit = serde_json::from_value(json!({ "png_b64": "", "labels": [] })).unwrap();
        assert!(old.element.is_none());
        let gone: RegionCommit = serde_json::from_value(json!({ "png_b64": "", "element": null })).unwrap();
        assert!(gone.element.is_none());
    }

    fn png(w: u32, v: u8) -> Vec<u8> {
        use image::ImageEncoder;
        let mut buf = Vec::new();
        let px: Vec<u8> = (0..w).flat_map(|_| [v, 24, 24, 255]).collect();
        image::codecs::png::PngEncoder::new(&mut buf).write_image(&px, w, 1, image::ExtendedColorType::Rgba8).unwrap();
        buf
    }

    #[test]
    fn a_history_picture_opens_for_editing_and_one_commit_goes_back_into_it() {
        let dir = std::env::temp_dir().join(format!("pc-edit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = history::Store::new(&dir);
        let pic = prompt::Capture {
            id: 0,
            kind: prompt::CaptureKind::Region,
            t_ms: 10,
            png: png(2, 200),
            width: 2,
            height: 1,
            labels: vec![Label { n: 1, text: "old".into() }],
            pointer: None,
            trail: vec![],
            element: None,
            place: None,
            anchors: vec![],
            auto: false,
        };
        let items = vec![
            prompt::Item::Speech { t_ms: 0, end_ms: 9, text: "look".into() },
            prompt::Item::Image { t_ms: 10, capture: pic },
        ];
        let m = store.save(&items, 1_793_000_000_000, 100).unwrap();
        let ed = Editing(std::sync::Mutex::new(None));
        assert!(ed.commit(&store, &png(3, 1), vec![]).is_none(), "not editing: a commit belongs to the recording");
        assert!(ed.begin(&store, "../settings", 1).is_err() && ed.begin(&store, &m.id, 2).is_err());
        assert!(ed.0.lock().unwrap().is_none(), "a refused edit opens nothing");

        let f = ed.begin(&store, &m.id, 1).unwrap();
        assert_eq!((&f["edit"], &f["width"], &f["height"], &f["base"]), (&json!(true), &json!(2), &json!(1), &json!(1)));
        assert!(f["pointer"].is_null() && f["element"].is_null(), "marks already drawn into it are not placed again");
        let b64 = f["png_b64"].as_str().unwrap();
        assert_eq!(base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap(), png(2, 200));
        ed.cancel();
        assert!(ed.commit(&store, &png(3, 1), vec![]).is_none(), "Esc: nothing is written");
        assert_eq!(store.picture(&m.id, 1).unwrap().png, png(2, 200));

        ed.begin(&store, &m.id, 1).unwrap();
        assert!(ed.commit(&store, b"not a png", vec![]).unwrap().is_err());
        assert!(ed.0.lock().unwrap().is_some(), "a failed write keeps the edit open");
        let (id, cap) = ed.commit(&store, &png(3, 1), vec![Label { n: 1, text: "new".into() }]).unwrap().unwrap();
        assert_eq!((id.as_str(), cap.width, cap.labels.len()), (m.id.as_str(), 3, 2));
        assert_eq!(store.picture(&m.id, 1).unwrap().png, png(3, 1));
        assert!(ed.commit(&store, &png(4, 1), vec![]).is_none(), "written once; the next commit is the recording's again");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_patch_sets_the_capture_marks_and_refuses_unknown_ones() {
        let p = |v: Value| v.as_object().unwrap().clone();
        let next = merge(&Settings::default(), p(json!({ "mark_point": "trail", "zoom_target": "window", "zoom_sharpen": false }))).unwrap();
        assert_eq!((next.mark_point, next.zoom_target, next.zoom_sharpen), (settings::MarkPoint::Trail, settings::ZoomTarget::Window, false));
        assert!(merge(&next, p(json!({ "mark_point": "sparkles" }))).is_err());
        let back = merge(&next, p(json!({ "mark_point": null }))).unwrap();
        assert_eq!(back.mark_point, settings::MarkPoint::Ring);
        let quiet = merge(&back, p(json!({ "outline_element": false }))).unwrap();
        assert!(!quiet.outline_element && back.outline_element);
        let mute = merge(&quiet, p(json!({ "spoken_anchors": "off" }))).unwrap();
        assert_eq!((mute.spoken_anchors, quiet.spoken_anchors), (settings::SpokenAnchors::Off, settings::SpokenAnchors::Ring));
        assert!(merge(&mute, p(json!({ "spoken_anchors": "trail" }))).is_err());
    }

    #[test]
    fn only_the_model_a_recording_would_load_counts_as_in_use() {
        let both = ["moonshine-tiny-en", "moonshine-base-en"];
        assert!(in_use("moonshine-tiny-en", "moonshine-base-en", &both, "moonshine-tiny-en"));
        assert!(!in_use("moonshine-tiny-en", "moonshine-base-en", &both, "moonshine-base-en"));
        assert!(in_use("auto", "moonshine-base-en", &both, "moonshine-base-en"));
        assert!(!in_use("auto", "moonshine-base-en", &both, "moonshine-tiny-en"));
        // The chosen one is not downloaded: the engine falls back to what is there.
        assert!(in_use("whisper-turbo", "moonshine-base-en", &["moonshine-tiny-en"], "moonshine-tiny-en"));
    }

    #[test]
    fn a_paste_that_cannot_happen_says_the_prompt_was_only_copied() {
        let tried = std::cell::Cell::new(false);
        let r = paste_or_say(false, || {
            tried.set(true);
            Ok(())
        });
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert!(!tried.get(), "without permission no keys are sent");
        assert_eq!(paste_or_say(true, || Err("refused".into())).unwrap_err(), PASTE_YOURSELF);
        assert!(paste_or_say(true, || Ok(())).is_ok());
    }

    #[test]
    fn history_stays_or_comes_back_to_say_a_paste_did_not_happen() {
        let log = std::cell::RefCell::new(Vec::<&str>::new());
        let step = |s| log.borrow_mut().push(s);
        let pasted = || {
            step("paste");
            Ok(())
        };
        let refused = || {
            step("paste");
            Err("refused".to_string())
        };
        let r = paste_out_of(false, || step("hide"), pasted, || step("back"));
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert!(log.borrow().is_empty(), "without permission to paste History is not hidden");
        let r = paste_out_of(true, || step("hide"), refused, || step("back"));
        assert_eq!(r.unwrap_err(), PASTE_YOURSELF);
        assert_eq!(*log.borrow(), ["hide", "paste", "back"], "a paste the app refused brings History back");
        log.borrow_mut().clear();
        assert!(paste_out_of(true, || step("hide"), pasted, || step("back")).is_ok());
        assert_eq!(*log.borrow(), ["hide", "paste"]);
    }

    #[test]
    fn agent_setup_is_the_one_server_for_user_settings_not_a_whole_project_file() {
        let token = "ab".repeat(32);
        let claude = agent_setup("claude", "127.0.0.1:47821", &token).unwrap();
        assert_eq!(
            claude,
            format!("claude mcp add --transport http --scope user pointcast http://127.0.0.1:47821/mcp --header {q}Authorization: Bearer {token}{q}", q = '"')
        );
        let cursor = agent_setup("cursor", "127.0.0.1:47821", &token).unwrap();
        assert!(cursor.starts_with(r#""pointcast": {"#) && cursor.contains(&token) && cursor.contains("http://127.0.0.1:47821/mcp"));
        assert!(!cursor.contains("mcpServers"), "{cursor}");
        assert!(agent_setup("vim", "127.0.0.1:47821", &token).is_err());
    }

    #[test]
    fn a_saved_chord_this_version_refuses_says_why_not_taken() {
        assert!(refused(&Settings::default()).is_empty());
        let old = Settings { zoom_hotkey: Some("F5".into()), ..Settings::default() };
        let r = refused(&old);
        assert_eq!(r.len(), 1, "{r:?}");
        assert_eq!(r[0].0, "zoom");
        assert!(r[0].1.contains("Add Ctrl, Alt or Shift") && r[0].1.contains(&old.zoom_chord().label()), "{}", r[0].1);
    }

    #[test]
    fn the_clock_reads_minutes_and_seconds() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(65_400), "1:05");
    }
}
