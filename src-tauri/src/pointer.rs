//! Where the pointer went during a recording: sampled about 30 times a second into a bounded
//! trail (positions only, never pixels), so a capture can show where the person was pointing.
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// One sample. In a capture's fields the coordinates are picture pixels; in the live trail they
/// are the OS's screen coordinates (physical pixels on Windows, points on macOS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub t_ms: u64,
    pub x: i32,
    pub y: i32,
}

/// Ten minutes at 30 Hz: about 280 KB, enough to look back across any recording.
pub const CAPACITY: usize = 30 * 600;
/// The pointer counts as resting while it stays within this many pixels...
pub const DWELL_RADIUS: i32 = 12;
/// ...for at least this long.
pub const DWELL_MS: u64 = 500;
/// How much of the trail a capture keeps.
pub const TRAIL_MS: u64 = 3_000;

#[derive(Debug, Default)]
pub struct Trail {
    buf: VecDeque<Point>,
}

impl Trail {
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    /// Record a sample; a pointer that has not moved only refreshes the time of the last one
    /// when it is the start of a rest, so long rests do not fill the buffer.
    pub fn push(&mut self, p: Point) {
        if self.buf.len() >= 2 {
            let n = self.buf.len();
            let (a, b) = (self.buf[n - 2], self.buf[n - 1]);
            if (a.x, a.y) == (p.x, p.y) && (b.x, b.y) == (p.x, p.y) {
                self.buf[n - 1].t_ms = p.t_ms;
                return;
            }
        }
        if self.buf.len() == CAPACITY {
            self.buf.pop_front();
        }
        self.buf.push_back(p);
    }

    /// Samples from `t_ms` on, oldest first.
    pub fn since(&self, t_ms: u64) -> Vec<Point> {
        let start = self.buf.partition_point(|p| p.t_ms < t_ms);
        self.buf.range(start..).copied().collect()
    }

    pub fn last(&self) -> Option<Point> {
        self.buf.back().copied()
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }
}

/// A stretch in which the pointer stayed within `DWELL_RADIUS` of one spot for at least `DWELL_MS`: its middle, timed
/// at its end, and when it began.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rest {
    pub at: Point,
    pub from: u64,
}

/// Every rest in the trail, newest first.
fn rests_back(trail: &[Point]) -> impl Iterator<Item = Rest> + '_ {
    let mut end = trail.len();
    std::iter::from_fn(move || {
        while end > 0 {
            let anchor = trail[end - 1];
            let mut start = end - 1;
            while start > 0 && near(trail[start - 1], anchor) {
                start -= 1;
            }
            let span = &trail[start..end];
            end = start;
            if anchor.t_ms.saturating_sub(span[0].t_ms) >= DWELL_MS {
                let n = span.len() as i64;
                let (sx, sy) = span.iter().fold((0i64, 0i64), |(x, y), p| (x + p.x as i64, y + p.y as i64));
                return Some(Rest { at: Point { t_ms: anchor.t_ms, x: (sx / n) as i32, y: (sy / n) as i32 }, from: span[0].t_ms });
            }
        }
        None
    })
}

/// Where the person pointed: the most recent rest, its middle timed at its end.
pub fn dwell(trail: &[Point]) -> Option<Point> {
    rests_back(trail).next().map(|r| r.at)
}

/// Every rest in the trail, oldest first.
pub fn rests(trail: &[Point]) -> Vec<Rest> {
    let mut v: Vec<Rest> = rests_back(trail).collect();
    v.reverse();
    v
}

fn near(a: Point, b: Point) -> bool {
    let (dx, dy) = ((a.x - b.x) as i64, (a.y - b.y) as i64);
    dx * dx + dy * dy <= (DWELL_RADIUS as i64).pow(2)
}

/// A screen position in the picture of one monitor: `origin` is the monitor's top-left in screen
/// coordinates and `scale` is picture pixels per screen unit (1 on Windows, the Retina factor on
/// macOS, where the pointer is in points and the picture in pixels). `None` if it falls outside.
pub fn to_image(p: Point, origin: (i32, i32), scale: f64, size: (u32, u32)) -> Option<Point> {
    let x = ((p.x - origin.0) as f64 * scale).round() as i64;
    let y = ((p.y - origin.1) as f64 * scale).round() as i64;
    (x >= 0 && y >= 0 && x < size.0 as i64 && y < size.1 as i64).then_some(Point { t_ms: p.t_ms, x: x as i32, y: y as i32 })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(t_ms: u64, x: i32, y: i32) -> Point {
        Point { t_ms, x, y }
    }

    /// A trail at 30 Hz: moving from `from` to `to` over `move_ms`, then resting `rest_ms` with a
    /// little hand jitter.
    fn glide_then_rest(t0: u64, from: (i32, i32), to: (i32, i32), move_ms: u64, rest_ms: u64) -> Vec<Point> {
        let mut v = vec![];
        let steps = move_ms / 33;
        for i in 0..steps {
            let f = i as f64 / steps as f64;
            let x = from.0 + ((to.0 - from.0) as f64 * f) as i32;
            let y = from.1 + ((to.1 - from.1) as f64 * f) as i32;
            v.push(p(t0 + i * 33, x, y));
        }
        let t1 = t0 + steps * 33;
        for i in 0..=rest_ms / 33 {
            let jitter = [0, 3, -2, 4, -3][(i % 5) as usize];
            v.push(p(t1 + i * 33, to.0 + jitter, to.1 - jitter));
        }
        v
    }

    #[test]
    fn a_rest_of_half_a_second_is_where_they_pointed() {
        let t = glide_then_rest(0, (0, 0), (400, 300), 600, 700);
        let d = dwell(&t).unwrap();
        assert!((d.x - 400).abs() <= 4 && (d.y - 300).abs() <= 4, "{d:?}");
        assert_eq!(d.t_ms, t.last().unwrap().t_ms);
    }

    #[test]
    fn a_pointer_that_never_rests_has_no_dwell() {
        let t: Vec<Point> = (0..60).map(|i| p(i * 33, i as i32 * 20, 5)).collect();
        assert_eq!(dwell(&t), None);
        let short = glide_then_rest(0, (0, 0), (400, 300), 600, 300);
        assert_eq!(dwell(&short), None, "300 ms is a pause, not a point");
        assert_eq!(dwell(&[]), None);
    }

    #[test]
    fn the_latest_rest_wins_and_a_move_after_it_does_not_erase_it() {
        let mut t = glide_then_rest(0, (0, 0), (100, 100), 300, 800);
        let t2 = t.last().unwrap().t_ms + 33;
        t.extend(glide_then_rest(t2, (100, 100), (900, 500), 400, 900));
        let d = dwell(&t).unwrap();
        assert!((d.x - 900).abs() <= 4, "{d:?}");
        // Then the hand moves away briefly: the last rest still stands.
        let t3 = t.last().unwrap().t_ms;
        t.extend((1..6).map(|i| p(t3 + i * 33, 900 + i as i32 * 40, 500)));
        assert!((dwell(&t).unwrap().x - 900).abs() <= 4);
    }

    #[test]
    fn every_rest_is_found_in_time_order_and_the_last_is_the_dwell() {
        let mut t = glide_then_rest(0, (0, 0), (100, 100), 300, 800);
        let t2 = t.last().unwrap().t_ms + 33;
        t.extend(glide_then_rest(t2, (100, 100), (900, 500), 400, 300));
        let t3 = t.last().unwrap().t_ms + 33;
        t.extend(glide_then_rest(t3, (900, 500), (1500, 200), 400, 1_200));
        let r = rests(&t);
        assert_eq!(r.len(), 2, "the 300 ms pause in the middle is not a rest: {r:?}");
        assert!((r[0].at.x - 100).abs() <= 4 && (r[1].at.x - 1500).abs() <= 4, "{r:?}");
        assert!(r[0].at.t_ms - r[0].from >= DWELL_MS && r[0].at.t_ms <= t2, "{r:?}");
        assert_eq!(r[1].at.t_ms, t.last().unwrap().t_ms);
        assert_eq!(dwell(&t), Some(r[1].at));
        assert!(rests(&[]).is_empty());
    }

    #[test]
    fn screen_points_map_to_picture_pixels_on_every_kind_of_display() {
        let size = (3840, 2160);
        // Windows at 100 %: screen and picture are both physical pixels.
        assert_eq!(to_image(p(1, 960, 540), (0, 0), 1.0, size), Some(p(1, 960, 540)));
        // macOS Retina: the pointer is in points, the picture in pixels.
        assert_eq!(to_image(p(1, 960, 540), (0, 0), 2.0, size), Some(p(1, 1920, 1080)));
        // A 150 % display.
        assert_eq!(to_image(p(1, 100, 101), (0, 0), 1.5, size), Some(p(1, 150, 152)));
        // A second monitor to the left of the main one has a negative origin.
        assert_eq!(to_image(p(1, -1500, 200), (-1920, 0), 1.0, (1920, 1080)), Some(p(1, 420, 200)));
        // Off this monitor: no point in its picture.
        assert_eq!(to_image(p(1, 10, 10), (-1920, 0), 1.0, (1920, 1080)), None);
        assert_eq!(to_image(p(1, 1920, 5), (0, 0), 1.0, (1920, 1080)), None);
    }

    #[test]
    fn the_trail_is_bounded_and_a_still_pointer_does_not_fill_it() {
        let mut tr = Trail::default();
        for i in 0..(CAPACITY as u64 + 50) {
            tr.push(p(i, i as i32, 0));
        }
        assert_eq!(tr.len(), CAPACITY);
        assert_eq!(tr.since(0)[0].t_ms, 50);
        let mut still = Trail::default();
        for i in 0..1000 {
            still.push(p(i, 7, 7));
        }
        assert!(still.len() <= 2);
        assert_eq!(still.last().unwrap().t_ms, 999);
        // A rest kept as two samples still reads as a dwell.
        assert!(dwell(&still.since(0)).is_some());
    }

    #[test]
    fn since_returns_the_window_asked_for() {
        let mut tr = Trail::default();
        for i in 0..100 {
            tr.push(p(i * 100, i as i32, 0));
        }
        let w = tr.since(9_000);
        assert_eq!(w.len(), 10);
        assert_eq!(w[0].t_ms, 9_000);
        assert!(tr.since(20_000).is_empty());
    }

    #[test]
    fn old_history_without_pointer_fields_still_loads() {
        let old = r#"{"id":0,"kind":"full","t_ms":5,"png":[],"width":1,"height":1,"labels":[]}"#;
        let c: crate::prompt::Capture = serde_json::from_str(old).unwrap();
        assert_eq!((c.pointer, c.trail.len()), (None, 0));
    }
}
