//! Key frames (opt-in): while recording, the pointer's monitor is kept once a second in memory, made small and bounded;
//! at stop the moments the person talked about become pictures marked `auto`. Nothing here is written to disk or
//! logged: a frame leaves memory only as a picture in the prompt.
use crate::anchor::{self, Token};
use crate::capture::{self, Mapping, Place};
use crate::image_ops;
use crate::pointer::{self, Point};
use crate::prompt::{self, Capture, CaptureKind, Item};
use crate::settings::MarkPoint;
use std::collections::VecDeque;

/// One frame a second...
pub const EVERY_MS: u64 = 1_000;
/// ...for the last two minutes...
pub const WINDOW_MS: u64 = 120_000;
pub const MAX_FRAMES: usize = 120;
/// ...and never more memory than this.
pub const MAX_BYTES: usize = 96 * 1024 * 1024;
/// A frame is made smaller to fit this on its long side.
pub const LONG_SIDE: u32 = 1_600;
/// Frames are compared on a thumbnail this big on its long side...
pub const THUMB: u32 = 64;
/// ...where a pixel changed when one of its channels moved more than this...
pub const PIXEL_DIFF: u8 = 24;
/// ...and the screen changed when more than this share of its pixels did.
pub const CHANGE: f32 = 0.12;
/// A frame stands for a moment only this close to it.
pub const NEAR_MS: u64 = 1_500;
/// Chosen moments are at least this far apart, and this far from a picture the person took.
pub const SPACING_MS: u64 = 2_000;
/// At most this many key frames in a prompt.
pub const MOST: usize = 6;

/// A small RGB picture frames are compared on.
#[derive(Debug, Clone, PartialEq)]
pub struct Thumb {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

/// One kept frame: its PNG, how screen positions land in it, and its thumbnail.
#[derive(Debug, Clone)]
pub struct Still {
    pub t_ms: u64,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub place: Place,
    pub thumb: Thumb,
}

impl Still {
    fn bytes(&self) -> usize {
        self.png.len() + self.thumb.px.len()
    }
}

/// The last `WINDOW_MS` of frames, at most `MAX_FRAMES` of them and `MAX_BYTES` in all; the oldest go first.
#[derive(Debug, Default)]
pub struct Buffer {
    frames: VecDeque<Still>,
    bytes: usize,
}

impl Buffer {
    pub fn push(&mut self, s: Still) {
        let newest = s.t_ms;
        self.bytes += s.bytes();
        self.frames.push_back(s);
        while let Some(old) = self.frames.front() {
            let (t, b) = (old.t_ms, old.bytes());
            if self.frames.len() <= MAX_FRAMES && self.bytes <= MAX_BYTES && t + WINDOW_MS >= newest {
                break;
            }
            self.frames.pop_front();
            self.bytes -= b;
        }
    }

    pub fn frames(&self) -> &VecDeque<Still> {
        &self.frames
    }
}

/// A frame as grabbed (RGBA, with its screen mapping and density), made to fit `LONG_SIDE` with its mapping and density
/// scaled to match, then encoded.
pub fn still(buf: Vec<u8>, width: u32, height: u32, map: Mapping, density: f32, t_ms: u64) -> Result<Still, String> {
    let (buf, w, h) = fit(buf, width, height);
    let f = w as f64 / width.max(1) as f64;
    let map = Mapping { scale: map.scale * f, frame: (w, h), crop: (0, 0), size: (w, h), ..map };
    Ok(Still {
        t_ms,
        png: image_ops::encode_png(&buf, w, h)?,
        width: w,
        height: h,
        place: Place { map, k: (density * f as f32).max(1.0) },
        thumb: thumb(&buf, w, h),
    })
}

/// An RGBA picture made smaller, when it has to be, to fit `LONG_SIDE` on its long side.
pub fn fit(buf: Vec<u8>, width: u32, height: u32) -> (Vec<u8>, u32, u32) {
    let long = width.max(height);
    if long <= LONG_SIDE || buf.len() != (width * height * 4) as usize {
        return (buf, width, height);
    }
    let side = |n: u32| ((n as u64 * LONG_SIDE as u64 / long as u64) as u32).max(1);
    let (w, h) = (side(width), side(height));
    let img = image::RgbaImage::from_raw(width, height, buf).expect("length checked");
    (image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle).into_raw(), w, h)
}

/// The average colour of each block of an RGBA picture, `THUMB` blocks along its long side.
pub fn thumb(buf: &[u8], width: u32, height: u32) -> Thumb {
    let long = width.max(height).max(1);
    let side = |n: u32| ((n as u64 * THUMB as u64 / long as u64) as u32).clamp(1, THUMB);
    let (w, h) = (side(width), side(height));
    let mut sum = vec![[0u64; 4]; (w * h) as usize];
    if buf.len() == (width * height * 4) as usize {
        for y in 0..height {
            let row = (y as u64 * h as u64 / height as u64) as u32 * w;
            for x in 0..width {
                let i = ((y * width + x) * 4) as usize;
                let s = &mut sum[(row + (x as u64 * w as u64 / width as u64) as u32) as usize];
                for c in 0..3 {
                    s[c] += buf[i + c] as u64;
                }
                s[3] += 1;
            }
        }
    }
    let px = sum.iter().flat_map(|s| (0..3).map(move |c| (s[c] / s[3].max(1)) as u8)).collect();
    Thumb { w, h, px }
}

/// The share of a thumbnail's pixels that changed from `a` to `b`. A frame of another size (another monitor) is all
/// change.
pub fn change(a: &Thumb, b: &Thumb) -> f32 {
    if (a.w, a.h) != (b.w, b.h) || a.px.len() != b.px.len() {
        return 1.0;
    }
    let n = a.px.len() / 3;
    let moved = a
        .px
        .chunks_exact(3)
        .zip(b.px.chunks_exact(3))
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > PIXEL_DIFF))
        .count();
    moved as f32 / n.max(1) as f32
}

/// The frames where the screen changed by more than `CHANGE` since the frame before, biggest change first.
pub fn changes(frames: &[&Still]) -> Vec<u64> {
    let mut v: Vec<(f32, u64)> = frames
        .windows(2)
        .map(|w| (change(&w[0].thumb, &w[1].thumb), w[1].t_ms))
        .filter(|(c, _)| *c > CHANGE)
        .collect();
    v.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    v.into_iter().map(|(_, t)| t).collect()
}

/// In frame times oldest first, the frame nearest `t` within `NEAR_MS`.
fn nearest(frames: &[u64], t: u64) -> Option<usize> {
    let i = frames.partition_point(|f| *f < t);
    [i.checked_sub(1), Some(i).filter(|i| *i < frames.len())]
        .into_iter()
        .flatten()
        .filter(|j| frames[*j].abs_diff(t) <= NEAR_MS)
        .min_by_key(|j| frames[*j].abs_diff(t))
}

/// Which frames to add. `wanted` holds moments, most important first; each takes the frame nearest it unless it is
/// within `SPACING_MS` of a moment already chosen or of a picture the person took (`taken`), or its frame was already
/// chosen. At most `MOST`, returned in time order as (moment, frame).
pub fn pick(frames: &[u64], wanted: &[u64], taken: &[u64]) -> Vec<(u64, usize)> {
    let mut out: Vec<(u64, usize)> = vec![];
    for &t in wanted {
        if out.len() == MOST {
            break;
        }
        let Some(i) = nearest(frames, t) else { continue };
        let close = |u: u64| u.abs_diff(t) < SPACING_MS;
        if out.iter().any(|(u, j)| close(*u) || *j == i) || taken.iter().any(|u| close(*u)) {
            continue;
        }
        out.push((t, i));
    }
    out.sort();
    out
}

/// At stop: add the frames at the moments the person talked about as pictures marked `auto`. Pointing words come
/// first, then the longest rests (at their end), then the biggest screen changes. Each is marked where the person
/// pointed then, as a capture is (`how`). Returns how many were added.
pub fn add(items: &mut Vec<Item>, buf: &Buffer, trail: &[Point], tokens: &[(u64, Vec<Token>)], how: MarkPoint) -> usize {
    let frames: Vec<&Still> = buf.frames().iter().collect();
    if frames.is_empty() {
        return 0;
    }
    let mut rests = pointer::rests(trail);
    rests.sort_by_key(|r| std::cmp::Reverse(r.at.t_ms - r.from));
    let mut wanted = anchor::pointing_times(items, tokens);
    wanted.extend(rests.iter().map(|r| r.at.t_ms));
    wanted.extend(changes(&frames));
    let taken: Vec<u64> = items
        .iter()
        .filter_map(|i| match i {
            Item::Image { t_ms, .. } => Some(*t_ms),
            _ => None,
        })
        .collect();
    let times: Vec<u64> = frames.iter().map(|f| f.t_ms).collect();
    let mut n = 0;
    for (t, i) in pick(&times, &wanted, &taken) {
        if let Ok(capture) = picture(frames[i], t, trail, how) {
            items.push(Item::Image { t_ms: t, capture });
            n += 1;
        }
    }
    *items = prompt::ordered(std::mem::take(items));
    n
}

/// A frame as a picture at `t`, marked where the person pointed then: their rest in the seconds before, else the
/// pointer.
fn picture(f: &Still, t: u64, trail: &[Point], how: MarkPoint) -> Result<Capture, String> {
    let upto = trail.partition_point(|p| p.t_ms <= t);
    let from = trail[..upto].partition_point(|p| p.t_ms < t.saturating_sub(pointer::TRAIL_MS));
    let (at, path) = match trail[..upto].last() {
        Some(now) => capture::pointing(&f.place.map, &trail[from..upto], Point { t_ms: t, ..*now }),
        None => (None, vec![]),
    };
    let png = if at.is_some() && how != MarkPoint::Off {
        let (mut buf, w, h) = image_ops::decode_rgba(&f.png)?;
        capture::mark(&mut buf, w, h, how, at, &path, f.place.k);
        image_ops::encode_png(&buf, w, h)?
    } else {
        f.png.clone()
    };
    Ok(Capture {
        id: 0,
        kind: CaptureKind::Full,
        t_ms: t,
        png,
        width: f.width,
        height: f.height,
        labels: vec![],
        pointer: at,
        trail: path,
        element: None,
        place: Some(f.place),
        anchors: vec![],
        auto: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t_ms: u64, x: i32, y: i32) -> Point {
        Point { t_ms, x, y }
    }
    fn canvas(w: u32, h: u32, c: [u8; 3]) -> Vec<u8> {
        (0..w * h).flat_map(|_| [c[0], c[1], c[2], 255]).collect()
    }
    /// `buf` with the block from `(x0, y0)`, `w` by `h`, painted `c`.
    fn block(mut buf: Vec<u8>, width: u32, (x0, y0, w, h): (u32, u32, u32, u32), c: [u8; 3]) -> Vec<u8> {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                buf[((y * width + x) * 4) as usize..][..3].copy_from_slice(&c);
            }
        }
        buf
    }
    fn screen(w: u32, h: u32) -> Mapping {
        Mapping { origin: (0, 0), scale: 1.0, frame: (w, h), crop: (0, 0), size: (w, h) }
    }
    /// A 200 by 100 frame at `t` of a plain screen in colour `c`.
    fn frame(t: u64, c: [u8; 3]) -> Still {
        still(canvas(200, 100, c), 200, 100, screen(200, 100), 1.0, t).unwrap()
    }
    fn sized(t: u64, bytes: usize) -> Still {
        Still { png: vec![0; bytes], ..frame(t, [255; 3]) }
    }

    #[test]
    fn the_buffer_keeps_the_last_two_minutes_at_one_a_second() {
        let mut b = Buffer::default();
        for s in 0..200 {
            b.push(sized(s * 1_000, 100));
        }
        assert_eq!(b.frames.len(), MAX_FRAMES);
        assert_eq!(b.frames().front().unwrap().t_ms, 80_000);
        assert_eq!(b.frames().back().unwrap().t_ms, 199_000);
        assert_eq!(b.bytes, b.frames().iter().map(Still::bytes).sum::<usize>());
        // After a gap, what is more than two minutes older than the newest frame goes.
        let mut gap = Buffer::default();
        for t in [0, 1_000, 30_000, 150_000] {
            gap.push(sized(t, 100));
        }
        let times: Vec<u64> = gap.frames().iter().map(|f| f.t_ms).collect();
        assert_eq!(times, [30_000, 150_000], "exactly two minutes old is still kept");
    }

    #[test]
    fn the_buffer_never_holds_more_than_its_byte_bound() {
        let one = sized(0, 8 * 1024 * 1024);
        let fits = MAX_BYTES / one.bytes();
        let mut b = Buffer::default();
        for s in 0..(fits as u64 + 10) {
            b.push(sized(s * 1_000, 8 * 1024 * 1024));
            assert!(b.bytes <= MAX_BYTES, "{}", b.bytes);
        }
        assert_eq!(b.frames.len(), fits);
        assert_eq!(b.frames().back().unwrap().t_ms, (fits as u64 + 9) * 1_000, "the newest stay");
        let mut huge = Buffer::default();
        huge.push(sized(0, MAX_BYTES + 1));
        assert_eq!((huge.frames.len(), huge.bytes), (0, 0), "a frame bigger than the bound is not kept");
    }

    #[test]
    fn a_frame_is_made_to_fit_1600_pixels_and_its_mapping_follows() {
        // A 3200 by 400 monitor left of the main one at 2x density.
        let map = Mapping { origin: (-3200, 0), ..screen(3200, 400) };
        let s = still(canvas(3200, 400, [40, 80, 120]), 3200, 400, map, 2.0, 7_000).unwrap();
        assert_eq!((s.width, s.height, s.t_ms), (1_600, 200, 7_000));
        assert_eq!(image_ops::png_size(&s.png), Some((1_600, 200)));
        assert_eq!(s.place.map.place(p(1, -1600, 300)), Some(p(1, 800, 150)));
        assert_eq!(s.place.map.place(p(1, -2, 398)), Some(p(1, 1599, 199)), "the far corner is still on it");
        assert_eq!(s.place.map.place(p(1, 0, 10)), None);
        assert_eq!(s.place.k, 1.0);
        assert_eq!((s.thumb.w, s.thumb.h), (THUMB, 8));
        // Already small enough: kept as it is.
        let small = still(canvas(1280, 720, [9; 3]), 1280, 720, screen(1280, 720), 1.0, 0).unwrap();
        assert_eq!((small.width, small.height, small.place.map), (1280, 720, screen(1280, 720)));
        assert_eq!(small.place.map.place(p(1, 1000, 700)), Some(p(1, 1000, 700)));
    }

    #[test]
    fn the_change_metric_counts_the_share_of_the_thumbnail_that_moved() {
        let (w, h) = (320u32, 180u32);
        let white = canvas(w, h, [255; 3]);
        let t = |buf: &[u8]| thumb(buf, w, h);
        assert_eq!(change(&t(&white), &t(&white)), 0.0);
        // A dialog over a fifth of the screen is a big change; a tooltip over a twentieth is not.
        let dialog = block(white.clone(), w, (0, 0, 160, 72), [30, 30, 30]);
        let c = change(&t(&white), &t(&dialog));
        assert!((c - 0.2).abs() < 0.03 && c > CHANGE, "{c}");
        let tip = block(white.clone(), w, (40, 40, 80, 36), [30, 30, 30]);
        let c = change(&t(&white), &t(&tip));
        assert!(c > 0.0 && c < CHANGE, "{c}");
        // Faint changes everywhere (a fade, a cursor blink smeared by the thumbnail) are not changes.
        let dim = canvas(w, h, [240; 3]);
        assert_eq!(change(&t(&white), &t(&dim)), 0.0);
        // A whole new screen, in colour only.
        let red = canvas(w, h, [255, 0, 0]);
        assert_eq!(change(&t(&white), &t(&red)), 1.0);
        assert_eq!(change(&t(&white), &thumb(&canvas(100, 100, [255; 3]), 100, 100)), 1.0, "another monitor");
        // Only frames past the threshold count, biggest first.
        let fr = |ts: u64, buf: &[u8]| Still { thumb: t(buf), ..sized(ts, 10) };
        let seq = [fr(0, &white), fr(1_000, &tip), fr(2_000, &dialog), fr(3_000, &dialog), fr(4_000, &red)];
        let refs: Vec<&Still> = seq.iter().collect();
        assert_eq!(changes(&refs), [4_000, 2_000]);
    }

    #[test]
    fn a_moment_takes_the_nearest_frame_and_moments_stay_two_seconds_apart() {
        let frames: Vec<u64> = (0..60).map(|s| s * 1_000 + 40).collect();
        assert_eq!(pick(&frames, &[10_300], &[]), [(10_300, 10)]);
        assert_eq!(pick(&frames, &[10_600], &[]), [(10_600, 11)]);
        // A rest end 600 ms after a pointing word is the same moment; one at 2 s is another.
        assert_eq!(pick(&frames, &[10_300, 10_900, 12_300], &[]), [(10_300, 10), (12_300, 12)]);
        // Near a picture the person took: left out.
        assert_eq!(pick(&frames, &[10_300, 30_000], &[31_500]), [(10_300, 10)]);
        assert_eq!(pick(&frames, &[30_000], &[32_000]), [(30_000, 30)], "2 s away is far enough");
        // Outside what the buffer still holds, or in a gap of it.
        assert!(pick(&frames, &[61_600, 200_000], &[]).is_empty());
        assert!(pick(&[0, 5_000], &[2_500], &[]).is_empty());
        assert!(pick(&[], &[0], &[]).is_empty());
    }

    #[test]
    fn at_most_six_are_picked_most_important_first_and_returned_in_time_order() {
        let frames: Vec<u64> = (0..120).map(|s| s * 1_000).collect();
        let wanted: Vec<u64> = (0..10).rev().map(|i| i * 10_000).collect();
        let got = pick(&frames, &wanted, &[]);
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), [40_000, 50_000, 60_000, 70_000, 80_000, 90_000]);
        // One frame never stands for two moments, even in a gap where both are near it.
        assert_eq!(pick(&[0, 10_000], &[8_700, 11_400], &[]), [(8_700, 1)]);
    }

    fn speech(t: u64, end: u64, text: &str) -> Item {
        Item::Speech { t_ms: t, end_ms: end, text: text.into() }
    }
    fn autos(items: &[Item]) -> Vec<(u64, bool)> {
        items
            .iter()
            .filter_map(|i| match i {
                Item::Image { t_ms, capture } => Some((*t_ms, capture.auto)),
                _ => None,
            })
            .collect()
    }
    fn px(c: &Capture, x: u32, y: u32) -> [u8; 3] {
        let (buf, w, _) = image_ops::decode_rgba(&c.png).unwrap();
        buf[((y * w + x) * 4) as usize..][..3].try_into().unwrap()
    }

    fn taken(t: u64) -> Item {
        let f = frame(t, [255; 3]);
        let capture = Capture { auto: false, ..picture(&f, t, &[], MarkPoint::Off).unwrap() };
        Item::Image { t_ms: t, capture }
    }

    #[test]
    fn at_stop_the_moments_talked_about_are_added_as_auto_pictures() {
        // 30 s of frames of a white screen; from 20 s to 26 s a dark dialog covers its left half.
        let dialog = block(canvas(200, 100, [255; 3]), 200, (0, 0, 100, 100), [20, 20, 20]);
        let mut b = Buffer::default();
        for s in 0..30 {
            let buf = if (20..26).contains(&s) { dialog.clone() } else { canvas(200, 100, [255; 3]) };
            b.push(still(buf, 200, 100, screen(200, 100), 1.0, s * 1_000).unwrap());
        }
        // The pointer moves along y = 50 until 5 s, rests at (150, 30) until 8.96 s, then moves along y = 80.
        let mut trail: Vec<Point> = (0..150).map(|i| p(i * 33, (i * 7 % 200) as i32, 50)).collect();
        trail.extend((0..=120).map(|i| p(5_000 + i * 33, 150 + [0, 2, -1][(i % 3) as usize], 30)));
        trail.extend((0..600).map(|i| p(9_000 + i * 33, (i * 7 % 200) as i32, 80)));
        // "this" is said at 1.57 s and "here" at 2.29 s; the person took a picture at 21 s.
        let mut items = vec![speech(1_000, 3_000, "put this here."), taken(21_000)];
        assert_eq!(add(&mut items, &b, &trail, &[], MarkPoint::Ring), 3);
        // "here" is too near "this", and the dialog opening is in the picture the person took; its closing is not.
        assert_eq!(autos(&items), [(1_571, true), (8_960, true), (21_000, false), (26_000, true)]);
        let pics: Vec<&Capture> = items
            .iter()
            .filter_map(|i| match i {
                Item::Image { capture, .. } => Some(capture),
                _ => None,
            })
            .collect();
        // Said while moving: ringed where the pointer was.
        assert_eq!(pics[0].pointer, Some(p(1_571, 47 * 7 % 200, 50)));
        let rest = pics[1];
        let at = rest.pointer.unwrap();
        assert!((at.x - 150).abs() <= 2 && at.y == 30, "{at:?}");
        assert_eq!(px(rest, (at.x + image_ops::RING_R as i32) as u32, 30), [245, 165, 36], "ringed where they rested");
        assert_eq!(px(rest, at.x as u32, 30), [255; 3]);
        assert_eq!((rest.place, rest.kind, rest.element.clone()), (Some(b.frames()[9].place), CaptureKind::Full, None));
        assert!(!rest.trail.is_empty());
        // The marker lands at the word; the words are untouched.
        assert_eq!(prompt::compose(&items).0, "put [image 1] this here. [image 2] [image 3] [image 4]");
        // Marking off: the same moments, nothing drawn on them.
        let mut plain = vec![speech(1_000, 3_000, "put this here.")];
        assert_eq!(add(&mut plain, &b, &trail, &[], MarkPoint::Off), 4);
        let Item::Image { capture: c, .. } = &plain[2] else { panic!("{:?}", autos(&plain)) };
        assert_eq!((c.t_ms, c.png.clone()), (8_960, b.frames()[9].png.clone()));
        // An empty buffer adds nothing.
        let mut none = vec![speech(1_000, 3_000, "put this here.")];
        assert_eq!(add(&mut none, &Buffer::default(), &trail, &[], MarkPoint::Ring), 0);
        assert_eq!(none.len(), 1);
    }

    #[test]
    fn with_more_moments_than_room_the_words_go_first() {
        let mut b = Buffer::default();
        for s in 0..60 {
            b.push(sized(s * 1_000, 10));
        }
        // Eight pointing words 3 s apart, and a long rest at 50 s.
        let words: Vec<Item> = (0..8).map(|i| speech(i * 3_000, i * 3_000 + 1, "this")).collect();
        let rest: Vec<Point> = (0..=60).map(|i| p(48_000 + i * 33, 20, 20)).collect();
        let mut items = words.clone();
        assert_eq!(add(&mut items, &b, &rest, &[], MarkPoint::Off), MOST);
        let got: Vec<u64> = autos(&items).into_iter().map(|a| a.0).collect();
        assert_eq!(got, [0, 3_000, 6_000, 9_000, 12_000, 15_000]);
        // With fewer words, the rest gets in.
        let mut items = words[..2].to_vec();
        add(&mut items, &b, &rest, &[], MarkPoint::Off);
        assert_eq!(autos(&items).into_iter().map(|a| a.0).collect::<Vec<_>>(), [0, 3_000, 49_980]);
    }
}
