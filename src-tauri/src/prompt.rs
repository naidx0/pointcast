//! Assemble a Pointcast session — transcript plus captures — into pasteable prompts.
//!
//! Markers land at the moment a capture was taken: inside the words being spoken then,
//! or between speech segments. Captures are numbered in time order as `[image n]`.
//! Labels from pins ride along on the marker.
use serde::{Deserialize, Serialize};
use std::path::Path;

/// How the picture was taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureKind {
    Full,
    Region,
    Zoom,
}

impl CaptureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            CaptureKind::Full => "full",
            CaptureKind::Region => "region",
            CaptureKind::Zoom => "zoom",
        }
    }
}

/// A numbered pin or typed note on a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub n: u32,
    pub text: String,
}

/// One screenshot held with the session (PNG bytes stay with it until export).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capture {
    pub id: u32,
    pub kind: CaptureKind,
    pub t_ms: u64,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub labels: Vec<Label>,
}

/// A session item: speech from `t_ms` to `end_ms`, or a picture taken at `t_ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Speech {
        t_ms: u64,
        end_ms: u64,
        text: String,
    },
    Image {
        t_ms: u64,
        capture: Capture,
    },
}

impl Item {
    pub fn t(&self) -> u64 {
        match self {
            Item::Speech { t_ms, .. } | Item::Image { t_ms, .. } => *t_ms,
        }
    }
}

/// Put pieces and pictures in time order. At the same instant, words come before the picture
/// (the person was speaking, then hit the capture key).
pub fn ordered(mut v: Vec<Item>) -> Vec<Item> {
    v.sort_by_key(|i| (i.t(), matches!(i, Item::Image { .. })));
    v
}

fn marker(n: u32, labels: &[Label]) -> String {
    if labels.is_empty() {
        return format!("[image {n}]");
    }
    let bits: Vec<String> = labels
        .iter()
        .map(|l| {
            let t = l.text.trim();
            if t.is_empty() {
                format!("{}", l.n)
            } else {
                format!("{}={}", l.n, t)
            }
        })
        .collect();
    format!("[image {n}: {}]", bits.join("; "))
}

/// The transcript with each picture as `[image n]` at the second it was taken: inside the
/// piece being spoken then, at the word that far through it, or between pieces. Images are
/// numbered in time order. Returns the text and the pictures in that order (ids rewritten 1..n).
pub fn compose(items: &[Item]) -> (String, Vec<Capture>) {
    let mut said: Vec<(u64, u64, Vec<String>)> = vec![];
    let mut shots: Vec<(u64, Capture)> = vec![];
    for i in items {
        match i {
            Item::Speech { t_ms, end_ms, text } => said.push((
                *t_ms,
                (*end_ms).max(*t_ms),
                text.split_whitespace().map(String::from).collect(),
            )),
            Item::Image { t_ms, capture } => shots.push((*t_ms, capture.clone())),
        }
    }
    said.sort_by_key(|x| x.0);
    shots.sort_by_key(|x| x.0);

    let mut at: Vec<(usize, usize)> = vec![];
    for (t, _) in &shots {
        let place = match said.iter().position(|(a, b, _)| *t >= *a && *t < *b) {
            Some(k) => {
                let (a, b, w) = &said[k];
                let f = (*t - *a) as f64 / (*b - *a).max(1) as f64;
                (k, ((f * w.len() as f64).round() as usize).min(w.len()))
            }
            None => match said.iter().rposition(|(a, _, _)| *a <= *t) {
                Some(k) => (k, said[k].2.len()),
                None => (0, 0),
            },
        };
        at.push(place);
    }

    let mut out: Vec<String> = vec![];
    let mut n = 0u32;
    let mut next = 0usize;
    let put = |k: usize, w: usize, n: &mut u32, next: &mut usize, out: &mut Vec<String>| {
        while *next < at.len() && at[*next] == (k, w) {
            *n += 1;
            out.push(marker(*n, &shots[*next].1.labels));
            *next += 1;
        }
    };
    if said.is_empty() {
        put(0, 0, &mut n, &mut next, &mut out);
    }
    for (k, (_, _, words)) in said.iter().enumerate() {
        for (w, word) in words.iter().enumerate() {
            put(k, w, &mut n, &mut next, &mut out);
            out.push(word.clone());
        }
        put(k, words.len(), &mut n, &mut next, &mut out);
    }

    let captures: Vec<Capture> = shots
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut c))| {
            c.id = (i as u32) + 1;
            c
        })
        .collect();
    (out.join(" "), captures)
}

/// `m:ss` for a walkthrough stamp.
pub fn stamp(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

fn png_data_url(png: &[u8]) -> String {
    format!("data:image/png;base64,{}", base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        png,
    ))
}

fn filename(n: u32, kind: CaptureKind) -> String {
    format!("{n:02}-{}.png", kind.as_str())
}

/// Markdown with images inlined at the capture moment, pointing at `images/NN-kind.png`.
pub fn assemble_markdown(items: &[Item]) -> (String, Vec<Capture>) {
    let (plain, captures) = compose(items);
    if plain.is_empty() {
        return (String::new(), captures);
    }
    let mut md = plain;
    for c in &captures {
        let needle = marker(c.id, &c.labels);
        let img = format!(
            "![{}](images/{})",
            escape_md_alt(&needle),
            filename(c.id, c.kind)
        );
        md = md.replacen(&needle, &img, 1);
    }
    (md, captures)
}

fn escape_md_alt(s: &str) -> String {
    s.replace('[', "").replace(']', "")
}

/// HTML fragment with inline `data:` images, plus a plain-text fallback (the `[image n]` form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardPayload {
    pub html: String,
    pub plain: String,
}

pub fn assemble_clipboard(items: &[Item]) -> ClipboardPayload {
    let (plain, captures) = compose(items);
    if plain.is_empty() {
        return ClipboardPayload {
            html: String::new(),
            plain: String::new(),
        };
    }
    let mut html_body = escape_html(&plain);
    for c in &captures {
        let needle = escape_html(&marker(c.id, &c.labels));
        let img = format!(
            "<img src=\"{}\" alt=\"{}\" width=\"{}\" height=\"{}\" style=\"max-width:100%;height:auto;vertical-align:middle;border:1px solid #d4d4d8;border-radius:6px;\" />",
            png_data_url(&c.png),
            escape_md_alt(&marker(c.id, &c.labels)),
            c.width.max(1),
            c.height.max(1)
        );
        html_body = html_body.replacen(&needle, &img, 1);
    }
    let html = format!(
        "<html><body><div style=\"font-family:ui-sans-serif,system-ui,sans-serif;line-height:1.5;white-space:pre-wrap;\">{html_body}</div></body></html>"
    );
    ClipboardPayload { html, plain }
}

/// Write `prompt.md` and `images/NN-kind.png` under `dir`. Returns the markdown path.
pub fn export_folder(items: &[Item], dir: &Path) -> Result<std::path::PathBuf, String> {
    let (md, captures) = assemble_markdown(items);
    let images = dir.join("images");
    std::fs::create_dir_all(&images).map_err(|e| e.to_string())?;
    for c in &captures {
        let path = images.join(filename(c.id, c.kind));
        std::fs::write(&path, &c.png).map_err(|e| e.to_string())?;
    }
    let prompt = dir.join("prompt.md");
    let mut body = String::new();
    if !captures.is_empty() {
        body.push_str(&format!(
            "<!-- {} image{} captured with Pointcast -->\n\n",
            captures.len(),
            if captures.len() == 1 { "" } else { "s" }
        ));
    }
    body.push_str(&md);
    if !md.ends_with('\n') {
        body.push('\n');
    }
    std::fs::write(&prompt, body).map_err(|e| e.to_string())?;
    Ok(prompt)
}

/// JSON the MCP / REST endpoint returns.
#[derive(Debug, Clone, Serialize)]
pub struct SessionExport {
    pub plain: String,
    pub markdown: String,
    pub html: String,
    pub images: Vec<SessionImage>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionImage {
    pub n: u32,
    pub kind: CaptureKind,
    pub width: u32,
    pub height: u32,
    pub labels: Vec<Label>,
    pub data_url: String,
}

pub fn assemble_export(items: &[Item]) -> SessionExport {
    let clip = assemble_clipboard(items);
    let (markdown, captures) = assemble_markdown(items);
    SessionExport {
        plain: clip.plain,
        markdown,
        html: clip.html,
        images: captures
            .into_iter()
            .map(|c| SessionImage {
                n: c.id,
                kind: c.kind,
                width: c.width,
                height: c.height,
                labels: c.labels,
                data_url: png_data_url(&c.png),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn png_1x1() -> Vec<u8> {
        // 1×1 opaque red PNG, generated once and checked by png_size in image_ops tests.
        let mut buf = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut buf)
                .write_image(&[200, 24, 24, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        buf
    }

    fn shot(t: u64, kind: CaptureKind, labels: Vec<Label>) -> Item {
        Item::Image {
            t_ms: t,
            capture: Capture {
                id: t as u32,
                kind,
                t_ms: t,
                png: png_1x1(),
                width: 1,
                height: 1,
                labels,
            },
        }
    }

    fn said(t: u64, e: u64, s: &str) -> Item {
        Item::Speech {
            t_ms: t,
            end_ms: e,
            text: s.into(),
        }
    }

    #[test]
    fn a_walkthrough_reads_in_time_order_with_words_before_a_picture_at_one_time() {
        let got = ordered(vec![
            shot(4_000, CaptureKind::Full, vec![]),
            said(1_000, 2_000, "open the menu"),
            shot(12_500, CaptureKind::Zoom, vec![]),
            said(9_000, 10_000, "now save"),
            said(4_000, 5_000, "here"),
        ]);
        let ts: Vec<u64> = got.iter().map(Item::t).collect();
        assert_eq!(ts, [1_000, 4_000, 4_000, 9_000, 12_500]);
        assert!(matches!(got[1], Item::Speech { .. }) && matches!(got[2], Item::Image { .. }));
        assert_eq!(stamp(0), "0:00");
        assert_eq!(stamp(12_500), "0:12");
        assert_eq!(stamp(61_000), "1:01");
    }

    #[test]
    fn images_land_in_the_transcript_at_the_second_they_were_taken() {
        // Half way through "open the settings menu" (0 to 2 s): after the second of four words.
        // A photo between pieces (2.5 s) goes after the first piece; one before anything goes first.
        let (text, photos) = compose(&[
            said(4_000, 6_000, "then press save"),
            shot(2_500, CaptureKind::Full, vec![]),
            said(0, 2_000, "open the settings menu"),
            shot(1_000, CaptureKind::Region, vec![]),
            shot(5_000, CaptureKind::Zoom, vec![]),
        ]);
        assert_eq!(
            text,
            "open the [image 1] settings menu [image 2] then press [image 3] save"
        );
        assert_eq!(
            photos.iter().map(|p| p.id).collect::<Vec<_>>(),
            [1, 2, 3],
            "the files follow the numbers"
        );
        assert_eq!(
            photos.iter().map(|p| p.kind).collect::<Vec<_>>(),
            [CaptureKind::Region, CaptureKind::Full, CaptureKind::Zoom]
        );
        assert_eq!(
            compose(&[shot(10, CaptureKind::Full, vec![]), said(500, 900, "hello there")]).0,
            "[image 1] hello there"
        );
        assert_eq!(
            compose(&[
                shot(10, CaptureKind::Full, vec![]),
                shot(20, CaptureKind::Zoom, vec![])
            ])
            .0,
            "[image 1] [image 2]"
        );
        assert_eq!(
            compose(&[said(0, 1_000, "a b"), shot(999, CaptureKind::Full, vec![])]).0,
            "a b [image 1]"
        );
        assert_eq!(compose(&[]).0, "");
    }

    #[test]
    fn pin_labels_ride_on_the_image_marker() {
        let labels = vec![
            Label {
                n: 1,
                text: "Save".into(),
            },
            Label {
                n: 2,
                text: "too far left".into(),
            },
        ];
        let (text, _) = compose(&[
            said(0, 1_000, "this button"),
            shot(400, CaptureKind::Region, labels),
        ]);
        assert_eq!(text, "this [image 1: 1=Save; 2=too far left] button");
    }

    #[test]
    fn markdown_inlines_relative_pngs_at_the_marker() {
        let (md, caps) = assemble_markdown(&[
            said(0, 1_000, "look here"),
            shot(500, CaptureKind::Region, vec![]),
        ]);
        assert_eq!(md, "look ![image 1](images/01-region.png) here");
        assert_eq!(caps[0].id, 1);
    }

    #[test]
    fn html_clipboard_embeds_pngs_and_keeps_plain_fallback() {
        let payload = assemble_clipboard(&[
            said(0, 1_000, "look here"),
            shot(500, CaptureKind::Zoom, vec![]),
        ]);
        assert_eq!(payload.plain, "look [image 1] here");
        assert!(
            payload.html.contains("data:image/png;base64,"),
            "{}",
            payload.html
        );
        assert!(
            payload.html.contains("<img src="),
            "{}",
            payload.html
        );
        assert!(
            !payload.html.contains("look [image 1] here"),
            "the marker is replaced by the image: {}",
            payload.html
        );
        assert!(payload.html.contains("look "));
        assert!(payload.html.contains(" here"));
    }

    #[test]
    fn folder_export_writes_prompt_md_and_pngs() {
        let dir: PathBuf = std::env::temp_dir().join(format!(
            "pointcast-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let items = vec![
            said(0, 800, "the submit control"),
            shot(
                400,
                CaptureKind::Region,
                vec![Label {
                    n: 1,
                    text: "misaligned".into(),
                }],
            ),
            shot(900, CaptureKind::Zoom, vec![]),
        ];
        let prompt = export_folder(&items, &dir).unwrap();
        assert_eq!(prompt, dir.join("prompt.md"));
        let md = std::fs::read_to_string(&prompt).unwrap();
        assert!(md.contains("images/01-region.png"), "{md}");
        assert!(md.contains("images/02-zoom.png"), "{md}");
        assert!(md.contains("1=misaligned"), "{md}");
        let p1 = std::fs::read(dir.join("images/01-region.png")).unwrap();
        let p2 = std::fs::read(dir.join("images/02-zoom.png")).unwrap();
        assert_eq!(p1, png_1x1());
        assert_eq!(p2, png_1x1());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mcp_export_lists_images_in_marker_order() {
        let exp = assemble_export(&[
            shot(5, CaptureKind::Full, vec![]),
            said(10, 20, "hi"),
        ]);
        assert_eq!(exp.plain, "[image 1] hi");
        assert_eq!(exp.images.len(), 1);
        assert_eq!(exp.images[0].n, 1);
        assert!(exp.images[0].data_url.starts_with("data:image/png;base64,"));
        assert!(exp.html.contains("<img"));
    }
}
