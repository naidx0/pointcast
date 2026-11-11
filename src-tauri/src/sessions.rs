//! Conversations with the assistant, kept on disk beside History (`<data dir>/sessions/<id>/`) and never pruned by
//! count. A session is `session.json` and `history/<turn id>/`: exactly a History entry for the question, so every
//! History operation works on a turn, plus `answer.json` and the copy of each picture as it was sent (`sent-<n>.png`
//! or `.jpg`). A picture sent again when a session is resumed is those bytes, never the original.

use crate::assist::Mime;
use crate::history::{self, Meta, Store};
use crate::prompt::Item;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A session keeps the audio of this many of its newest turns, as History does of its entries.
pub const KEEP_AUDIO: usize = history::KEEP_AUDIO;

/// A session's title until it is renamed: this many words of its first question.
pub const TITLE_WORDS: usize = 6;

const SESSION: &str = "session.json";
const ANSWER: &str = "answer.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub created_ms: u64,
    pub updated_ms: u64,
    pub turns: u32,
    /// The pictures its questions kept.
    pub pictures: u32,
    #[serde(default)]
    pub style: String,
    /// The provider that answered last, by the name the person reads.
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub has_summary: bool,
}

/// When each stage of a turn happened, Unix ms; 0 for a stage it never reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Times {
    pub press_ms: u64,
    pub release_ms: u64,
    pub words_ms: u64,
    pub sent_ms: u64,
    pub first_ms: u64,
    pub done_ms: u64,
}

/// What an answer cost, as the provider counted it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Used {
    pub input: u64,
    pub cached: u64,
    pub output: u64,
}

/// A picture as it was sent: `n` counts from 1 within its turn, and the bytes are in `sent-<n>` beside this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentPicture {
    pub n: u32,
    pub w: u32,
    pub h: u32,
    pub bytes: u64,
    pub sha256: String,
    pub mime: Mime,
}

/// Exactly what left the computer for a turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sent {
    /// The assembled text: the question and one caption line a picture.
    pub text: String,
    pub pictures: Vec<SentPicture>,
    /// The earlier turns that went along, by id, oldest first.
    pub earlier: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    /// The answer as it streamed, in markdown.
    pub text: String,
    pub provider: String,
    pub model: Option<String>,
    #[serde(flatten)]
    pub times: Times,
    /// The person stopped it; `text` is what had arrived.
    pub stopped: bool,
    pub error: Option<String>,
    pub usage: Option<Used>,
    pub sent: Sent,
}

/// A picture on its way out: the bytes the provider gets.
pub struct Wire<'a> {
    pub bytes: &'a [u8],
    pub w: u32,
    pub h: u32,
    pub mime: Mime,
}

/// A question to keep: what was said and shown, its recording as a WAV file when that was kept, and its pictures as
/// they are sent.
pub struct Question<'a> {
    pub items: &'a [Item],
    pub created_ms: u64,
    pub duration_ms: u64,
    pub audio: Option<Vec<u8>>,
    pub pictures: &'a [Wire<'a>],
}

/// A turn just kept: its session, its id, the answer as written (each sent picture described) and where each sent
/// picture's file is.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub session: Session,
    pub turn: String,
    pub answer: Answer,
    pub pictures: Vec<PathBuf>,
}

/// A turn read back: the question's entry and its answer, when one was written.
#[derive(Debug, Clone, PartialEq)]
pub struct Kept {
    pub meta: Meta,
    pub answer: Option<Answer>,
}

pub struct Sessions {
    root: PathBuf,
}

impl Sessions {
    pub fn new(data_dir: &Path) -> Self {
        Sessions { root: data_dir.join("sessions") }
    }

    /// Ids come from the UI: only a plain folder name under the sessions folder is accepted.
    fn dir(&self, id: &str) -> Result<PathBuf, String> {
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err("no such session".into());
        }
        let d = self.root.join(id);
        if d.join(SESSION).is_file() {
            Ok(d)
        } else {
            Err("no such session".into())
        }
    }

    fn turns_in(dir: &Path) -> Store {
        Store::with_keep(dir, usize::MAX, KEEP_AUDIO)
    }

    /// A session's turns as a History store: every turn is kept, and the audio of its newest ten.
    pub fn turns(&self, id: &str) -> Result<Store, String> {
        Ok(Self::turns_in(&self.dir(id)?))
    }

    /// Newest first, by when each was last added to.
    #[allow(dead_code)] // The session list arrives with slice 6.
    pub fn list(&self) -> Vec<Session> {
        let mut v: Vec<Session> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir() && !e.file_name().to_string_lossy().ends_with(".tmp"))
            .filter_map(|e| std::fs::read(e.path().join(SESSION)).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        v.sort_by(|a: &Session, b: &Session| b.updated_ms.cmp(&a.updated_ms).then(b.id.cmp(&a.id)));
        v
    }

    /// One session and its turns, oldest first.
    pub fn read(&self, id: &str) -> Result<(Session, Vec<Kept>), String> {
        let dir = self.dir(id)?;
        let session = read_json(&dir.join(SESSION)).ok_or("no such session")?;
        let store = Self::turns_in(&dir);
        let mut turns: Vec<Kept> = store
            .list()
            .into_iter()
            .map(|meta| Kept { answer: store.dir(&meta.id).ok().and_then(|d| read_json(&d.join(ANSWER))), meta })
            .collect();
        turns.reverse();
        Ok((session, turns))
    }

    #[allow(dead_code)] // Deleting a session arrives with slice 6.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        std::fs::remove_dir_all(self.dir(id)?).map_err(|e| e.to_string())
    }

    /// Keep a question as the next turn of `session`, or as the first turn of a new one: its History entry, the copy of
    /// each picture as sent, and `answer` so far (what is sent, and whatever of the answer is known). The question is
    /// on disk before anything is sent, so no question is lost to a failure after this.
    pub fn save_turn(&self, session: Option<&str>, q: Question, mut answer: Answer, style: &str, now_ms: u64) -> Result<Saved, String> {
        // A new session is built aside and moved into place whole.
        let dir = match session {
            Some(id) => self.dir(id)?,
            None => {
                let d = self.root.join(format!("new-{}.tmp", std::process::id()));
                let _ = std::fs::remove_dir_all(&d);
                std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
                d
            }
        };
        let store = Self::turns_in(&dir);
        let meta = store.save_heard(q.items, q.created_ms, q.duration_ms, q.audio).inspect_err(|_| {
            if session.is_none() {
                let _ = std::fs::remove_dir_all(&dir);
            }
        })?;
        let undo = |e: String| {
            let _ = if session.is_none() { std::fs::remove_dir_all(&dir) } else { std::fs::remove_dir_all(dir.join("history").join(&meta.id)) };
            e
        };
        let turn = store.dir(&meta.id).map_err(undo)?;
        answer.sent.pictures.clear();
        let mut files = vec![];
        for (k, p) in q.pictures.iter().enumerate() {
            let n = k as u32 + 1;
            let file = turn.join(sent_name(n, p.mime));
            history::write_atomic(&file, p.bytes).map_err(undo)?;
            answer.sent.pictures.push(SentPicture { n, w: p.w, h: p.h, bytes: p.bytes.len() as u64, sha256: sha256(p.bytes), mime: p.mime });
            files.push(file);
        }
        write_json(&turn.join(ANSWER), &answer).map_err(undo)?;
        let id = session.map_or_else(|| meta.id.clone(), str::to_string);
        let mut s = read_json(&dir.join(SESSION)).unwrap_or(Session {
            id: id.clone(),
            title: title(q.items),
            created_ms: q.created_ms,
            updated_ms: now_ms,
            turns: 0,
            pictures: 0,
            style: style.into(),
            provider: String::new(),
            has_summary: false,
        });
        count(&mut s, &store, &answer.provider, now_ms);
        write_json(&dir.join(SESSION), &s).map_err(undo)?;
        if session.is_none() {
            let to = self.root.join(&id);
            if to.exists() {
                return Err(undo("a session from this moment is already kept".into()));
            }
            std::fs::rename(&dir, &to).map_err(|e| undo(e.to_string()))?;
            files = files.iter().filter_map(|f| f.strip_prefix(&dir).ok().map(|rest| to.join(rest))).collect();
        }
        Ok(Saved { session: s, turn: meta.id, answer, pictures: files })
    }

    /// Write a turn's answer again as it grew or ended; what was sent stays as it was written first.
    pub fn write_answer(&self, session: &str, turn: &str, answer: &Answer, now_ms: u64) -> Result<Session, String> {
        let dir = self.dir(session)?;
        let store = Self::turns_in(&dir);
        let at = store.dir(turn)?.join(ANSWER);
        let mut next = answer.clone();
        if let Some(old) = read_json::<Answer>(&at) {
            next.sent = old.sent;
        }
        write_json(&at, &next)?;
        let mut s: Session = read_json(&dir.join(SESSION)).ok_or("no such session")?;
        count(&mut s, &store, &next.provider, now_ms);
        write_json(&dir.join(SESSION), &s)?;
        Ok(s)
    }

    /// Picture `n` of a turn as it was sent: its file and its bytes.
    pub fn sent_picture(&self, session: &str, turn: &str, n: u32) -> Result<(PathBuf, Vec<u8>), String> {
        let dir = self.turns(session)?.dir(turn)?;
        let answer: Answer = read_json(&dir.join(ANSWER)).ok_or("no such picture")?;
        let p = answer.sent.pictures.iter().find(|p| p.n == n).ok_or("no such picture")?;
        let file = dir.join(sent_name(n, p.mime));
        let bytes = std::fs::read(&file).map_err(|_| "no such picture".to_string())?;
        Ok((file, bytes))
    }
}

/// The session's counts, from what is on disk now.
fn count(s: &mut Session, store: &Store, provider: &str, now_ms: u64) {
    let turns = store.list();
    s.turns = turns.len() as u32;
    s.pictures = turns.iter().map(|m| m.images).sum();
    s.updated_ms = now_ms;
    if !provider.is_empty() {
        s.provider = provider.into();
    }
}

/// The first words of the question, in the order they were said.
fn title(items: &[Item]) -> String {
    let mut said: Vec<(u64, &str)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Speech { t_ms, text, .. } => Some((*t_ms, text.as_str())),
            _ => None,
        })
        .collect();
    said.sort_by_key(|s| s.0);
    said.iter().flat_map(|s| s.1.split_whitespace()).take(TITLE_WORDS).collect::<Vec<_>>().join(" ")
}

fn sent_name(n: u32, mime: Mime) -> String {
    format!("sent-{n}.{}", if mime == Mime::Png { "png" } else { "jpg" })
}

pub fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), String> {
    history::write_atomic(path, &serde_json::to_vec(v).map_err(|e| e.to_string())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::{Capture, CaptureKind};

    fn root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-sessions-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn png(v: u8) -> Vec<u8> {
        crate::image_ops::encode_png(&[v, 24, 24, 255], 1, 1).unwrap()
    }

    fn said(text: &str) -> Item {
        Item::Speech { t_ms: 0, end_ms: 900, text: text.into() }
    }

    fn shown(v: u8) -> Item {
        let capture = Capture {
            id: 0,
            kind: CaptureKind::Full,
            t_ms: 0,
            png: png(v),
            width: 1,
            height: 1,
            labels: vec![],
            pointer: None,
            trail: vec![],
            element: None,
            place: None,
            anchors: vec![],
            auto: false,
        };
        Item::Image { t_ms: 0, capture }
    }

    fn words(items: &[Item]) -> Question<'_> {
        Question { items, created_ms: 0, duration_ms: 900, audio: None, pictures: &[] }
    }

    const T0: u64 = 1_793_000_000_000;

    #[test]
    fn a_turn_folder_round_trips_with_its_question_its_answer_and_what_was_sent() {
        let s = Sessions::new(&root("roundtrip"));
        let items = [said("what does the denominator in this rule mean here"), shown(200)];
        // What is sent is not the original: other bytes, another size.
        let wire = [9u8, 8, 7, 6, 5];
        let asked = Answer {
            provider: "Scripted".into(),
            times: Times { press_ms: T0, release_ms: T0 + 900, words_ms: T0 + 950, sent_ms: T0 + 960, ..Times::default() },
            sent: Sent { text: "what does the denominator mean\n[image 1] 3x2".into(), ..Sent::default() },
            ..Answer::default()
        };
        let q = Question { created_ms: T0, pictures: &[Wire { bytes: &wire, w: 3, h: 2, mime: Mime::Jpeg }], ..words(&items) };
        let kept = s.save_turn(None, q, asked, "direct", T0 + 960).unwrap();
        assert_eq!(kept.session.id, kept.turn, "a new session is named for its first turn");
        assert_eq!((kept.session.title.as_str(), kept.session.turns, kept.session.pictures), ("what does the denominator in this", 1, 1));
        assert_eq!((kept.session.created_ms, kept.session.updated_ms, kept.session.style.as_str()), (T0, T0 + 960, "direct"));
        assert_eq!(kept.answer.sent.pictures.len(), 1);
        let p = &kept.answer.sent.pictures[0];
        assert_eq!((p.n, p.w, p.h, p.bytes, p.mime, p.sha256.len()), (1, 3, 2, 5, Mime::Jpeg, 64));
        assert_eq!(p.sha256, sha256(&wire));
        // The answer ends: the same file, written again; what was sent is as it was first written.
        let mut done = Answer { text: "The **evidence**.".into(), usage: Some(Used { input: 900, cached: 0, output: 12 }), ..kept.answer.clone() };
        done.times.first_ms = T0 + 2_000;
        done.times.done_ms = T0 + 3_000;
        let tampered = Answer { sent: Sent::default(), ..done.clone() };
        let after = s.write_answer(&kept.session.id, &kept.turn, &tampered, T0 + 3_000).unwrap();
        assert_eq!((after.updated_ms, after.turns, after.provider.as_str()), (T0 + 3_000, 1, "Scripted"));

        let (session, turns) = s.read(&kept.session.id).unwrap();
        assert_eq!(session, after);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].answer.as_ref(), Some(&done));
        assert_eq!(turns[0].meta.id, kept.turn);
        // The question is exactly a History entry: its words and its picture come back through the History store.
        assert_eq!(s.turns(&session.id).unwrap().items(&kept.turn).unwrap(), items);
        // On disk, the keys the plan names.
        let dir = s.turns(&session.id).unwrap().dir(&kept.turn).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("answer.json")).unwrap()).unwrap();
        for key in ["text", "provider", "model", "press_ms", "release_ms", "words_ms", "sent_ms", "first_ms", "done_ms", "stopped", "error", "usage", "sent"] {
            assert!(raw.get(key).is_some(), "answer.json has {key}");
        }
        assert_eq!(raw["sent"]["pictures"][0], serde_json::json!({ "n": 1, "w": 3, "h": 2, "bytes": 5, "sha256": sha256(&wire), "mime": "image/jpeg" }));
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("../../session.json")).unwrap()).unwrap();
        for key in ["id", "title", "created_ms", "updated_ms", "turns", "pictures", "style", "provider", "has_summary"] {
            assert!(raw.get(key).is_some(), "session.json has {key}");
        }
        assert!(!dir.join("answer.tmp").exists() && !s.root.join(format!("new-{}.tmp", std::process::id())).exists(), "nothing half written is left");
    }

    #[test]
    fn the_sent_picture_read_back_is_the_bytes_that_were_sent_not_the_original() {
        let s = Sessions::new(&root("sent"));
        let items = [said("and this one"), shown(90)];
        let (first, second) = (vec![1u8, 2, 3, 4, 5, 6, 7], png(33));
        let pictures = [Wire { bytes: &first, w: 4, h: 3, mime: Mime::Jpeg }, Wire { bytes: &second, w: 1, h: 1, mime: Mime::Png }];
        let kept = s.save_turn(None, Question { created_ms: T0, pictures: &pictures, ..words(&items) }, Answer::default(), "direct", T0).unwrap();
        let (file, bytes) = s.sent_picture(&kept.session.id, &kept.turn, 1).unwrap();
        assert_eq!((bytes.as_slice(), file.as_path()), (first.as_slice(), kept.pictures[0].as_path()));
        assert!(file.ends_with("sent-1.jpg") && std::fs::read(&file).unwrap() == first, "the file a provider is handed holds them");
        assert_eq!(s.sent_picture(&kept.session.id, &kept.turn, 2).unwrap().1, second);
        assert!(kept.pictures[1].ends_with("sent-2.png"));
        // The original stays what History kept, and is another file.
        let original = s.turns(&kept.session.id).unwrap().picture(&kept.turn, 1).unwrap().png;
        assert_eq!(original, png(90));
        assert_ne!(original, first);
        assert_eq!(kept.answer.sent.pictures.iter().map(|p| p.sha256.clone()).collect::<Vec<_>>(), [sha256(&first), sha256(&second)]);
        assert_eq!(sha256(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad", "the digest is SHA-256");
        assert!(s.sent_picture(&kept.session.id, &kept.turn, 3).is_err());
    }

    #[test]
    fn a_session_is_not_pruned_at_101_turns_while_history_still_keeps_100() {
        let d = root("keep");
        let (s, h) = (Sessions::new(&d), Store::new(&d));
        let items = [said("next")];
        let mut sid = None;
        for i in 0..(history::KEEP as u64 + 1) {
            let q = Question { created_ms: T0 + i * 1000, ..words(&items) };
            sid = Some(s.save_turn(sid.as_deref(), q, Answer::default(), "direct", T0 + i * 1000).unwrap().session.id);
            h.save(&items, T0 + i * 1000, 10).unwrap();
        }
        let (session, turns) = s.read(sid.as_deref().unwrap()).unwrap();
        assert_eq!((session.turns as usize, turns.len()), (history::KEEP + 1, history::KEEP + 1));
        assert_eq!((turns[0].meta.created_ms, turns[history::KEEP].meta.created_ms), (T0, T0 + history::KEEP as u64 * 1000), "oldest first, the first still there");
        assert_eq!(h.list().len(), history::KEEP, "History pruned the same run to its hundred");
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn a_session_keeps_the_audio_of_its_newest_ten_turns_only() {
        let s = Sessions::new(&root("audio"));
        let items = [said("listen")];
        let audio = || Some(history::wav(&history::pcm16(&vec![0.25f32; 1_600])));
        let mut sid = None;
        for i in 0..(KEEP_AUDIO as u64 + 3) {
            let q = Question { created_ms: T0 + i * 1000, audio: audio(), ..words(&items) };
            sid = Some(s.save_turn(sid.as_deref(), q, Answer::default(), "direct", T0).unwrap().session.id);
        }
        let (_, turns) = s.read(sid.as_deref().unwrap()).unwrap();
        let has: Vec<bool> = turns.iter().map(|t| t.meta.has_audio).collect();
        assert_eq!(has, (0..KEEP_AUDIO + 3).map(|i| i >= 3).collect::<Vec<_>>(), "oldest first: the three oldest lost theirs");
        let store = s.turns(sid.as_deref().unwrap()).unwrap();
        assert_eq!(store.audio(&turns[3].meta.id).unwrap().len(), 1_600);
        assert_eq!(store.audio(&turns[0].meta.id).unwrap_err(), history::NO_AUDIO);
        assert_eq!(store.items(&turns[0].meta.id).unwrap(), items, "their words stay");
    }

    #[test]
    fn sessions_list_newest_first_and_one_can_be_read_or_deleted_by_a_plain_id() {
        let s = Sessions::new(&root("list"));
        assert!(s.list().is_empty());
        let items = [said("first question of the older one")];
        let old = s.save_turn(None, Question { created_ms: T0, ..words(&items) }, Answer::default(), "direct", T0).unwrap();
        let new = s.save_turn(None, Question { created_ms: T0 + 5_000, ..words(&[said("newer")]) }, Answer::default(), "direct", T0 + 5_000).unwrap();
        assert_eq!(s.list().iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [new.session.id.clone(), old.session.id.clone()]);
        // A turn added to the older one brings it to the top; its title stays its first question.
        let again = s.save_turn(Some(&old.session.id), Question { created_ms: T0 + 9_000, ..words(&[said("a follow up")]) }, Answer::default(), "direct", T0 + 9_000).unwrap();
        assert_eq!((again.session.turns, again.session.title.as_str(), again.session.created_ms), (2, "first question of the older one", T0));
        assert_eq!(s.list()[0].id, old.session.id);
        assert_ne!(again.turn, old.turn);
        // Ids come from the UI.
        for bad in ["", "..", "../x", "a/b", "nope"] {
            assert_eq!(s.read(bad).unwrap_err(), "no such session", "{bad:?}");
            assert!(s.delete(bad).is_err() && s.save_turn(Some(bad), words(&items), Answer::default(), "direct", T0).is_err());
        }
        s.delete(&old.session.id).unwrap();
        assert_eq!(s.list().iter().map(|x| x.id.clone()).collect::<Vec<_>>(), [new.session.id]);
        assert!(s.read(&old.session.id).is_err());
    }
}
