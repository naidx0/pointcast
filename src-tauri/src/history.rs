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
    let text = prompt::compose(items).0;
    let words: String = text
        .split_whitespace()
        .filter(|w| !w.starts_with("[image") && !w.ends_with(']'))
        .take(24)
        .collect::<Vec<_>>()
        .join(" ");
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

fn read_items(dir: &Path) -> Result<Vec<Item>, String> {
    let b = std::fs::read(dir.join("items.json")).map_err(|e| e.to_string())?;
    let mut items: Vec<Item> = serde_json::from_slice(&b).map_err(|e| e.to_string())?;
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
        assert!(m.preview.starts_with("the button is off"), "{}", m.preview);
        assert_eq!(s.items(&m.id).unwrap(), items());
        let (a, b) = (s.export(&m.id).unwrap(), prompt::assemble_export(&items()));
        assert_eq!((a.plain, a.html, a.images.len()), (b.plain, b.html, b.images.len()));
        assert_eq!(s.list(), vec![m.clone()]);
        s.delete(&m.id).unwrap();
        assert!(s.list().is_empty());
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
}
