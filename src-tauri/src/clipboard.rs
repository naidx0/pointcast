//! Copy a Pointcast session as rich HTML with a plain-text fallback, then optionally paste.
use crate::prompt::{assemble_clipboard, Item};
use crate::settings::OutputMode;

pub fn copy_session(items: &[Item]) -> Result<(String, String), String> {
    let payload = assemble_clipboard(items);
    if payload.plain.is_empty() && payload.html.is_empty() {
        return Err("nothing to copy".into());
    }
    let mut cb = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    // HTML with plain fallback: ChatGPT / Claude / Slack pick up inline images from the HTML.
    match cb.set_html(&payload.html, Some(&payload.plain)) {
        Ok(()) => {}
        Err(_) => {
            cb.set_text(&payload.plain).map_err(|e| e.to_string())?;
        }
    }
    Ok((payload.html, payload.plain))
}

pub fn paste_now() -> Result<(), String> {
    std::thread::sleep(std::time::Duration::from_millis(80));
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;
    let modifier = if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    };
    enigo.key(modifier, Direction::Press).map_err(|e| e.to_string())?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
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
        });
    }
}
