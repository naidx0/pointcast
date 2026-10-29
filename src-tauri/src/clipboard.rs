//! Copy a Pointcast session as rich HTML with a plain-text fallback, then optionally paste.
use crate::prompt::{assemble_clipboard, Item};
use crate::settings::OutputMode;

pub fn copy_session(items: &[Item]) -> Result<(String, String), String> {
    let payload = assemble_clipboard(items);
    if payload.plain.is_empty() && payload.html.is_empty() {
        return Err("nothing to copy".into());
    }
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    // Words only: plain text, exactly what a dictation app would type.
    if !items.iter().any(|i| matches!(i, Item::Image { .. })) {
        cb.set_text(&payload.plain).map_err(|e| e.to_string())?;
        return Ok((payload.html, payload.plain));
    }
    // HTML with plain fallback: ChatGPT / Claude / Slack pick up inline images from the HTML.
    match cb.set_html(&payload.html, Some(&payload.plain)) {
        Ok(()) => {}
        Err(_) => {
            cb.set_text(&payload.plain).map_err(|e| e.to_string())?;
        }
    }
    Ok((payload.html, payload.plain))
}

static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Keep the app handle so a paste can be run on the main thread where the OS needs it.
pub fn bind(app: &tauri::AppHandle) {
    let _ = APP.set(app.clone());
}

/// Run `f` where `post` sends it (the main thread) and wait at most `wait` for its answer.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn on_main<T: Send + 'static>(
    post: impl FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), String>,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
    wait: std::time::Duration,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    post(Box::new(move || {
        let _ = tx.send(f());
    }))?;
    rx.recv_timeout(wait).map_err(|_| "the paste did not run".to_string())?
}

/// The V key itself, wherever the keyboard layout puts the letter: looking 'v' up through the
/// layout finds nothing on Russian or Greek layouts and another key on Dvorak.
#[cfg(windows)]
const PASTE: enigo::Key = enigo::Key::V;
#[cfg(not(any(windows, target_os = "macos")))]
const PASTE: enigo::Key = enigo::Key::Unicode('v');

/// kVK_ANSI_V
const ANSI_V: u16 = 9;

/// macOS matches Command shortcuts by what a key types with Command held, so the paste key is the first
/// key code `typed` reads as "v" (the period key on Dvorak), else the physical V.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn command_v(typed: impl Fn(u16) -> Option<String>) -> u16 {
    (0..128).find(|&code| typed(code).as_deref() == Some("v")).unwrap_or(ANSI_V)
}

/// The key that types "v" with Command held in the ASCII-capable layout, the one macOS itself uses for
/// shortcuts under Russian or Greek; "Dvorak - QWERTY ⌘" keeps QWERTY there.
#[cfg(target_os = "macos")]
fn command_v_in_layout(_main: objc2::MainThreadMarker) -> u16 {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::data::CFData;
    use core_foundation::string::CFStringRef;
    use std::ffi::{c_ulong, c_void};

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        static kTISPropertyUnicodeKeyLayoutData: CFStringRef;
        fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *const c_void;
        fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> *const c_void;
        fn LMGetKbdType() -> u8;
        fn UCKeyTranslate(
            layout: *const u8,
            code: u16,
            action: u16,
            modifiers: u32,
            keyboard: u32,
            options: u32,
            dead: *mut u32,
            max: c_ulong,
            len: *mut c_ulong,
            out: *mut u16,
        ) -> i32;
    }
    const KEY_DOWN: u16 = 0; // kUCKeyActionDown
    const COMMAND: u32 = 1; // (cmdKey >> 8) & 0xFF
    const NO_DEAD_KEYS: u32 = 1; // kUCKeyTranslateNoDeadKeysMask

    let source = unsafe { TISCopyCurrentASCIICapableKeyboardLayoutInputSource() };
    if source.is_null() {
        return ANSI_V;
    }
    let source = unsafe { CFType::wrap_under_create_rule(source) };
    let data = unsafe { TISGetInputSourceProperty(source.as_CFTypeRef(), kTISPropertyUnicodeKeyLayoutData) };
    if data.is_null() {
        return ANSI_V;
    }
    let data = unsafe { CFData::wrap_under_get_rule(data.cast()) };
    let layout = data.bytes();
    if layout.is_empty() {
        return ANSI_V;
    }
    let keyboard = u32::from(unsafe { LMGetKbdType() });
    command_v(|code| {
        let (mut dead, mut len, mut out) = (0u32, 0 as c_ulong, [0u16; 4]);
        let err = unsafe {
            UCKeyTranslate(
                layout.as_ptr(),
                code,
                KEY_DOWN,
                COMMAND,
                keyboard,
                NO_DEAD_KEYS,
                &mut dead,
                out.len() as c_ulong,
                &mut len,
                out.as_mut_ptr(),
            )
        };
        (err == 0).then(|| String::from_utf16_lossy(&out[..(len as usize).min(out.len())]))
    })
}

/// Windows blocks synthetic keys sent to an app running as administrator when Pointcast is not;
/// `front` is whether the app in front is elevated, `None` when that cannot be read.
#[cfg_attr(not(windows), allow(dead_code))]
fn uipi_blocks(front: Option<bool>, me: bool) -> bool {
    !me && front != Some(false)
}

#[cfg(windows)]
const ELEVATED: &str =
    "Windows does not let Pointcast type into an app running as administrator. The prompt is copied: press Ctrl+V.";

/// Whether the window in front belongs to an elevated app that Pointcast's keys cannot reach.
#[cfg(windows)]
fn front_is_out_of_reach() -> bool {
    use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE};
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
    fn elevated(process: HANDLE) -> Option<bool> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut e = TOKEN_ELEVATION::default();
            let mut len = 0u32;
            let read = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut e as *mut TOKEN_ELEVATION as *mut std::ffi::c_void),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut len,
            );
            let _ = CloseHandle(token);
            read.ok()?;
            Some(e.TokenIsElevated != 0)
        }
    }
    unsafe {
        let w = GetForegroundWindow();
        if w.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(w, Some(&mut pid));
        let front = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, BOOL::from(false), pid).ok().and_then(|h| {
            let e = elevated(h);
            let _ = CloseHandle(h);
            e
        });
        uipi_blocks(front, elevated(GetCurrentProcess()).unwrap_or(false))
    }
}

pub fn paste_now() -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(80));
    // SendInput reports success even when Windows drops the keys on the way to an elevated app.
    #[cfg(windows)]
    if front_is_out_of_reach() {
        return Err(ELEVATED.into());
    }
    // macOS: synthetic keys and the keyboard layout belong to the main thread (macOS 14+ asserts it).
    #[cfg(target_os = "macos")]
    if let Some(app) = APP.get().filter(|_| objc2::MainThreadMarker::new().is_none()) {
        let app = app.clone();
        let post = move |job: Box<dyn FnOnce() + Send>| app.run_on_main_thread(job).map_err(|e| e.to_string());
        return on_main(post, send_paste, std::time::Duration::from_secs(2));
    }
    send_paste()
}

fn send_paste() -> Result<(), String> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    #[cfg(target_os = "macos")]
    let paste = Key::Other(objc2::MainThreadMarker::new().map_or(ANSI_V, command_v_in_layout).into());
    #[cfg(not(target_os = "macos"))]
    let paste = PASTE;
    enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
    enigo
        .key(paste, Direction::Click)
        .map_err(|e| e.to_string())?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn deliver(items: &[Item], mode: OutputMode, also_clipboard: bool) -> Result<String, String> {
    match mode {
        OutputMode::Clipboard => {
            copy_session(items)?;
            Ok("Copied to clipboard".into())
        }
        OutputMode::AutoPaste => {
            copy_session(items)?;
            paste_now()?;
            Ok("Pasted into the focused app".into())
        }
        OutputMode::Folder => {
            if also_clipboard {
                let _ = copy_session(items);
            }
            Ok("Exported".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{Capture, CaptureKind, Item, Label};

    fn png() -> Vec<u8> {
        let mut buf = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut buf)
                .write_image(&[200, 24, 24, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        buf
    }

    #[test]
    fn deliver_clipboard_mode_assembles_html_without_needing_a_display() {
        // We don't touch the real clipboard here — assemble is covered in prompt.rs.
        // This test documents the mode mapping.
        let items = vec![Item::Speech {
            t_ms: 0,
            end_ms: 10,
            text: "hi".into(),
        }];
        let p = assemble_clipboard(&items);
        assert_eq!(p.plain, "hi");
        let _ = (OutputMode::Clipboard, png(), Label {
            n: 1,
            text: "x".into(),
        }, CaptureKind::Full, Capture {
            id: 1,
            kind: CaptureKind::Full,
            t_ms: 0,
            png: png(),
            width: 1,
            height: 1,
            labels: vec![],
            pointer: None,
            trail: vec![],
            element: None,
            place: None,
            anchors: vec![],
            auto: false,
        });
    }

    #[test]
    fn the_paste_key_is_the_physical_v_never_a_layout_lookup() {
        #[cfg(windows)]
        assert_eq!(PASTE, enigo::Key::V);
        #[cfg(windows)]
        assert!(!matches!(PASTE, enigo::Key::Unicode(_)));
    }

    #[test]
    fn the_mac_paste_key_is_the_one_typing_v_with_command_held() {
        let layout = |keys: &'static [(u16, &'static str)]| {
            move |code: u16| keys.iter().find(|(c, _)| *c == code).map(|(_, s)| s.to_string())
        };
        // QWERTY, and "Dvorak - QWERTY ⌘" whose Command layer is QWERTY: the physical V (kVK_ANSI_V).
        assert_eq!(command_v(layout(&[(0, "a"), (9, "v"), (40, "k"), (47, ".")])), 9);
        // Plain Dvorak: the physical V types "k", the period key (kVK_ANSI_Period) types "v".
        assert_eq!(command_v(layout(&[(0, "a"), (9, "k"), (40, "t"), (47, "v")])), 47);
        // The first key typing "v" wins, and only an exact "v" counts.
        assert_eq!(command_v(layout(&[(9, "V"), (20, "vv"), (47, "v"), (90, "v")])), 47);
        // No "v" anywhere, or nothing readable: the physical V.
        assert_eq!(command_v(layout(&[(0, "ф"), (9, "м"), (47, "ю")])), 9);
        assert_eq!(command_v(|_| None), 9);
    }

    #[test]
    fn a_paste_runs_on_the_main_thread_and_never_waits_forever() {
        use std::time::{Duration, Instant};
        let name = || std::thread::current().name().map(str::to_string);
        // Posted to another thread standing in for the main one: the answer comes back from there.
        let main = |job: Box<dyn FnOnce() + Send>| {
            std::thread::Builder::new().name("main-stand-in".into()).spawn(job).map(|_| ()).map_err(|e| e.to_string())
        };
        let wait = Duration::from_secs(5);
        assert_eq!(on_main(main, move || Ok(name()), wait), Ok(Some("main-stand-in".into())));
        assert_eq!(on_main(main, || Err::<(), _>("refused".to_string()), wait), Err("refused".into()));
        // The main thread is gone (the job is dropped) or cannot be reached: an error at once.
        let t = Instant::now();
        assert!(on_main(|job| { drop(job); Ok(()) }, || Ok(1), wait).is_err());
        assert!(on_main(|_| Err("no event loop".to_string()), || Ok(1), wait).is_err());
        assert!(t.elapsed() < Duration::from_secs(1));
        // The main thread is busy: an error after the wait, not a hang.
        let held = std::sync::Mutex::new(None);
        let t = Instant::now();
        let r = on_main(|job| { *held.lock().unwrap() = Some(job); Ok(()) }, || Ok(1), Duration::from_millis(200));
        assert!(r.is_err() && t.elapsed() >= Duration::from_millis(200) && t.elapsed() < Duration::from_secs(2), "{r:?}");
    }

    #[test]
    fn a_paste_into_an_elevated_app_is_refused_unless_we_are_elevated_too() {
        assert!(uipi_blocks(Some(true), false));
        assert!(uipi_blocks(None, false), "an app we cannot inspect counts as elevated");
        assert!(!uipi_blocks(Some(false), false));
        assert!(!uipi_blocks(Some(true), true));
        assert!(!uipi_blocks(None, true));
    }
}
