//! Cross-platform screen grab via xcap. Hidden (password-manager) windows are painted over.
//! Pointcast's own windows are treated as hidden unless `include_app` is on.
use crate::element::{self, ElementMark};
use crate::image_ops::{self, crop, crop_rect, encode_png, enlarge, sharpen_factor};
use crate::keyframes;
use crate::pointer::{self, dwell, Point};
use crate::privacy::{self, Rect};
use crate::prompt::CaptureKind;
use crate::settings::{MarkPoint, Settings, ZoomTarget};
use mouse_position::mouse_position::Mouse;
use serde::{Deserialize, Serialize};

/// A close-up centred on where the person paused looks this far back for the pause.
pub const ZOOM_DWELL_MS: u64 = 1_500;

#[derive(Debug, Clone, Serialize)]
pub struct Shot {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub kind: CaptureKind,
    pub cursor: (i32, i32),
    /// Where the person pointed (their last rest, else the pointer) and the path there, in picture pixels.
    pub pointer: Option<Point>,
    pub trail: Vec<Point>,
    /// The control where the person pointed, in picture pixels, when they asked for outlines.
    pub element: Option<ElementMark>,
    /// How screen positions land in this picture.
    #[serde(skip)]
    pub map: Mapping,
    /// The picture's density, what its marks were drawn at.
    #[serde(skip)]
    pub k: f32,
}

impl Shot {
    /// What a mark added later needs: where screen positions land and how big to draw.
    pub fn place(&self) -> Place {
        Place { map: self.map, k: self.k }
    }
}

/// A capture's screen mapping and density, kept so a ring can be added after it was taken.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Place {
    pub map: Mapping,
    pub k: f32,
}

/// Screen coordinates to this picture's pixels: the monitor's origin and scale, then the crop.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
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

    /// A screen rect in this picture's pixels, clipped to the picture; `None` when none of it is on it.
    pub fn place_rect(&self, r: Rect) -> Option<Rect> {
        let at = |v: i32, origin: i32, crop: i32, n: u32| {
            ((((v - origin) as f64 * self.scale).round() as i64) - crop as i64).clamp(0, n as i64) as i32
        };
        let (x0, x1) = (at(r.x, self.origin.0, self.crop.0, self.size.0), at(r.x.saturating_add(r.w), self.origin.0, self.crop.0, self.size.0));
        let (y0, y1) = (at(r.y, self.origin.1, self.crop.1, self.size.1), at(r.y.saturating_add(r.h), self.origin.1, self.crop.1, self.size.1));
        (x1 > x0 && y1 > y0).then_some(Rect { x: x0, y: y0, w: x1 - x0, h: y1 - y0 })
    }

    /// The same picture enlarged `n` times.
    pub fn times(self, n: u32) -> Mapping {
        let k = n as i32;
        Mapping {
            origin: self.origin,
            scale: self.scale * n as f64,
            frame: (self.frame.0 * n, self.frame.1 * n),
            crop: (self.crop.0 * k, self.crop.1 * k),
            size: (self.size.0 * n, self.size.1 * n),
        }
    }
}

/// Where the person pointed in a picture: their last rest in `recent`, else the pointer `now`; and the path, all in
/// picture pixels. `recent` and `now` are screen coordinates.
pub fn pointing(map: &Mapping, recent: &[Point], now: Point) -> (Option<Point>, Vec<Point>) {
    let trail = recent.iter().filter_map(|p| map.place(*p)).collect();
    (map.place(spot(map, recent, now)), trail)
}

/// Where the person pointed in screen coordinates: their last rest in `recent` when it is on the picture, else `now`.
pub fn spot(map: &Mapping, recent: &[Point], now: Point) -> Point {
    dwell(recent).filter(|d| map.place(*d).is_some()).unwrap_or(now)
}

/// The control at `at` (screen coordinates) on this picture, when the person asked for outlines.
fn element_at(settings: &Settings, map: &Mapping, at: Point) -> Option<ElementMark> {
    if !settings.outline_element {
        return None;
    }
    element::mark(&element::at(at.x, at.y)?, at, map)
}

/// Outline the control the person pointed at; `k` is the picture's density.
pub fn outline(buf: &mut [u8], width: u32, height: u32, element: Option<&ElementMark>, k: f32) {
    if let Some(e) = element {
        image_ops::outline(buf, width, height, e.rect.rect(), k);
    }
}

/// What a close-up shows, in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    At(Point),
    Window(Rect),
}

/// A window as a close-up sees it.
#[derive(Debug, Clone, Copy)]
pub struct Win {
    pub rect: Rect,
    pub minimized: bool,
    /// One of Pointcast's own windows.
    pub own: bool,
}

/// The topmost window under `(x, y)` in a front-to-back list, passing over minimised windows and Pointcast's own.
pub fn window_at(list: &[Win], x: i32, y: i32) -> Option<Rect> {
    list.iter()
        .filter(|w| !w.minimized && !w.own)
        .map(|w| w.rect)
        .find(|r| r.w > 0 && r.h > 0 && x >= r.x && y >= r.y && x < r.x + r.w && y < r.y + r.h)
}

/// What a close-up centres on: the pointer `now`; the rest within the last `ZOOM_DWELL_MS`, else the pointer; or the
/// window under the pointer, else the pointer. `windows` is only asked for when it is needed.
pub fn focus(target: ZoomTarget, now: Point, recent: &[Point], windows: impl FnOnce() -> Vec<Win>) -> Focus {
    match target {
        ZoomTarget::Pointer => Focus::At(now),
        ZoomTarget::Dwell => {
            let from = recent.partition_point(|p| p.t_ms < now.t_ms.saturating_sub(ZOOM_DWELL_MS));
            Focus::At(dwell(&recent[from..]).unwrap_or(now))
        }
        ZoomTarget::Window => window_at(&windows(), now.x, now.y).map_or(Focus::At(now), Focus::Window),
    }
}

/// The part of a picture a close-up keeps, in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cut {
    /// A square around this point.
    Around(i32, i32),
    Rect(Rect),
}

/// The focus in picture pixels: a window clipped to the picture, or a point; `pointer` when the focus is not on it.
pub fn cut(map: &Mapping, focus: Focus, pointer: (i32, i32)) -> Cut {
    let around = |(x, y): (i32, i32)| Cut::Around(x, y);
    match focus {
        Focus::Window(r) => map.place_rect(r).map_or(around(pointer), Cut::Rect),
        Focus::At(p) => around(map.place(p).map_or(pointer, |q| (q.x, q.y))),
    }
}

/// Draw what the person asked for where they pointed: nothing, a ring, or the ring and the path that led to it.
/// `k` is the picture's density.
pub fn mark(buf: &mut [u8], width: u32, height: u32, how: MarkPoint, at: Option<Point>, trail: &[Point], k: f32) {
    let Some(at) = at else { return };
    if how == MarkPoint::Trail {
        let upto = trail.partition_point(|p| p.t_ms <= at.t_ms);
        image_ops::trail(buf, width, height, &trail[..upto], at, k);
    }
    if how != MarkPoint::Off {
        image_ops::ring(buf, width, height, at.x, at.y, k);
    }
}

struct Frame {
    buf: Vec<u8>,
    width: u32,
    height: u32,
    cursor: (i32, i32),
    /// The pointer in screen coordinates.
    screen: (i32, i32),
    /// Picture pixels per logical pixel: 1 on a plain display, 1.5 at 150 %, 2 on Retina.
    density: f32,
    map: Mapping,
    /// The windows on screen when it was taken, for a close-up of the window under the pointer.
    windows: Vec<Win>,
}

impl Frame {
    fn now(&self, t_ms: u64) -> Point {
        Point { t_ms, x: self.screen.0, y: self.screen.1 }
    }
}

pub fn cursor_pos() -> Result<(i32, i32), String> {
    match Mouse::get_mouse_position() {
        Mouse::Position { x, y } => Ok((x, y)),
        Mouse::Error => Err("could not read the pointer".into()),
    }
}

/// Why there is no capture when macOS would hand back only the wallpaper and the menu bar.
pub const SCREEN_OFF: &str = "Screen Recording is off for Pointcast.";

fn grab_frame(settings: &Settings) -> Result<Frame, String> {
    if !crate::permissions::can_capture() {
        return Err(SCREEN_OFF.into());
    }
    let (cx, cy) = cursor_pos()?;
    let mon = xcap::Monitor::from_point(cx, cy).map_err(|e| e.to_string())?;
    let mx = mon.x().unwrap_or(0);
    let my = mon.y().unwrap_or(0);
    // Pointcast's own windows are content-protected (see platform::protect), so the OS leaves
    // them out of the frame; only password managers and the hide-list are painted over here.
    let (rects, windows) = sort_out(&listed(), &settings.hidden_list(), std::process::id());
    let img = mon.capture_image().map_err(|e| e.to_string())?;
    let (width, height) = img.dimensions();
    let mut buf = img.into_raw();
    // macOS reports the pointer and the monitor in points but captures in pixels; Windows is
    // physical pixels throughout (scale 1).
    let scale = width as f64 / mon.width().unwrap_or(width).max(1) as f64;
    let density = mon.scale_factor().unwrap_or(scale as f32).clamp(1.0, 4.0);
    let map = Mapping { origin: (mx, my), scale, frame: (width, height), crop: (0, 0), size: (width, height) };
    paint_out(&mut buf, &map, &rects);
    let at = map.place(Point { t_ms: 0, x: cx, y: cy }).map(|p| (p.x, p.y)).unwrap_or((0, 0));
    Ok(Frame {
        buf,
        width,
        height,
        cursor: at,
        screen: (cx, cy),
        density,
        map,
        windows,
    })
}

/// Paint over hidden windows; `rects` are in screen coordinates (points on macOS, so a Retina picture is twice as big).
fn paint_out(buf: &mut [u8], map: &Mapping, rects: &[Rect]) {
    for r in rects.iter().filter_map(|r| map.place_rect(*r)) {
        privacy::mask(buf, map.size.0, map.size.1, r);
    }
}

/// A window as one read of the window list saw it.
struct Listed {
    app: String,
    title: String,
    rect: Rect,
    minimized: bool,
    pid: Option<u32>,
}

/// From one read of the window list: what to paint over (visible windows the hide-list names) and the windows a
/// close-up can centre on. `me` is this process.
fn sort_out(seen: &[Listed], hidden: &[String], me: u32) -> (Vec<Rect>, Vec<Win>) {
    let rects = seen
        .iter()
        .filter(|w| !w.minimized && privacy::hides(&w.app, &w.title, hidden))
        .map(|w| w.rect)
        .collect();
    let windows = seen
        .iter()
        .filter(|w| !shell(&w.app))
        .map(|w| Win { rect: w.rect, minimized: w.minimized, own: w.pid == Some(me) })
        .collect();
    (rects, windows)
}

/// Every window on screen, front to back (the order both xcap backends list them in).
#[cfg(not(target_os = "macos"))]
fn listed() -> Vec<Listed> {
    xcap::Window::all()
        .unwrap_or_default()
        .iter()
        .map(|w| {
            let minimized = w.is_minimized().unwrap_or(true);
            let (app, title) = if minimized {
                Default::default()
            } else {
                (w.app_name().unwrap_or_default(), w.title().unwrap_or_default())
            };
            Listed {
                app,
                title,
                rect: Rect {
                    x: w.x().unwrap_or(0),
                    y: w.y().unwrap_or(0),
                    w: w.width().unwrap_or(0) as i32,
                    h: w.height().unwrap_or(0) as i32,
                },
                minimized,
                pid: w.pid().ok(),
            }
        })
        .collect()
}

/// macOS: xcap copies the whole window list again for every property of every window, so the list is copied once
/// here, keeping the windows xcap would list, in its order.
#[cfg(target_os = "macos")]
fn listed() -> Vec<Listed> {
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> CFArrayRef;
    }
    fn get(d: &CFDictionary, key: &str) -> Option<CFType> {
        let key = CFString::new(key);
        d.find(key.as_CFTypeRef()).map(|v| unsafe { CFType::wrap_under_get_rule(*v) })
    }
    let text = |d: &CFDictionary, k: &str| get(d, k).and_then(|v| v.downcast::<CFString>()).map(|s| s.to_string());
    let number = |d: &CFDictionary, k: &str| get(d, k).and_then(|v| v.downcast::<CFNumber>()).and_then(|n| n.to_f64());
    // kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements, relative to no window.
    let raw = unsafe { CGWindowListCopyWindowInfo(1 | 16, 0) };
    if raw.is_null() {
        return vec![];
    }
    let list: CFArray = unsafe { CFArray::wrap_under_create_rule(raw) };
    list.iter()
        .filter_map(|item| {
            let d: CFDictionary = unsafe { CFDictionary::wrap_under_get_rule((*item).cast()) };
            let (app, title) = (text(&d, "kCGWindowOwnerName")?, text(&d, "kCGWindowName")?);
            // Like xcap: not the menu bar's status indicator, nor windows that refuse to be captured.
            if (title == "StatusIndicator" && app == "Window Server") || number(&d, "kCGWindowSharingState")? == 0.0 {
                return None;
            }
            let bounds = get(&d, "kCGWindowBounds")?.downcast::<CFDictionary>()?;
            let at = |k: &str| number(&bounds, k).unwrap_or(0.0);
            let shown = get(&d, "kCGWindowIsOnscreen").and_then(|v| v.downcast::<CFBoolean>()).is_some_and(bool::from);
            Some(Listed {
                rect: Rect { x: at("X") as i32, y: at("Y") as i32, w: at("Width") as i32, h: at("Height") as i32 },
                minimized: !shown,
                pid: number(&d, "kCGWindowOwnerPID").map(|p| p as u32),
                app,
                title,
            })
        })
        .collect()
}

/// The menu bar and the Dock are windows over everything on macOS, never what "the window" means.
fn shell(app: &str) -> bool {
    cfg!(target_os = "macos") && matches!(app, "Window Server" | "Dock")
}

/// What the pointer did lately: `recent` is the last few seconds of the trail in screen coordinates and `t_ms` the
/// session clock now.
pub fn grab_full(settings: &Settings, recent: &[Point], t_ms: u64) -> Result<Shot, String> {
    let mut f = grab_frame(settings)?;
    let now = f.now(t_ms);
    let (pointer, trail) = pointing(&f.map, recent, now);
    let element = element_at(settings, &f.map, spot(&f.map, recent, now));
    outline(&mut f.buf, f.width, f.height, element.as_ref(), f.density);
    mark(&mut f.buf, f.width, f.height, settings.mark_point, pointer, &trail, f.density);
    Ok(Shot {
        png: encode_png(&f.buf, f.width, f.height)?,
        width: f.width,
        height: f.height,
        kind: CaptureKind::Full,
        cursor: f.cursor,
        pointer,
        trail,
        element,
        map: f.map,
        k: f.density,
    })
}

pub fn grab_zoom(settings: &Settings, recent: &[Point], t_ms: u64) -> Result<Shot, String> {
    let mut f = grab_frame(settings)?;
    let now = f.now(t_ms);
    let n = settings.zoom_px();
    let windows = std::mem::take(&mut f.windows);
    let (cut_buf, ox, oy, cw, ch) = match cut(&f.map, focus(settings.zoom_target, now, recent, || windows), f.cursor) {
        Cut::Rect(r) => crop_rect(&f.buf, f.width, f.height, r),
        Cut::Around(x, y) => {
            let (cw, ch) = (n.min(f.width), n.min(f.height));
            let (c, ox, oy) = crop(&f.buf, f.width, f.height, x, y, cw, ch);
            (c, ox, oy, cw, ch)
        }
    };
    let up = sharpen_factor(settings.zoom_sharpen, cw, ch);
    let (mut buf, width, height) = enlarge(cut_buf, cw, ch, up);
    let map = Mapping { crop: (ox as i32, oy as i32), size: (cw, ch), ..f.map }.times(up);
    let (pointer, trail) = pointing(&map, recent, now);
    let element = element_at(settings, &map, spot(&map, recent, now));
    let k = f.density * up as f32;
    outline(&mut buf, width, height, element.as_ref(), k);
    mark(&mut buf, width, height, settings.mark_point, pointer, &trail, k);
    Ok(Shot {
        png: encode_png(&buf, width, height)?,
        width,
        height,
        kind: CaptureKind::Zoom,
        cursor: map.place(now).map_or((0, 0), |p| (p.x, p.y)),
        pointer,
        trail,
        element,
        map,
        k,
    })
}

/// The frame region mode annotates: unmarked, since the overlay places the ring and the outline as shapes the person
/// can move or remove.
pub fn grab_freeze(settings: &Settings, recent: &[Point], t_ms: u64) -> Result<Shot, String> {
    let f = grab_frame(settings)?;
    let now = f.now(t_ms);
    let (pointer, trail) = pointing(&f.map, recent, now);
    let element = element_at(settings, &f.map, spot(&f.map, recent, now));
    Ok(Shot {
        png: encode_png(&f.buf, f.width, f.height)?,
        width: f.width,
        height: f.height,
        kind: CaptureKind::Full,
        cursor: f.cursor,
        pointer,
        trail,
        element,
        map: f.map,
        k: f.density,
    })
}

/// A key frame of the pointer's monitor, painted over like any capture and made small; `t_ms` is the session clock
/// when it was taken. Nothing is drawn on it until it is picked.
pub fn grab_key(settings: &Settings, t_ms: u64) -> Result<keyframes::Still, String> {
    let f = grab_frame(settings)?;
    keyframes::still(f.buf, f.width, f.height, f.map, f.density, t_ms)
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
        pointer: None,
        trail: vec![],
        element: None,
        map: Mapping::default(),
        k: 1.0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn p(t_ms: u64, x: i32, y: i32) -> Point {
        Point { t_ms, x, y }
    }
    fn r(x: i32, y: i32, w: i32, h: i32) -> Rect {
        Rect { x, y, w, h }
    }
    fn screen(origin: (i32, i32), scale: f64, w: u32, h: u32) -> Mapping {
        Mapping { origin, scale, frame: (w, h), crop: (0, 0), size: (w, h) }
    }
    fn win(rect: Rect) -> Win {
        Win { rect, minimized: false, own: false }
    }

    /// 30 Hz samples moving in steps too long to rest from `t0` for `ms`, starting at `from`.
    fn moving(t0: u64, ms: u64, from: (i32, i32)) -> Vec<Point> {
        (0..ms / 33).map(|i| p(t0 + i * 33, from.0 + i as i32 * 20, from.1)).collect()
    }
    /// 30 Hz samples resting at `at` with a little jitter from `t0` for `ms`.
    fn resting(t0: u64, ms: u64, at: (i32, i32)) -> Vec<Point> {
        (0..=ms / 33).map(|i| p(t0 + i * 33, at.0 + [0, 3, -2][(i % 3) as usize], at.1)).collect()
    }

    #[test]
    fn a_close_up_centres_on_the_pointer_the_pause_or_the_window_and_falls_back_to_the_pointer() {
        let asked = Cell::new(0);
        let none = || {
            asked.set(asked.get() + 1);
            vec![]
        };
        // Rest at (400, 300) from 1.0 s to 1.7 s, then move away.
        let mut t = moving(0, 1_000, (0, 300));
        t.extend(resting(1_000, 700, (400, 300)));
        t.extend(moving(1_733, 300, (420, 300)));
        let now = p(2_100, 900, 500);
        assert_eq!(focus(ZoomTarget::Pointer, now, &t, none), Focus::At(now));
        let Focus::At(d) = focus(ZoomTarget::Dwell, now, &t, none) else { panic!() };
        assert!((d.x - 400).abs() <= 3 && d.y == 300, "{d:?}");
        assert_eq!(asked.get(), 0, "the window list is only read for the window target");
        // The same rest seen from 2.5 s later has left the last 1.5 s: centre on the pointer.
        let mut later = t.clone();
        later.extend(moving(2_100, 2_000, (0, 600)));
        let now = p(4_100, 700, 600);
        assert_eq!(focus(ZoomTarget::Dwell, now, &later, none), Focus::At(now));
        // A rest that mostly fell before the window is too short inside it.
        let edge = p(1_700 + ZOOM_DWELL_MS - 300, 5, 5);
        assert_eq!(focus(ZoomTarget::Dwell, edge, &t, none), Focus::At(edge));
        assert_eq!(focus(ZoomTarget::Dwell, now, &[], none), Focus::At(now));
        // Window: the one under the pointer, else the pointer.
        let front = r(600, 400, 500, 300);
        let list = vec![win(r(0, 0, 300, 300)), win(front), win(r(0, 0, 1920, 1080))];
        assert_eq!(focus(ZoomTarget::Window, p(9, 900, 500), &t, || list.clone()), Focus::Window(front));
        assert_eq!(focus(ZoomTarget::Window, p(9, 2500, 500), &t, || list.clone()), Focus::At(p(9, 2500, 500)));
        assert_eq!(focus(ZoomTarget::Window, now, &t, none), Focus::At(now));
        assert_eq!(asked.get(), 1);
    }

    #[test]
    fn the_window_under_the_pointer_is_the_topmost_visible_one_that_is_not_ours() {
        let ours = Win { own: true, ..win(r(0, 0, 800, 600)) };
        let tucked = Win { minimized: true, ..win(r(0, 0, 800, 600)) };
        let (front, back) = (r(100, 100, 400, 300), r(0, 0, 1920, 1080));
        let list = [ours, tucked, win(r(900, 0, 200, 200)), win(r(0, 0, 0, 0)), win(front), win(back)];
        assert_eq!(window_at(&list, 200, 200), Some(front));
        assert_eq!(window_at(&list, 50, 50), Some(back), "ours and minimised ones are passed over");
        assert_eq!(window_at(&list, 499, 399), Some(front));
        assert_eq!(window_at(&list, 500, 400), Some(back), "right and bottom edges are outside");
        assert_eq!(window_at(&list, -5, 10), None);
        assert_eq!(window_at(&[ours, tucked], 10, 10), None);
        // A window on a monitor left of the main one.
        assert_eq!(window_at(&[win(r(-1800, 100, 900, 700))], -1000, 300), Some(r(-1800, 100, 900, 700)));
    }

    #[test]
    fn a_close_up_cut_is_clipped_to_the_monitor_and_falls_back_to_the_pointer() {
        let m = screen((0, 0), 1.0, 1920, 1080);
        let ptr = (960, 540);
        assert_eq!(cut(&m, Focus::At(p(1, 500, 400)), ptr), Cut::Around(500, 400));
        // A pause on another monitor: the pointer.
        assert_eq!(cut(&m, Focus::At(p(1, -300, 400)), ptr), Cut::Around(960, 540));
        assert_eq!(cut(&m, Focus::Window(r(300, 200, 800, 600)), ptr), Cut::Rect(r(300, 200, 800, 600)));
        // A maximised window hangs a few pixels over every edge.
        assert_eq!(cut(&m, Focus::Window(r(-8, -8, 1936, 1056)), ptr), Cut::Rect(r(0, 0, 1920, 1048)));
        assert_eq!(cut(&m, Focus::Window(r(1800, 900, 400, 400)), ptr), Cut::Rect(r(1800, 900, 120, 180)));
        assert_eq!(cut(&m, Focus::Window(r(-900, 0, 800, 600)), ptr), Cut::Around(960, 540));
        // Retina: points to pixels.
        let retina = screen((0, 0), 2.0, 3840, 2160);
        assert_eq!(cut(&retina, Focus::Window(r(100, 50, 400, 300)), ptr), Cut::Rect(r(200, 100, 800, 600)));
        assert_eq!(cut(&retina, Focus::At(p(1, 100, 50)), ptr), Cut::Around(200, 100));
        // A second monitor with a negative origin.
        let left = screen((-1920, 0), 1.0, 1920, 1080);
        assert_eq!(cut(&left, Focus::Window(r(-1000, 100, 500, 400)), ptr), Cut::Rect(r(920, 100, 500, 400)));
    }

    #[test]
    fn a_hidden_window_is_painted_over_where_it_is_in_the_picture() {
        let paint = |m: &Mapping, rects: &[Rect]| {
            let (w, h) = m.frame;
            let mut b = vec![255u8; (w * h * 4) as usize];
            paint_out(&mut b, m, rects);
            move |x: u32, y: u32| b[((y * w + x) * 4) as usize]
        };
        // Retina: a 400x300 point window at (100, 50) covers pixels 200..1000 across and 100..700 down.
        let px = paint(&screen((0, 0), 2.0, 1000, 800), &[r(100, 50, 400, 300)]);
        assert_eq!((px(200, 100), px(600, 400), px(999, 699)), (24, 24, 24));
        assert_eq!((px(199, 100), px(200, 99), px(500, 700)), (255, 255, 255));
        // A Retina screen left of the main one, and a window hanging off its right edge.
        let px = paint(&screen((-1512, 0), 2.0, 3024, 1964), &[r(-212, 100, 600, 200)]);
        assert_eq!((px(2600, 200), px(3023, 599)), (24, 24));
        assert_eq!((px(2599, 200), px(2600, 199), px(2600, 600)), (255, 255, 255));
        // Windows: physical pixels throughout.
        let px = paint(&screen((1920, 0), 1.0, 1280, 1024), &[r(2000, 10, 100, 50)]);
        assert_eq!((px(80, 10), px(179, 59), px(180, 10), px(79, 10)), (24, 24, 255, 255));
        // Off this screen: nothing.
        let px = paint(&screen((0, 0), 2.0, 100, 100), &[r(-300, 0, 200, 50)]);
        assert_eq!(px(0, 0), 255);
    }

    #[test]
    fn one_read_of_the_window_list_gives_what_to_paint_over_and_what_to_zoom_to() {
        let w = |app: &str, title: &str, rect, minimized, pid| Listed { app: app.into(), title: title.into(), rect, minimized, pid };
        let seen = [
            w("Bitwarden", "Vault", r(0, 0, 400, 300), false, Some(7)),
            w("1Password", "", r(10, 10, 50, 50), true, Some(8)),
            w("Pointcast", "HUD", r(0, 900, 392, 64), false, Some(42)),
            w("Notes", "Bitwarden Web Vault", r(500, 0, 300, 300), false, None),
            w("Dock", "", r(0, 1000, 1920, 80), false, Some(9)),
        ];
        let (rects, wins) = sort_out(&seen, &privacy::default_hidden_list(), 42);
        assert_eq!(rects, vec![r(0, 0, 400, 300), r(500, 0, 300, 300)], "a minimised password manager shows nothing");
        // The Dock is never "the window" on macOS; elsewhere it is an ordinary name.
        assert_eq!(wins.len(), if cfg!(target_os = "macos") { 4 } else { 5 });
        let own: Vec<bool> = wins.iter().map(|w| w.own).collect();
        assert_eq!(own[..4], [false, false, true, false]);
        assert!(wins[1].minimized && !wins[0].minimized);
        assert_eq!(wins[3].rect, r(500, 0, 300, 300));
    }

    #[test]
    fn a_capture_points_at_the_last_rest_else_the_pointer_now() {
        let m = screen((0, 0), 1.0, 1920, 1080);
        let mut t = moving(0, 600, (0, 100));
        t.extend(resting(600, 700, (700, 450)));
        let now = p(1_400, 1200, 800);
        let (at, trail) = pointing(&m, &t, now);
        let at = at.unwrap();
        assert!((at.x - 700).abs() <= 3 && at.y == 450, "{at:?}");
        assert_eq!(trail.len(), t.len());
        assert_eq!(pointing(&m, &moving(0, 600, (0, 100)), now).0, Some(now));
        assert_eq!(pointing(&m, &[], now), (Some(now), vec![]));
        // A rest on another monitor does not land in this picture: the pointer does.
        let off: Vec<Point> = resting(0, 700, (-500, 450));
        assert_eq!(pointing(&m, &off, now), (Some(now), vec![]));
        // Through a crop that was then enlarged 2x.
        let zoomed = Mapping { crop: (1000, 600), size: (512, 512), ..m }.times(2);
        assert_eq!(pointing(&zoomed, &[], now).0, Some(p(1_400, 400, 400)));
        assert_eq!(zoomed.size, (1024, 1024));
    }

    #[test]
    fn the_control_is_looked_up_where_the_person_pointed_and_outlined_when_found() {
        let m = screen((0, 0), 1.0, 1920, 1080);
        let mut t = moving(0, 600, (0, 100));
        t.extend(resting(600, 700, (700, 450)));
        let now = p(1_400, 1200, 800);
        let s = spot(&m, &t, now);
        assert!((s.x - 700).abs() <= 3 && s.y == 450, "{s:?}");
        assert_eq!(pointing(&m, &t, now).0, Some(s), "the ring and the outline are asked for at one spot");
        assert_eq!(spot(&m, &[], now), now);
        assert_eq!(spot(&m, &resting(0, 700, (-500, 450)), now), now, "a rest off this picture");
        let off = Settings { outline_element: false, ..Settings::default() };
        assert_eq!(element_at(&off, &m, now), None, "off never asks the OS");
        let blank = vec![255u8; 120 * 60 * 4];
        let mut b = blank.clone();
        outline(&mut b, 120, 60, None, 1.0);
        assert_eq!(b, blank);
        let ok = ElementMark { role: "button".into(), name: "OK".into(), rect: element::Bounds { x: 30, y: 20, w: 40, h: 16 } };
        outline(&mut b, 120, 60, Some(&ok), 1.0);
        assert_eq!(&b[(28 * 120 + 27) * 4..][..3], &[245, 165, 36]);
        assert_eq!(&b[(28 * 120 + 40) * 4..][..3], &[255; 3]);
    }

    #[test]
    fn a_capture_marks_the_spot_only_as_asked() {
        let blank = vec![255u8; 120 * 60 * 4];
        let at = Some(p(2_000, 80, 30));
        let path: Vec<Point> = (0..8).map(|i| p(i * 200, 5 + i as i32 * 8, 30)).collect();
        let draw = |how, at, trail: &[Point]| {
            let mut b = blank.clone();
            mark(&mut b, 120, 60, how, at, trail, 1.0);
            b
        };
        let px = |b: &[u8], x: usize, y: usize| b[(y * 120 + x) * 4];
        assert_eq!(draw(MarkPoint::Off, at, &path), blank);
        assert_eq!(draw(MarkPoint::Ring, None, &path), blank);
        let on_ring = (30 * 120 + 80 + image_ops::RING_R as usize) * 4;
        let ring = draw(MarkPoint::Ring, at, &path);
        assert_eq!(&ring[on_ring..][..3], &[245, 165, 36]);
        assert_eq!(px(&ring, 20, 30), 255, "a ring alone draws no path");
        let both = draw(MarkPoint::Trail, at, &path);
        assert_eq!(&both[on_ring..][..3], &[245, 165, 36]);
        assert!(px(&both, 40, 30) < 255);
        // The path ends at the rest: where the hand went after it is not drawn.
        let mut after = path.clone();
        after.push(p(2_200, 110, 55));
        assert_eq!(draw(MarkPoint::Trail, at, &after), both);
    }
}
