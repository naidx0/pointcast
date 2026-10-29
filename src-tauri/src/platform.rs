//! Where Pointcast's floating windows go and how they sit above other apps, per OS.
use tauri::{AppHandle, Manager, Monitor, WebviewWindow};

/// The monitor under the pointer, else the primary one.
pub fn pointer_monitor(app: &AppHandle) -> Option<Monitor> {
    app.cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// Bottom centre of the pointer's screen, clear of the Dock or taskbar.
pub fn hud_origin(area: (i32, i32, u32, u32), size: (u32, u32), gap: u32) -> (i32, i32) {
    let (x, y, w, h) = area;
    let (sw, sh) = size;
    (
        x + (w.saturating_sub(sw) / 2) as i32,
        y + h.saturating_sub(sh + gap) as i32,
    )
}

/// Show the recording HUD without taking focus from the app being talked about.
pub fn show_indicator(app: &AppHandle) {
    let Some(w) = app.get_webview_window("indicator") else {
        return;
    };
    if let (Some(m), Ok(size)) = (pointer_monitor(app), w.outer_size()) {
        let wa = m.work_area();
        let s = m.scale_factor();
        let (x, y) = hud_origin(
            (wa.position.x, wa.position.y, wa.size.width, wa.size.height),
            (size.width, size.height),
            (28.0 * s) as u32,
        );
        let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
    }
    let _ = w.set_always_on_top(true);
    let _ = w.show();
    raise(&w, Level::Hud);
}

/// Cover the screen the frame was taken from, above the menu bar and Dock, then take keys.
pub fn show_overlay(app: &AppHandle) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    if let Some(m) = pointer_monitor(app) {
        place(&w, &m);
    }
    let _ = w.set_always_on_top(true);
    let _ = w.show();
    raise(&w, Level::Overlay);
    let _ = w.set_focus();
}

fn place(w: &WebviewWindow, m: &Monitor) {
    let pos = *m.position();
    let size = *m.size();
    if cfg!(target_os = "macos") {
        // AppKit works in points; converting with the monitor's own scale is exact on every screen.
        let s = m.scale_factor();
        let _ = w.set_size(size.to_logical::<f64>(s));
        let _ = w.set_position(pos.to_logical::<f64>(s));
    } else {
        let _ = w.set_size(size);
        let _ = w.set_position(pos);
    }
}

/// Keep the recording HUD and the annotate overlay out of every screenshot (unless the user wants
/// them in). Only those two: on Windows a protected window is also blank in the taskbar preview,
/// Alt+Tab and screen sharing, which ordinary windows like Settings must not be.
pub fn protect(app: &AppHandle, on: bool) {
    for label in ["indicator", "overlay"] {
        if let Some(w) = app.get_webview_window(label) {
            let _ = w.set_content_protected(on);
        }
    }
}

pub enum Level {
    Hud,
    Overlay,
}

/// macOS: float over full-screen apps on every Space; the overlay also covers the menu bar.
#[allow(unused_variables)]
fn raise(w: &WebviewWindow, level: Level) {
    #[cfg(target_os = "macos")]
    {
        let w = w.clone();
        let _ = w.clone().run_on_main_thread(move || {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            let Ok(ns) = w.ns_window() else {
                return;
            };
            let ns = ns as *mut AnyObject;
            // NSStatusWindowLevel sits above the menu bar; NSPopUpMenuWindowLevel above the Dock too.
            let lvl: isize = match level {
                Level::Hud => 25,
                Level::Overlay => 101,
            };
            // CanJoinAllSpaces | Stationary | IgnoresCycle | FullScreenAuxiliary
            let join: usize = (1 << 0) | (1 << 4) | (1 << 6) | (1 << 8);
            unsafe {
                let _: () = msg_send![ns, setLevel: lvl];
                let cur: usize = msg_send![ns, collectionBehavior];
                let _: () = msg_send![ns, setCollectionBehavior: cur | join];
            }
        });
    }
}

/// Whether the window materials in tauri.conf.json really draw: always on macOS 12+, only from
/// Windows 11 (build 22000) on Windows. Without them the UI paints opaque surfaces.
pub fn materials() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
        let build = *BUILD.get_or_init(|| {
            let out = std::process::Command::new("reg")
                .args([
                    "query",
                    r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion",
                    "/v",
                    "CurrentBuildNumber",
                ])
                .creation_flags(0x0800_0000)
                .output();
            out.ok()
                .and_then(|o| {
                    String::from_utf8_lossy(&o.stdout)
                        .split_whitespace()
                        .last()
                        .and_then(|n| n.parse().ok())
                })
                .unwrap_or(0)
        });
        build >= 22000
    }
    #[cfg(not(windows))]
    {
        cfg!(target_os = "macos")
    }
}

/// A short native cue when a session starts or stops.
pub fn chime(start: bool) {
    #[cfg(target_os = "macos")]
    {
        let name = if start { "Tink" } else { "Pop" };
        let _ = std::process::Command::new("afplay")
            .args(["-v", "0.35", &format!("/System/Library/Sounds/{name}.aiff")])
            .spawn();
    }
    #[cfg(windows)]
    {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT};
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:/Windows".into());
        let file = if start { "Speech On.wav" } else { "Speech Off.wav" };
        let path = std::path::Path::new(&windir).join("Media").join(file);
        if path.is_file() {
            unsafe {
                let _ = PlaySoundW(
                    &HSTRING::from(path.as_os_str()),
                    HMODULE::default(),
                    SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
                );
            }
        }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hud_sits_bottom_centre_inside_the_work_area() {
        // 1920x1040 work area under a 40 px top bar, a 300x52 HUD, 28 px off the bottom.
        assert_eq!(hud_origin((0, 40, 1920, 1040), (300, 52), 28), (810, 1000));
        // A second screen to the left of the first.
        assert_eq!(hud_origin((-1280, 0, 1280, 800), (300, 52), 28), (-790, 720));
        // A HUD wider than the screen pins to its left edge instead of going negative.
        assert_eq!(hud_origin((0, 0, 200, 100), (300, 52), 28), (0, 20));
    }
}
