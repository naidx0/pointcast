//! Cross-platform screen grab via xcap. Hidden (password-manager) windows are painted over.
//! Pointcast's own windows are treated as hidden unless `include_app` is on.
use crate::image_ops::{self, crop, encode_png, ring};
use crate::privacy::{self, Rect};
use crate::prompt::CaptureKind;
use crate::settings::Settings;
use crate::pointer::{self, Point};
use mouse_position::mouse_position::Mouse;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub kind: CaptureKind,
    pub cursor: (i32, i32),
    /// How screen positions land in this picture.
    #[serde(skip)]
    pub map: Mapping,
}

/// Screen coordinates to this picture's pixels: the monitor's origin and scale, then the crop.
#[derive(Debug, Clone, Copy, Default)]
pub struct Mapping {
    pub origin: (i32, i32),
    pub scale: f64,
    pub frame: (u32, u32),
    pub crop: (i32, i32),
    pub size: (u32, u32),
}

impl Mapping {
    pub fn place(&self, p: Point) -> Option<Point> {
        let q = pointer::to_image(p, self.origin, self.scale, self.frame)?;
        let (x, y) = (q.x - self.crop.0, q.y - self.crop.1);
        (x >= 0 && y >= 0 && x < self.size.0 as i32 && y < self.size.1 as i32).then_some(Point { x, y, ..q })
    }
}

struct Frame {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    cursor: (i32, i32),
    map: Mapping,
}

pub fn cursor_pos() -> Result<(i32, i32), String> {
    match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => Ok((x, y)),
        Mouse::Error => Err("could not read the pointer".into()),
    }
}

fn grab_frame(settings: &Settings) -> Result<Frame, String> {
    let (cx, cy) = cursor_pos()?;
    let mon = xcap::Monitor::from_point(cx, cy).map_err(|e| e.to_string())?;
    let mx = mon.x().unwrap_or(0);
    let my = mon.y().unwrap_or(0);
    let list = settings.hidden_list();
    // Pointcast's own windows are content-protected (see platform::protect), so the OS leaves
    // them out of the frame; only password managers and the hide-list are painted over here.
    let mut rects = vec![];
    for w in xcap::Window::all().unwrap_or_default() {
        if w.is_minimized().unwrap_or(true) {
            continue;
        }
        let app = w.app_name().unwrap_or_default();
        let title = w.title().unwrap_or_default();
        if privacy::hides(&app, &title, &list) {
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
    // macOS reports the pointer and the monitor in points but captures in pixels; Windows is
    // physical pixels throughout (scale 1).
    let scale = width as f64 / mon.width().unwrap_or(width).max(1) as f64;
    let map = Mapping { origin: (mx, my), scale, frame: (width, height), crop: (0, 0), size: (width, height) };
    let at = map.place(Point { t_ms: 0, x: cx, y: cy }).map(|p| (p.x, p.y)).unwrap_or((0, 0));
    Ok(Frame {
        buf,
        width,
        height,
        cursor: at,
        map,
    })
}

pub fn grab_full(settings: &Settings) -> Result<Shot, String> {
    let f = grab_frame(settings)?;
    Ok(Shot {
        png: encode_png(&f.buf, f.width, f.height)?,
        width: f.width,
        height: f.height,
        kind: CaptureKind::Full,
        cursor: f.cursor,
        map: f.map,
    })
}

pub fn grab_zoom(settings: &Settings) -> Result<Shot, String> {
    let f = grab_frame(settings)?;
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
        map: Mapping { crop: (ox as i32, oy as i32), size: (cw, ch), ..f.map },
    })
}

pub fn grab_freeze(settings: &Settings) -> Result<Shot, String> {
    grab_full(settings)
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
        map: Mapping::default(),
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
