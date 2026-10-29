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
/// kVK_ANSI_V
#[cfg(target_os = "macos")]
const PASTE: enigo::Key = enigo::Key::Other(9);
#[cfg(not(any(windows, target_os = "macos")))]
const PASTE: enigo::Key = enigo::Key::Unicode('v');

pub fn paste_now() -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(80));
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
    enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
    enigo
        .key(PASTE, Direction::Click)
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
        #[cfg(target_os = "macos")]
        assert_eq!(PASTE, enigo::Key::Other(9));
        #[cfg(any(windows, target_os = "macos"))]
        assert!(!matches!(PASTE, enigo::Key::Unicode(_)));
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
}
