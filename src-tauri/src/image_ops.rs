//! Crop, pointer ring and trail, enlarge, PNG encode/decode helpers used by capture and Ask/zoom.
use crate::pointer::Point;
use crate::privacy::Rect;

/// A `cw` by `ch` cut of an RGBA picture, centred on `(cx, cy)` and moved inside the picture at its edges.
pub fn crop(
    buf: &[u8],
    width: u32,
    height: u32,
    cx: i32,
    cy: i32,
    cw: u32,
    ch: u32,
) -> (Vec<u8>, u32, u32) {
    let cw = cw.min(width).max(1);
    let ch = ch.min(height).max(1);
    let ox = (cx - cw as i32 / 2).clamp(0, (width - cw) as i32) as u32;
    let oy = (cy - ch as i32 / 2).clamp(0, (height - ch) as i32) as u32;
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in oy..oy + ch {
        let i = ((y * width + ox) * 4) as usize;
        out.extend_from_slice(&buf[i..i + (cw * 4) as usize]);
    }
    (out, ox, oy)
}

/// A region cut of an RGBA picture. The rect is clipped to the image.
pub fn crop_rect(buf: &[u8], width: u32, height: u32, r: Rect) -> (Vec<u8>, u32, u32, u32, u32) {
    let (w, h) = (width as i32, height as i32);
    let x0 = r.x.clamp(0, w);
    let y0 = r.y.clamp(0, h);
    let x1 = r.x.saturating_add(r.w).clamp(0, w);
    let y1 = r.y.saturating_add(r.h).clamp(0, h);
    let cw = (x1 - x0).max(1) as u32;
    let ch = (y1 - y0).max(1) as u32;
    let (ox, oy) = (x0 as u32, y0 as u32);
    let mut out = Vec::with_capacity((cw * ch * 4) as usize);
    for y in oy..oy + ch {
        let i = ((y * width + ox) * 4) as usize;
        out.extend_from_slice(&buf[i..i + (cw * 4) as usize]);
    }
    (out, ox, oy, cw, ch)
}

const AMBER: [u8; 3] = [245, 165, 36];
const SHADE: [u8; 3] = [16, 16, 20];
/// Ring radius at density 1; at density `k` every size is `k` times as big.
pub const RING_R: f32 = 16.0;
/// Close-ups smaller than this on their long side are enlarged when sharpening is on.
pub const SHARPEN_BELOW: u32 = 1024;

/// How much of a band `hw` either side of a line covers a pixel `d` from it: 1 inside, a one-pixel ramp at the edge.
fn cover(d: f32, hw: f32) -> f32 {
    (hw - d + 0.5).clamp(0.0, 1.0)
}

fn blend(buf: &mut [u8], width: u32, x: i32, y: i32, c: [u8; 3], a: f32) {
    if a <= 0.0 {
        return;
    }
    let i = ((y as u32 * width + x as u32) * 4) as usize;
    if let Some(px) = buf.get_mut(i..i + 3) {
        for (v, c) in px.iter_mut().zip(c) {
            *v = (c as f32 * a + *v as f32 * (1.0 - a)).round() as u8;
        }
    }
}

/// A ring around `(cx, cy)` so the agent sees where the person pointed: amber on a soft dark edge, so it reads on light
/// and dark screens alike. `k` is the picture's density (2 on a Retina display or after a 2x enlarge).
pub fn ring(buf: &mut [u8], width: u32, height: u32, cx: i32, cy: i32, k: f32) {
    let (r, hw, edge) = (RING_R * k, 1.75 * k, 1.25 * k);
    let reach = (r + hw + edge + 1.0).ceil() as i32;
    for y in (cy - reach).max(0)..(cy + reach + 1).min(height as i32) {
        for x in (cx - reach).max(0)..(cx + reach + 1).min(width as i32) {
            let off = ((((x - cx).pow(2) + (y - cy).pow(2)) as f32).sqrt() - r).abs();
            blend(buf, width, x, y, SHADE, 0.6 * cover(off, hw + edge));
            blend(buf, width, x, y, AMBER, cover(off, hw));
        }
    }
}

/// The path the pointer took to `end`, as a thin amber line that fades in with time and stops inside the ring drawn
/// there. Points are picture pixels, oldest first.
pub fn trail(buf: &mut [u8], width: u32, height: u32, pts: &[Point], end: Point, k: f32) {
    let mut path: Vec<Point> = vec![];
    for p in pts.iter().chain([&end]) {
        match path.last() {
            Some(l) if (((p.x - l.x).pow(2) + (p.y - l.y).pow(2)) as f32) < (1.5 * k).powi(2) => {}
            _ => path.push(*p),
        }
    }
    if path.len() < 2 {
        return;
    }
    let (t0, t1) = (path[0].t_ms, end.t_ms.max(path[0].t_ms + 1));
    let fade = |t: u64| 0.15 + 0.85 * (t.saturating_sub(t0) as f32 / (t1 - t0) as f32).min(1.0);
    let (hw, edge, hole) = (0.9 * k, 1.0 * k, (RING_R - 1.75) * k);
    let reach = (hw + edge + 1.0).ceil() as i32;
    // The strongest cover per pixel, so the joins between segments do not darken.
    let mut ink: std::collections::HashMap<(i32, i32), (f32, f32)> = Default::default();
    for s in path.windows(2) {
        let (a, b) = (s[0], s[1]);
        let (dx, dy) = ((b.x - a.x) as f32, (b.y - a.y) as f32);
        let len2 = (dx * dx + dy * dy).max(1e-6);
        for y in (a.y.min(b.y) - reach).max(0)..(a.y.max(b.y) + reach + 1).min(height as i32) {
            for x in (a.x.min(b.x) - reach).max(0)..(a.x.max(b.x) + reach + 1).min(width as i32) {
                if (((x - end.x).pow(2) + (y - end.y).pow(2)) as f32).sqrt() < hole {
                    continue;
                }
                let u = (((x - a.x) as f32 * dx + (y - a.y) as f32 * dy) / len2).clamp(0.0, 1.0);
                let d = ((x - a.x) as f32 - u * dx).hypot((y - a.y) as f32 - u * dy);
                let f = fade(a.t_ms) + (fade(b.t_ms) - fade(a.t_ms)) * u;
                let e = ink.entry((x, y)).or_default();
                e.0 = e.0.max(0.45 * f * cover(d, hw + edge));
                e.1 = e.1.max(f * cover(d, hw));
            }
        }
    }
    for ((x, y), (dark, amber)) in ink {
        blend(buf, width, x, y, SHADE, dark);
        blend(buf, width, x, y, AMBER, amber);
    }
}

/// 2 when sharpening is on and a close-up is smaller than `SHARPEN_BELOW` on its long side, else 1.
pub fn sharpen_factor(on: bool, width: u32, height: u32) -> u32 {
    if on && width.max(height) < SHARPEN_BELOW {
        2
    } else {
        1
    }
}

/// An RGBA picture enlarged `n` times with Lanczos, which keeps small text crisp.
pub fn enlarge(buf: Vec<u8>, width: u32, height: u32, n: u32) -> (Vec<u8>, u32, u32) {
    if n <= 1 || buf.len() != (width * height * 4) as usize {
        return (buf, width, height);
    }
    let img = image::RgbaImage::from_raw(width, height, buf).expect("length checked");
    let (w, h) = (width * n, height * n);
    (image::imageops::resize(&img, w, h, image::imageops::FilterType::Lanczos3).into_raw(), w, h)
}

/// The width and height a PNG's header states (IHDR).
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24
        || png[..8] != [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]
        || &png[12..16] != b"IHDR"
    {
        return None;
    }
    let w = u32::from_be_bytes(png[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(png[20..24].try_into().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

pub fn encode_png(buf: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png = Vec::with_capacity(buf.len() / 4);
    {
        use image::ImageEncoder;
        image::codecs::png::PngEncoder::new_with_quality(
            &mut png,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Sub,
        )
        .write_image(buf, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    }
    Ok(png)
}

pub fn decode_rgba(png: &[u8]) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(png).map_err(|e| e.to_string())?.to_rgba8();
    let (w, h) = img.dimensions();
    Ok((img.into_raw(), w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_size_reads_the_header_and_refuses_anything_else() {
        let mut p = vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        p.extend_from_slice(&640u32.to_be_bytes());
        p.extend_from_slice(&360u32.to_be_bytes());
        assert_eq!(png_size(&p), Some((640, 360)));
        assert_eq!(png_size(&p[..20]), None);
        let mut bad = p.clone();
        bad[1] = b'X';
        assert_eq!(png_size(&bad), None);
        let mut chunk = p.clone();
        chunk[12] = b'X';
        assert_eq!(png_size(&chunk), None);
        let mut zero = p.clone();
        zero[16..20].copy_from_slice(&0u32.to_be_bytes());
        assert_eq!(png_size(&zero), None);
    }

    #[test]
    fn the_zoom_crop_centres_on_the_pointer_and_stays_inside_the_picture() {
        let (w, h) = (10u32, 6u32);
        let buf: Vec<u8> = (0..h)
            .flat_map(|y| (0..w).flat_map(move |x| [x as u8, y as u8, 0, 255]))
            .collect();
        let at = |b: &[u8], cw: u32, x: u32, y: u32| {
            (
                b[((y * cw + x) * 4) as usize],
                b[((y * cw + x) * 4 + 1) as usize],
            )
        };
        let (c, ox, oy) = crop(&buf, w, h, 5, 3, 4, 2);
        assert_eq!((ox, oy, c.len()), (3, 2, 4 * 2 * 4));
        assert_eq!(at(&c, 4, 0, 0), (3, 2));
        assert_eq!(crop(&buf, w, h, 0, 0, 4, 2).1, 0);
        let (c, ox, oy) = crop(&buf, w, h, 99, 99, 4, 2);
        assert_eq!((ox, oy), (6, 4));
        assert_eq!(at(&c, 4, 3, 1), (9, 5));
        assert_eq!(crop(&buf, w, h, -5, 2, w, h).0, buf);
    }

    fn canvas(w: u32, h: u32, v: u8) -> Vec<u8> {
        (0..w * h).flat_map(|_| [v, v, v, 255]).collect()
    }
    fn px(b: &[u8], w: u32, x: u32, y: u32) -> [u8; 3] {
        b[((y * w + x) * 4) as usize..][..3].try_into().unwrap()
    }

    #[test]
    fn the_ring_lands_on_the_pointer_reads_on_light_and_dark_and_clips_at_the_edge() {
        let mut white = canvas(64, 64, 255);
        ring(&mut white, 64, 64, 30, 30, 1.0);
        let r = RING_R as u32;
        assert_eq!(px(&white, 64, 30 + r, 30), AMBER);
        assert_eq!(px(&white, 64, 30, 30 - r), AMBER);
        assert_eq!(px(&white, 64, 30, 30), [255; 3], "the spot itself stays clear");
        assert_eq!(px(&white, 64, 63, 63), [255; 3]);
        // A dark edge just outside the amber, so it still shows on white.
        let rim = px(&white, 64, 30 + r + 2, 30);
        assert!(rim[0] < 200, "{rim:?}");
        // Anti-aliased: the outer edge ramps instead of stepping.
        let edge = px(&white, 64, 30 + r + 3, 30)[0];
        assert!(edge > rim[0] && edge < 255, "{edge}");
        // At 2x density it is twice the size.
        let mut retina = canvas(96, 96, 0);
        ring(&mut retina, 96, 96, 48, 48, 2.0);
        assert_eq!(px(&retina, 96, 48 + 2 * r, 48), AMBER);
        assert_eq!(px(&retina, 96, 48 + r, 48), [0; 3]);
        // Near a corner only the part inside is drawn.
        let mut corner = canvas(40, 40, 255);
        ring(&mut corner, 40, 40, 2, 38, 1.0);
        assert_eq!(px(&corner, 40, 2 + r, 38), AMBER);
        ring(&mut corner, 40, 40, -30, 90, 1.0);
        ring(&mut corner, 40, 40, 0, 0, 3.0);
    }

    #[test]
    fn the_trail_fades_in_toward_the_ring_and_leaves_its_centre_clear() {
        let mut b = canvas(200, 40, 255);
        let pts: Vec<Point> = (0..10).map(|i| Point { t_ms: i * 300, x: 10 + i as i32 * 15, y: 20 }).collect();
        let end = Point { t_ms: 3_000, x: 170, y: 20 };
        trail(&mut b, 200, 40, &pts, end, 1.0);
        let (old, new) = (px(&b, 200, 12, 20), px(&b, 200, 140, 20));
        assert!(old != [255; 3] && new != [255; 3]);
        assert!(new[2] < old[2], "newer is stronger: {old:?} {new:?}");
        assert_eq!(px(&b, 200, 165, 20), [255; 3], "nothing inside the ring");
        assert_eq!(px(&b, 200, 100, 15), [255; 3], "thin");
        trail(&mut b, 200, 40, &[], end, 1.0);
    }

    #[test]
    fn close_ups_are_enlarged_only_below_the_threshold() {
        assert_eq!(sharpen_factor(true, 512, 512), 2);
        assert_eq!(sharpen_factor(true, 1023, 300), 2);
        assert_eq!(sharpen_factor(true, 1024, 300), 1);
        assert_eq!(sharpen_factor(true, 300, 1600), 1);
        assert_eq!(sharpen_factor(false, 256, 256), 1);
        let (big, w, h) = enlarge(canvas(6, 4, 90), 6, 4, 2);
        assert_eq!((w, h, big.len()), (12, 8, 12 * 8 * 4));
        assert_eq!(px(&big, 12, 5, 3), [90; 3]);
        let (same, w, h) = enlarge(canvas(6, 4, 90), 6, 4, 1);
        assert_eq!((w, h, same.len()), (6, 4, 6 * 4 * 4));
    }

    #[test]
    fn a_region_crop_clips_to_the_picture() {
        let (w, h) = (8u32, 6u32);
        let buf: Vec<u8> = vec![9u8; (w * h * 4) as usize];
        let (_c, ox, oy, cw, ch) = crop_rect(
            &buf,
            w,
            h,
            Rect {
                x: -2,
                y: 1,
                w: 5,
                h: 20,
            },
        );
        assert_eq!((ox, oy, cw, ch), (0, 1, 3, 5));
    }

    #[test]
    fn encode_then_decode_round_trips_size() {
        let buf = vec![10u8, 20, 30, 255, 1, 2, 3, 255];
        let png = encode_png(&buf, 2, 1).unwrap();
        assert_eq!(png_size(&png), Some((2, 1)));
        let (raw, w, h) = decode_rgba(&png).unwrap();
        assert_eq!((w, h), (2, 1));
        assert_eq!(raw.len(), 8);
    }
}
