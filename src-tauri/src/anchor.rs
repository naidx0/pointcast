//! Spoken anchors: people say "this" or "here" while pointing. At stop, the spot the pointer was at when they said it
//! is ringed on the picture nearest in time, even if they never paused there.
use crate::capture::Place;
use crate::image_ops;
use crate::pointer::Point;
use crate::prompt::Item;

/// The words people say while pointing.
pub const POINTING: [&str; 6] = ["this", "here", "that", "these", "those", "there"];
/// A trail sample stands for the moment of a word only this close to it.
pub const SAMPLE_MS: u64 = 300;
/// A picture is ringed for a word only when it was taken this close to it.
pub const NEAR_MS: u64 = 4_000;
/// A ring already this close, in picture pixels at density 1, makes another one there redundant.
pub const CROWD_PX: f32 = 40.0;

/// A model token and when it started, in ms from the start of its piece.
pub type Token = (String, u64);

/// When each word of a piece was said, on the session clock. With the model's token times a word starts at the token
/// that begins it (one with a leading space or `▁`); without them, or when they do not line up with the words, the
/// piece's span is shared out by character position.
pub fn word_times(t_ms: u64, end_ms: u64, text: &str, tokens: Option<&[Token]>) -> Vec<(String, u64)> {
    // Each word with the character it starts at.
    let mut words: Vec<(u64, String)> = vec![];
    let mut total = 0u64;
    let mut inside = false;
    for (i, ch) in text.chars().enumerate() {
        total += 1;
        if ch.is_whitespace() {
            inside = false;
        } else if inside {
            words.last_mut().expect("inside a word").1.push(ch);
        } else {
            inside = true;
            words.push((i as u64, ch.to_string()));
        }
    }
    if let Some(tokens) = tokens {
        let starts: Vec<u64> = tokens
            .iter()
            .enumerate()
            .filter(|(i, (s, _))| s.starts_with(['▁', ' ']) || (*i == 0 && !s.trim().is_empty()))
            .map(|(_, (_, t))| t_ms + t)
            .collect();
        if starts.len() == words.len() {
            return words.into_iter().zip(starts).map(|((_, w), t)| (w, t)).collect();
        }
    }
    let span = end_ms.saturating_sub(t_ms);
    words.into_iter().map(|(c, w)| (w, t_ms + span * c / total.max(1))).collect()
}

/// A pointing word, ignoring case and the punctuation around it: "This," and "here." are; "thistle" and "that's" are
/// not.
pub fn is_pointing(word: &str) -> bool {
    let w = word.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    POINTING.contains(&w.as_str())
}

/// Where the pointer was at `t`, in screen coordinates: the sample nearest in time within `SAMPLE_MS`, or the spot it
/// rested on when `t` falls inside a rest (the trail keeps only both ends of one).
pub fn pointer_at(trail: &[Point], t: u64) -> Option<Point> {
    let i = trail.partition_point(|p| p.t_ms < t);
    let (before, after) = (i.checked_sub(1).map(|j| trail[j]), trail.get(i).copied());
    if let (Some(a), Some(b)) = (before, after) {
        if (a.x, a.y) == (b.x, b.y) {
            return Some(Point { t_ms: t, ..a });
        }
    }
    [before, after]
        .into_iter()
        .flatten()
        .filter(|p| p.t_ms.abs_diff(t) <= SAMPLE_MS)
        .min_by_key(|p| p.t_ms.abs_diff(t))
}

/// The picture a word said at `t` belongs to: of those taken within `NEAR_MS` that `at` (screen coordinates) falls on,
/// the nearest in time. Returns its index in `pictures` and the spot in its pixels.
pub fn picture_for(pictures: &[(u64, Place)], at: Point, t: u64) -> Option<(usize, Point)> {
    pictures
        .iter()
        .enumerate()
        .filter(|(_, (taken, _))| taken.abs_diff(t) <= NEAR_MS)
        .filter_map(|(i, (taken, place))| Some((taken.abs_diff(t), i, place.map.place(at)?)))
        .min_by_key(|(d, i, _)| (*d, *i))
        .map(|(_, i, p)| (i, p))
}

/// A ring at `at` would add nothing: another ring is within `CROWD_PX` (times the picture's density `k`).
pub fn crowded(at: Point, rings: impl IntoIterator<Item = Point>, k: f32) -> bool {
    let r = CROWD_PX * k.max(1.0);
    rings.into_iter().any(|p| ((p.x - at.x) as f32).hypot((p.y - at.y) as f32) < r)
}

/// Ring, on the picture nearest in time, where the pointer was each time a pointing word was said. `trail` is the whole
/// recording's, in screen coordinates; `tokens` holds the model's token times by the start of their piece;
/// `rest_ringed` says the captures drew a ring where the person rested. Regions have no mapping (the overlay cropped
/// them) and are left alone. Returns how many rings were added.
pub fn ring_spoken(items: &mut [Item], trail: &[Point], tokens: &[(u64, Vec<Token>)], rest_ringed: bool) -> usize {
    let mut said: Vec<u64> = vec![];
    for it in items.iter() {
        if let Item::Speech { t_ms, end_ms, text } = it {
            let tok = tokens.iter().find(|(s, _)| s == t_ms).map(|(_, v)| v.as_slice());
            said.extend(word_times(*t_ms, *end_ms, text, tok).into_iter().filter(|(w, _)| is_pointing(w)).map(|(_, t)| t));
        }
    }
    let pictures: Vec<(usize, u64, Place)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| match it {
            Item::Image { t_ms, capture } => capture.place.map(|p| (i, *t_ms, p)),
            _ => None,
        })
        .collect();
    let places: Vec<(u64, Place)> = pictures.iter().map(|(_, t, p)| (*t, *p)).collect();
    let mut added: Vec<(usize, Point)> = vec![];
    for t in said {
        let Some((j, at)) = pointer_at(trail, t).and_then(|p| picture_for(&places, p, t)) else {
            continue;
        };
        let (i, _, place) = pictures[j];
        let Item::Image { capture: c, .. } = &items[i] else { continue };
        let rest = c.pointer.filter(|_| rest_ringed);
        let mine = added.iter().filter(|(k, _)| *k == i).map(|(_, p)| *p);
        if crowded(at, rest.into_iter().chain(c.anchors.iter().copied()).chain(mine), place.k) {
            continue;
        }
        added.push((i, Point { t_ms: t, ..at }));
    }
    let mut n = 0;
    for (i, it) in items.iter_mut().enumerate() {
        let rings: Vec<Point> = added.iter().filter(|(k, _)| *k == i).map(|(_, p)| *p).collect();
        let Item::Image { capture: c, .. } = it else { continue };
        if rings.is_empty() {
            continue;
        }
        let k = c.place.map_or(1.0, |p| p.k);
        let Ok((mut buf, w, h)) = image_ops::decode_rgba(&c.png) else { continue };
        for p in &rings {
            image_ops::ring(&mut buf, w, h, p.x, p.y, k);
        }
        let Ok(png) = image_ops::encode_png(&buf, w, h) else { continue };
        c.png = png;
        n += rings.len();
        c.anchors.extend(rings);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::Mapping;
    use crate::prompt::{Capture, CaptureKind};

    fn p(t_ms: u64, x: i32, y: i32) -> Point {
        Point { t_ms, x, y }
    }
    fn tok(s: &str, t: u64) -> Token {
        (s.to_string(), t)
    }
    fn screen(origin: (i32, i32), w: u32, h: u32, k: f32) -> Place {
        Place { map: Mapping { origin, scale: 1.0, frame: (w, h), crop: (0, 0), size: (w, h) }, k }
    }
    fn times(v: &[(String, u64)]) -> Vec<u64> {
        v.iter().map(|(_, t)| *t).collect()
    }

    #[test]
    fn without_token_times_a_word_is_timed_by_where_it_starts_in_the_piece() {
        // 20 characters over 2 s: 100 ms a character.
        let w = word_times(1_000, 3_000, "move this one there.", None);
        let words: Vec<&str> = w.iter().map(|(w, _)| w.as_str()).collect();
        assert_eq!(words, ["move", "this", "one", "there."]);
        assert_eq!(times(&w), [1_000, 1_500, 2_000, 2_400]);
        assert!(word_times(0, 500, "   ", None).is_empty());
        // A piece with no length puts every word at its start.
        assert_eq!(times(&word_times(700, 700, "right here", None)), [700, 700]);
    }

    #[test]
    fn with_token_times_a_word_starts_at_the_token_that_begins_it() {
        let toks = [tok("▁move", 40), tok("▁th", 900), tok("is", 980), tok("▁one", 1_300), tok(".", 1_500)];
        let w = word_times(1_000, 3_000, "move this one.", Some(&toks));
        assert_eq!(times(&w), [1_040, 1_900, 2_300]);
        // Whisper marks a word with a space, and a first token may have no mark at all.
        let toks = [tok("Put", 0), tok(" it", 200), tok(" here", 760)];
        assert_eq!(times(&word_times(5_000, 6_000, "Put it here", Some(&toks))), [5_000, 5_200, 5_760]);
        // Tokens that do not line up with the words fall back to the estimate.
        let odd = [tok("▁move", 40), tok("▁this", 900)];
        assert_eq!(word_times(1_000, 3_000, "move this one.", Some(&odd)), word_times(1_000, 3_000, "move this one.", None));
        assert_eq!(word_times(1_000, 3_000, "move this one.", Some(&[])), word_times(1_000, 3_000, "move this one.", None));
    }

    #[test]
    fn pointing_words_are_whole_words_in_any_case_with_punctuation_around_them() {
        for w in ["this", "This,", "HERE.", "that", "these", "those?", "there!", "“this”", "(here)"] {
            assert!(is_pointing(w), "{w}");
        }
        for w in ["thistle", "that's", "there's", "where", "the", "hear", "", "..."] {
            assert!(!is_pointing(w), "{w}");
        }
    }

    #[test]
    fn the_pointer_at_a_word_is_the_nearest_sample_within_300_ms() {
        let trail = [p(1_000, 10, 10), p(1_033, 40, 10), p(2_000, 900, 500)];
        assert_eq!(pointer_at(&trail, 1_040), Some(p(1_033, 40, 10)));
        assert_eq!(pointer_at(&trail, 1_010), Some(p(1_000, 10, 10)));
        assert_eq!(pointer_at(&trail, 1_900), Some(p(2_000, 900, 500)));
        assert_eq!(pointer_at(&trail, 2_300), Some(p(2_000, 900, 500)), "300 ms is still close enough");
        assert_eq!(pointer_at(&trail, 2_301), None);
        assert_eq!(pointer_at(&trail, 1_500), None, "a gap while moving is too far from either side");
        assert_eq!(pointer_at(&trail, 600), None);
        assert_eq!(pointer_at(&[], 600), None);
    }

    #[test]
    fn a_pointer_resting_through_the_word_is_found_though_the_trail_kept_only_both_ends() {
        let mut tr = crate::pointer::Trail::default();
        tr.push(p(0, 5, 5));
        for t in (1_000..=6_000).step_by(33) {
            tr.push(p(t, 700, 450));
        }
        let trail = tr.since(0);
        assert!(trail.len() <= 3, "{trail:?}");
        assert_eq!(pointer_at(&trail, 3_500), Some(p(3_500, 700, 450)));
    }

    #[test]
    fn a_word_belongs_to_the_nearest_picture_within_4_s_that_shows_where_it_pointed() {
        let left = screen((-1920, 0), 1920, 1080, 1.0);
        let main = screen((0, 0), 2560, 1440, 1.0);
        let pics = [(1_000, main), (6_000, left), (6_500, main), (9_000, main)];
        assert_eq!(picture_for(&pics, p(6_200, 300, 200), 6_200), Some((2, p(6_200, 300, 200))));
        // On the monitor to the left: the main monitor's nearer pictures are passed over.
        assert_eq!(picture_for(&pics, p(6_400, -1500, 200), 6_400), Some((1, p(6_400, 420, 200))));
        let one = [(1_000, main)];
        assert_eq!(picture_for(&one, p(4_900, 300, 200), 4_900), Some((0, p(4_900, 300, 200))), "3.9 s after is near");
        assert_eq!(picture_for(&one, p(5_100, 300, 200), 5_100), None, "4.1 s after is not");
        assert_eq!(picture_for(&pics, p(1_900, -1500, 200), 1_900), None, "4.1 s from the left picture");
        assert_eq!(picture_for(&pics, p(1, 4000, 200), 6_200), None, "on no picture");
        // A close-up shows only its crop.
        let zoom = Place { map: Mapping { crop: (1000, 600), size: (512, 512), ..main.map }.times(2), k: 2.0 };
        assert_eq!(picture_for(&[(5_000, zoom)], p(1, 1100, 700), 5_000), Some((0, p(1, 200, 200))));
        assert_eq!(picture_for(&[(5_000, zoom)], p(1, 900, 700), 5_000), None);
    }

    #[test]
    fn a_ring_is_left_out_where_one_already_is() {
        let at = p(0, 500, 300);
        assert!(crowded(at, [p(0, 530, 320)], 1.0));
        assert!(!crowded(at, [p(0, 541, 300)], 1.0));
        assert!(crowded(at, [p(0, 560, 300)], 2.0), "at density 2 the rings are twice as big");
        assert!(!crowded(at, Vec::<Point>::new(), 1.0));
    }

    fn picture(t: u64, place: Option<Place>, pointer: Option<Point>) -> Item {
        let (w, h) = place.map_or((64, 64), |p| p.map.size);
        Item::Image {
            t_ms: t,
            capture: Capture {
                id: 0,
                kind: if place.is_some() { CaptureKind::Full } else { CaptureKind::Region },
                t_ms: t,
                png: image_ops::encode_png(&vec![255u8; (w * h * 4) as usize], w, h).unwrap(),
                width: w,
                height: h,
                labels: vec![],
                pointer,
                trail: vec![],
                element: None,
                place,
                anchors: vec![],
            },
        }
    }
    fn speech(t: u64, end: u64, text: &str) -> Item {
        Item::Speech { t_ms: t, end_ms: end, text: text.into() }
    }
    fn amber_at(it: &Item, x: i32, y: i32) -> bool {
        let Item::Image { capture: c, .. } = it else { panic!() };
        let (buf, w, _) = image_ops::decode_rgba(&c.png).unwrap();
        let i = ((y as u32 * w + x as u32) * 4) as usize;
        buf[i..i + 3] == [245, 165, 36]
    }
    fn anchors(it: &Item) -> Vec<(i32, i32)> {
        let Item::Image { capture: c, .. } = it else { panic!() };
        c.anchors.iter().map(|p| (p.x, p.y)).collect()
    }

    #[test]
    fn saying_this_rings_where_the_pointer_was_on_the_nearest_picture() {
        let place = screen((0, 0), 400, 300, 1.0);
        // "click this button": "this" at 1 s + 6/17 of 1.7 s = 1600 ms, while the pointer sweeps past (100, 80).
        let mut items = vec![speech(1_000, 2_700, "click this button"), picture(3_000, Some(place), None)];
        let trail: Vec<Point> = (0..15).map(|i| p(1_000 + i * 100, 40 + i as i32 * 10, 80)).collect();
        assert_eq!(pointer_at(&trail, 1_600).map(|q| (q.x, q.y)), Some((100, 80)));
        assert_eq!(ring_spoken(&mut items, &trail, &[], true), 1);
        assert_eq!(anchors(&items[1]), [(100, 80)]);
        let r = image_ops::RING_R as i32;
        assert!(amber_at(&items[1], 100 + r, 80) && !amber_at(&items[1], 100, 80));
        // Done again, nothing more is drawn: the ring it added is already there.
        let before = items.clone();
        assert_eq!(ring_spoken(&mut items, &trail, &[], true), 0);
        assert_eq!(items, before);
    }

    #[test]
    fn token_times_move_the_ring_to_where_the_pointer_was_when_the_word_was_said() {
        let place = screen((0, 0), 400, 300, 1.0);
        let trail: Vec<Point> = (0..60).map(|i| p(i * 33, 10 + i as i32 * 6, 150)).collect();
        let toks = vec![(0, vec![tok("▁click", 0), tok("▁this", 330), tok("▁button", 700)])];
        let mut items = vec![speech(0, 1_700, "click this button"), picture(1_900, Some(place), None)];
        assert_eq!(ring_spoken(&mut items, &trail, &toks, true), 1);
        assert_eq!(anchors(&items[1]), [(70, 150)], "the sample at 330 ms, not the estimate's at 600 ms");
    }

    #[test]
    fn a_spoken_ring_beside_the_rest_ring_or_on_a_region_is_left_out() {
        let place = screen((0, 0), 400, 300, 1.0);
        let still: Vec<Point> = vec![p(0, 200, 120), p(3_000, 200, 120)];
        let rest = Some(p(900, 210, 130));
        let mut items = vec![speech(0, 1_000, "here"), picture(1_000, Some(place), rest)];
        assert_eq!(ring_spoken(&mut items, &still, &[], true), 0, "the rest ring is already there");
        // With rest rings off, nothing was drawn there: the spoken one is.
        assert_eq!(ring_spoken(&mut items, &still, &[], false), 1);
        // A region has no mapping and is never drawn on.
        let mut region = vec![speech(0, 1_000, "here"), picture(1_000, None, None)];
        let before = region.clone();
        assert_eq!(ring_spoken(&mut region, &still, &[], true), 0);
        assert_eq!(region, before);
    }

    /// A sample picture for a person to look at: a made-up screen with the rest ring drawn at capture and a spoken ring
    /// where the pointer passed while "this" was said. `POINTCAST_SHOTS=<dir> cargo test -- --ignored sample_picture`
    #[test]
    #[ignore]
    fn sample_picture() {
        let dir = std::path::PathBuf::from(std::env::var("POINTCAST_SHOTS").expect("set POINTCAST_SHOTS"));
        let (w, h) = (960u32, 540u32);
        let mut buf = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let c: [u8; 3] = match (x, y) {
                    (_, 0..=47) => [32, 33, 38],
                    (0..=219, _) => [236, 236, 240],
                    (480.., _) => [24, 24, 28],
                    _ => [250, 250, 250],
                };
                let button = |x0: u32, y0: u32| x >= x0 && x < x0 + 120 && y >= y0 && y < y0 + 36;
                let c = if button(260, 120) || button(260, 200) { [37, 99, 235] } else if button(620, 120) { [63, 63, 70] } else { c };
                buf[((y * w + x) * 4) as usize..][..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        // The person rested on the dark panel's button, then swept up through the first blue one saying "this".
        let rest = p(3_800, 680, 138);
        crate::capture::mark(&mut buf, w, h, crate::settings::MarkPoint::Ring, Some(rest), &[], 1.0);
        let place = screen((0, 0), w, h, 1.0);
        let mut items = vec![
            speech(1_000, 3_000, "make this one match the other"),
            Item::Image {
                t_ms: 4_000,
                capture: Capture {
                    id: 0,
                    kind: CaptureKind::Full,
                    t_ms: 4_000,
                    png: image_ops::encode_png(&buf, w, h).unwrap(),
                    width: w,
                    height: h,
                    labels: vec![],
                    pointer: Some(rest),
                    trail: vec![],
                    element: None,
                    place: Some(place),
                    anchors: vec![],
                },
            },
        ];
        // "this" is said at about 1.34 s, when the pointer is at (320, 138).
        let trail: Vec<Point> = (0..20).map(|i| p(1_000 + i * 50, 250 + i as i32 * 10, 180 - i as i32 * 6)).collect();
        assert_eq!(ring_spoken(&mut items, &trail, &[], true), 1);
        let Item::Image { capture: c, .. } = &items[1] else { panic!() };
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("spoken-anchor-sample.png"), &c.png).unwrap();
        eprintln!("spoken ring at {:?}, rest ring at {:?}", c.anchors, rest);
    }

    #[test]
    fn words_far_from_any_picture_or_sample_ring_nothing() {
        let place = screen((0, 0), 400, 300, 1.0);
        let trail = vec![p(500, 100, 100), p(533, 140, 100)];
        // "this" at 500 ms, the picture 4.5 s later.
        let mut late = vec![speech(0, 1_000, "put this"), picture(5_000, Some(place), None)];
        assert_eq!(ring_spoken(&mut late, &trail, &[], true), 0);
        // "this" at 2.5 s, the last sample 2 s before.
        let mut lost = vec![speech(2_000, 3_000, "put this"), picture(3_000, Some(place), None)];
        assert_eq!(ring_spoken(&mut lost, &trail, &[], true), 0);
        // No pointing word at all.
        let mut plain = vec![speech(0, 1_000, "make it blue"), picture(1_000, Some(place), None)];
        assert_eq!(ring_spoken(&mut plain, &trail, &[], true), 0);
    }
}
