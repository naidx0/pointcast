mod capture;
mod chord;
mod clipboard;
mod image_ops;
mod mcp;
mod overlay;
mod permissions;
mod privacy;
mod prompt;
mod session;
mod settings;
mod tools;
mod tray;
mod voice;

use mcp::Latest;
use prompt::{Label, SessionExport};
use serde::Deserialize;
use serde_json::{json, Value};
use session::{App, Phase};
use settings::Settings;
use std::sync::Arc;
use tauri::Emitter;
use tauri::{AppHandle, Manager};

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

fn emit_state(app: &AppHandle, svc: &App) {
    let st = svc.ui();
    tray::set_recording(app, matches!(st.phase, Phase::Recording | Phase::Loading));
    let _ = app.emit("session://state", &st);
}

#[tauri::command]
fn settings_get(svc: tauri::State<'_, Arc<App>>) -> Value {
    let s = svc.settings.lock().unwrap().clone();
    json!({
        "session_hotkey": s.session_chord().label(),
        "full_hotkey": s.full_chord().label(),
        "region_hotkey": s.region_chord().label(),
        "zoom_hotkey": s.zoom_chord().label(),
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
        "defaults": {
            "session": crate::chord::default_session().label(),
            "full": crate::chord::default_full().label(),
            "region": crate::chord::default_region().label(),
            "zoom": crate::chord::default_zoom().label(),
        },
        "reserved": crate::chord::reserved().into_iter().map(|(k, why)| json!({"keys": k, "why": why})).collect::<Vec<_>>(),
        "speech": crate::voice::models::view(&crate::voice::models::speech_dir(), &crate::voice::models::catalog()),
        "devices": crate::voice::audio::devices(),
        "permissions": permissions::probe(),
        "recommended": crate::voice::machine::recommend(&crate::voice::machine::measure()),
    })
}

#[tauri::command]
fn settings_set(svc: tauri::State<'_, Arc<App>>, change: Settings) -> Result<Value, String> {
    svc.save_settings(change)?;
    Ok(settings_get(svc))
}

#[tauri::command]
fn session_state(svc: tauri::State<'_, Arc<App>>) -> session::UiState {
    svc.ui()
}

#[tauri::command]
fn session_toggle(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<session::UiState, String> {
    let phase = svc.ui().phase;
    if matches!(phase, Phase::Recording | Phase::Loading) {
        finish(&app, &svc)?;
    } else {
        svc.start()?;
        show_window(&app, "indicator");
        emit_state(&app, &svc);
    }
    Ok(svc.ui())
}

fn finish(app: &AppHandle, svc: &Arc<App>) -> Result<Option<SessionExport>, String> {
    let exp = svc.stop(false)?;
    hide_window(app, "indicator");
    hide_window(app, "overlay");
    if let Some(ref e) = exp {
        if let Some(latest) = app.try_state::<Latest>() {
            latest.set(e.clone());
        }
        show_window(app, "result");
        let _ = app.emit("session://result", e);
    }
    emit_state(app, svc);
    Ok(exp)
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
    Ok(json!({"id": cap.id, "kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
fn capture_zoom(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let cap = svc.capture_zoom()?;
    emit_state(&app, &svc);
    Ok(json!({"kind": cap.kind, "width": cap.width, "height": cap.height}))
}

#[tauri::command]
fn region_begin(app: AppHandle, svc: tauri::State<'_, Arc<App>>) -> Result<Value, String> {
    let shot = svc.begin_region()?;
    let payload = json!({
        "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.png),
        "width": shot.width,
        "height": shot.height,
    });
    let _ = app.emit("overlay://frame", &payload);
    if let Some(w) = app.get_webview_window("overlay") {
        let _ = w.show();
        let _ = w.set_focus();
        let _ = w.set_always_on_top(true);
    }
    Ok(payload)
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
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn tray_toggle(app: &AppHandle, svc: &Arc<App>) -> Result<(), String> {
    let phase = svc.ui().phase;
    if matches!(phase, Phase::Recording | Phase::Loading) {
        finish(app, svc)?;
    } else {
        svc.start()?;
        show_window(app, "indicator");
        emit_state(app, svc);
    }
    Ok(())
}

#[tauri::command]
fn copy_again(svc: tauri::State<'_, Arc<App>>) -> Result<(), String> {
    let items = {
        // Reconstruct from last export's plain is lossy; copy HTML from last export.
        svc.last_export().ok_or("no session yet")?
    };
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let _ = cb.set_html(&items.html, Some(&items.plain));
    Ok(())
}

#[tauri::command]
fn paste_again() -> Result<(), String> {
    crate::clipboard::paste_now()
}

#[tauri::command]
fn download_model(app: AppHandle, id: String) -> Result<(), String> {
    std::thread::spawn(move || {
        let root = crate::voice::models::speech_dir();
        let cat = crate::voice::models::catalog();
        let emit = |p| {
            let _ = app.emit("speech://progress", p);
        };
        if id == "runtime" {
            if let Some(rt) = cat.runtime_for_host() {
                let r = crate::voice::models::install_runtime(&root, rt, &emit);
                let _ = app.emit(
                    "speech://done",
                    json!({ "id": "runtime", "ok": r.is_ok(), "error": r.err() }),
                );
            }
            return;
        }
        if let Some(m) = cat.model(&id) {
            let r = crate::voice::models::install_model(&root, m, &emit);
            let _ = app.emit(
                "speech://done",
                json!({ "id": id, "ok": r.is_ok(), "error": r.err() }),
            );
        }
    });
    Ok(())
}

#[tauri::command]
fn request_permissions() -> permissions::PermissionStatus {
    let _ = permissions::request_screen();
    permissions::probe()
}

/// One-click setup: permissions, then runtime + recommended STT model.
#[tauri::command]
fn setup_run(app: AppHandle) -> Result<(), String> {
    let _ = permissions::request_screen();
    let _ = app.emit(
        "setup://progress",
        json!({ "step": "permissions", "label": "Checking microphone and screen…" }),
    );
    std::thread::spawn(move || {
        let root = crate::voice::models::speech_dir();
        let cat = crate::voice::models::catalog();
        let emit_speech = |p| {
            let _ = app.emit("speech://progress", p);
        };
        if let Some(rt) = cat.runtime_for_host() {
            let _ = app.emit(
                "setup://progress",
                json!({ "step": "runtime", "label": "Downloading speech runtime…" }),
            );
            if let Err(e) = crate::voice::models::install_runtime(&root, rt, &emit_speech) {
                let _ = app.emit(
                    "setup://done",
                    json!({ "ok": false, "error": e, "step": "runtime" }),
                );
                return;
            }
        }
        let rec = crate::voice::machine::recommend(&crate::voice::machine::measure());
        let model_id = if cat.model(rec).is_some() {
            rec.to_string()
        } else {
            "moonshine-tiny-en".into()
        };
        if let Some(m) = cat.model(&model_id) {
            let _ = app.emit(
                "setup://progress",
                json!({
                    "step": "model",
                    "label": format!("Downloading {}…", m.name),
                    "id": m.id,
                }),
            );
            if let Err(e) = crate::voice::models::install_model(&root, m, &emit_speech) {
                let _ = app.emit(
                    "setup://done",
                    json!({ "ok": false, "error": e, "step": "model" }),
                );
                return;
            }
        }
        let _ = app.emit(
            "setup://done",
            json!({ "ok": true, "recommended": model_id }),
        );
        let _ = app.emit("speech://done", json!({ "id": "setup", "ok": true }));
    });
    Ok(())
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
    let items = svc.last_export().ok_or("no session yet")?;
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let _ = cb.set_html(&items.html, Some(&items.plain));
    tools::open(&id)?;
    // Brief pause then paste into the focused app.
    std::thread::sleep(std::time::Duration::from_millis(700));
    crate::clipboard::paste_now()
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

fn register_shortcuts(app: &AppHandle, s: &Settings) -> Result<(), String> {
    use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};
    // Unregister all first so a settings change is clean.
    let _ = app.global_shortcut().unregister_all();
    let parse = |c: &chord::Chord| -> Result<Shortcut, String> {
        let mut mods = Modifiers::empty();
        if c.ctrl {
            mods |= Modifiers::CONTROL;
        }
        if c.alt {
            mods |= Modifiers::ALT;
        }
        if c.shift {
            mods |= Modifiers::SHIFT;
        }
        if c.meta {
            mods |= Modifiers::META;
        }
        let code = match c.key.as_str() {
            "Space" => Code::Space,
            "1" => Code::Digit1,
            "2" => Code::Digit2,
            "3" => Code::Digit3,
            "4" => Code::Digit4,
            "5" => Code::Digit5,
            "6" => Code::Digit6,
            "7" => Code::Digit7,
            "8" => Code::Digit8,
            "9" => Code::Digit9,
            "0" => Code::Digit0,
            "Z" => Code::KeyZ,
            "A" => Code::KeyA,
            "B" => Code::KeyB,
            "C" => Code::KeyC,
            "D" => Code::KeyD,
            "E" => Code::KeyE,
            "F" => Code::KeyF,
            "G" => Code::KeyG,
            "H" => Code::KeyH,
            "I" => Code::KeyI,
            "J" => Code::KeyJ,
            "K" => Code::KeyK,
            "L" => Code::KeyL,
            "M" => Code::KeyM,
            "N" => Code::KeyN,
            "O" => Code::KeyO,
            "P" => Code::KeyP,
            "Q" => Code::KeyQ,
            "R" => Code::KeyR,
            "S" => Code::KeyS,
            "T" => Code::KeyT,
            "U" => Code::KeyU,
            "V" => Code::KeyV,
            "W" => Code::KeyW,
            "X" => Code::KeyX,
            "Y" => Code::KeyY,
            k if k.starts_with('F') => match k {
                "F1" => Code::F1,
                "F2" => Code::F2,
                "F3" => Code::F3,
                "F4" => Code::F4,
                "F5" => Code::F5,
                "F6" => Code::F6,
                "F7" => Code::F7,
                "F8" => Code::F8,
                "F13" => Code::F13,
                _ => return Err(format!("cannot bind {k}")),
            },
            k => return Err(format!("cannot bind {k}")),
        };
        Ok(Shortcut::new(Some(mods), code))
    };
    let session = parse(&s.session_chord())?;
    let full = parse(&s.full_chord())?;
    let region = parse(&s.region_chord())?;
    let zoom = parse(&s.zoom_chord())?;
    let gs = app.global_shortcut();
    gs.on_shortcut(session, |app, _s, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(svc) = app.try_state::<Arc<App>>() {
                let phase = svc.ui().phase;
                if matches!(phase, Phase::Recording | Phase::Loading) {
                    let _ = finish(app, &svc);
                } else {
                    let _ = svc.start();
                    show_window(app, "indicator");
                    emit_state(app, &svc);
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    gs.on_shortcut(full, |app, _s, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(svc) = app.try_state::<Arc<App>>() {
                if matches!(svc.ui().phase, Phase::Recording) {
                    let _ = svc.capture_full();
                    emit_state(app, &svc);
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    gs.on_shortcut(region, |app, _s, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(svc) = app.try_state::<Arc<App>>() {
                if matches!(svc.ui().phase, Phase::Recording) {
                    if let Ok(shot) = svc.begin_region() {
                        let _ = app.emit(
                            "overlay://frame",
                            json!({
                                "png_b64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &shot.png),
                                "width": shot.width,
                                "height": shot.height,
                            }),
                        );
                        show_window(app, "overlay");
                    }
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    gs.on_shortcut(zoom, |app, _s, event| {
        if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
            if let Some(svc) = app.try_state::<Arc<App>>() {
                if matches!(svc.ui().phase, Phase::Recording) {
                    let _ = svc.capture_zoom();
                    emit_state(app, &svc);
                }
            }
        }
    })
    .map_err(|e| e.to_string())?;
    Ok(())
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
            settings_set,
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
            download_model,
            request_permissions,
            setup_run,
            detect_tools,
            open_tool,
            send_to_tool,
            overlay_esc,
            overlay_hint,
            inject_speech,
            parse_chord,
        ])
        .setup(move |app| {
            tray::setup(app.handle())?;
            let handle = app.handle().clone();
            if let Err(e) = register_shortcuts(&handle, &settings) {
                eprintln!("hotkeys: {e}");
            }
            // Tray only at launch.
            for label in ["settings", "indicator", "overlay", "result"] {
                if let Some(w) = app.get_webview_window(label) {
                    let _ = w.hide();
                }
            }
            // Pulse live state to the indicator while recording.
            let handle = app.handle().clone();
            let svc2 = svc.clone();
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(200));
                if matches!(svc2.ui().phase, Phase::Recording | Phase::Loading | Phase::Transcribing)
                {
                    emit_state(&handle, &svc2);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // Settings / result close to tray, never quit.
                if window.label() == "settings" || window.label() == "result" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Pointcast");
}
