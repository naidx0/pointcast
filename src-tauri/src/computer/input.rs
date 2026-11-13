//! What macOS and Windows do the same way: synthetic input through enigo, the fallback when an element has no
//! accessibility action, and the screenshot through `capture.rs`, so password managers are painted out as in every
//! capture.
use super::{Combo, Modifier, Picture};
use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};

fn enigo() -> Result<Enigo, String> {
    Enigo::new(&Settings::default()).map_err(|e| format!("synthetic input is not available: {e}"))
}

fn ok<T, E: std::fmt::Display>(r: Result<T, E>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

pub fn click_at(x: i32, y: i32, count: u32) -> Result<(), String> {
    let mut e = enigo()?;
    ok(e.move_mouse(x, y, Coordinate::Abs))?;
    for _ in 0..count.clamp(1, 2) {
        ok(e.button(Button::Left, Direction::Click))?;
    }
    Ok(())
}

pub fn type_text(text: &str) -> Result<(), String> {
    ok(enigo()?.text(text))
}

pub fn scroll(x: i32, y: i32, dx: i32, dy: i32) -> Result<(), String> {
    let mut e = enigo()?;
    ok(e.move_mouse(x, y, Coordinate::Abs))?;
    if dy != 0 {
        ok(e.scroll(dy, Axis::Vertical))?;
    }
    if dx != 0 {
        ok(e.scroll(dx, Axis::Horizontal))?;
    }
    Ok(())
}

/// A letter or digit as the key it is on: by its virtual key code on Windows (as the paste key is, see clipboard.rs),
/// by the character on macOS.
fn char_key(c: char) -> Key {
    #[cfg(windows)]
    if c.is_ascii_alphanumeric() {
        return Key::Other(c.to_ascii_uppercase() as u32);
    }
    Key::Unicode(c)
}

fn key_of(name: &str) -> Option<Key> {
    let fkeys = [Key::F1, Key::F2, Key::F3, Key::F4, Key::F5, Key::F6, Key::F7, Key::F8, Key::F9, Key::F10, Key::F11, Key::F12];
    Some(match name {
        "return" => Key::Return,
        "tab" => Key::Tab,
        "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        f if f.starts_with('f') && f.len() > 1 => *fkeys.get(f[1..].parse::<usize>().ok()?.checked_sub(1)?)?,
        k => {
            let mut cs = k.chars();
            let c = cs.next()?;
            if cs.next().is_some() {
                return None;
            }
            char_key(c)
        }
    })
}

pub fn key(combo: &Combo) -> Result<(), String> {
    let k = key_of(&combo.key).ok_or_else(|| format!("no key named {}", combo.key))?;
    let mods: Vec<Key> = combo
        .mods
        .iter()
        .map(|m| match m {
            Modifier::Cmd => Key::Meta,
            Modifier::Ctrl => Key::Control,
            Modifier::Alt => Key::Alt,
            Modifier::Shift => Key::Shift,
        })
        .collect();
    let mut e = enigo()?;
    for m in &mods {
        ok(e.key(*m, Direction::Press))?;
    }
    let r = ok(e.key(k, Direction::Click));
    // Modifiers are let go whatever happened, so none is left held down.
    for m in mods.iter().rev() {
        let _ = e.key(*m, Direction::Release);
    }
    r
}

/// The screen under the pointer, through the same path as an Ask capture: the never-capture list is honoured and
/// Pointcast's own windows are left out. Nothing is drawn on it.
pub fn screen() -> Result<Picture, String> {
    use crate::settings::{AskTarget, MarkPoint, Settings};
    let saved = Settings::load(&crate::settings::settings_path());
    let settings = Settings { mark_point: MarkPoint::Off, outline_element: false, ..saved };
    let (cx, cy) = crate::capture::cursor_pos()?;
    let asked = crate::capture::grab_ask(&settings, AskTarget::Screen, crate::capture::ASK_EDGE)?;
    // The monitor the capture took, in screen units, so the AI can turn a pixel into a point.
    let mon = xcap::Monitor::from_point(cx, cy).map_err(|e| e.to_string())?;
    let area = (
        mon.x().unwrap_or(0),
        mon.y().unwrap_or(0),
        mon.width().unwrap_or(asked.width) as i32,
        mon.height().unwrap_or(asked.height) as i32,
    );
    let mime = match asked.mime {
        crate::assist::Mime::Png => "image/png",
        crate::assist::Mime::Jpeg => "image/jpeg",
    };
    Ok(Picture { bytes: asked.wire, mime, width: asked.wire_w, height: asked.wire_h, area })
}
