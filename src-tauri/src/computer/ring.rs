//! The pointer ring: while an Ask turn holds the computer tools, a small ring is drawn around the pointer so the
//! person can see where the AI is pointing, and it pulses at each click it makes. The window is click-through
//! (`set_ignore_cursor_events`), so it never takes a click meant for the app under the pointer, and it is on the
//! protected list (`platform::PROTECTED`), so the AI's own screenshots are not confused by it.
//!
//! One thread for the app's life: it sleeps while no turn acts, and while one does it follows the pointer at about
//! thirty frames a second. `clicked` is called by `input::click_at` after each synthetic click; it only reaches the
//! ring page (`ring://click`), which plays the pulse.
use std::sync::OnceLock;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

static APP: OnceLock<AppHandle> = OnceLock::new();

const SIZE: i32 = 44;

/// Where the pointer is, in the window's own units: points on macOS, physical pixels on Windows.
fn place(w: &tauri::WebviewWindow, x: i32, y: i32) {
    #[cfg(target_os = "macos")]
    let _ = w.set_position(tauri::LogicalPosition::new(x - SIZE / 2, y - SIZE / 2));
    #[cfg(not(target_os = "macos"))]
    let _ = w.set_position(tauri::PhysicalPosition::new(x - SIZE / 2, y - SIZE / 2));
}

/// Called once at app start: remembers the app for `clicked` and runs the follower.
pub fn watch(app: &AppHandle) {
    let app = app.clone();
    let _ = APP.set(app.clone());
    let _ = std::thread::Builder::new().name("pointcast-ring".into()).spawn(move || {
        let mut shown = false;
        let mut through = false;
        loop {
            if super::desk().granted() {
                if let Some(w) = app.get_webview_window("ring").or_else(|| crate::platform::ensure(&app, "ring")) {
                    if !through {
                        // The ring must never swallow a click: synthetic or the person's.
                        let _ = w.set_ignore_cursor_events(true);
                        through = true;
                    }
                    if let Ok((x, y)) = crate::capture::cursor_pos() {
                        place(&w, x, y);
                    }
                    if !shown {
                        let _ = w.show();
                        shown = true;
                    }
                    std::thread::sleep(Duration::from_millis(33));
                    continue;
                }
                std::thread::sleep(Duration::from_millis(150));
            } else {
                if shown {
                    if let Some(w) = app.get_webview_window("ring") {
                        let _ = w.hide();
                    }
                    shown = false;
                }
                std::thread::sleep(Duration::from_millis(150));
            }
        }
    });
}

/// The AI just clicked: the ring pulses. Safe before `watch` and without the window: the event just goes nowhere.
pub fn clicked() {
    if let Some(app) = APP.get() {
        let _ = app.emit_to("ring", "ring://click", ());
    }
}
