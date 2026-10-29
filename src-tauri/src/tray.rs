//! Menu bar extra on macOS (template glyph, elapsed time beside it while recording), tray icon on
//! Windows. The menu names the user's real shortcuts and follows the session state.
use crate::session::{App, Phase, UiState};
use std::sync::{Arc, Mutex};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};

pub struct Tray {
    record: MenuItem<Wry>,
    full: MenuItem<Wry>,
    region: MenuItem<Wry>,
    zoom: MenuItem<Wry>,
    paste: MenuItem<Wry>,
    update: MenuItem<Wry>,
    shown: Mutex<Option<bool>>,
}

fn glyph(recording: bool) -> Option<Image<'static>> {
    let bytes: &'static [u8] = match (cfg!(target_os = "macos"), recording) {
        (true, false) => include_bytes!("../icons/tray/mac.png"),
        (true, true) => include_bytes!("../icons/tray/mac-rec.png"),
        (false, false) => include_bytes!("../icons/tray/win.png"),
        (false, true) => include_bytes!("../icons/tray/win-rec.png"),
    };
    Image::from_bytes(bytes).ok()
}

/// A menu item that shows its shortcut; falls back to plain text if the OS cannot draw it.
fn item(app: &AppHandle, id: &str, text: &str, accel: Option<String>) -> tauri::Result<MenuItem<Wry>> {
    match accel {
        Some(a) => MenuItem::with_id(app, id, text, true, Some(a.as_str()))
            .or_else(|_| MenuItem::with_id(app, id, text, true, None::<&str>)),
        None => MenuItem::with_id(app, id, text, true, None::<&str>),
    }
}

pub fn setup(app: &AppHandle, svc: &Arc<App>) -> tauri::Result<()> {
    let s = svc.settings.lock().unwrap().clone();
    let record = item(app, "record", "Start Recording", Some(s.session_chord().accelerator()))?;
    let full = item(app, "full", "Capture Screen", Some(s.full_chord().accelerator()))?;
    let region = item(app, "region", "Capture Region…", Some(s.region_chord().accelerator()))?;
    let zoom = item(app, "zoom", "Capture Around Pointer", Some(s.zoom_chord().accelerator()))?;
    let paste = item(app, "paste", "Paste Last Prompt", None)?;
    let last = item(app, "last", "History…", None)?;
    let settings = item(app, "settings", "Settings…", Some("CmdOrCtrl+,".into()))?;
    let setup = item(app, "setup", "Setup Assistant…", None)?;
    let update = item(app, "update", CHECK, None)?;
    let quit = item(app, "quit", "Quit Pointcast", Some("CmdOrCtrl+Q".into()))?;
    let sep = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(
        app,
        &[
            &record, &sep()?, &full, &region, &zoom, &sep()?, &paste, &last, &sep()?, &settings, &setup,
            &update, &sep()?, &quit,
        ],
    )?;
    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("Pointcast")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| {
            let Some(svc) = app.try_state::<Arc<App>>() else {
                return;
            };
            let svc = svc.inner().clone();
            match ev.id().as_ref() {
                "record" => {
                    let _ = crate::toggle(app, &svc);
                }
                "full" => crate::hotkey(app, crate::Action::Full, true),
                "region" => crate::hotkey(app, crate::Action::Region, true),
                "zoom" => crate::hotkey(app, crate::Action::Zoom, true),
                "paste" => {
                    if crate::deliver_last(&svc).is_err() {
                        crate::show_window(app, "result");
                    }
                }
                "last" => crate::show_window(app, "result"),
                "settings" => crate::show_window(app, "settings"),
                "setup" => crate::show_window(app, "onboarding"),
                // Open Settings, then check: it says what an update does before anything is replaced, and reads the
                // answer from settings_view if it arrives before the window listens.
                "update" => {
                    crate::show_window(app, "settings");
                    let _ = app.emit_to("settings", "settings://pane", "general");
                    let app = app.clone();
                    std::thread::spawn(move || {
                        match crate::update::check() {
                            Ok(v) => {
                                offer_update(&app, v.as_deref());
                                let _ = app.emit("update://checked", &v);
                            }
                            Err(e) => {
                                let _ = app.emit("update://failed", e);
                            }
                        }
                    });
                }
                "quit" => crate::quit(app),
                _ => {}
            }
        });
    if let Some(icon) = glyph(false) {
        b = b.icon(icon).icon_as_template(cfg!(target_os = "macos"));
    } else if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    app.manage(Tray {
        record,
        full,
        region,
        zoom,
        paste,
        update,
        shown: Mutex::new(None),
    });
    refresh(app, &svc.ui(), svc.last_export().is_some());
    Ok(())
}

/// Follow the session: menu wording, icon and (macOS) the elapsed time in the menu bar.
pub fn refresh(app: &AppHandle, st: &UiState, has_last: bool) {
    let (Some(tray), Some(t)) = (app.try_state::<Tray>(), app.tray_by_id("main")) else {
        return;
    };
    let rec = matches!(st.phase, Phase::Recording | Phase::Loading);
    let _ = tray.paste.set_enabled(has_last);
    #[cfg(target_os = "macos")]
    {
        let _ = t.set_title(rec.then(|| crate::clock(st.duration_ms)));
    }
    // Decide under the lock, update outside it: off the main thread each setter waits for the main
    // thread, which may itself be waiting for this lock in another refresh.
    if !flip(&tray.shown, rec) {
        return;
    }
    let _ = tray
        .record
        .set_text(if rec { "Stop Recording" } else { "Start Recording" });
    let _ = t.set_tooltip(Some(if rec { "Pointcast — recording" } else { "Pointcast" }));
    if let Some(icon) = glyph(rec) {
        let _ = t.set_icon(Some(icon));
        let _ = t.set_icon_as_template(cfg!(target_os = "macos"));
    }
}

/// Record the new state; true only for the caller that changed it.
fn flip(shown: &Mutex<Option<bool>>, rec: bool) -> bool {
    let mut s = shown.lock().unwrap();
    if *s == Some(rec) {
        return false;
    }
    *s = Some(rec);
    true
}

const CHECK: &str = "Check for Updates…";

/// Name the newer version in the menu once a check finds one.
pub fn offer_update(app: &AppHandle, version: Option<&str>) {
    if let Some(tray) = app.try_state::<Tray>() {
        let text = version.map_or_else(|| CHECK.to_string(), |v| format!("Update to Pointcast {v}…"));
        let _ = tray.update.set_text(text);
    }
}

/// Rebuild the shortcut hints after the user changes a chord.
pub fn rekey(app: &AppHandle, svc: &Arc<App>) {
    let Some(tray) = app.try_state::<Tray>() else {
        return;
    };
    let s = svc.settings.lock().unwrap().clone();
    let pairs = [
        (&tray.record, s.session_chord()),
        (&tray.full, s.full_chord()),
        (&tray.region, s.region_chord()),
        (&tray.zoom, s.zoom_chord()),
    ];
    for (item, chord) in pairs {
        let _ = item.set_accelerator(Some(chord.accelerator().as_str()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_change_of_state_updates_the_tray_and_the_lock_is_free_after() {
        let shown = Mutex::new(None);
        assert!(flip(&shown, true));
        assert!(!flip(&shown, true));
        assert!(flip(&shown, false));
        assert!(shown.try_lock().is_ok(), "nothing holds the lock while the tray is updated");
    }
}
