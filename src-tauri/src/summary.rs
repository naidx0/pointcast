//! Summarize: a session's key points and open questions, each with its receipt. The AI is asked for JSON in which
//! every point cites one turn and copies a quote from it; faithfulness is then enforced here, not asked for: a point
//! whose quote is not in the turn it cites is dropped and counted. A summary is faithful to the session, not a
//! guarantee the session was right, so a point from an answer says it is the AI's explanation. It is asked through
//! `Provider::once`, never the session's own conversation, and kept beside the session as `summary.json`.

// Nothing calls this module yet: the wiring slice adds the command and the panel, and removes this line.
#![allow(dead_code)]

use crate::assist::{Event, Failure, Kind, Part, Provider, Role, Turn};
use crate::history;
use crate::prompt::{self, Item};
use crate::sessions::Sessions;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::Duration;

/// A summary holds at most this many of each; what the AI wrote past them is left out.
pub const KEY_POINTS: usize = 7;
pub const OPEN_QUESTIONS: usize = 5;

/// What the person reads when the reply was not the JSON asked for, twice.
pub const UNREADABLE: &str = "The summary came back in a form Pointcast could not read. Try again.";

const FILE: &str = "summary.json";

const SYSTEM: &str = r#"Summarise this study session for the person who had it. Return JSON only, in this shape:
{ "title": "", "key_points": [P], "open_questions": [P] }
P = { "text": "", "turn": n, "from": "question" | "answer", "quote": "" }
Rules:
- Every point cites one turn by its number (T3 is 3) and copies a quote of 12 to 200 characters from that turn, character for character. "from" says whether the quote is in the turn's question (Q) or its answer (A).
- Use only what is in the session. If the session does not say it, leave it out. Do not add background knowledge.
- "open_questions" is what the person asked and did not get resolved, or said they still do not understand.
- At most 7 key points and 5 open questions. Fewer is fine. Empty lists are fine."#;

/// One turn as Summarize reads it: what was said, each picture by its label, and the answer in markdown.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Moment {
    pub id: String,
    pub at_ms: u64,
    pub question: String,
    /// One label a picture, in the order they were taken; empty for a picture nothing names.
    pub pictures: Vec<String>,
    pub answer: String,
}

/// A session as Summarize reads it, oldest turn first, and the folder its summary is kept in.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Told {
    pub id: String,
    pub title: String,
    pub dir: PathBuf,
    pub turns: Vec<Moment>,
}

/// Which side of a turn a quote is from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum From {
    Question,
    Answer,
}

/// A point that kept its receipt: `turn` counts from 1, and `quote` is in that turn's `from`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Point {
    pub text: String,
    pub turn: u32,
    pub from: From,
    pub quote: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub title: String,
    pub key_points: Vec<Point>,
    pub open_questions: Vec<Point>,
    /// The points here, and the points the AI wrote that had no source.
    pub kept: u32,
    pub dropped: u32,
    pub made_ms: u64,
    pub provider: String,
    pub model: Option<String>,
}

/// A point as the AI wrote it, nothing checked: `turn` is 0 and `from` is empty when it named none.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawPoint {
    pub text: String,
    pub turn: u32,
    pub from: String,
    pub quote: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Raw {
    pub title: String,
    pub key_points: Vec<RawPoint>,
    pub open_questions: Vec<RawPoint>,
}

/// A session read from disk for a summary: each turn's words in the order they were said, its pictures by label and
/// its answer.
pub fn told(sessions: &Sessions, id: &str) -> Result<Told, String> {
    let (session, kept) = sessions.read(id)?;
    let store = sessions.turns(id)?;
    // `Sessions` does not say where a session is: its folder is two above any of its turns.
    let first = kept.first().ok_or("no such session")?;
    let dir = store.dir(&first.meta.id)?.ancestors().nth(2).map(Path::to_path_buf).ok_or("no such session")?;
    let turns = kept
        .into_iter()
        .map(|k| {
            let items = store.items(&k.meta.id).unwrap_or_default();
            moment(k.meta.id, k.meta.created_ms, items, k.answer.map(|a| a.text).unwrap_or_default())
        })
        .collect();
    Ok(Told { id: session.id, title: session.title, dir, turns })
}

fn moment(id: String, at_ms: u64, items: Vec<Item>, answer: String) -> Moment {
    let (mut said, mut pictures) = (Vec::new(), Vec::new());
    for i in prompt::ordered(items) {
        match i {
            Item::Speech { text, .. } => said.push(text.trim().to_string()),
            Item::Image { capture, .. } => pictures.push(label(&capture)),
        }
    }
    said.retain(|s| !s.is_empty());
    Moment { id, at_ms, question: said.join(" "), pictures, answer }
}

/// What names a picture in words: the control the person pointed at, then its pins, steps and notes.
fn label(c: &prompt::Capture) -> String {
    let mut bits = Vec::new();
    if let Some(e) = &c.element {
        let name = e.name.split_whitespace().collect::<Vec<_>>().join(" ").replace('"', "'");
        let named = if name.is_empty() { e.role.trim().to_string() } else { format!(r#"{} "{name}""#, e.role.trim()) };
        bits.push(named.trim().to_string());
    }
    bits.extend(prompt::caption_marks(&c.labels));
    bits.retain(|b| !b.is_empty());
    bits.join("; ")
}

/// The time of day a turn was asked, on this computer's clock.
fn hhmm(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()
}

/// What is sent for a summary: the instructions, and the whole session as numbered turns in one turn of words. No
/// picture goes along; each is there by its label.
pub fn request(session: &Told) -> (String, Vec<Turn>) {
    let mut text = format!("The session, {} turns:", session.turns.len());
    for (i, t) in session.turns.iter().enumerate() {
        let question = if t.question.is_empty() { "(nothing said)" } else { &t.question };
        text.push_str(&format!("\n\nT{} [{}]\nQ: {question}", i + 1, hhmm(t.at_ms)));
        for (k, label) in t.pictures.iter().enumerate() {
            match label.as_str() {
                "" => text.push_str(&format!("\n[image {}]", k + 1)),
                l => text.push_str(&format!("\n[image {}: {l}]", k + 1)),
            }
        }
        let answer = if t.answer.trim().is_empty() { "(no answer)" } else { t.answer.trim() };
        text.push_str(&format!("\nA: {answer}"));
    }
    (SYSTEM.to_string(), vec![Turn { role: Role::Person, parts: vec![Part::Text(text)] }])
}

/// The summary in a reply, whatever is around it: a code fence, or a sentence before and after. A point that is not
/// in the shape asked for is kept as it is, for `verify` to drop and count.
pub fn parse(reply: &str) -> Result<Raw, String> {
    let lists = ["key_points", "open_questions"];
    let found = reply.char_indices().filter(|(_, c)| *c == '{').find_map(|(i, _)| {
        let v = serde_json::Deserializer::from_str(&reply[i..]).into_iter::<Value>().next()?.ok()?;
        (v.is_object() && (v.get("title").is_some() || lists.iter().any(|k| v.get(k).is_some()))).then_some(v)
    });
    let v = found.ok_or(UNREADABLE)?;
    let points = |key: &str| v.get(key).and_then(Value::as_array).map(|a| a.iter().map(raw_point).collect()).unwrap_or_default();
    Ok(Raw { title: words(&v, "title"), key_points: points(lists[0]), open_questions: points(lists[1]) })
}

fn words(v: &Value, key: &str) -> String {
    v.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string()
}

fn raw_point(v: &Value) -> RawPoint {
    // A turn is asked for as a number; "T3" and "3" are read as one too.
    let turn = match v.get("turn") {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0),
        Some(Value::String(s)) => s.trim().trim_start_matches(['T', 't', '#']).parse().unwrap_or(0),
        _ => 0,
    };
    RawPoint { text: words(v, "text"), turn, from: words(v, "from").to_lowercase(), quote: words(v, "quote") }
}

/// Text as a quote is compared: lowercase, markdown's emphasis marks gone, every run of whitespace one space.
fn plain(s: &str) -> String {
    let s: String = s.chars().filter(|c| !matches!(c, '*' | '_' | '`' | '~')).flat_map(char::to_lowercase).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The point, when the turn it cites is there and holds its quote on the side it names.
fn receipt(p: &RawPoint, session: &Told) -> Option<Point> {
    let from = match p.from.as_str() {
        "question" => From::Question,
        "answer" => From::Answer,
        _ => return None,
    };
    let turn = session.turns.get((p.turn as usize).checked_sub(1)?)?;
    let quote = plain(&p.quote);
    let source = plain(if from == From::Question { &turn.question } else { &turn.answer });
    (!p.text.is_empty() && !quote.is_empty() && source.contains(&quote)).then(|| Point { text: p.text.clone(), turn: p.turn, from, quote: p.quote.clone() })
}

/// The points that kept their receipt, `most` at the most, and how many had none.
fn checked(points: &[RawPoint], most: usize, session: &Told) -> (Vec<Point>, u32) {
    let (mut kept, mut dropped) = (Vec::new(), 0);
    for p in points {
        match receipt(p, session) {
            Some(point) if kept.len() < most => kept.push(point),
            Some(_) => {}
            None => dropped += 1,
        }
    }
    (kept, dropped)
}

/// Hold every point to the session: one without a source is dropped and counted. Who made it and when is `run`'s to
/// say.
pub fn verify(raw: Raw, session: &Told) -> Summary {
    let (key_points, a) = checked(&raw.key_points, KEY_POINTS, session);
    let (open_questions, b) = checked(&raw.open_questions, OPEN_QUESTIONS, session);
    Summary {
        title: if raw.title.is_empty() { session.title.clone() } else { raw.title },
        kept: (key_points.len() + open_questions.len()) as u32,
        dropped: a + b,
        key_points,
        open_questions,
        ..Summary::default()
    }
}

/// One whole reply through a conversation of its own, closed when it ends. `cancel` stops it.
fn reply(provider: &dyn Provider, system: &str, turns: &[Turn], cancel: &AtomicBool) -> Result<String, Failure> {
    let Some((last, earlier)) = turns.split_last() else { return Ok(String::new()) };
    let mut chat = provider.once(system, earlier)?;
    let (tx, rx) = channel();
    let id = match chat.ask(last.clone(), tx) {
        Ok(id) => id,
        Err(f) => {
            chat.close();
            return Err(f);
        }
    };
    let mut text = String::new();
    let end = loop {
        if cancel.load(Ordering::SeqCst) {
            chat.cancel();
            break Err(Failure::Stopped);
        }
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Event { turn, kind }) if turn == id => match kind {
                Kind::Delta(d) => text.push_str(&d),
                Kind::Done => break Ok(()),
                Kind::Failed(f) => break Err(f),
                _ => {}
            },
            Ok(_) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break Err(Failure::Broken("The answer ended without saying so.".into())),
        }
    };
    chat.close();
    end.map(|_| text)
}

/// Summarize `session` through `provider`. A reply that is not the JSON asked for is asked for once more, the same
/// way; a second one fails with `UNREADABLE`. A session without turns is an empty summary and nothing is sent.
pub fn run(provider: &dyn Provider, session: &Told, cancel: &AtomicBool) -> Result<Summary, Failure> {
    let made = |s: Summary| Summary { made_ms: crate::session::now_ms(), provider: provider.id().into(), ..s };
    if session.turns.is_empty() {
        return Ok(made(Summary { title: session.title.clone(), ..Summary::default() }));
    }
    let (system, turns) = request(session);
    let raw = match parse(&reply(provider, &system, &turns, cancel)?) {
        Ok(raw) => raw,
        Err(_) => parse(&reply(provider, &system, &turns, cancel)?).map_err(Failure::Broken)?,
    };
    Ok(made(verify(raw, session)))
}

/// Keep `summary` beside its session. The one it replaces stays, as `summary-<when it was made>.json`.
pub fn save(session: &Told, summary: &Summary) -> Result<(), String> {
    let at = session.dir.join(FILE);
    if at.is_file() {
        let ms = load(session).map_or(summary.made_ms, |old| old.made_ms);
        std::fs::rename(&at, session.dir.join(format!("summary-{ms}.json"))).map_err(|e| e.to_string())?;
    }
    history::write_atomic(&at, &serde_json::to_vec(summary).map_err(|e| e.to_string())?)
}

/// The session's newest summary, when it has one.
pub fn load(session: &Told) -> Option<Summary> {
    std::fs::read(session.dir.join(FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok())
}

/// "9 kept, 1 dropped: no source": the line a summary shows about its own receipts.
pub fn counts(summary: &Summary) -> String {
    match summary.dropped {
        0 => format!("{} kept", summary.kept),
        n => format!("{} kept, {n} dropped: no source", summary.kept),
    }
}

/// The summary as markdown, for export: each point with the turn it cites as a `#tN` link, whose words they are, and
/// its quote under it.
pub fn to_markdown(summary: &Summary, session: &Told) -> String {
    let title = if summary.title.is_empty() { &session.title } else { &summary.title };
    let mut md = format!("# {title}\n\nA summary of {} turns. {}.\n", session.turns.len(), counts(summary));
    for (heading, points) in [("Key points", &summary.key_points), ("Open questions", &summary.open_questions)] {
        if points.is_empty() {
            continue;
        }
        md.push_str(&format!("\n## {heading}\n\n"));
        for p in points {
            let whose = if p.from == From::Question { "your question" } else { "the AI's explanation" };
            let quote = p.quote.split_whitespace().collect::<Vec<_>>().join(" ");
            md.push_str(&format!("- {} ([T{n}](#t{n}), {whose})\n  > {quote}\n", p.text, n = p.turn));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assist::Scripted;
    use crate::prompt::{Capture, CaptureKind, Label, LabelKind};
    use crate::sessions::{Answer, Question};
    use serde_json::json;

    const T0: u64 = 1_793_000_000_000;

    /// Two turns, held in memory: nothing on disk.
    fn session() -> Told {
        let turn = |n: u64, question: &str, answer: &str| Moment {
            id: format!("turn-{n}"),
            at_ms: T0 + n * 60_000,
            question: question.into(),
            pictures: vec![],
            answer: answer.into(),
        };
        Told {
            id: "s".into(),
            title: "what does the denominator in this".into(),
            dir: PathBuf::new(),
            turns: vec![
                turn(0, "what does the denominator in this rule mean here", "It is the **total evidence**:\nevery way the data\ncould have come about."),
                turn(1, "so why does a rare disease make the answer small", "Because the prior is small, and the prior multiplies the likelihood."),
            ],
        }
    }

    fn point(text: &str, turn: u32, from: &str, quote: &str) -> RawPoint {
        RawPoint { text: text.into(), turn, from: from.into(), quote: quote.into() }
    }

    fn only(p: RawPoint) -> Raw {
        Raw { key_points: vec![p], ..Raw::default() }
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-summary-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn an_invented_quote_is_dropped_and_counted() {
        let reply = json!({
            "title": "Bayes' rule",
            "key_points": [
                { "text": "The denominator is the total evidence.", "turn": 1, "from": "answer", "quote": "every way the data could have come about" },
                { "text": "It always sums to one.", "turn": 1, "from": "answer", "quote": "the denominator always sums to one" },
            ],
            "open_questions": [{ "text": "Why a rare disease gives a small answer.", "turn": 2, "from": "question", "quote": "why does a rare disease make the answer small" }],
        });
        let script = Scripted::new(&[(1, &reply.to_string())]);
        let s = run(&script, &session(), &AtomicBool::new(false)).unwrap();
        assert_eq!((s.kept, s.dropped, s.title.as_str(), s.provider.as_str()), (2, 1, "Bayes' rule", "scripted"));
        assert_eq!(s.key_points, [Point { text: "The denominator is the total evidence.".into(), turn: 1, from: From::Answer, quote: "every way the data could have come about".into() }]);
        assert_eq!((s.open_questions.len(), s.open_questions[0].turn, s.open_questions[0].from), (1, 2, From::Question));
        assert!(s.made_ms >= T0);
        assert_eq!(counts(&s), "2 kept, 1 dropped: no source");
        // Asked once, in a conversation of its own that starts from nothing.
        let (system, turns) = request(&session());
        assert_eq!(*script.opened.lock().unwrap(), [(system, vec![])]);
        assert_eq!(*script.asked.lock().unwrap(), turns);
    }

    #[test]
    fn a_quote_differing_only_in_case_spacing_or_emphasis_marks_is_kept() {
        for quote in ["it is the total evidence: every way", "It  is the _total evidence_:   every way", "IT IS THE **TOTAL EVIDENCE**:\n\tEVERY WAY"] {
            let s = verify(only(point("p", 1, "answer", quote)), &session());
            assert_eq!((s.kept, s.dropped), (1, 0), "{quote:?}");
            assert_eq!(s.key_points[0].quote, quote, "the quote is kept as it was written");
        }
        // Another word is another quote, and a quote of nothing but marks is no receipt.
        for quote in ["it is the whole evidence: every way", "** **", ""] {
            assert_eq!(verify(only(point("p", 1, "answer", quote)), &session()).dropped, 1, "{quote:?}");
        }
        assert_eq!(verify(Raw::default(), &session()).title, "what does the denominator in this", "without a title, the session's");
    }

    #[test]
    fn a_wrong_turn_number_is_dropped() {
        // The quote is in turn 2 and nowhere else.
        let quote = "the prior multiplies the likelihood";
        assert_eq!(verify(only(point("p", 2, "answer", quote)), &session()).kept, 1);
        for turn in [0, 1, 3, u32::MAX] {
            let s = verify(only(point("p", turn, "answer", quote)), &session());
            assert_eq!((s.kept, s.dropped), (0, 1), "turn {turn}");
        }
    }

    #[test]
    fn a_quote_from_the_answer_cited_as_the_question_is_dropped() {
        let quote = "every way the data could have come about";
        assert_eq!(verify(only(point("p", 1, "answer", quote)), &session()).kept, 1);
        for from in ["question", "", "both"] {
            let s = verify(only(point("p", 1, from, quote)), &session());
            assert_eq!((s.kept, s.dropped), (0, 1), "{from:?}");
        }
    }

    #[test]
    fn points_past_the_most_a_summary_holds_are_left_out_and_not_counted_as_dropped() {
        let many = |n: usize| (0..n).map(|i| point(&format!("p{i}"), 1, "question", "what does the denominator")).collect::<Vec<_>>();
        let s = verify(Raw { title: String::new(), key_points: many(9), open_questions: many(6) }, &session());
        assert_eq!((s.key_points.len(), s.open_questions.len(), s.kept, s.dropped), (KEY_POINTS, OPEN_QUESTIONS, 12, 0));
    }

    #[test]
    fn bad_json_is_retried_once_and_then_fails_with_a_plain_message() {
        let script = Scripted::new(&[(1, "Here are the key points: the denominator is the evidence {and so on}.")]);
        assert_eq!(run(&script, &session(), &AtomicBool::new(false)), Err(Failure::Broken(UNREADABLE.into())));
        let (opened, asked) = (script.opened.lock().unwrap(), script.asked.lock().unwrap());
        assert_eq!((opened.len(), asked.len()), (2, 2), "asked twice, each in its own conversation");
        assert_eq!((&opened[0], &asked[0]), (&opened[1], &asked[1]), "the same request");
        // A provider that fails is not asked again, and says why itself.
        let offline = Scripted::new(&[(1, "{")]).failing(Failure::Offline);
        assert_eq!(run(&offline, &session(), &AtomicBool::new(false)), Err(Failure::Offline));
        assert_eq!(offline.asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn json_inside_a_code_fence_parses() {
        let body = json!({ "title": "T", "key_points": [{ "text": "a", "turn": "T2", "from": "Answer", "quote": "q {braces}" }, "not a point"], "open_questions": [] });
        let want = Raw { title: "T".into(), key_points: vec![point("a", 2, "answer", "q {braces}"), RawPoint::default()], open_questions: vec![] };
        let pretty = serde_json::to_string_pretty(&body).unwrap();
        for reply in [body.to_string(), format!("```json\n{pretty}\n```"), format!("Here is the summary {{as asked}}:\n\n```\n{body}\n```\nAnything else?")] {
            assert_eq!(parse(&reply).as_ref(), Ok(&want), "{reply}");
        }
        // What is not the shape asked for is one plain error: no object, another object, half an object.
        for reply in ["", "I cannot summarise this.", "{\"error\": \"busy\"}", "[1, 2]", &pretty[..pretty.len() / 2]] {
            assert_eq!(parse(reply), Err(UNREADABLE.to_string()), "{reply}");
        }
        // A point that is not in shape is dropped and counted, not a reason to fail.
        assert_eq!(verify(want, &session()).dropped, 2);
    }

    #[test]
    fn an_empty_session_never_calls_the_provider() {
        let script = Scripted::new(&[(1, "{}")]);
        let empty = Told { title: "nothing yet".into(), ..Told::default() };
        let s = run(&script, &empty, &AtomicBool::new(false)).unwrap();
        assert_eq!((s.title.as_str(), s.kept, s.dropped, s.key_points.len(), s.open_questions.len()), ("nothing yet", 0, 0, 0, 0));
        assert!(script.opened.lock().unwrap().is_empty() && script.asked.lock().unwrap().is_empty());
    }

    #[test]
    fn a_summary_that_is_cancelled_ends_stopped() {
        let script = Scripted::new(&[(5_000, "{}")]);
        assert_eq!(run(&script, &session(), &AtomicBool::new(true)), Err(Failure::Stopped));
    }

    fn said(t_ms: u64, text: &str) -> Item {
        Item::Speech { t_ms, end_ms: t_ms + 900, text: text.into() }
    }

    fn shown(t_ms: u64, labels: Vec<Label>, element: Option<crate::element::ElementMark>) -> Item {
        let capture = Capture {
            id: 0,
            kind: CaptureKind::Full,
            t_ms,
            png: crate::image_ops::encode_png(&[9, 24, 24, 255], 1, 1).unwrap(),
            width: 1,
            height: 1,
            labels,
            pointer: None,
            trail: vec![],
            element,
            place: None,
            anchors: vec![],
            auto: false,
        };
        Item::Image { t_ms, capture }
    }

    /// A session of two turns on disk, the first with two pictures and the second with no answer.
    fn kept(name: &str) -> (Sessions, Told) {
        let sessions = Sessions::new(&scratch(name));
        let button = crate::element::ElementMark { role: "button".into(), name: "Place  order".into(), rect: Default::default() };
        let pin = Label { n: 1, text: "too far left".into(), kind: LabelKind::Pin };
        let first = [said(0, "what does this button do"), shown(400, vec![pin], Some(button)), said(900, "and this page"), shown(1_500, vec![], None)];
        let q = |items, created_ms| Question { items, created_ms, duration_ms: 2_000, audio: None, pictures: &[] };
        let a = sessions.save_turn(None, q(&first, T0), Answer { text: "It places the **order**.".into(), ..Answer::default() }, "direct", T0).unwrap();
        let second = [said(0, "is it safe to press")];
        sessions.save_turn(Some(&a.session.id), q(&second, T0 + 60_000), Answer::default(), "direct", T0 + 60_000).unwrap();
        let told = told(&sessions, &a.session.id).unwrap();
        (sessions, told)
    }

    #[test]
    fn the_request_numbers_turns_and_labels_pictures() {
        let (_sessions, told) = kept("request");
        assert_eq!(told.title, "what does this button do and");
        assert_eq!(told.turns.iter().map(|t| t.question.as_str()).collect::<Vec<_>>(), ["what does this button do and this page", "is it safe to press"]);
        let (system, turns) = request(&told);
        assert_eq!(turns.len(), 1, "one turn of words, no picture");
        let [Part::Text(text)] = turns[0].parts.as_slice() else { panic!("words only: {:?}", turns[0].parts) };
        let (one, two) = (hhmm(T0), hhmm(T0 + 60_000));
        assert_eq!((one.len(), &one[2..3]), (5, ":"), "a time of day: {one}");
        assert_eq!(
            text.lines().collect::<Vec<_>>(),
            [
                "The session, 2 turns:",
                "",
                &format!("T1 [{one}]"),
                "Q: what does this button do and this page",
                "[image 1: button \"Place order\"; pins: 1=too far left]",
                "[image 2]",
                "A: It places the **order**.",
                "",
                &format!("T2 [{two}]"),
                "Q: is it safe to press",
                "A: (no answer)",
            ]
        );
        for asked in ["JSON only", "\"key_points\"", "\"open_questions\"", "\"question\" | \"answer\"", "12 to 200 characters", "character for character", "At most 7 key points and 5 open questions"] {
            assert!(system.contains(asked), "the instructions say {asked}");
        }
        for out in ["cards", "quiz", "learned"] {
            assert!(!system.contains(out), "{out} is a later phase");
        }
        // What was read from disk is what a point is held to.
        let s = verify(only(point("p", 1, "answer", "it places the order")), &told);
        assert_eq!((s.kept, s.dropped), (1, 0));
    }

    #[test]
    fn save_then_load_round_trips_and_keeps_the_previous_file() {
        let (sessions, told) = kept("save");
        assert!(told.dir.join("session.json").is_file(), "the summary is kept in the session's own folder");
        assert_eq!(load(&told), None);
        let p = |text: &str, from| Point { text: text.into(), turn: 1, from, quote: "It places the order".into() };
        let first = Summary { title: "Buttons".into(), key_points: vec![p("one", From::Answer)], open_questions: vec![p("two", From::Question)], kept: 2, dropped: 1, made_ms: T0 + 5, provider: "scripted".into(), model: Some("m-1".into()) };
        save(&told, &first).unwrap();
        assert_eq!(load(&told).as_ref(), Some(&first));
        let raw: Value = serde_json::from_slice(&std::fs::read(told.dir.join("summary.json")).unwrap()).unwrap();
        for key in ["title", "key_points", "open_questions", "kept", "dropped", "made_ms", "provider", "model"] {
            assert!(raw.get(key).is_some(), "summary.json has {key}");
        }
        assert_eq!(raw["key_points"][0], json!({ "text": "one", "turn": 1, "from": "answer", "quote": "It places the order" }));
        let second = Summary { title: "Buttons, again".into(), made_ms: T0 + 9_000, ..first.clone() };
        save(&told, &second).unwrap();
        assert_eq!(load(&told).as_ref(), Some(&second));
        let before: Summary = serde_json::from_slice(&std::fs::read(told.dir.join(format!("summary-{}.json", T0 + 5))).unwrap()).unwrap();
        assert_eq!(before, first, "the one before is kept under when it was made");
        assert!(!told.dir.join("summary.tmp").exists(), "nothing half written is left");
        // The session reads as it did: a summary is not a turn.
        assert_eq!(sessions.read(&told.id).unwrap().1.len(), 2);
    }

    #[test]
    fn the_markdown_gives_each_point_its_turn_anchor_and_its_quote() {
        let p = |text: &str, turn, from, quote: &str| Point { text: text.into(), turn, from, quote: quote.into() };
        let s = Summary {
            title: "Bayes' rule".into(),
            key_points: vec![p("The denominator is the total evidence.", 1, From::Answer, "the **total evidence**:\nevery way")],
            open_questions: vec![p("Why a rare disease gives a small answer.", 2, From::Question, "why does a rare disease")],
            kept: 2,
            dropped: 1,
            ..Summary::default()
        };
        assert_eq!(
            to_markdown(&s, &session()).lines().collect::<Vec<_>>(),
            [
                "# Bayes' rule",
                "",
                "A summary of 2 turns. 2 kept, 1 dropped: no source.",
                "",
                "## Key points",
                "",
                "- The denominator is the total evidence. ([T1](#t1), the AI's explanation)",
                "  > the **total evidence**: every way",
                "",
                "## Open questions",
                "",
                "- Why a rare disease gives a small answer. ([T2](#t2), your question)",
                "  > why does a rare disease",
            ]
        );
        // Nothing kept: the title and the count, and no empty heading.
        let none = Summary { dropped: 0, key_points: vec![], open_questions: vec![], kept: 0, title: String::new(), ..s };
        assert_eq!(to_markdown(&none, &session()), "# what does the denominator in this\n\nA summary of 2 turns. 0 kept.\n");
    }
}
