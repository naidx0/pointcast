//! Always-on system tray / menu bar icon.
use crate::session::{App, Phase};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let last = MenuItem::with_id(app, "last", "Show last prompt", true, None::<&str>)?;
    let rec = MenuItem::with_id(app, "record", "Start / stop recording", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Pointcast", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&rec, &last, &settings, &sep, &quit])?;
    let mut b = TrayIconBuilder::with_id("main")
        .tooltip("Pointcast — idle")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "settings" => crate::show_window(app, "settings"),
            "last" => crate::show_window(app, "result"),
            "record" => {
                if let Some(svc) = app.try_state::<Arc<App>>() {
                    let _ = crate::tray_toggle(app, &svc);
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                crate::show_window(tray.app_handle(), "settings");
            }
        });
    if let Some(icon) = app.default_window_icon() {
        b = b.icon(icon.clone());
    }
    b.build(app)?;
    Ok(())
}

pub fn set_recording(app: &AppHandle, on: bool) {
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(if on {
            "Pointcast — recording"
        } else {
            "Pointcast — idle"
        }));
    }
}

pub fn is_recording(phase: Phase) -> bool {
    matches!(phase, Phase::Recording | Phase::Loading)
}
