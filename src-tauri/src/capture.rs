//! Cross-platform screen grab via xcap. Hidden (password-manager) windows are painted over.
//! Pointcast's own windows are treated as hidden unless `include_app` is on.
use crate::image_ops::{self, crop, encode_png, ring};
use crate::privacy::{self, Rect};
use crate::prompt::CaptureKind;
use crate::settings::Settings;
use mouse_position::mouse_position::Mouse;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub kind: CaptureKind,
    pub cursor: (i32, i32),
}

struct Frame {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    cursor: (i32, i32),
}

fn cursor_pos() -> Result<(i32, i32), String> {
    match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => Ok((x, y)),
        Mouse::Error => Err("could not read the pointer".into()),
    }
}

fn grab_frame(settings: &Settings, hide_titles: &[&str]) -> Result<Frame, String> {
    let (cx, cy) = cursor_pos()?;
    let mon = xcap::Monitor::from_point(cx, cy).map_err(|e| e.to_string())?;
    let mx = mon.x().unwrap_or(0);
    let my = mon.y().unwrap_or(0);
    let list = settings.hidden_list();
    let me = std::process::id();
    let mut rects = vec![];
    for w in xcap::Window::all().unwrap_or_default() {
        if w.is_minimized().unwrap_or(true) {
            continue;
        }
        let pid = w.pid().ok();
        let app = w.app_name().unwrap_or_default();
        let title = w.title().unwrap_or_default();
        let ours = pid == Some(me)
            || title.to_ascii_lowercase().contains("pointcast")
            || hide_titles.iter().any(|t| title.contains(t));
        let secret = privacy::hides(&app, &title, &list);
        if (ours && !settings.include_app) || secret {
            rects.push(Rect {
                x: w.x().unwrap_or(0) - mx,
                y: w.y().unwrap_or(0) - my,
                w: w.width().unwrap_or(0) as i32,
                h: w.height().unwrap_or(0) as i32,
            });
        }
    }
    let img = mon.capture_image().map_err(|e| e.to_string())?;
    let (width, height) = img.dimensions();
    let mut buf = img.into_raw();
    for r in rects {
        privacy::mask(&mut buf, width, height, r);
    }
    Ok(Frame {
        buf,
        width,
        height,
        cursor: (cx - mx, cy - my),
    })
}

pub fn grab_full(settings: &Settings, hide_titles: &[&str]) -> Result<Shot, String> {
    let f = grab_frame(settings, hide_titles)?;
    Ok(Shot {
        png: encode_png(&f.buf, f.width, f.height)?,
        width: f.width,
        height: f.height,
        kind: CaptureKind::Full,
        cursor: f.cursor,
    })
}

pub fn grab_zoom(settings: &Settings, hide_titles: &[&str]) -> Result<Shot, String> {
    let f = grab_frame(settings, hide_titles)?;
    let n = settings.zoom_px();
    let (cw, ch) = (n.min(f.width), n.min(f.height));
    let (cx, cy) = f.cursor;
    let (mut cut, ox, oy) = crop(&f.buf, f.width, f.height, cx, cy, cw, ch);
    ring(&mut cut, cw, ch, cx - ox as i32, cy - oy as i32);
    Ok(Shot {
        png: encode_png(&cut, cw, ch)?,
        width: cw,
        height: ch,
        kind: CaptureKind::Zoom,
        cursor: (cx - ox as i32, cy - oy as i32),
    })
}

pub fn grab_freeze(settings: &Settings, hide_titles: &[&str]) -> Result<Shot, String> {
    grab_full(settings, hide_titles)
}

/// Crop `png` to `rect` (in image pixels). Used after the overlay confirms a box.
pub fn crop_png(png: &[u8], x: i32, y: i32, w: i32, h: i32) -> Result<Shot, String> {
    let (buf, width, height) = image_ops::decode_rgba(png)?;
    let (cut, _, _, cw, ch) = image_ops::crop_rect(&buf, width, height, Rect { x, y, w, h });
    Ok(Shot {
        png: encode_png(&cut, cw, ch)?,
        width: cw,
        height: ch,
        kind: CaptureKind::Region,
        cursor: (0, 0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crop_png_cuts_a_region_from_an_encoded_picture() {
        let mut buf = vec![0u8; 8 * 6 * 4];
        for i in (0..buf.len()).step_by(4) {
            buf[i] = 10;
            buf[i + 3] = 255;
        }
        let png = encode_png(&buf, 8, 6).unwrap();
        let shot = crop_png(&png, 2, 1, 3, 2).unwrap();
        assert_eq!(
            (shot.width, shot.height, shot.kind),
            (3, 2, CaptureKind::Region)
        );
        assert_eq!(crate::image_ops::png_size(&shot.png), Some((3, 2)));
    }
}
