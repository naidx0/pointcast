//! Every finished prompt, kept on disk (`<data dir>/history/<id>/`), and the recording in
//! progress, checkpointed as it grows (`<data dir>/recovery/`) so a crash or an accidental quit
//! loses nothing that was already transcribed or captured.
//!
//! An entry is `meta.json`, `items.json` (the session with picture bytes left out) and one
//! `image-<k>.png` per picture, in order.
use crate::prompt::{self, Item, SessionExport};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Entries kept; older ones are removed as new ones arrive.
pub const KEEP: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub id: String,
    pub created_ms: u64,
    pub duration_ms: u64,
    /// The first words, for the list.
    pub preview: String,
    pub images: u32,
    /// Rebuilt from a checkpoint after Pointcast closed mid-recording.
    #[serde(default)]
    pub recovered: bool,
}

pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(data_dir: &Path) -> Self {
        Store { root: data_dir.to_path_buf() }
    }

    fn history(&self) -> PathBuf {
        self.root.join("history")
    }

    fn recovery(&self) -> PathBuf {
        self.root.join("recovery")
    }

    /// Keep a finished session. Returns its entry.
    pub fn save(&self, items: &[Item], created_ms: u64, duration_ms: u64) -> Result<Meta, String> {
        let meta = meta_for(items, created_ms, duration_ms, false);
        let dir = self.history().join(&meta.id);
        let tmp = self.history().join(format!("{}.tmp", meta.id));
        let _ = std::fs::remove_dir_all(&tmp);
        write_entry(&tmp, items, &meta)?;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::rename(&tmp, &dir).map_err(|e| e.to_string())?;
        self.prune(KEEP);
        Ok(meta)
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Meta> {
        let mut v: Vec<Meta> = std::fs::read_dir(self.history())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().ends_with(".tmp"))
            .filter_map(|e| std::fs::read(e.path().join("meta.json")).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        v.sort_by(|a: &Meta, b: &Meta| b.created_ms.cmp(&a.created_ms).then(b.id.cmp(&a.id)));
        v
    }

    pub fn items(&self, id: &str) -> Result<Vec<Item>, String> {
        read_items(&self.entry_dir(id)?)
    }

    pub fn export(&self, id: &str) -> Result<SessionExport, String> {
        Ok(prompt::assemble_export(&self.items(id)?))
    }

    /// Picture `n` of an entry (1-based, as `[image n]` numbers it), with its bytes.
    pub fn picture(&self, id: &str, n: usize) -> Result<prompt::Capture, String> {
        let dir = self.entry_dir(id)?;
        let items = stored(&dir)?;
        let (i, k) = locate(&dir, &items, n)?;
        let Item::Image { capture, .. } = &items[i] else { unreachable!() };
        let png = std::fs::read(dir.join(format!("image-{k}.png"))).map_err(|e| e.to_string())?;
        Ok(prompt::Capture { png, ..capture.clone() })
    }

    /// Put a marked-up picture in place of picture `n`: its pixels and size, and its labels after the ones it had,
    /// numbered on from them. The picture and the session are written together or not at all.
    pub fn replace(&self, id: &str, n: usize, png: &[u8], labels: Vec<prompt::Label>) -> Result<prompt::Capture, String> {
        let (width, height) = crate::image_ops::png_size(png).ok_or("that is not a PNG")?;
        let dir = self.entry_dir(id)?;
        let mut items = stored(&dir)?;
        let (i, k) = locate(&dir, &items, n)?;
        let Item::Image { capture: c, .. } = &mut items[i] else { unreachable!() };
        let base = c.labels.iter().map(|l| l.n).max().unwrap_or(0);
        c.labels.extend(labels.into_iter().zip(base + 1..).map(|(l, n)| prompt::Label { n, ..l }));
        (c.width, c.height) = (width, height);
        let out = prompt::Capture { png: png.to_vec(), ..c.clone() };
        write_together(&[
            (dir.join(format!("image-{k}.png")), png.to_vec()),
            (dir.join("items.json"), serde_json::to_vec(&items).map_err(|e| e.to_string())?),
        ])?;
        Ok(out)
    }

    /// Take picture `n` out of an entry; only a key frame Pointcast picked (`auto`) can go. The pictures after it move up
    /// a file and the words stay as they were: the prompt numbers what is left. Written together or not at all.
    pub fn remove(&self, id: &str, n: usize) -> Result<(), String> {
        let dir = self.entry_dir(id)?;
        let mut items = stored(&dir)?;
        let (i, k) = locate(&dir, &items, n)?;
        if !matches!(&items[i], Item::Image { capture, .. } if capture.auto) {
            return Err("only a picture Pointcast picked can be removed".into());
        }
        let total = items.iter().filter(|it| matches!(it, Item::Image { .. })).count();
        items.remove(i);
        let file = |j: usize| dir.join(format!("image-{j}.png"));
        let mut files: Vec<(PathBuf, Vec<u8>)> = (k..total - 1).map(|j| (file(j), std::fs::read(file(j + 1)).unwrap_or_default())).collect();
        files.push((dir.join("items.json"), serde_json::to_vec(&items).map_err(|e| e.to_string())?));
        if let Some(mut meta) = std::fs::read(dir.join("meta.json")).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok()) {
            meta.images = (total - 1) as u32;
            files.push((dir.join("meta.json"), serde_json::to_vec(&meta).map_err(|e| e.to_string())?));
        }
        write_together(&files)?;
        let _ = std::fs::remove_file(file(total - 1));
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        std::fs::remove_dir_all(self.entry_dir(id)?).map_err(|e| e.to_string())
    }

    pub fn clear(&self) -> Result<(), String> {
        match std::fs::remove_dir_all(self.history()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }

    fn prune(&self, keep: usize) {
        for m in self.list().into_iter().skip(keep) {
            let _ = std::fs::remove_dir_all(self.history().join(&m.id));
        }
    }

    /// Ids come from the UI: only a plain entry name under the history folder is accepted.
    fn entry_dir(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("no such prompt".into());
        }
        let d = self.history().join(id);
        if d.is_dir() {
            Ok(d)
        } else {
            Err("no such prompt".into())
        }
    }

    /// Write the recording so far. Pictures already on disk are not written again.
    pub fn checkpoint(&self, items: &[Item], created_ms: u64) -> Result<(), String> {
        let dir = self.recovery();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut k = 0;
        for it in items {
            if let Item::Image { capture, .. } = it {
                let p = dir.join(format!("image-{k}.png"));
                if !p.exists() {
                    write_atomic(&p, &capture.png)?;
                }
                k += 1;
            }
        }
        let meta = meta_for(items, created_ms, 0, true);
        write_atomic(&dir.join("items.json"), &serde_json::to_vec(&stripped(items)).map_err(|e| e.to_string())?)?;
        write_atomic(&dir.join("meta.json"), &serde_json::to_vec(&meta).map_err(|e| e.to_string())?)
    }

    /// The session ended normally (kept or discarded): nothing to recover.
    pub fn clear_checkpoint(&self) {
        let _ = std::fs::remove_dir_all(self.recovery());
    }

    /// At launch: turn a checkpoint left by a crash into a history entry marked recovered.
    pub fn recover(&self) -> Option<Meta> {
        let dir = self.recovery();
        let items = read_items(&dir).ok().filter(|v| !v.is_empty());
        let Some(items) = items else {
            self.clear_checkpoint();
            return None;
        };
        let created = std::fs::read(dir.join("meta.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Meta>(&b).ok())
            .map(|m| m.created_ms)
            .unwrap_or_else(crate::session::now_ms);
        let duration = items.iter().map(|i| match i {
            Item::Speech { end_ms, .. } => *end_ms,
            Item::Image { t_ms, .. } => *t_ms,
        });
        let mut meta = self.save(&items, created, duration.max().unwrap_or(0)).ok()?;
        meta.recovered = true;
        let _ = std::fs::write(
            self.history().join(&meta.id).join("meta.json"),
            serde_json::to_vec(&meta).unwrap_or_default(),
        );
        self.clear_checkpoint();
        Some(meta)
    }
}

fn meta_for(items: &[Item], created_ms: u64, duration_ms: u64, recovered: bool) -> Meta {
    // The words only: a marker can carry a control's name and pin labels.
    let mut said: Vec<(u64, &str)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Speech { t_ms, text, .. } => Some((*t_ms, text.as_str())),
            _ => None,
        })
        .collect();
    said.sort_by_key(|s| s.0);
    let words = said.iter().flat_map(|s| s.1.split_whitespace()).take(24).collect::<Vec<_>>().join(" ");
    let stamp = chrono::DateTime::from_timestamp_millis(created_ms as i64)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| "session".into());
    Meta {
        id: format!("{stamp}-{:03}", created_ms % 1000),
        created_ms,
        duration_ms,
        preview: words,
        images: items.iter().filter(|i| matches!(i, Item::Image { .. })).count() as u32,
        recovered,
    }
}

/// The session without picture bytes; pictures live beside it as files.
fn stripped(items: &[Item]) -> Vec<Item> {
    items
        .iter()
        .map(|i| match i {
            Item::Image { t_ms, capture: c } => Item::Image {
                t_ms: *t_ms,
                capture: prompt::Capture {
                    id: c.id,
                    kind: c.kind,
                    t_ms: c.t_ms,
                    png: vec![],
                    width: c.width,
                    height: c.height,
                    labels: c.labels.clone(),
                    pointer: c.pointer,
                    trail: c.trail.clone(),
                    element: c.element.clone(),
                    place: c.place,
                    anchors: c.anchors.clone(),
                    auto: c.auto,
                },
            },
            s => s.clone(),
        })
        .collect()
}

fn write_entry(dir: &Path, items: &[Item], meta: &Meta) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut k = 0;
    for it in items {
        if let Item::Image { capture, .. } = it {
            std::fs::write(dir.join(format!("image-{k}.png")), &capture.png).map_err(|e| e.to_string())?;
            k += 1;
        }
    }
    std::fs::write(dir.join("items.json"), serde_json::to_vec(&stripped(items)).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    std::fs::write(dir.join("meta.json"), serde_json::to_vec(meta).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

/// The session as written, picture bytes left out.
fn stored(dir: &Path) -> Result<Vec<Item>, String> {
    let b = std::fs::read(dir.join("items.json")).map_err(|e| e.to_string())?;
    serde_json::from_slice(&b).map_err(|e| e.to_string())
}

/// Where picture `n` lives: its place in `items` and its file number. Counted as the prompt counts them, in time order
/// over the pictures whose files are there.
fn locate(dir: &Path, items: &[Item], n: usize) -> Result<(usize, usize), String> {
    let mut found = vec![];
    let mut k = 0;
    for (i, it) in items.iter().enumerate() {
        if let Item::Image { t_ms, .. } = it {
            if std::fs::metadata(dir.join(format!("image-{k}.png"))).is_ok_and(|m| m.len() > 0) {
                found.push((*t_ms, i, k));
            }
            k += 1;
        }
    }
    found.sort_by_key(|f| f.0);
    let at = n.checked_sub(1).and_then(|j| found.get(j)).ok_or("no such picture")?;
    Ok((at.1, at.2))
}

/// Write each file to a temp beside it; only once every one has landed are they renamed into place, so a write that
/// fails part way leaves the old files as they were.
fn write_together(files: &[(PathBuf, Vec<u8>)]) -> Result<(), String> {
    let tmp = |p: &Path| p.with_extension("tmp");
    let undo = || {
        for (p, _) in files {
            let _ = std::fs::remove_file(tmp(p));
        }
    };
    for (p, b) in files {
        if let Err(e) = std::fs::write(tmp(p), b) {
            undo();
            return Err(e.to_string());
        }
    }
    for (p, _) in files {
        if let Err(e) = std::fs::rename(tmp(p), p) {
            undo();
            return Err(e.to_string());
        }
    }
    Ok(())
}

fn read_items(dir: &Path) -> Result<Vec<Item>, String> {
    let mut items = stored(dir)?;
    let mut k = 0;
    for it in items.iter_mut() {
        if let Item::Image { capture, .. } = it {
            // A picture whose file never landed (the crash came mid-write) is dropped, not fatal.
            capture.png = std::fs::read(dir.join(format!("image-{k}.png"))).unwrap_or_default();
            k += 1;
        }
    }
    items.retain(|i| !matches!(i, Item::Image { capture, .. } if capture.png.is_empty()));
    Ok(items)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{Capture, CaptureKind, Label};

    fn png(v: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            use image::ImageEncoder;
            image::codecs::png::PngEncoder::new(&mut buf)
                .write_image(&[v, 24, 24, 255], 1, 1, image::ExtendedColorType::Rgba8)
                .unwrap();
        }
        buf
    }

    fn items() -> Vec<Item> {
        vec![
            Item::Speech { t_ms: 0, end_ms: 900, text: "the button is off by a few pixels".into() },
            Item::Image {
                t_ms: 1000,
                capture: Capture {
                    id: 0,
                    kind: CaptureKind::Region,
                    t_ms: 1000,
                    png: png(200),
                    width: 1,
                    height: 1,
                    labels: vec![Label { n: 1, text: "this one".into() }],
                    pointer: None,
                    trail: vec![],
                    element: Some(crate::element::ElementMark {
                        role: "button".into(),
                        name: "Place order".into(),
                        rect: crate::element::Bounds { x: 0, y: 0, w: 1, h: 1 },
                    }),
                    place: None,
                    anchors: vec![],
                    auto: false,
                },
            },
            Item::Speech { t_ms: 1200, end_ms: 2000, text: "move it left".into() },
        ]
    }

    fn store(name: &str) -> Store {
        let d = std::env::temp_dir().join(format!("pc-hist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        Store::new(&d)
    }

    #[test]
    fn a_saved_prompt_comes_back_word_for_word_with_its_picture() {
        let s = store("roundtrip");
        let m = s.save(&items(), 1_793_000_000_000, 2000).unwrap();
        assert_eq!(m.images, 1);
        assert_eq!(m.preview, "the button is off by a few pixels move it left", "words only, no marker");
        assert_eq!(s.items(&m.id).unwrap(), items());
        let (a, b) = (s.export(&m.id).unwrap(), prompt::assemble_export(&items()));
        assert_eq!((a.plain, a.html, a.images.len()), (b.plain, b.html, b.images.len()));
        assert_eq!(s.list(), vec![m.clone()]);
        s.delete(&m.id).unwrap();
        assert!(s.list().is_empty());
    }

    #[test]
    fn an_entry_saved_before_outlines_existed_still_loads() {
        let old = r#"[{"type":"image","t_ms":5,"capture":{"id":0,"kind":"full","t_ms":5,"png":[],"width":1,"height":1,"labels":[]}}]"#;
        let items: Vec<Item> = serde_json::from_str(old).unwrap();
        let Item::Image { capture, .. } = &items[0] else { panic!("{items:?}") };
        assert_eq!(capture.element, None);
        assert_eq!((capture.place, capture.anchors.len()), (None, 0));
        assert_eq!(serde_json::to_string(&items).unwrap(), old, "and one without a control writes no field for it");
    }

    #[test]
    fn a_picture_keeps_its_screen_mapping_and_spoken_rings_in_history() {
        let s = store("place");
        let mut v = items();
        let Item::Image { capture, .. } = &mut v[1] else { panic!() };
        let map = crate::capture::Mapping { origin: (-1920, 0), scale: 1.5, frame: (2880, 1620), crop: (0, 0), size: (2880, 1620) };
        capture.place = Some(crate::capture::Place { map, k: 1.5 });
        capture.anchors = vec![crate::pointer::Point { t_ms: 1400, x: 640, y: 360 }];
        let m = s.save(&v, 1_793_000_000_000, 2000).unwrap();
        assert_eq!(s.items(&m.id).unwrap(), v);
    }

    #[test]
    fn history_lists_newest_first_and_keeps_the_newest_hundred() {
        let s = store("prune");
        for i in 0..(KEEP as u64 + 3) {
            s.save(&items()[..1], 1_793_000_000_000 + i * 1000, 10).unwrap();
        }
        let l = s.list();
        assert_eq!(l.len(), KEEP);
        assert!(l.windows(2).all(|w| w[0].created_ms > w[1].created_ms));
        assert_eq!(l[0].created_ms, 1_793_000_000_000 + (KEEP as u64 + 2) * 1000);
    }

    #[test]
    fn a_recording_cut_off_by_a_crash_is_recovered_at_the_next_launch() {
        let s = store("crash");
        let mut so_far = items();
        s.checkpoint(&so_far[..2], 1_793_000_123_000).unwrap();
        so_far.truncate(3);
        s.checkpoint(&so_far, 1_793_000_123_000).unwrap();
        // The process dies here: no save, no clear. Next launch:
        let m = s.recover().expect("recovered");
        assert!(m.recovered);
        assert_eq!(s.items(&m.id).unwrap(), items());
        assert!(s.list()[0].recovered);
        assert!(s.recover().is_none(), "recovered once, then gone");
    }

    #[test]
    fn a_session_that_ended_normally_leaves_nothing_to_recover() {
        let s = store("clean");
        s.checkpoint(&items(), 1).unwrap();
        s.clear_checkpoint();
        assert!(s.recover().is_none());
        assert!(s.list().is_empty());
    }

    #[test]
    fn ids_from_outside_cannot_reach_outside_the_history_folder() {
        let s = store("ids");
        for bad in ["", "..", "../settings", "a/b", "C:\\x"] {
            assert!(s.items(bad).is_err(), "{bad}");
            assert!(s.delete(bad).is_err(), "{bad}");
        }
    }

    fn png_of(w: u32, h: u32, v: u8) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            use image::ImageEncoder;
            let px: Vec<u8> = (0..w * h).flat_map(|_| [v, 24, 24, 255]).collect();
            image::codecs::png::PngEncoder::new(&mut buf).write_image(&px, w, h, image::ExtendedColorType::Rgba8).unwrap();
        }
        buf
    }

    /// Words, then picture 1, more words, then picture 2 (written to disk before picture 1, as a late commit is).
    fn two_pictures() -> Vec<Item> {
        let mut v = items();
        let Item::Image { capture, .. } = &v[1] else { panic!() };
        let mut late = capture.clone();
        (late.t_ms, late.png, late.labels, late.element) = (1500, png(90), vec![], None);
        v.insert(1, Item::Image { t_ms: 1500, capture: late });
        v
    }

    fn label(n: u32, text: &str) -> Label {
        Label { n, text: text.into() }
    }

    #[test]
    fn a_picture_marked_up_again_replaces_that_picture_and_keeps_the_rest() {
        let s = store("replace");
        let m = s.save(&two_pictures(), 1_793_000_000_000, 2000).unwrap();
        let before = s.export(&m.id).unwrap();
        let first = s.picture(&m.id, 1).unwrap();
        assert_eq!((first.png.clone(), first.labels.clone()), (png(200), vec![label(1, "this one")]), "numbered in time order, as the prompt numbers them");
        let new = png_of(3, 2, 10);
        let cap = s.replace(&m.id, 1, &new, vec![label(1, "too far left"), label(2, "and this")]).unwrap();
        assert_eq!((cap.width, cap.height, cap.png.clone()), (3, 2, new.clone()));
        let after = s.export(&m.id).unwrap();
        let labels = vec![label(1, "this one"), label(2, "too far left"), label(3, "and this")];
        assert_eq!(after.images[0].labels, labels, "new labels go after the old ones, numbered on");
        assert_eq!((after.images[0].width, after.images[0].height), (3, 2));
        assert!(after.plain.contains(r#"[image 1: button "Place order"; 1=this one; 2=too far left; 3=and this]"#), "{}", after.plain);
        assert_eq!(s.picture(&m.id, 1).unwrap().png, new);
        assert_eq!(s.picture(&m.id, 2).unwrap().png, png(90), "the other picture is untouched");
        assert_eq!((&after.images[1].data_url, &after.images[1].labels), (&before.images[1].data_url, &before.images[1].labels));
        let said = |v: Vec<Item>| v.into_iter().filter(|i| matches!(i, Item::Speech { .. })).collect::<Vec<_>>();
        assert_eq!(said(s.items(&m.id).unwrap()), said(two_pictures()), "and so are the words");
        assert_eq!(s.list(), vec![m.clone()], "the entry keeps its place and its summary");
        let dir = s.history().join(&m.id);
        let left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(left.iter().all(|f| !f.ends_with(".tmp")), "{left:?}");
    }

    #[test]
    fn a_replace_refuses_bad_ids_and_pictures_that_are_not_there() {
        let s = store("replace-bad");
        let m = s.save(&items(), 1_793_000_000_000, 2000).unwrap();
        let new = png_of(2, 2, 10);
        for bad in ["", "..", "../settings", "a/b", "nope"] {
            assert!(s.picture(bad, 1).is_err(), "{bad}");
            assert!(s.replace(bad, 1, &new, vec![]).is_err(), "{bad}");
        }
        for n in [0, 2, usize::MAX] {
            assert!(s.picture(&m.id, n).is_err(), "{n}");
            assert!(s.replace(&m.id, n, &new, vec![]).is_err(), "{n}");
        }
        assert!(s.replace(&m.id, 1, b"not a png", vec![]).is_err());
        assert_eq!(s.items(&m.id).unwrap(), items(), "nothing changed");
    }

    #[test]
    fn a_replace_that_fails_part_way_leaves_the_entry_as_it_was() {
        let s = store("replace-fail");
        let m = s.save(&items(), 1_793_000_000_000, 2000).unwrap();
        let dir = s.history().join(&m.id);
        let (png0, json0) = (std::fs::read(dir.join("image-0.png")).unwrap(), std::fs::read(dir.join("items.json")).unwrap());
        // The picture's temp lands; the session's cannot be written.
        std::fs::create_dir(dir.join("items.tmp")).unwrap();
        assert!(s.replace(&m.id, 1, &png_of(3, 2, 10), vec![label(1, "lost")]).is_err());
        assert_eq!(std::fs::read(dir.join("image-0.png")).unwrap(), png0, "the picture was not renamed into place");
        assert_eq!(std::fs::read(dir.join("items.json")).unwrap(), json0);
        assert!(!dir.join("image-0.tmp").exists(), "and its temp is gone");
        assert_eq!(s.items(&m.id).unwrap(), items());
    }

    /// Words, a picture the person took, a key frame Pointcast picked, then another picture they took.
    fn with_a_key_frame() -> Vec<Item> {
        let mut v = items();
        let Item::Image { capture, .. } = &v[1] else { panic!() };
        let mut auto = capture.clone();
        (auto.t_ms, auto.png, auto.labels, auto.element, auto.auto) = (1_100, png(90), vec![], None, true);
        let mut last = capture.clone();
        (last.t_ms, last.png, last.labels, last.element) = (2_500, png(30), vec![], None);
        v.insert(2, Item::Image { t_ms: 1_100, capture: auto });
        v.push(Item::Image { t_ms: 2_500, capture: last });
        v
    }

    #[test]
    fn a_key_frame_stays_marked_auto_in_history_and_old_pictures_are_not() {
        let s = store("auto");
        let m = s.save(&with_a_key_frame(), 1_793_000_000_000, 3000).unwrap();
        let back = s.items(&m.id).unwrap();
        assert_eq!(back, with_a_key_frame());
        let auto: Vec<bool> = s.export(&m.id).unwrap().images.iter().map(|i| i.auto).collect();
        assert_eq!(auto, [false, true, false]);
        let raw = String::from_utf8(std::fs::read(s.history().join(&m.id).join("items.json")).unwrap()).unwrap();
        assert_eq!(raw.matches(r#""auto":true"#).count(), 1, "{raw}");
        assert!(!raw.contains(r#""auto":false"#), "a picture the person took writes no field for it");
        let old: Vec<Item> = serde_json::from_str(r#"[{"type":"image","t_ms":5,"capture":{"id":0,"kind":"full","t_ms":5,"png":[],"width":1,"height":1,"labels":[]}}]"#).unwrap();
        let Item::Image { capture, .. } = &old[0] else { panic!() };
        assert!(!capture.auto);
    }

    #[test]
    fn removing_a_key_frame_takes_out_only_that_picture_and_the_prompt_renumbers() {
        let s = store("remove");
        let m = s.save(&with_a_key_frame(), 1_793_000_000_000, 3000).unwrap();
        assert_eq!(s.export(&m.id).unwrap().plain, r#"the button is off by a few pixels [image 1: button "Place order"; 1=this one] [image 2] move it left [image 3]"#);
        assert!(s.remove(&m.id, 1).is_err() && s.remove(&m.id, 3).is_err(), "pictures the person took stay");
        s.remove(&m.id, 2).unwrap();
        let after = s.export(&m.id).unwrap();
        assert_eq!(after.plain, r#"the button is off by a few pixels [image 1: button "Place order"; 1=this one] move it left [image 2]"#);
        assert_eq!((s.picture(&m.id, 1).unwrap().png, s.picture(&m.id, 2).unwrap().png), (png(200), png(30)));
        assert!(after.images.iter().all(|i| !i.auto));
        let said = |v: Vec<Item>| v.into_iter().filter(|i| matches!(i, Item::Speech { .. })).collect::<Vec<_>>();
        assert_eq!(said(s.items(&m.id).unwrap()), said(with_a_key_frame()), "the words are untouched");
        assert_eq!(s.list()[0].images, 2);
        let dir = s.history().join(&m.id);
        let mut left: Vec<String> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["image-0.png", "image-1.png", "items.json", "meta.json"]);
        for bad in ["", "..", "../settings", "nope"] {
            assert!(s.remove(bad, 1).is_err(), "{bad}");
        }
        assert!(s.remove(&m.id, 3).is_err() && s.remove(&m.id, 0).is_err());
        assert_eq!(s.export(&m.id).unwrap().plain, after.plain, "a refused remove changes nothing");
    }

    #[test]
    fn an_entry_saved_before_outlines_existed_can_be_marked_up_again() {
        let s = store("replace-old");
        let dir = s.history().join("20260901-101500-000");
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"[{"type":"speech","t_ms":0,"end_ms":900,"text":"look here"},{"type":"image","t_ms":5,"capture":{"id":0,"kind":"full","t_ms":5,"png":[],"width":1,"height":1,"labels":[]}}]"#;
        std::fs::write(dir.join("items.json"), old).unwrap();
        std::fs::write(dir.join("image-0.png"), png(200)).unwrap();
        let id = "20260901-101500-000";
        assert_eq!(s.items(id).unwrap().len(), 2, "it still loads");
        assert_eq!(s.picture(id, 1).unwrap().png, png(200));
        s.replace(id, 1, &png_of(2, 1, 10), vec![label(1, "pin 1")]).unwrap();
        let v = s.items(id).unwrap();
        let Item::Image { capture, .. } = &v[1] else { panic!("{v:?}") };
        assert_eq!((capture.width, capture.labels.clone(), capture.element.clone()), (2, vec![label(1, "pin 1")], None));
        let back = String::from_utf8(std::fs::read(dir.join("items.json")).unwrap()).unwrap();
        assert!(!back.contains("element") && !back.contains("anchors"), "no fields it never had: {back}");
    }
}
