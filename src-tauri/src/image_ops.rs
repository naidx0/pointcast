//! Crop, pointer ring, PNG encode/decode helpers used by capture and Ask/zoom.
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

/// A ring in accent amber around `(cx, cy)`, so the agent sees where the pointer is.
pub fn ring(buf: &mut [u8], width: u32, height: u32, cx: i32, cy: i32) {
    let (r_in, r_out) = (11.0f32, 14.0f32);
    for y in (cy - 15).max(0)..(cy + 16).min(height as i32) {
        for x in (cx - 15).max(0)..(cx + 16).min(width as i32) {
            let d = (((x - cx).pow(2) + (y - cy).pow(2)) as f32).sqrt();
            if (r_in..=r_out).contains(&d) {
                let i = ((y as u32 * width + x as u32) * 4) as usize;
                if i + 4 <= buf.len() {
                    buf[i..i + 4].copy_from_slice(&[245, 165, 36, 255]);
                }
            }
        }
    }
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
        let mut px = vec![0u8; 40 * 40 * 4];
        ring(&mut px, 40, 40, 20, 20);
        let amber =
            |b: &[u8], x: u32, y: u32| b[((y * 40 + x) * 4) as usize..][..3] == [245, 165, 36];
        assert!(amber(&px, 32, 20) && amber(&px, 20, 8));
        assert!(!amber(&px, 20, 20) && !amber(&px, 39, 39));
        ring(&mut px, 40, 40, 0, 39);
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
