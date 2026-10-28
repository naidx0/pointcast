//! Always-on system tray / menu bar icon — Nova-style short verbs.
use crate::session::{App, Phase};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    // Nova-style short verbs. Accelerators in the label (cross-platform menus rarely paint OS accelerators).
    let rec = MenuItem::with_id(app, "record", "Record\t⌃⇧Space", true, None::<&str>)?;
    let stop = MenuItem::with_id(app, "stop", "Stop", true, None::<&str>)?;
    let paste = MenuItem::with_id(app, "paste", "Paste last prompt", true, None::<&str>)?;
    let last = MenuItem::with_id(app, "last", "Show last prompt", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Pointcast", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&rec, &stop, &paste, &last, &sep1, &settings, &sep2, &quit])?;
    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("Pointcast")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "settings" => crate::show_window(app, "settings"),
            "last" => crate::show_window(app, "result"),
            "paste" => {
                if let Some(svc) = app.try_state::<Arc<App>>() {
                    if let Some(items) = svc.last_export() {
                        if let Ok(mut cb) = arboard::Clipboard::new() {
                            let _ = cb.set_html(&items.html, Some(&items.plain));
                            let _ = crate::clipboard::paste_now();
                        }
                    } else {
                        crate::show_window(app, "result");
                    }
                }
            }
            "record" | "stop" => {
                if let Some(svc) = app.try_state::<Arc<App>>() {
                    let _ = crate::tray_toggle(app, &svc);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });
    // Left-click shows this menu (Nova-style tray). Record / Paste / Settings live here.
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

pub fn set_recording(app: &AppHandle, on: bool) {
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(if on { "Pointcast — recording" } else { "Pointcast" }));
    }
}

pub fn is_recording(phase: Phase) -> bool {
    matches!(phase, Phase::Recording | Phase::Loading)
}
