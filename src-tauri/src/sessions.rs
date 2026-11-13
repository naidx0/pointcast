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
    /// The window it shows, by its title; empty for a whole screen or a window that is never named.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// The person marked it up before it was sent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub marked: bool,
}

/// A picture of an earlier turn that was sent again when the conversation was opened anew: the turn, by id, and the
/// picture's number in it. The bytes are that turn's `sent-<n>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Again {
    pub turn: String,
    pub n: u32,
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
    /// What the AI's own app sent or started along with the question (Codex: the person's own instructions file, and
    /// a server of theirs it started although Pointcast switched it off).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub adds: Vec<String>,
    /// The transcript of the earlier turns, exactly as it went along when the conversation was opened anew; empty
    /// when the open conversation already held them.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub replay: String,
    /// The earlier pictures sent again with that transcript, in the order they went.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub again: Vec<Again>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    /// The answer as it streamed, in markdown.
    pub text: String,
    pub provider: String,
    /// The model it was asked with, as the person picked it; none leaves the provider's own.
    pub model: Option<String>,
    /// The model that answered, as the provider reported it: an alias is resolved to its id here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_by: Option<String>,
    /// One line under the answer when something about it is worth a word (the answering model was not the one picked).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(flatten)]
    pub times: Times,
    /// The person stopped it; `text` is what had arrived.
    pub stopped: bool,
    pub error: Option<String>,
    pub usage: Option<Used>,
    pub sent: Sent,
    /// What the AI did on the computer during this answer, in order (`computer::journal`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub acts: Vec<crate::computer::Entry>,
}

/// A picture on its way out: the bytes the provider gets.
pub struct Wire<'a> {
    pub bytes: &'a [u8],
    pub w: u32,
    pub h: u32,
    pub mime: Mime,
    /// The title of the window it shows, or empty.
    pub label: &'a str,
    /// The person marked it up.
    pub marked: bool,
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
    /// The session's own folder; none is no session.
    pub fn dir(&self, id: &str) -> Result<PathBuf, String> {
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
            answer.sent.pictures.push(SentPicture { n, w: p.w, h: p.h, bytes: p.bytes.len() as u64, sha256: sha256(p.bytes), mime: p.mime, label: p.label.into(), marked: p.marked });
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

    /// Write a turn's answer again as it grew or ended. The words and pictures that were sent stay as they were
    /// written first; what went along with them (the earlier turns, and what the AI's own app added) is as this
    /// asking of the question sent it, which is known only once the conversation is open.
    pub fn write_answer(&self, session: &str, turn: &str, answer: &Answer, now_ms: u64) -> Result<Session, String> {
        let dir = self.dir(session)?;
        let store = Self::turns_in(&dir);
        let at = store.dir(turn)?.join(ANSWER);
        let mut next = answer.clone();
        if let Some(old) = read_json::<Answer>(&at) {
            next.sent.text = old.sent.text;
            next.sent.pictures = old.sent.pictures;
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

    /// A session's own line, without reading its turns.
    pub fn head(&self, id: &str) -> Result<Session, String> {
        read_json(&self.dir(id)?.join(SESSION)).ok_or_else(|| "no such session".into())
    }

    /// The answer kept for one turn, as far as it has come.
    pub fn answer(&self, session: &str, turn: &str) -> Option<Answer> {
        read_json(&self.turns(session).ok()?.dir(turn).ok()?.join(ANSWER))
    }

    /// Give a session another title. When it was last added to stays what it was.
    pub fn rename(&self, id: &str, title: &str) -> Result<Session, String> {
        let title: String = title.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(TITLE_MAX).collect();
        if title.is_empty() {
            return Err("A session needs a title.".into());
        }
        let at = self.dir(id)?.join(SESSION);
        let mut s: Session = read_json(&at).ok_or("no such session")?;
        s.title = title;
        write_json(&at, &s)?;
        Ok(s)
    }

    /// Say whether a summary is kept beside a session. When it was last added to stays what it was.
    pub fn set_summary(&self, id: &str, has: bool) -> Result<(), String> {
        let at = self.dir(id)?.join(SESSION);
        let mut s: Session = read_json(&at).ok_or("no such session")?;
        s.has_summary = has;
        write_json(&at, &s)
    }

    /// A session as the list shows it, with the bytes its folder holds.
    fn row(&self, s: Session, hit: Option<String>) -> Row {
        let name = crate::assist::catalog::name_of(&s.provider);
        Row {
            size_bytes: folder_bytes(&self.root.join(&s.id)),
            provider: if name.is_empty() { s.provider } else { name.into() },
            id: s.id,
            title: s.title,
            updated_ms: s.updated_ms,
            turns: s.turns,
            pictures: s.pictures,
            has_summary: s.has_summary,
            hit,
        }
    }

    /// Every session as the list shows it, newest first.
    pub fn rows(&self) -> Vec<Row> {
        self.list().into_iter().map(|s| self.row(s, None)).collect()
    }

    /// The sessions that hold `q`, whatever its case, in their title, a question, an answer or the name of a picture,
    /// newest first, each with the words around the first place it was found. A scan of the folders: there is no index.
    pub fn search(&self, q: &str) -> Vec<Row> {
        let q = q.trim();
        if q.is_empty() {
            return Vec::new();
        }
        let found = |s: &Session| -> Option<String> {
            if let Some(hit) = snippet(&s.title, q) {
                return Some(hit);
            }
            let (_, turns) = self.read(&s.id).ok()?;
            turns.iter().filter_map(|t| t.answer.as_ref()).find_map(|a| {
                let labels = a.sent.pictures.iter().map(|p| p.label.as_str());
                [question(&a.sent.text), a.text.as_str()].into_iter().chain(labels).find_map(|text| snippet(text, q))
            })
        };
        self.list().into_iter().filter_map(|s| found(&s).map(|hit| self.row(s, Some(hit)))).collect()
    }

    /// Every picture a session sent, in order, numbered through the session from 1.
    fn pictures(&self, id: &str, turns: &[Kept]) -> Vec<Shown> {
        let mut all = Vec::new();
        for (i, k) in turns.iter().enumerate() {
            for p in k.answer.iter().flat_map(|a| &a.sent.pictures) {
                if let Ok((file, _)) = self.sent_picture(id, &k.meta.id, p.n) {
                    all.push(Shown { n: all.len() as u32 + 1, turn: i as u32 + 1, file, w: p.w, h: p.h, mime: p.mime, label: p.label.clone() });
                }
            }
        }
        all
    }

    /// A session as markdown, for a reader that opens files: each turn under its anchor (`t1`, `t2`, ...), the
    /// question, its pictures as links to where they are kept, and the answer. With the pictures it links.
    pub fn markdown(&self, id: &str) -> Result<(Session, String, Vec<Shown>), String> {
        let (session, turns) = self.read(id)?;
        let pictures = self.pictures(id, &turns);
        let md = markdown_of(&session, &turns, &pictures, &|p| file_link(&p.file));
        Ok((session, md, pictures))
    }

    /// Write a session out for a person or an agent to read: `<root>/sessions/<YYYY-MM-DD_HHMM> <slug>/` with
    /// `session.md` (each turn under its anchor), its pictures as they were sent (`01.png`, ...) and `manifest.json`,
    /// which says `"kind": "session"` so that pruning the handoff folders never takes it. Exporting a session again
    /// writes its folder anew. A session with a summary gets that too, as `summary.md`. Returns the folder.
    pub fn export(&self, id: &str, root: &Path, now_ms: u64) -> Result<PathBuf, String> {
        use chrono::TimeZone;
        let (session, turns) = self.read(id)?;
        let pictures = self.pictures(id, &turns);
        let local = chrono::Local.timestamp_millis_opt(session.created_ms as i64).single().unwrap_or_else(chrono::Local::now);
        let base = format!("{} {}", local.format("%Y-%m-%d_%H%M"), slug(&session.title));
        let home = root.join(EXPORTS);
        std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
        // The folder this session was exported to before, else the first free name.
        let ours = |d: &Path| read_json::<serde_json::Value>(&d.join("manifest.json")).is_some_and(|m| m["kind"] == EXPORT_KIND && m["id"] == session.id.as_str());
        let dir = (1..1000)
            .map(|k| home.join(if k == 1 { base.clone() } else { format!("{base} {k}") }))
            .find(|d| !d.exists() || ours(d))
            .ok_or("no free folder name for the export")?;
        let width = if pictures.len() >= 100 { 3 } else { 2 };
        let name = |p: &Shown| format!("{:0width$}.{}", p.n, ext(p.mime));
        let write = |dir: &Path| -> Result<(), String> {
            for p in &pictures {
                std::fs::copy(&p.file, dir.join(name(p))).map_err(|e| e.to_string())?;
            }
            let md = markdown_of(&session, &turns, &pictures, &name);
            std::fs::write(dir.join("session.md"), md).map_err(|e| e.to_string())?;
            // Its summary, when it has one, beside it: each point's turn is a link into `session.md`.
            if let Some((summary, told)) = crate::summary::newest(self, id) {
                let md = crate::summary::to_markdown(&summary, &told).replace("](#t", "](session.md#t");
                std::fs::write(dir.join("summary.md"), md).map_err(|e| e.to_string())?;
            }
            let manifest = serde_json::json!({
                "app": "pointcast",
                "kind": EXPORT_KIND,
                "id": session.id,
                "title": session.title,
                "created_ms": session.created_ms,
                "created": local.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
                "exported_ms": now_ms,
                "turns": turns.len(),
                "pictures": pictures.iter().map(|p| serde_json::json!({ "n": p.n, "turn": p.turn, "file": name(p), "width": p.w, "height": p.h, "label": p.label })).collect::<Vec<_>>(),
            });
            std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
        };
        write_anew(&dir, &format!("{now_ms}"), write)?;
        Ok(dir)
    }
}

/// Fill `dir` anew through `fill`, which writes into an empty folder beside it: only once it is all written does the new
/// folder take the place of the old one. On any failure the folder is as it was, and nothing is left beside it.
fn write_anew(dir: &Path, stamp: &str, fill: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    let parent = dir.parent().ok_or("the export has no folder to go in")?;
    let base = dir.file_name().ok_or("the export has no name")?.to_string_lossy().into_owned();
    // Hidden, and no export's name: neither is ever taken for an export.
    let (new, old) = (parent.join(format!(".{base}.new-{stamp}")), parent.join(format!(".{base}.old-{stamp}")));
    let _ = std::fs::remove_dir_all(&new);
    crate::prompt::create_private(&new).map_err(|e| e.to_string())?;
    if let Err(e) = fill(&new) {
        let _ = std::fs::remove_dir_all(&new);
        return Err(e);
    }
    let had = dir.exists();
    if had {
        if let Err(e) = std::fs::rename(dir, &old) {
            let _ = std::fs::remove_dir_all(&new);
            return Err(e.to_string());
        }
    }
    if let Err(e) = std::fs::rename(&new, dir) {
        if had {
            let _ = std::fs::rename(&old, dir);
        }
        let _ = std::fs::remove_dir_all(&new);
        return Err(e.to_string());
    }
    if had {
        let _ = std::fs::remove_dir_all(&old);
    }
    Ok(())
}

/// A session's title is cut at this many characters.
pub const TITLE_MAX: usize = 120;

/// The folder under the handoff root that session exports go to, and what their manifest says they are.
pub const EXPORTS: &str = "sessions";
pub const EXPORT_KIND: &str = "session";

/// A session as the list shows it. `hit` is there only for a search: the words around what was found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub updated_ms: u64,
    pub turns: u32,
    pub pictures: u32,
    /// The AI that answered last, as the person knows it.
    pub provider: String,
    pub size_bytes: u64,
    /// A summary is kept beside it (`summary`).
    pub has_summary: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit: Option<String>,
}

/// A picture of a session as it was sent: its number through the session, the turn it is of (from 1) and its file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shown {
    pub n: u32,
    pub turn: u32,
    pub file: PathBuf,
    pub w: u32,
    pub h: u32,
    pub mime: Mime,
    pub label: String,
}

/// The words of a question without the caption line its picture was sent with (`live::caption`).
pub fn question(sent: &str) -> &str {
    match sent.rfind("\n\n[Picture: ") {
        Some(at) if sent.ends_with(']') => &sent[..at],
        _ => sent,
    }
}

/// The words around the first place `text` holds `q`, whatever the case of either; none when it does not.
fn snippet(text: &str, q: &str) -> Option<String> {
    const BEFORE: usize = 30;
    const AFTER: usize = 60;
    let fold = |c: char| c.to_lowercase().next().unwrap_or(c);
    let (hay, needle): (Vec<char>, Vec<char>) = (text.chars().collect(), q.chars().map(fold).collect());
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    let at = (0..=hay.len() - needle.len()).find(|i| hay[*i..*i + needle.len()].iter().zip(&needle).all(|(a, b)| fold(*a) == *b))?;
    let (from, to) = (at.saturating_sub(BEFORE), (at + needle.len() + AFTER).min(hay.len()));
    let words: String = hay[from..to].iter().map(|c| if c.is_whitespace() { ' ' } else { *c }).collect();
    Some(format!("{}{}{}", if from > 0 { "..." } else { "" }, words.trim(), if to < hay.len() { "..." } else { "" }))
}

/// A title as part of a folder name: its letters and digits in lower case, words joined by `-`, never empty.
fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect();
    let s: String = words.join("-").chars().take(40).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() {
        EXPORT_KIND.into()
    } else {
        s
    }
}

fn ext(mime: Mime) -> &'static str {
    if mime == Mime::Png {
        "png"
    } else {
        "jpg"
    }
}

/// A file as a link a markdown reader opens: `file:///C:/...`, with anything unusual percent-encoded.
fn file_link(path: &Path) -> String {
    let p = path.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let mut out = String::from(if p.starts_with('/') { "file://" } else { "file:///" });
    for b in p.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn markdown_of(session: &Session, turns: &[Kept], pictures: &[Shown], link: &dyn Fn(&Shown) -> String) -> String {
    use chrono::TimeZone;
    let when = chrono::Local.timestamp_millis_opt(session.created_ms as i64).single().map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default();
    let mut md = format!("# {}\n\n{when}, {} {}\n", session.title, turns.len(), if turns.len() == 1 { "turn" } else { "turns" });
    for (i, k) in turns.iter().enumerate() {
        let n = i as u32 + 1;
        md.push_str(&format!("\n<a id=\"t{n}\"></a>\n## {n}\n\n"));
        let Some(a) = &k.answer else { continue };
        md.push_str(&format!("**You:** {}\n", question(&a.sent.text)));
        for p in pictures.iter().filter(|p| p.turn == n) {
            let alt: String = if p.label.is_empty() { format!("picture {}", p.n) } else { p.label.replace(['[', ']'], "") };
            md.push_str(&format!("\n![{alt}]({})\n", link(p)));
        }
        if !a.text.trim().is_empty() {
            md.push_str(&format!("\n{}\n", a.text.trim()));
        }
        if a.stopped {
            md.push_str("\n_Stopped before it ended._\n");
        }
        if let Some(why) = &a.error {
            md.push_str(&format!("\n_No answer: {why}_\n"));
        }
    }
    md
}

/// The bytes of every file under `dir`.
fn folder_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => folder_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// The session's counts, from what is on disk now.
fn count(s: &mut Session, store: &Store, provider: &str, now_ms: u64) {
    let turns = store.list();
    s.turns = turns.len() as u32;
    // A question asked through the panel keeps its picture as it was sent, beside the entry, not in it.
    let sent = |m: &Meta| store.dir(&m.id).ok().and_then(|d| read_json::<Answer>(&d.join(ANSWER))).map_or(0, |a| a.sent.pictures.len() as u32);
    s.pictures = turns.iter().map(|m| m.images.max(sent(m))).sum();
    s.updated_ms = now_ms;
    if !provider.is_empty() {
        s.provider = provider.into();
    }
}

/// The first words of the question, in the order they were said; "…" when there were more, so a title cut short never
/// reads as a whole question.
fn title(items: &[Item]) -> String {
    let mut said: Vec<(u64, &str)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Speech { t_ms, text, .. } => Some((*t_ms, text.as_str())),
            _ => None,
        })
        .collect();
    said.sort_by_key(|s| s.0);
    let words: Vec<&str> = said.iter().flat_map(|s| s.1.split_whitespace()).collect();
    if words.len() <= TITLE_WORDS {
        return words.join(" ");
    }
    let head = words[..TITLE_WORDS].join(" ");
    format!("{}…", head.trim_end_matches([',', ';', ':', '.', '-']))
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
        let q = Question { created_ms: T0, pictures: &[Wire { bytes: &wire, w: 3, h: 2, mime: Mime::Jpeg, label: "", marked: false }], ..words(&items) };
        let kept = s.save_turn(None, q, asked, "direct", T0 + 960).unwrap();
        assert_eq!(kept.session.id, kept.turn, "a new session is named for its first turn");
        assert_eq!((kept.session.title.as_str(), kept.session.turns, kept.session.pictures), ("what does the denominator in this…", 1, 1));
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
        let pictures = [Wire { bytes: &first, w: 4, h: 3, mime: Mime::Jpeg, label: "", marked: false }, Wire { bytes: &second, w: 1, h: 1, mime: Mime::Png, label: "", marked: false }];
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

    /// A question asked through the panel: its words, one picture as sent with the name of its window, and its answer.
    fn asked(s: &Sessions, session: Option<&str>, at: u64, q: &str, a: &str, label: &str) -> Saved {
        let items = [said(q)];
        let picture = png((at / 1000) as u8);
        let wires = [Wire { bytes: &picture, w: 1, h: 1, mime: Mime::Png, label, marked: false }];
        let sent = Sent { text: format!("{q}\n\n[Picture: 1 x 1 pixels]"), ..Sent::default() };
        let answer = Answer { text: a.into(), provider: crate::assist::codex::ID.into(), sent, ..Answer::default() };
        s.save_turn(session, Question { created_ms: at, pictures: &wires, ..words(&items) }, answer, "", at).unwrap()
    }

    #[test]
    fn sessions_are_listed_with_their_size_renamed_and_found_by_a_word_in_a_question_an_answer_or_a_picture_label() {
        let s = Sessions::new(&root("search"));
        let a = asked(&s, None, T0, "what does the Mitochondrion do", "It makes ATP for the cell.", "Biology notes").session.id;
        let b = asked(&s, None, T0 + 5_000, "why is the sky blue", "Rayleigh scattering.", "").session.id;
        asked(&s, Some(&a), T0 + 9_000, "and the ribosome", "It builds proteins.", "Lecture 4.pdf");
        assert_eq!(question("and the ribosome\n\n[Picture: 1 x 1 pixels]"), "and the ribosome");
        assert_eq!(question("no picture [here]"), "no picture [here]");

        // The list: newest first, the AI by the name the person knows, the pictures that were sent, the bytes kept.
        let rows = s.rows();
        let seen: Vec<_> = rows.iter().map(|r| (r.id.as_str(), r.turns, r.pictures, r.provider.as_str(), r.updated_ms, r.hit.clone())).collect();
        assert_eq!(seen, [(a.as_str(), 2, 2, "ChatGPT", T0 + 9_000, None), (b.as_str(), 1, 1, "ChatGPT", T0 + 5_000, None)]);
        assert!(rows[0].size_bytes > rows[1].size_bytes && rows[1].size_bytes > png(0).len() as u64, "{rows:?}");
        let json = serde_json::to_value(&rows[0]).unwrap();
        let mut keys: Vec<&str> = json.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort();
        // Rule changed: a row says whether its session has a summary.
        assert_eq!(keys, ["has_summary", "id", "pictures", "provider", "size_bytes", "title", "turns", "updated_ms"], "no hit outside a search");
        assert_eq!(json["has_summary"], false);

        // The search, whatever the case: a title, a question, an answer, a picture's label; newest first.
        let ids = |q: &str| s.search(q).into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids("mitochondrion"), [a.clone()], "a word of a question");
        assert_eq!(ids("RAYLEIGH"), [b.clone()], "a word of an answer");
        assert_eq!(ids("lecture 4"), [a.clone()], "a picture's label");
        assert_eq!(ids("ribosome"), [a.clone()], "a later turn's question");
        assert_eq!(ids("the"), [a.clone(), b.clone()]);
        // Not what Pointcast wrote around the question, and nothing for no words.
        assert!(ids("pixels").is_empty() && ids("chloroplast").is_empty() && ids("  ").is_empty());
        let hit = s.search("atp").remove(0);
        assert_eq!((hit.hit.as_deref(), hit.turns), (Some("It makes ATP for the cell."), 2));
        assert_eq!(serde_json::to_value(&hit).unwrap()["hit"], "It makes ATP for the cell.");
        let long = format!("{} needle {}", "before ".repeat(20), "after ".repeat(20));
        let cut = snippet(&long, "NEEDLE").unwrap();
        assert!(cut.starts_with("...") && cut.ends_with("...") && cut.contains("needle") && cut.len() < 110, "{cut}");
        assert_eq!(snippet("short", "longer than it"), None);

        // Another title: found by it, and when it was last added to is as it was.
        let renamed = s.rename(&b, "  Light   and air ").unwrap();
        assert_eq!((renamed.title.as_str(), renamed.updated_ms), ("Light and air", T0 + 5_000));
        assert_eq!((s.head(&b).unwrap(), ids("light and AIR")), (renamed, vec![b.clone()]));
        assert!(s.rename(&b, "   ").is_err() && s.rename("nope", "x").is_err() && s.head("../x").is_err());
        assert_eq!(s.rename(&b, &"x".repeat(TITLE_MAX + 50)).unwrap().title.len(), TITLE_MAX);
        // A turn added later keeps the title it was given.
        assert_eq!(asked(&s, Some(&a), T0 + 12_000, "and the nucleus", "It holds the DNA.", "").session.title, "what does the Mitochondrion do");
        s.delete(&b).unwrap();
        assert!(ids("sky").is_empty() && s.rows().len() == 1);
    }

    #[test]
    fn every_link_of_an_export_is_a_file_in_it_and_pruning_the_handoff_folders_never_takes_an_export() {
        let d = root("export");
        let s = Sessions::new(&d);
        let id = asked(&s, None, T0, "What is: this/that?", "A **cell**.", "Notes [draft]").session.id;
        let second = asked(&s, Some(&id), T0 + 9_000, "and this", "A nucleus.", "");
        s.write_answer(&id, &second.turn, &Answer { stopped: true, ..second.answer.clone() }, T0 + 9_500).unwrap();
        let handoff = d.join("handoff");
        let dir = s.export(&id, &handoff, T0 + 20_000).unwrap();
        assert_eq!(dir.parent(), Some(handoff.join(EXPORTS).as_path()));
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with(" what-is-this-that") && name.as_bytes()[10] == b'_', "{name}");
        // The turns under their anchors, in order, with their words.
        let md = std::fs::read_to_string(dir.join("session.md")).unwrap();
        let at = |what: &str| md.find(what).unwrap_or_else(|| panic!("no {what} in {md}"));
        let order = [at("# What is: this/that?"), at(r#"<a id="t1"></a>"#), at("**You:** What is: this/that?"), at("A **cell**."), at(r#"<a id="t2"></a>"#), at("**You:** and this"), at("A nucleus."), at("_Stopped before it ended._")];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{md}");
        assert!(!md.contains("[Picture:"), "the caption line is not part of the question: {md}");
        // Every link is a file in the folder, and each picture is the bytes that were sent.
        let links: Vec<&str> = md.split("](").skip(1).map(|rest| rest.split(')').next().unwrap()).collect();
        assert_eq!(links, ["01.png", "02.png"]);
        assert!(md.contains("![Notes draft](01.png)") && md.contains("![picture 2](02.png)"), "{md}");
        assert_eq!((std::fs::read(dir.join("01.png")).unwrap(), std::fs::read(dir.join("02.png")).unwrap()), (png((T0 / 1000) as u8), png((T0 / 1000 + 9) as u8)));
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!((&manifest["kind"], &manifest["app"], &manifest["id"], &manifest["turns"]), (&serde_json::json!("session"), &serde_json::json!("pointcast"), &serde_json::json!(id), &serde_json::json!(2)));
        let files: Vec<&str> = manifest["pictures"].as_array().unwrap().iter().map(|p| p["file"].as_str().unwrap()).collect();
        assert_eq!(files, links, "the manifest names the same files");
        assert_eq!(manifest["pictures"][0], serde_json::json!({ "n": 1, "turn": 1, "file": "01.png", "width": 1, "height": 1, "label": "Notes [draft]" }));
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 4, "session.md, manifest.json and the two pictures");
        // Exported again: the same folder, written anew.
        std::fs::remove_file(dir.join("02.png")).unwrap();
        assert_eq!(s.export(&id, &handoff, T0 + 30_000).unwrap(), dir);
        assert!(dir.join("02.png").is_file());
        // Another session with the same first words at the same minute gets a folder of its own.
        let twin = asked(&s, None, T0 + 30_000, "What is: this/that?", "Another.", "").session.id;
        let other = s.export(&twin, &handoff, T0 + 31_000).unwrap();
        assert!(other != dir && other.file_name().unwrap().to_string_lossy().ends_with(" what-is-this-that 2"), "{other:?}");

        // Pruning the handoff folders years later, from the root or from the exports' own folder, takes none of it.
        let later = T0 + 3 * 365 * 86_400_000;
        crate::prompt::prune_handoff(&handoff, later, None);
        crate::prompt::prune_handoff(&handoff.join(EXPORTS), later, None);
        assert!(dir.join("session.md").is_file() && other.join("session.md").is_file());
        // Even an export that lies where handoff folders do, named like one, with a manifest a handoff's reader can
        // read (no pictures): it says it is a session, so it stays. A handoff folder beside it, as old, goes.
        let bare = s.save_turn(None, Question { created_ms: T0 + 60_000, ..words(&[said("only words")]) }, Answer::default(), "", T0 + 60_000).unwrap().session.id;
        let exported = s.export(&bare, &handoff, T0 + 61_000).unwrap();
        let (like, plain) = (handoff.join("2020-01-01_0000"), handoff.join("2020-01-01_0001"));
        std::fs::rename(&exported, &like).unwrap();
        let raw = std::fs::read(like.join("manifest.json")).unwrap();
        let read: crate::prompt::Manifest = serde_json::from_slice(&raw).expect("a handoff's reader reads it");
        assert_eq!(read.kind, EXPORT_KIND);
        std::fs::create_dir_all(&plain).unwrap();
        let mut as_handoff: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        as_handoff.as_object_mut().unwrap().remove("kind");
        std::fs::write(plain.join("manifest.json"), as_handoff.to_string()).unwrap();
        crate::prompt::prune_handoff(&handoff, later, None);
        assert!(like.join("session.md").is_file(), "an export is never pruned");
        assert!(!plain.exists(), "and the scan does prune a handoff folder as old");
    }

    #[test]
    fn an_export_written_again_that_fails_leaves_the_earlier_one_whole_and_nothing_beside_it() {
        let d = root("export-again");
        let s = Sessions::new(&d);
        let id = asked(&s, None, T0, "what is this", "A cell.", "").session.id;
        let handoff = d.join("handoff");
        let dir = s.export(&id, &handoff, T0 + 20_000).unwrap();
        let before: Vec<(String, Vec<u8>)> = {
            let mut v: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect();
            v.sort();
            v
        };
        assert_eq!(before.len(), 3, "session.md, manifest.json and its picture");
        // A write that fails partway, as a picture that cannot be copied does: the earlier export stands as it was.
        let failed = write_anew(&dir, "1", |into| {
            std::fs::write(into.join("session.md"), "half").unwrap();
            Err("a picture could not be copied".into())
        });
        assert_eq!(failed, Err("a picture could not be copied".to_owned()));
        let mut after: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| (e.file_name().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap())).collect();
        after.sort();
        assert_eq!(after, before);
        assert_eq!(std::fs::read_dir(handoff.join(EXPORTS)).unwrap().count(), 1, "no half-written folder beside it");
        // One that succeeds takes its place whole, and leaves nothing beside it either.
        let fill = |into: &Path| -> Result<(), String> {
            std::fs::copy(dir.join("manifest.json"), into.join("manifest.json")).map_err(|e| e.to_string())?;
            std::fs::write(into.join("session.md"), "new").map_err(|e| e.to_string())
        };
        assert_eq!(write_anew(&dir, "2", fill), Ok(()));
        assert_eq!((std::fs::read_to_string(dir.join("session.md")).unwrap(), std::fs::read_dir(&dir).unwrap().count()), ("new".to_owned(), 2));
        assert_eq!(std::fs::read_dir(handoff.join(EXPORTS)).unwrap().count(), 1);
        // And through the export itself: written again, the same folder, whole.
        assert_eq!(s.export(&id, &handoff, T0 + 30_000).unwrap(), dir);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 3);
        assert_eq!(std::fs::read_dir(handoff.join(EXPORTS)).unwrap().count(), 1);
    }
}
