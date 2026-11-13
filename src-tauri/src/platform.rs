//! Where Pointcast's floating windows go and how they sit above other apps, per OS.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Listener, Manager, Monitor, WebviewWindow};

/// Whether the HUD and overlay are kept out of captures; applied to each as it is built.
static PROTECT: AtomicBool = AtomicBool::new(true);

/// A window from its tauri.conf.json entry, built the first time it is needed. Nothing is built
/// at launch, so an idle Pointcast is a tray icon and no web views.
pub fn ensure(app: &AppHandle, label: &str) -> Option<WebviewWindow> {
    ensure_with(app, label, None)
}

/// `ensure`, running `init` in the page before its own scripts when the window is built now.
fn ensure_with(app: &AppHandle, label: &str, init: Option<String>) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(label) {
        return Some(w);
    }
    listen_for_puts(app);
    let cfg = app.config().app.windows.iter().find(|w| w.label == label)?.clone();
    let mut b = tauri::WebviewWindowBuilder::from_config(app, &cfg).ok()?;
    if own_title_bar(label, std::env::consts::OS) {
        // The OS keeps the shadow and, on Windows 11, the rounded corners of a window without its frame.
        b = b.decorations(false).shadow(true);
    }
    if let Some(js) = init {
        b = b.initialization_script(&js);
    }
    let w = b.build().ok()?;
    if protects(label) {
        let _ = w.set_content_protected(PROTECT.load(Ordering::SeqCst));
    }
    if label == PANEL {
        note_panel(&w);
    }
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        icons::apply(windows::Win32::Foundation::HWND(h.0 as _));
    }
    Some(w)
}

/// The windows kept out of captures while that is on: the HUD and the overlay, and never the answer panel.
const PROTECTED: [&str; 2] = ["indicator", "overlay"];

/// The answer panel's window. It shows in every screen share and recording: nothing ever protects it.
pub const PANEL: &str = "live";

/// Whether `label` is one of the windows content protection is applied to.
fn protects(label: &str) -> bool {
    label != PANEL && PROTECTED.contains(&label)
}

/// Tell the capture path which window is the answer panel, by the OS's own name for it, so every picture Pointcast
/// takes paints it out.
fn note_panel(w: &WebviewWindow) {
    #[cfg(windows)]
    if let Ok(h) = w.hwnd() {
        crate::capture::set_panel(Some(h.0 as usize as u64));
    }
    #[cfg(target_os = "macos")]
    {
        let w = w.clone();
        let _ = w.clone().run_on_main_thread(move || {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            if let Ok(ns) = w.ns_window() {
                let ns = ns as *mut AnyObject;
                let n: isize = unsafe { msg_send![ns, windowNumber] };
                if n > 0 {
                    crate::capture::set_panel(Some(n as u64));
                }
            }
        });
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = w;
}

/// Windows: the taskbar, Alt+Tab and Task Manager draw a window's big icon, and Tauri sets only the small one, so they
/// fell back to whatever Windows had cached for Pointcast (the old icon after an update). Both are set from the icon
/// built into the executable, at the sizes the window's display asks for.
#[cfg(windows)]
pub mod icons {
    use std::sync::Mutex;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, SendMessageW, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LR_LOADFROMFILE, SM_CXICON, SM_CXSMICON,
        WM_SETICON,
    };

    /// Where tauri-build puts the app icon in the executable.
    const APP_ICON: u16 = 32512;

    /// The executable's icon drawn at `px`, loaded once per size and kept for the life of the app.
    fn load(px: i32) -> Option<usize> {
        static LOADED: Mutex<Vec<(i32, usize)>> = Mutex::new(Vec::new());
        let mut loaded = LOADED.lock().unwrap();
        if let Some(&(_, h)) = loaded.iter().find(|(s, _)| *s == px) {
            return Some(h);
        }
        let module = unsafe { GetModuleHandleW(PCWSTR::null()) }.ok()?;
        let name = PCWSTR(APP_ICON as usize as *const u16);
        let h = unsafe { LoadImageW(HINSTANCE::from(module), name, IMAGE_ICON, px, px, LR_DEFAULTCOLOR) }.ok()?.0 as usize;
        loaded.push((px, h));
        Some(h)
    }

    /// The (big, small) sizes in pixels for a window: 32 and 16 at 100 %, 48 and 24 at 150 %.
    pub fn sizes(hwnd: HWND) -> (i32, i32) {
        let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
        unsafe { (GetSystemMetricsForDpi(SM_CXICON, dpi), GetSystemMetricsForDpi(SM_CXSMICON, dpi)) }
    }

    /// An .ico file drawn at `px`, for tests: the app icon is linked into the app's executable, not the test one.
    pub fn load_file(path: &std::path::Path, px: i32) -> Option<usize> {
        let wide: Vec<u16> = path.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
        let h = unsafe { LoadImageW(HINSTANCE::default(), PCWSTR(wide.as_ptr()), IMAGE_ICON, px, px, LR_LOADFROMFILE) };
        h.ok().map(|h| h.0 as usize)
    }

    /// Give a window both icons; false when the executable has no icon to give.
    pub fn apply(hwnd: HWND) -> bool {
        apply_with(hwnd, load)
    }

    pub fn apply_with(hwnd: HWND, load: impl Fn(i32) -> Option<usize>) -> bool {
        let (big, small) = sizes(hwnd);
        let (Some(b), Some(s)) = (load(big), load(small)) else {
            return false;
        };
        unsafe {
            SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_BIG as usize), LPARAM(b as isize));
            SendMessageW(hwnd, WM_SETICON, WPARAM(ICON_SMALL as usize), LPARAM(s as isize));
        }
        true
    }
}

/// The windows a person works in (not the HUD or overlay).
pub const DESK: [&str; 3] = ["settings", "onboarding", "result"];

/// Whether the window `label` draws its own slim title bar on `os` (as `std::env::consts::OS` names it): Settings on
/// Windows, built without the system's frame. On macOS its traffic lights sit over the sidebar (`titleBarStyle`
/// "Overlay" with the title hidden, in tauri.conf.json). Its title stays "Pointcast Settings" for the taskbar and
/// Alt+Tab either way.
pub fn own_title_bar(label: &str, os: &str) -> bool {
    matches!(label, "settings" | "onboarding") && os == "windows"
}

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

/// After a while without recording, let the HUD, the overlay and a closed answer panel go; they are rebuilt on demand.
pub fn reap(app: &AppHandle) {
    for label in ["indicator", "overlay", PANEL] {
        if let Some(w) = app.get_webview_window(label) {
            if !w.is_visible().unwrap_or(true) {
                let _ = w.destroy();
                if label == PANEL {
                    crate::capture::set_panel(None);
                }
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

/// The script that hands a freshly built HUD the state it is shown for, so its first frame is that state.
pub fn hud_first(st: &crate::session::UiState) -> String {
    format!("window.__POINTCAST_HUD__ = {};", serde_json::to_string(st).unwrap_or_else(|_| "null".into()))
}

/// Where the person last put the HUD (its top-left) and the answer panel (its top-right: it widens from its left edge),
/// in each OS's own placing units: points on macOS, physical pixels of the one desktop on Windows. Kept while Pointcast
/// runs and across launches, as long as the spot is on a screen; `None` until the person moves the window.
static HUD_HAND: Mutex<Option<(f64, f64)>> = Mutex::new(None);
static PANEL_HAND: Mutex<Option<(f64, f64)>> = Mutex::new(None);

/// The chosen spots, kept across launches as `data_dir/spots.json`.
fn spots_path() -> std::path::PathBuf {
    crate::settings::data_dir().join("spots.json")
}

/// `hud` and `panel` as the file's text: each a `[x, y]` pair under its name; a spot never chosen is left out.
fn spots_json(hud: Option<(f64, f64)>, panel: Option<(f64, f64)>) -> String {
    let mut v = serde_json::Map::new();
    if let Some((x, y)) = hud {
        v.insert("hud".into(), serde_json::json!([x, y]));
    }
    if let Some((x, y)) = panel {
        v.insert("panel".into(), serde_json::json!([x, y]));
    }
    serde_json::to_string(&v).unwrap_or_else(|_| "{}".into())
}

/// The spots `text` saved, each an `[x, y]` pair; anything else reads as never chosen.
fn spots_read(text: &str) -> (Option<(f64, f64)>, Option<(f64, f64)>) {
    let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
    let pt = |k: &str| {
        let a = v.get(k)?.as_array()?;
        Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?))
    };
    (pt("hud"), pt("panel"))
}

/// The spots from the last launch, read once.
fn load_hands() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let Ok(text) = std::fs::read_to_string(spots_path()) else {
            return;
        };
        let (hud, panel) = spots_read(&text);
        if let Some(p) = hud {
            *HUD_HAND.lock().unwrap() = Some(p);
        }
        if let Some(p) = panel {
            *PANEL_HAND.lock().unwrap() = Some(p);
        }
    });
}

/// Write the chosen spots back, so the next launch opens where the person put them.
fn save_hands() {
    let (hud, panel) = (*HUD_HAND.lock().unwrap(), *PANEL_HAND.lock().unwrap());
    let path = spots_path();
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, spots_json(hud, panel)).is_ok() {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// The page says it has come to rest where the person dragged it (api.ts startWindowDrag, `remember`).
const PUT_EVENT: &str = "pointcast://put";

fn listen_for_puts(app: &AppHandle) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let h = app.clone();
        app.listen_any(PUT_EVENT, move |e| {
            if let Ok(label) = serde_json::from_str::<String>(e.payload()) {
                note_put(&h, &label);
            }
        });
    });
}

/// Remember where window `label` is now, as the spot the person chose, for this launch and the next.
fn note_put(app: &AppHandle, label: &str) {
    let Some(w) = app.get_webview_window(label) else { return };
    let (Ok(p), Ok(z)) = (w.outer_position(), w.outer_size()) else { return };
    let k = native_k(w.scale_factor().unwrap_or(1.0));
    let (x, y, wide) = (p.x as f64 / k, p.y as f64 / k, z.width as f64 / k);
    let mut wrote = true;
    match label {
        "indicator" => *HUD_HAND.lock().unwrap() = Some((x, y)),
        PANEL => *PANEL_HAND.lock().unwrap() = Some((x + wide, y)),
        _ => wrote = false,
    }
    if wrote {
        save_hands();
    }
}

/// What a physical pixel is divided by to get the placing unit: the scale on macOS (points), 1 on Windows.
fn native_k(scale: f64) -> f64 {
    if cfg!(target_os = "macos") {
        scale
    } else {
        1.0
    }
}

/// A screen's work area in placing units.
fn native_area(m: &Monitor) -> (f64, f64, f64, f64) {
    let (wa, k) = (m.work_area(), native_k(m.scale_factor()));
    (wa.position.x as f64 / k, wa.position.y as f64 / k, wa.size.width as f64 / k, wa.size.height as f64 / k)
}

/// Put window `w` at (x, y) in placing units.
fn put_native(w: &WebviewWindow, x: f64, y: f64) {
    if cfg!(target_os = "macos") {
        let _ = w.set_position(tauri::LogicalPosition::new(x, y));
    } else {
        let _ = w.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
    }
}

/// The screen (index into `areas`) that holds point `p`, if any.
pub fn screen_holding(p: (f64, f64), areas: &[(f64, f64, f64, f64)]) -> Option<usize> {
    areas.iter().position(|&(x, y, w, h)| p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h)
}

/// A window of `size` with its top-left at `at`, moved only as far as it takes to lie inside `area`.
pub fn kept_inside(at: (f64, f64), size: (f64, f64), area: (f64, f64, f64, f64)) -> (f64, f64) {
    let (x, y, w, h) = area;
    (at.0.min(x + w - size.0).max(x), at.1.min(y + h - size.1).max(y))
}

/// Where the HUD goes when the person put it at `hand` (top-left): there, whole on the screen that holds its centre;
/// `None` when its centre is on no screen (a screen was unplugged), and it goes back to the bottom centre.
pub fn hud_kept(hand: (f64, f64), size: (f64, f64), areas: &[(f64, f64, f64, f64)]) -> Option<(f64, f64)> {
    let i = screen_holding((hand.0 + size.0 / 2.0, hand.1 + size.1 / 2.0), areas)?;
    Some(kept_inside(hand, size, areas[i]))
}

/// Show the recording HUD for state `st` without taking focus from the app being talked about.
pub fn show_indicator(app: &AppHandle, st: &crate::session::UiState) {
    let Some(w) = ensure_with(app, "indicator", Some(hud_first(st))) else {
        return;
    };
    load_hands();
    // Where the person put the HUD, while that is on a screen; else the bottom centre of the pointer's screen.
    let kept = w.outer_size().ok().and_then(|z| {
        let hand = (*HUD_HAND.lock().unwrap())?;
        let k = native_k(w.scale_factor().unwrap_or(1.0));
        let size = (z.width as f64 / k, z.height as f64 / k);
        let areas: Vec<_> = app.available_monitors().ok()?.iter().map(native_area).collect();
        let at = hud_kept(hand, size, &areas);
        if at.is_none() {
            *HUD_HAND.lock().unwrap() = None;
        }
        at
    });
    if let Some((x, y)) = kept {
        put_native(&w, x, y);
    } else if let (Some(m), Ok(size)) = (pointer_monitor(app), w.outer_size()) {
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

/// The answer panel's width and the space kept around it, in logical pixels: wide enough to read an answer in (owner,
/// 2026-11-12: "it should open up bigger"; 2026-11-13: bigger still). The page asks for the same width (live.ts PANEL_W).
pub const PANEL_WIDTH: f64 = 640.0;
pub const PANEL_MARGIN: f64 = 12.0;

/// How tall the panel opens, in logical pixels: its header and the listening row. It grows only as its page asks.
pub const PANEL_START: f64 = 120.0;

/// The height the panel shows at: the height it has while it is on screen; a panel that opens starts small.
pub fn opening_height(on_screen: bool, high: f64) -> f64 {
    if on_screen {
        high
    } else {
        PANEL_START
    }
}

/// Where the answer panel goes, in logical pixels, as (x, y, width, height): docked at the right edge of `area` (the
/// screen's work area in its physical pixels at `scale`) with `margin` around it, and narrower on a screen too small.
pub fn panel_spot(area: (i32, i32, u32, u32), scale: f64, width: f64, margin: f64) -> (f64, f64, f64, f64) {
    let at = |v: f64| v / scale;
    let (x, y, w, h) = (at(area.0 as f64), at(area.1 as f64), at(area.2 as f64), at(area.3 as f64));
    let wide = width.min((w - 2.0 * margin).max(0.0));
    (x + w - wide - margin, y + margin, wide, (h - 2.0 * margin).max(0.0))
}

/// The panel at `high` logical pixels tall, docked as `panel_spot` says: its top and right edges stay where they are and
/// it grows downward, never past the bottom margin of `area`.
pub fn panel_fit(area: (i32, i32, u32, u32), scale: f64, width: f64, margin: f64, high: f64) -> (f64, f64, f64, f64) {
    let (x, y, wide, most) = panel_spot(area, scale, width, margin);
    (x, y, wide, high.max(0.0).min(most))
}

/// Put the panel at `spot` (logical pixels of a screen at `scale`), moving or sizing only what is off by a pixel or more,
/// so a resize this causes finds nothing more to do.
fn place_panel(w: &tauri::Window, spot: (f64, f64, f64, f64), scale: f64) {
    let (x, y, wide, high) = spot;
    let off = |a: f64, b: f64| (a - b).abs() >= 1.0;
    if let Ok(size) = w.inner_size() {
        if off(size.width as f64 / scale, wide) || off(size.height as f64 / scale, high) {
            // As for the HUD: AppKit places windows in points, Windows in physical pixels of one desktop.
            if cfg!(target_os = "macos") {
                let _ = w.set_size(tauri::LogicalSize::new(wide, high));
            } else {
                let _ = w.set_size(tauri::PhysicalSize::new((wide * scale).round() as u32, (high * scale).round() as u32));
            }
        }
    }
    if let Ok(at) = w.outer_position() {
        if off(at.x as f64 / scale, x) || off(at.y as f64 / scale, y) {
            if cfg!(target_os = "macos") {
                let _ = w.set_position(tauri::LogicalPosition::new(x, y));
            } else {
                let _ = w.set_position(tauri::PhysicalPosition::new((x * scale).round() as i32, (y * scale).round() as i32));
            }
        }
    }
}

/// The narrowest and widest the person can drag the panel (live.ts PANEL_MIN_W, PANEL_MAX_W).
pub const PANEL_MIN_WIDTH: f64 = 380.0;
pub const PANEL_MAX_WIDTH: f64 = 900.0;

/// The width the panel keeps: the one its page last set (the person may have dragged it), within the panel's limits;
/// `PANEL_WIDTH` when the window has no width yet.
pub fn panel_width(now: f64) -> f64 {
    if now.is_finite() && now > 0.0 {
        now.clamp(PANEL_MIN_WIDTH, PANEL_MAX_WIDTH)
    } else {
        PANEL_WIDTH
    }
}

/// The panel `high` logical pixels tall where the person put it, `hand` being its top-right corner in logical pixels of
/// the screen `area` (physical pixels at `scale`): its width and height as `panel_fit` gives them, its top and right
/// edges where they were put, moved only as far as it takes to stay whole on that screen.
pub fn panel_kept(area: (i32, i32, u32, u32), scale: f64, width: f64, margin: f64, high: f64, hand: (f64, f64)) -> (f64, f64, f64, f64) {
    let (_, _, wide, high) = panel_fit(area, scale, width, margin, high);
    let at = |v: f64| v / scale;
    let a = (at(area.0 as f64), at(area.1 as f64), at(area.2 as f64), at(area.3 as f64));
    let (x, y) = kept_inside((hand.0 - wide, hand.1), (wide, high), a);
    (x, y, wide, high)
}

/// Where the panel goes at `wide` x `high` logical pixels, and the scale of the screen it goes on: where the person put
/// it while that spot is on a screen (`screens`), else docked at the top right of `dock`.
fn panel_place(screens: &[Monitor], dock: &Monitor, wide: f64, high: f64) -> ((f64, f64, f64, f64), f64) {
    let rect = |m: &Monitor| {
        let wa = m.work_area();
        (wa.position.x, wa.position.y, wa.size.width, wa.size.height)
    };
    load_hands();
    let mut hand = PANEL_HAND.lock().unwrap();
    if let Some((r, t)) = *hand {
        let areas: Vec<_> = screens.iter().map(native_area).collect();
        if let Some(i) = screen_holding((r - 1.0, t + 1.0), &areas) {
            let (m, s) = (&screens[i], screens[i].scale_factor());
            // Placing units to this screen's logical pixels: already points on macOS.
            let k = if cfg!(target_os = "macos") { 1.0 } else { s };
            return (panel_kept(rect(m), s, panel_width(wide), PANEL_MARGIN, high, (r / k, t / k)), s);
        }
        *hand = None;
    }
    let s = dock.scale_factor();
    (panel_fit(rect(dock), s, panel_width(wide), PANEL_MARGIN, high), s)
}

/// The panel's page set a new height: keep its top and right edges where they are (docked at the top right of its
/// screen, or where the person put it). AppKit keeps a window's bottom edge where it was when it is resized, so on a Mac
/// the panel would otherwise grow upward.
pub fn redock_panel(w: &tauri::Window) {
    let Ok(Some(m)) = w.current_monitor() else { return };
    let s = m.scale_factor();
    let (wide, high) = w.inner_size().map(|z| (z.width as f64 / s, z.height as f64 / s)).unwrap_or((0.0, 0.0));
    let screens = w.available_monitors().unwrap_or_default();
    let (spot, s) = panel_place(&screens, &m, wide, high);
    place_panel(w, spot, s);
}

/// Show the answer panel where the person put it, else at the right edge of the pointer's screen, without taking focus
/// from the app being asked about. On screen it keeps the height its page last asked for, within that screen; one that
/// opens starts `PANEL_START` tall.
pub fn show_panel(app: &AppHandle) {
    let Some(w) = ensure(app, PANEL) else {
        return;
    };
    if let Some(m) = pointer_monitor(app) {
        let own = w.scale_factor().unwrap_or(m.scale_factor());
        let (wide, high) = w.inner_size().map(|z| (z.width as f64 / own, z.height as f64 / own)).unwrap_or((0.0, 0.0));
        let high = opening_height(w.is_visible().unwrap_or(false), high);
        let screens = app.available_monitors().unwrap_or_default();
        let (spot, s) = panel_place(&screens, &m, wide, high);
        place_panel(&w.as_ref().window(), spot, s);
    }
    if !cfg!(target_os = "macos") {
        let _ = w.set_always_on_top(true);
    }
    raise(&w, Level::Hud);
    show_quietly(&w);
}

/// Take a window shown by `show_quietly` off screen. On Windows that window was shown with `ShowWindow` directly, so
/// Tauri still records it as hidden and its own `hide()` does nothing (seen 2026-11-11: an empty answer panel stayed
/// on screen after "Not sent"); it is hidden the same way it was shown.
pub fn hide_quietly(w: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        if let Ok(h) = w.hwnd() {
            unsafe {
                let _ = ShowWindow(HWND(h.0 as _), SW_HIDE);
            }
        }
    }
    let _ = w.hide();
}

/// Put a window on screen without making it the one that takes keys.
fn show_quietly(w: &WebviewWindow) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        match w.hwnd() {
            Ok(h) => unsafe {
                let _ = ShowWindow(HWND(h.0 as _), SW_SHOWNOACTIVATE);
            },
            Err(_) => {
                let _ = w.show();
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let w = w.clone();
        let _ = w.clone().run_on_main_thread(move || {
            use objc2::msg_send;
            use objc2::runtime::AnyObject;
            if let Ok(ns) = w.ns_window() {
                let ns = ns as *mut AnyObject;
                let _: () = unsafe { msg_send![ns, orderFrontRegardless] };
            }
        });
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = w.show();
    }
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

#[derive(Clone, Copy)]
enum Step {
    /// Pointcast covers the app in front (target, process id): remember it.
    Remember(Option<(isize, u32)>),
    /// Focus the remembered app and keep it, since another delivery (a stop after a region) may need it too.
    Refocus,
    /// Focus the remembered app and forget it: for the last delivery.
    Restore,
    /// A session began or ended, or History closed: an app remembered before must never get the next paste.
    Forget,
}

/// The remembered app after `step`, and the app to focus then.
fn front_after(old: Option<isize>, step: Step, own_pid: u32) -> (Option<isize>, Option<isize>) {
    match step {
        Step::Remember(now) => (keep_front(old, now, own_pid), None),
        Step::Refocus => (old, old),
        Step::Restore => (None, old),
        Step::Forget => (None, None),
    }
}

fn front_at(step: Step) {
    let focus = {
        let mut f = FRONT.lock().unwrap();
        let (keep, focus) = front_after(*f, step, std::process::id());
        *f = keep;
        focus
    };
    if let Some(t) = focus {
        activate(t);
    }
}

pub fn remember_front() {
    front_at(Step::Remember(front()));
}

pub fn restore_front() {
    front_at(Step::Restore);
}

pub fn refocus_front() {
    front_at(Step::Refocus);
}

pub fn forget_front() {
    front_at(Step::Forget);
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
    for label in PROTECTED.into_iter().filter(|l| protects(l)) {
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

    /// Windows: a window gets the big icon the taskbar, Alt+Tab and Task Manager draw, and the small one, both from the
    /// executable and both sized for its display; until it does, Windows shows whatever it cached for the app.
    #[cfg(windows)]
    #[test]
    fn a_window_gets_the_big_icon_the_taskbar_draws_and_a_small_one_sized_for_its_display() {
        use windows::core::w;
        use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, GetIconInfo, SendMessageW, HICON, HMENU, ICONINFO, ICON_BIG, ICON_SMALL,
            WINDOW_EX_STYLE, WM_GETICON, WS_OVERLAPPEDWINDOW,
        };
        use windows::Win32::Graphics::Gdi::{DeleteObject, GetObjectW, BITMAP};
        let hwnd = unsafe {
            CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!("icon test"), WS_OVERLAPPEDWINDOW, 0, 0, 100, 100,
                HWND::default(), HMENU::default(), HINSTANCE::default(), None)
        }
        .unwrap();
        let get = |kind: u32| unsafe { SendMessageW(hwnd, WM_GETICON, WPARAM(kind as usize), LPARAM(0)) }.0;
        let width = |h: isize| unsafe {
            let mut info = ICONINFO::default();
            GetIconInfo(HICON(h as _), &mut info).unwrap();
            let mut bm = BITMAP::default();
            GetObjectW(info.hbmColor, std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
            let _ = DeleteObject(info.hbmColor);
            let _ = DeleteObject(info.hbmMask);
            bm.bmWidth
        };
        assert_eq!(get(ICON_BIG), 0, "a new window has no big icon of its own");
        let ico = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("icons").join("icon.ico");
        assert!(icons::apply_with(hwnd, |px| icons::load_file(&ico, px)), "the app icon loads");
        let (big, small) = icons::sizes(hwnd);
        assert!(big > small && small >= 16, "{big} {small}");
        assert_eq!((width(get(ICON_BIG)), width(get(ICON_SMALL))), (big, small));
        unsafe {
            let _ = DestroyWindow(hwnd);
        }
    }

    #[test]
    fn the_answer_panel_is_never_protected_from_captures_by_rust_or_by_its_own_page() {
        // `ensure_with` and `protect` apply protection only where `protects` says, and it never says the panel.
        assert!(!protects(PANEL) && !PROTECTED.contains(&PANEL));
        assert!(protects("indicator") && protects("overlay"));
        assert!(!protects("settings") && !protects("result") && !protects("onboarding"));
        // The panel's page cannot ask for it either: no capability that reaches the panel grants that call.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        let mut reach = 0;
        for file in std::fs::read_dir(&dir).unwrap().flatten() {
            let cap: serde_json::Value = serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
            let windows: Vec<&str> = cap["windows"].as_array().unwrap().iter().filter_map(|w| w.as_str()).collect();
            if !windows.iter().any(|w| *w == PANEL || w.contains('*')) {
                continue;
            }
            reach += 1;
            let asks = cap["permissions"].as_array().unwrap().iter().any(|p| p.to_string().contains("set-content-protected"));
            assert!(!asks, "{} lets the panel hide itself", file.path().display());
        }
        assert!(reach > 0, "the panel has a capability, so the check above read something");
        // The panel's own file: the check finds the permission where it is planted, and the panel may size itself.
        let own: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("live.json")).unwrap()).unwrap();
        let grants = |cap: &serde_json::Value, p: &str| cap["permissions"].as_array().unwrap().iter().any(|q| q.to_string().contains(p));
        let mut planted = own.clone();
        planted["permissions"].as_array_mut().unwrap().push("core:window:allow-set-content-protected".into());
        assert!(grants(&planted, "set-content-protected") && !grants(&own, "set-content-protected"));
        assert!(grants(&own, "core:window:allow-set-size") && own["windows"] == serde_json::json!([PANEL]));
        // And the window is declared: built on demand, never at launch.
        let conf: serde_json::Value =
            serde_json::from_slice(&std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json")).unwrap()).unwrap();
        let live = conf["app"]["windows"].as_array().unwrap().iter().find(|w| w["label"] == PANEL).expect("a live window");
        assert_eq!((live["url"].as_str(), live["create"].as_bool(), live["contentProtected"].as_bool()), (Some("live.html"), Some(false), None));
        assert_eq!((live["width"].as_f64(), live["alwaysOnTop"].as_bool(), live["focus"].as_bool()), (Some(PANEL_WIDTH), Some(true), Some(false)));
        assert_eq!(live["height"].as_f64(), Some(PANEL_START), "built at its header and listening row");
        // A little of the desktop shows through: a transparent window over the platform's own material (popover
        // vibrancy on macOS, acrylic on Windows 11), drawn whether or not the panel has focus (it never takes it).
        let effects: Vec<&str> = live["windowEffects"]["effects"].as_array().expect("a material").iter().filter_map(|e| e.as_str()).collect();
        assert_eq!((live["transparent"].as_bool(), effects, live["windowEffects"]["state"].as_str()), (Some(true), vec!["popover", "acrylic"], Some("active")));
    }

    #[test]
    fn the_answer_panel_opens_small_at_the_top_right_and_keeps_the_height_its_page_set_while_on_screen() {
        let near = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].iter().all(|d| d.abs() < 1e-6);
        // About 640 x 120 logical: the header and the listening row, nothing empty below.
        assert_eq!((PANEL_WIDTH, PANEL_START), (640.0, 120.0));
        // The person can drag it wider or narrower; it keeps that width within the panel's limits.
        assert_eq!((panel_width(700.0), panel_width(100.0), panel_width(5000.0), panel_width(0.0), panel_width(f64::NAN)), (700.0, PANEL_MIN_WIDTH, PANEL_MAX_WIDTH, PANEL_WIDTH, PANEL_WIDTH));
        assert_eq!((opening_height(false, 649.0), opening_height(true, 300.0)), (PANEL_START, 300.0));
        // The owner's 1080p screen under its taskbar: docked 12 px in from the top right, 120 tall, not 649.
        let area = (0, 0, 1920, 1040);
        let spot = panel_spot(area, 1.0, PANEL_WIDTH, PANEL_MARGIN);
        assert!(near((spot.0, spot.1, spot.2, 0.0), (1268.0, 12.0, 640.0, 0.0)) && spot.3 >= PANEL_START, "{spot:?}");
        let at = |area, scale| panel_fit(area, scale, PANEL_WIDTH, PANEL_MARGIN, opening_height(false, 649.0));
        assert!(near(at(area, 1.0), (1268.0, 12.0, 640.0, 120.0)), "{:?}", at(area, 1.0));
        // At 150 % and on a Retina screen, in logical pixels of that screen.
        assert!(near(at((0, 0, 2880, 1560), 1.5), (1268.0, 12.0, 640.0, 120.0)), "{:?}", at((0, 0, 2880, 1560), 1.5));
        assert!(near(at((0, 50, 3024, 1888), 2.0), (860.0, 37.0, 640.0, 120.0)));
        // A screen too short for even that: it stays within the margins.
        assert!(near(at((0, 0, 1280, 100), 1.0), (628.0, 12.0, 640.0, 76.0)));
    }

    #[test]
    fn settings_and_onboarding_draw_their_own_title_bar_on_windows_and_keep_the_traffic_lights_on_macos() {
        // On Windows their buttons sit on the Glacier band (owner, 2026-11-12), so the OS frame goes.
        for label in ["settings", "onboarding"] {
            assert!(own_title_bar(label, "windows"), "{label}");
            assert!(!own_title_bar(label, "macos") && !own_title_bar(label, "linux"), "{label}");
        }
        assert!(DESK.iter().chain([PANEL, "indicator", "overlay"].iter()).filter(|l| !matches!(**l, "settings" | "onboarding")).all(|l| !own_title_bar(l, "windows")));
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let conf: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("tauri.conf.json")).unwrap()).unwrap();
        let w = conf["app"]["windows"].as_array().unwrap().iter().find(|w| w["label"] == "settings").expect("a settings window");
        // The taskbar and Alt+Tab still name it; on macOS the frame is kept (no `decorations` here) with the title hidden.
        assert_eq!((w["title"].as_str(), w["resizable"].as_bool(), w["decorations"].as_bool()), (Some("Pointcast Settings"), Some(true), None));
        assert_eq!((w["titleBarStyle"].as_str(), w["hiddenTitle"].as_bool()), (Some("Overlay"), Some(true)));
        // Its page may drag, minimize, maximize, ask whether it is maximized and close its own window.
        let mut granted = vec![];
        for file in std::fs::read_dir(root.join("capabilities")).unwrap().flatten() {
            let cap: serde_json::Value = serde_json::from_slice(&std::fs::read(file.path()).unwrap()).unwrap();
            if cap["windows"].as_array().unwrap().iter().any(|w| w == "settings") {
                granted.extend(cap["permissions"].as_array().unwrap().iter().filter_map(|p| p.as_str().map(String::from)));
            }
        }
        for p in ["start-dragging", "minimize", "toggle-maximize", "close", "is-maximized"] {
            assert!(granted.contains(&format!("core:window:allow-{p}")), "{p}: {granted:?}");
        }
        // The check can say no: a window permission no file grants Settings is not found.
        assert!(!granted.is_empty() && !granted.iter().any(|p| p == "core:window:allow-maximize"), "{granted:?}");
    }

    #[test]
    fn the_answer_panel_docks_at_the_right_edge_of_the_work_area() {
        let at = |area, scale, want: (f64, f64, f64, f64)| {
            let got = panel_spot(area, scale, PANEL_WIDTH, PANEL_MARGIN);
            let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
            assert!(near(got.0, want.0) && near(got.1, want.1) && near(got.2, want.2) && near(got.3, want.3), "{got:?} != {want:?}");
        };
        // 1920x1040 work area under a 40 px top bar: 12 px in from the right, the top and the bottom.
        at((0, 40, 1920, 1040), 1.0, (1268.0, 52.0, 640.0, 1016.0));
        // A second screen to the left of the first.
        at((-1280, 0, 1280, 800), 1.0, (-652.0, 12.0, 640.0, 776.0));
        // A 150 % screen right of a 1920 px one: logical pixels of that screen.
        at((1920, 0, 2560, 1400), 1.5, (1280.0 + 2560.0 / 1.5 - 652.0, 12.0, 640.0, 1400.0 / 1.5 - 24.0));
        // A Retina screen in points.
        at((0, 50, 3024, 1888), 2.0, (860.0, 37.0, 640.0, 920.0));
        // A screen narrower than the panel: it shrinks to fit inside the margins and never leaves the screen.
        at((0, 0, 300, 200), 1.0, (12.0, 12.0, 276.0, 176.0));
    }

    #[test]
    fn a_panel_that_sets_its_height_keeps_its_top_and_right_edges_and_grows_downward_within_the_screen() {
        let near = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].iter().all(|d| d.abs() < 1e-6);
        // A small 1280x720 screen under a 40 px bar; the page caps itself at 60 % of the screen, 432 px.
        let area = (0, 40, 1280, 680);
        let screen = 720.0;
        for high in [120.0, 300.0, 0.6 * screen] {
            let got = panel_fit(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, high);
            assert!(near(got, (628.0, 52.0, 640.0, high)), "{high}: {got:?}");
            assert!((got.0 + got.2 - (1280.0 - PANEL_MARGIN)).abs() < 1e-6, "the right edge stays");
        }
        // Taller than the screen allows: it stops at the bottom margin.
        assert!(near(panel_fit(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, 900.0), (628.0, 52.0, 640.0, 656.0)));
        // A Retina screen in points, at 60 % of its 944-point height.
        let got = panel_fit((0, 50, 3024, 1888), 2.0, PANEL_WIDTH, PANEL_MARGIN, 0.6 * 944.0);
        assert!(near(got, (860.0, 37.0, 640.0, 566.4)), "{got:?}");
        // A screen narrower and shorter than the panel: inside the margins either way.
        assert!(near(panel_fit((0, 0, 300, 200), 1.0, PANEL_WIDTH, PANEL_MARGIN, 0.6 * 200.0), (12.0, 12.0, 276.0, 120.0)));
        assert!(near(panel_fit((0, 0, 300, 200), 1.0, PANEL_WIDTH, PANEL_MARGIN, -5.0), (12.0, 12.0, 276.0, 0.0)));
    }

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

    /// What lib.rs does at each moment, in order; returns the app focused at each step.
    fn walk(steps: &[Step]) -> Vec<Option<isize>> {
        let mut f = None;
        steps
            .iter()
            .map(|s| {
                let (keep, focus) = front_after(f, *s, 4242);
                f = keep;
                focus
            })
            .collect()
    }

    #[test]
    fn a_stop_with_no_region_in_that_session_refocuses_nothing() {
        use Step::*;
        let (excel, chrome) = (Some((7, 1000)), Some((9, 2000)));
        let ours = Some((99, 4242));
        // A session in Excel with a region: it begins, the overlay opens over Excel, the commit and then the stop
        // both go back to Excel, and once the stop delivered it is forgotten.
        assert_eq!(walk(&[Forget, Remember(excel), Refocus, Refocus, Forget]), [None, None, Some(7), Some(7), None]);
        // The next session, in Chrome, marks no region: its stop brings nothing forward, so the paste lands in Chrome.
        assert_eq!(walk(&[Forget, Remember(excel), Refocus, Refocus, Forget, Forget, Refocus]).last(), Some(&None));
        // Discard or leaving practice after a region forgets it too.
        assert_eq!(walk(&[Forget, Remember(chrome), Refocus, Forget, Forget, Refocus]).last(), Some(&None));
        // History opened over Excel and closed without pasting, then opened from Settings: its paste goes nowhere stale.
        assert_eq!(walk(&[Remember(excel), Forget, Remember(ours), Restore]).last(), Some(&None));
        // History's paste goes back to the app it opened over, once.
        assert_eq!(walk(&[Remember(excel), Remember(ours), Restore, Refocus]), [None, None, Some(7), None]);
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

    #[test]
    fn a_hud_the_person_moved_stays_where_it_was_put_unless_that_is_off_every_screen() {
        // Two screens side by side: 1512x944 at (0, 25) and 1920x1055 at (1512, 25), in placing units.
        let screens = [(0.0, 25.0, 1512.0, 944.0), (1512.0, 25.0, 1920.0, 1055.0)];
        let size = (432.0, 64.0);
        // Put near the top of the second screen: it stays exactly there.
        assert_eq!(hud_kept((2000.0, 100.0), size, &screens), Some((2000.0, 100.0)));
        // Put half over the right edge of the second screen: it stays on it, pulled just inside.
        assert_eq!(hud_kept((3100.0, 500.0), size, &screens), Some((3432.0 - 432.0, 500.0)));
        // Its centre on no screen (that screen was unplugged, or above every screen): it goes back to its usual place.
        assert_eq!(hud_kept((4000.0, 500.0), size, &screens), None);
        assert_eq!(hud_kept((100.0, -200.0), size, &screens), None);
        // Its centre on the first screen while it sticks into the second: it stays whole on the first.
        assert_eq!(hud_kept((1200.0, 300.0), size, &screens), Some((1512.0 - 432.0, 300.0)));
        assert_eq!(screen_holding((1512.0, 30.0), &screens), Some(1));
        assert_eq!(screen_holding((1511.9, 30.0), &screens), Some(0));
    }

    #[test]
    fn a_panel_the_person_moved_keeps_its_top_right_corner_as_it_grows_and_stays_whole_on_its_screen() {
        let near = |a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)| [a.0 - b.0, a.1 - b.1, a.2 - b.2, a.3 - b.3].iter().all(|d| d.abs() < 1e-6);
        let area = (0, 40, 1920, 1040);
        // Put with its top-right corner at (900, 200): taller and taller, the corner stays.
        for high in [120.0, 300.0, 600.0] {
            let got = panel_kept(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, high, (900.0, 200.0));
            assert!(near(got, (260.0, 200.0, 640.0, high)), "{got:?}");
        }
        // Wider (the corner grip pulls its left edge): the right edge stays where it was put.
        let got = panel_kept(area, 1.0, 700.0, PANEL_MARGIN, 300.0, (900.0, 200.0));
        assert!(near(got, (200.0, 200.0, 700.0, 300.0)), "{got:?}");
        // Taller than the room below it: it rises only as far as it must to stay whole on the screen.
        let got = panel_kept(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, 1000.0, (900.0, 200.0));
        assert!(near(got, (260.0, 40.0 + 1040.0 - 1000.0, 640.0, 1000.0)), "{got:?}");
        // Put so its left edge would cross the screen's: pulled just inside.
        let got = panel_kept(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, 300.0, (200.0, 200.0));
        assert!(near(got, (0.0, 200.0, 640.0, 300.0)), "{got:?}");
        // A Retina screen: the corner is in points, the area in physical pixels at scale 2.
        let got = panel_kept((0, 50, 3024, 1888), 2.0, PANEL_WIDTH, PANEL_MARGIN, 300.0, (1000.0, 100.0));
        assert!(near(got, (360.0, 100.0, 640.0, 300.0)), "{got:?}");
        // Never moved: docked at the top right as before.
        assert!(near(panel_fit(area, 1.0, PANEL_WIDTH, PANEL_MARGIN, 300.0), (1268.0, 52.0, 640.0, 300.0)));
    }

    #[test]
    fn the_spots_the_person_chose_survive_a_launch_and_a_damaged_file_forgets_nothing_it_never_had() {
        let text = spots_json(Some((100.5, 40.0)), Some((1408.0, 12.0)));
        assert_eq!(spots_read(&text), (Some((100.5, 40.0)), Some((1408.0, 12.0))));
        // A spot never chosen is left out of the file.
        assert_eq!(spots_read(&spots_json(None, Some((12.0, 34.0)))), (None, Some((12.0, 34.0))));
        // Missing keys, wrong shapes and non-JSON each read as never chosen.
        for text in ["{}", "{\"hud\": \"no\"}", "not json", ""] {
            assert_eq!(spots_read(text), (None, None), "{text}");
        }
    }
}
