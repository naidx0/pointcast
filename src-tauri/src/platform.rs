//! Where Pointcast's floating windows go and how they sit above other apps, per OS.
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Manager, Monitor, WebviewWindow};

/// Whether the HUD and overlay are kept out of captures; applied to each as it is built.
static PROTECT: AtomicBool = AtomicBool::new(true);

/// A window from its tauri.conf.json entry, built the first time it is needed. Nothing is built
/// at launch, so an idle Pointcast is a tray icon and no web views.
pub fn ensure(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    let cfg = app.config().app.windows.iter().find(|w| w.label == label)?.clone();
    let w = tauri::WebviewWindowBuilder::from_config(app, &cfg).ok()?.build().ok()?;
    if matches!(label, "indicator" | "overlay") {
        let _ = w.set_content_protected(PROTECT.load(Ordering::SeqCst));
    }
    Some(w)
}

/// The windows a person works in (not the HUD or overlay).
pub const DESK: [&str; 3] = ["settings", "onboarding", "result"];

/// macOS: a menu bar app has no Dock icon, so its windows are out of reach of Cmd+Tab. While
/// Settings, setup or History is open Pointcast is a regular app; then it goes back to the menu
/// bar. `closing` is a window on its way out that still reports itself visible.
#[allow(unused_variables)]
pub fn dock(app: &AppHandle, closing: Option<&str>) {
    #[cfg(target_os = "macos")]
    {
        let open = DESK.iter().any(|l| {
            Some(*l) != closing && app.get_webview_window(l).is_some_and(|w| w.is_visible().unwrap_or(false))
        });
        let _ = app.set_activation_policy(if open {
            tauri::ActivationPolicy::Regular
        } else {
            tauri::ActivationPolicy::Accessory
        });
    }
}

/// After a while without recording, let the HUD and overlay go; they are rebuilt on demand.
pub fn reap(app: &AppHandle) {
    for label in ["indicator", "overlay"] {
        if let Some(w) = app.get_webview_window(label) {
            if !w.is_visible().unwrap_or(true) {
                let _ = w.destroy();
            }
        }
    }
}

/// The monitor under the pointer, else the primary one.
pub fn pointer_monitor(app: &AppHandle) -> Option<Monitor> {
    #[cfg(target_os = "macos")]
    let at = crate::capture::cursor_pos().ok().map(|(x, y)| (x as f64, y as f64));
    #[cfg(not(target_os = "macos"))]
    let at = app.cursor_position().ok().map(|p| (p.x, p.y));
    at.and_then(|(x, y)| app.monitor_from_point(x, y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten())
}

/// Bottom centre of the pointer's screen, clear of the Dock or taskbar.
pub fn hud_origin(area: (f64, f64, f64, f64), size: (f64, f64), gap: f64) -> (f64, f64) {
    let (x, y, w, h) = area;
    let (sw, sh) = size;
    (x + ((w - sw) / 2.0).max(0.0), y + (h - sh - gap).max(0.0))
}

/// Where the HUD goes in logical pixels: `area` is the target screen's work area in its physical pixels at `scale`,
/// `size` the HUD's physical size at `hud_scale`, the scale of the screen it is on now.
pub fn hud_spot(area: (i32, i32, u32, u32), scale: f64, size: (u32, u32), hud_scale: f64) -> (f64, f64) {
    let at = |v: f64| v / scale;
    hud_origin(
        (at(area.0 as f64), at(area.1 as f64), at(area.2 as f64), at(area.3 as f64)),
        (size.0 as f64 / hud_scale, size.1 as f64 / hud_scale),
        28.0,
    )
}

/// Show the recording HUD without taking focus from the app being talked about.
pub fn show_indicator(app: &AppHandle) {
    let Some(w) = ensure(app, "indicator") else {
        return;
    };
    if let (Some(m), Ok(size)) = (pointer_monitor(app), w.outer_size()) {
        let wa = m.work_area();
        let s = m.scale_factor();
        let (x, y) = hud_spot(
            (wa.position.x, wa.position.y, wa.size.width, wa.size.height),
            s,
            (size.width, size.height),
            w.scale_factor().unwrap_or(s),
        );
        // AppKit places windows in points; Windows in physical pixels of one desktop, whatever each screen's scale.
        if cfg!(target_os = "macos") {
            let _ = w.set_position(tauri::LogicalPosition::new(x, y));
        } else {
            let _ = w.set_position(tauri::PhysicalPosition::new((x * s).round() as i32, (y * s).round() as i32));
        }
    }
    if !cfg!(target_os = "macos") {
        let _ = w.set_always_on_top(true);
    }
    raise(&w, Level::Hud);
    let _ = w.show();
}

/// Cover the screen the frame was taken from, above the menu bar and Dock, then take keys.
pub fn show_overlay(app: &AppHandle) {
    let Some(w) = ensure(app, "overlay") else {
        return;
    };
    let m = pointer_monitor(app);
    if let Some(m) = &m {
        place(&w, m);
    }
    if !cfg!(target_os = "macos") {
        let _ = w.set_always_on_top(true);
    }
    raise(&w, Level::Overlay);
    let _ = w.show();
    if let Some(m) = m.as_ref().filter(|_| !cfg!(target_os = "macos")) {
        place(&w, m);
    }
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
        let _ = w.set_position(pos);
        let _ = w.set_size(size);
    }
}

/// The app the person was in before the overlay took the keyboard (a window on Windows, a
/// process on macOS), so focus goes back there afterwards and auto-paste lands in it.
static FRONT: std::sync::Mutex<Option<isize>> = std::sync::Mutex::new(None);

/// What to remember when `now` (target, process id) is in front: Pointcast's own windows never
/// replace the app the person came from, and nothing in front keeps what was there.
fn keep_front(old: Option<isize>, now: Option<(isize, u32)>, own_pid: u32) -> Option<isize> {
    match now {
        Some((t, pid)) if pid != own_pid => Some(t),
        _ => old,
    }
}

pub fn remember_front() {
    let mut f = FRONT.lock().unwrap();
    *f = keep_front(*f, front(), std::process::id());
}

/// Focus the remembered app and forget it: for the last delivery, so a later one never lands in a stale app.
pub fn restore_front() {
    if let Some(t) = FRONT.lock().unwrap().take() {
        activate(t);
    }
}

/// Focus the remembered app and keep it, since another delivery (a stop after a region) may need it too.
pub fn refocus_front() {
    let t = *FRONT.lock().unwrap();
    if let Some(t) = t {
        activate(t);
    }
}

#[cfg(windows)]
fn front() -> Option<(isize, u32)> {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    let h = unsafe { GetForegroundWindow() };
    if h.0.is_null() {
        return None;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    Some((h.0 as isize, pid))
}

#[cfg(windows)]
fn activate(t: isize) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    unsafe {
        let _ = SetForegroundWindow(HWND(t as *mut std::ffi::c_void));
    }
}

#[cfg(target_os = "macos")]
fn front() -> Option<(isize, u32)> {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let ws: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: *mut AnyObject = msg_send![ws, frontmostApplication];
        if app.is_null() {
            return None;
        }
        let pid: i32 = msg_send![app, processIdentifier];
        Some((pid as isize, pid as u32))
    }
}

#[cfg(target_os = "macos")]
fn activate(t: isize) {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let app: *mut AnyObject =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: t as i32];
        if !app.is_null() {
            // NSApplicationActivateIgnoringOtherApps
            let _: bool = msg_send![app, activateWithOptions: 2usize];
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn front() -> Option<(isize, u32)> {
    None
}

#[cfg(not(any(windows, target_os = "macos")))]
fn activate(_: isize) {}

/// Keep the recording HUD and the annotate overlay out of every screenshot (unless the user wants
/// them in). Only those two: on Windows a protected window is also blank in the taskbar preview,
/// Alt+Tab and screen sharing, which ordinary windows like Settings must not be.
pub fn protect(app: &AppHandle, on: bool) {
    PROTECT.store(on, Ordering::SeqCst);
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
    fn pointcast_in_front_keeps_the_app_it_came_from() {
        const OURS: u32 = 4242;
        assert_eq!(keep_front(Some(7), Some((99, OURS)), OURS), Some(7));
        assert_eq!(keep_front(None, Some((99, OURS)), OURS), None);
        assert_eq!(keep_front(Some(7), Some((99, 1000)), OURS), Some(99));
        assert_eq!(keep_front(None, Some((99, 1000)), OURS), Some(99));
        assert_eq!(keep_front(Some(7), None, OURS), Some(7));
        assert_eq!(keep_front(None, None, OURS), None);
    }

    #[test]
    fn the_hud_sits_bottom_centre_inside_the_work_area() {
        // 1920x1040 work area under a 40 px top bar, a 300x52 HUD, 28 px off the bottom.
        assert_eq!(hud_origin((0.0, 40.0, 1920.0, 1040.0), (300.0, 52.0), 28.0), (810.0, 1000.0));
        // A second screen to the left of the first.
        assert_eq!(hud_origin((-1280.0, 0.0, 1280.0, 800.0), (300.0, 52.0), 28.0), (-790.0, 720.0));
        // A HUD wider than the screen pins to its left edge instead of going negative.
        assert_eq!(hud_origin((0.0, 0.0, 200.0, 100.0), (300.0, 52.0), 28.0), (0.0, 20.0));
    }

    #[test]
    fn the_hud_lands_on_the_target_screen_whatever_scale_it_comes_from() {
        let at = |area, scale, size, hud_scale, want: (f64, f64)| {
            let got = hud_spot(area, scale, size, hud_scale);
            assert!((got.0 - want.0).abs() < 1e-6 && (got.1 - want.1).abs() < 1e-6, "{got:?} != {want:?}");
        };
        // Same screen, same scale: the old physical answer, in logical pixels.
        at((0, 40, 1920, 1040), 1.0, (300, 52), 1.0, (810.0, 1000.0));
        at((0, 0, 2560, 1400), 1.5, (450, 78), 1.5, (1055.0 / 1.5, 1280.0 / 1.5));
        // macOS: from a Retina laptop (scale 2, 392x64 points) to a 1x screen right of it at 1512 points.
        at((1512, 25, 1920, 1055), 1.0, (784, 128), 2.0, (2276.0, 988.0));
        // And back: the 1x HUD to the Retina screen, whose work area is (0, 25) 1512x944 points.
        at((0, 50, 3024, 1888), 2.0, (392, 64), 1.0, (560.0, 877.0));
        // Windows: a 100 % HUD to a 150 % screen keeps its logical size there.
        at((1920, 0, 2560, 1400), 1.5, (300, 52), 1.0, (1280.0 + 703.0 + 1.0 / 3.0, 853.0 + 1.0 / 3.0));
    }
}
