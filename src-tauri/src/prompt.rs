//! Assemble a Pointcast session — transcript plus captures — into pasteable prompts.
//!
//! Markers land at the moment a capture was taken: inside the words being spoken then,
//! or between speech segments. Captures are numbered in time order as `[image n]`.
//! The control the person pointed at and labels from pins ride along on the marker.
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub id: u32,
    pub kind: CaptureKind,
    pub t_ms: u64,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub labels: Vec<Label>,
    /// Where the person pointed, in picture pixels: their last rest, else the pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<crate::pointer::Point>,
    /// The pointer's path over the last few seconds, in picture pixels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trail: Vec<crate::pointer::Point>,
    /// The control the person pointed at, its rect in picture pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<crate::element::ElementMark>,
    /// How screen positions land in the picture; none for a region, which the overlay cropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<crate::capture::Place>,
    /// Rings added at stop where the person said "this" or "here", in picture pixels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<crate::pointer::Point>,
    /// A key frame Pointcast picked from the recording, not a picture the person took.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auto: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A session item: speech from `t_ms` to `end_ms`, or a picture taken at `t_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

/// A control's name in a marker is cut to this many characters.
pub const NAME_MAX: usize = 60;

/// `[image n]`, naming the control the person pointed at and then the pins: `[image 2: button "Place order"; 1=too far
/// left]`.
fn marker(n: u32, c: &Capture) -> String {
    let mut bits: Vec<String> = c.element.iter().map(named).filter(|b| !b.is_empty()).collect();
    bits.extend(pins(c));
    if bits.is_empty() {
        return format!("[image {n}]");
    }
    format!("[image {n}: {}]", bits.join("; "))
}

/// A label numbered 0 is a note on the whole picture, not a pin: how many areas the person blacked out.
const HIDDEN: &str = " hidden";

/// How many areas of the picture were blacked out, from its note.
pub fn hidden_count(labels: &[Label]) -> u32 {
    labels
        .iter()
        .filter(|l| l.n == 0)
        .filter_map(|l| l.text.strip_suffix(HIDDEN)?.split_whitespace().next()?.parse::<u32>().ok())
        .sum()
}

/// Add `more` blacked-out areas to the picture's note ("1 area hidden", "3 areas hidden"), one note at most.
pub fn note_hidden(labels: &mut Vec<Label>, more: u32) {
    let n = hidden_count(labels) + more;
    labels.retain(|l| !(l.n == 0 && l.text.ends_with(HIDDEN)));
    if n > 0 {
        labels.push(Label {
            n: 0,
            text: format!("{n} {}{HIDDEN}", if n == 1 { "area" } else { "areas" }),
        });
    }
}

/// A capture's pins as `1=too far left`, or the bare number for a pin without words; a note (0) as its words.
fn pins(c: &Capture) -> Vec<String> {
    c.labels
        .iter()
        .map(|l| {
            let t = l.text.trim();
            if l.n == 0 {
                t.to_string()
            } else if t.is_empty() {
                format!("{}", l.n)
            } else {
                format!("{}={}", l.n, t)
            }
        })
        .collect()
}

/// `button "Place order"`: the role, then the name in quotes on one line, its own double quotes made single and cut to
/// `NAME_MAX` characters.
fn named(e: &crate::element::ElementMark) -> String {
    let name = e.name.split_whitespace().collect::<Vec<_>>().join(" ").replace('"', "'");
    let name = if name.chars().count() > NAME_MAX {
        format!("{}…", name.chars().take(NAME_MAX - 1).collect::<String>().trim_end())
    } else {
        name
    };
    let role = e.role.trim();
    match (role.is_empty(), name.is_empty()) {
        (_, true) => role.to_string(),
        (true, false) => format!(r#""{name}""#),
        (false, false) => format!(r#"{role} "{name}""#),
    }
}

/// The transcript with each picture as `[image n]` at the second it was taken: inside the
/// piece being spoken then, at the word that far through it, or between pieces. Images are
/// numbered in time order. Returns the text and the pictures in that order (ids rewritten 1..n).
pub fn compose(items: &[Item]) -> (String, Vec<Capture>) {
    compose_marked(items, false, false)
}

/// For each picture in the order the prompt numbers them (`[image 1]` first), its place among the pictures in `items`.
/// Pictures are numbered by the time they were taken; at one time, in the order `items` holds them.
pub fn picture_order(items: &[Item]) -> Vec<usize> {
    let mut v: Vec<(u64, usize)> = items
        .iter()
        .filter_map(|i| match i {
            Item::Image { t_ms, .. } => Some(*t_ms),
            Item::Speech { .. } => None,
        })
        .enumerate()
        .map(|(k, t)| (t, k))
        .collect();
    v.sort_by_key(|x| x.0);
    v.into_iter().map(|x| x.1).collect()
}

/// `compose`, formatted by `tidy_pieces` when `tidy` is on (paragraphs at the pauses, no ums, no repeated words); off,
/// the pieces joined by single spaces exactly as `compose` joins them.
pub fn compose_with(items: &[Item], tidy: bool) -> (String, Vec<Capture>) {
    compose_marked(items, false, tidy)
}

/// One token of a composed prompt: a spoken word, or a picture marker that formatting never moves, changes or drops.
#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Word(String),
    Mark(String),
}

impl Tok {
    fn text(&self) -> &str {
        match self {
            Tok::Word(s) | Tok::Mark(s) => s,
        }
    }
    fn word(&self) -> Option<&str> {
        match self {
            Tok::Word(s) => Some(s),
            Tok::Mark(_) => None,
        }
    }
}

/// A speech piece from `t_ms` to `end_ms` with its words and the markers placed inside it.
#[derive(Debug, Clone)]
struct Piece {
    t_ms: u64,
    end_ms: u64,
    toks: Vec<Tok>,
}

/// `compose`; with `short`, each marker is the bare `[image n]` (the picture list carries the rest); with `tidy`,
/// formatted by `tidy_pieces`.
fn compose_marked(items: &[Item], short: bool, tidy: bool) -> (String, Vec<Capture>) {
    let (pieces, captures) = pieces_of(items, short);
    let text = if tidy {
        tidy_pieces(&pieces).0
    } else {
        pieces.iter().flat_map(|p| p.toks.iter().map(Tok::text)).collect::<Vec<_>>().join(" ")
    };
    (text, captures)
}

/// The speech pieces in time order with each picture's marker at the second it was taken, and the pictures in that
/// order (ids rewritten 1..n). Pictures without any speech sit in one empty piece.
fn pieces_of(items: &[Item], short: bool) -> (Vec<Piece>, Vec<Capture>) {
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

    let mut pieces: Vec<Piece> = vec![];
    let mut n = 0u32;
    let mut next = 0usize;
    let put = |k: usize, w: usize, n: &mut u32, next: &mut usize, out: &mut Vec<Tok>| {
        while *next < at.len() && at[*next] == (k, w) {
            *n += 1;
            out.push(Tok::Mark(if short { format!("[image {}]", *n) } else { marker(*n, &shots[*next].1) }));
            *next += 1;
        }
    };
    if said.is_empty() {
        let mut toks = vec![];
        put(0, 0, &mut n, &mut next, &mut toks);
        pieces.push(Piece { t_ms: 0, end_ms: 0, toks });
    }
    for (k, (a, b, words)) in said.iter().enumerate() {
        let mut toks = vec![];
        for (w, word) in words.iter().enumerate() {
            put(k, w, &mut n, &mut next, &mut toks);
            toks.push(Tok::Word(word.clone()));
        }
        put(k, words.len(), &mut n, &mut next, &mut toks);
        pieces.push(Piece { t_ms: *a, end_ms: *b, toks });
    }

    let captures: Vec<Capture> = shots
        .into_iter()
        .enumerate()
        .map(|(i, (_, mut c))| {
            c.id = (i as u32) + 1;
            c
        })
        .collect();
    (pieces, captures)
}

/// A pause at least this long after a sentence end, or before a capital, starts a paragraph.
const PAUSE_MS: u64 = 1_200;
/// A pause at least this long starts a paragraph whatever the punctuation.
const LONG_PAUSE_MS: u64 = 2_500;
/// A paragraph of fewer words joins the one before it, unless a long pause came first.
const FRAGMENT_WORDS: usize = 8;
/// A paragraph past this many words is split at its next sentence end.
const PARAGRAPH_WORDS: usize = 110;
/// Sounds that are never words. "you know", "like", "I mean" and "kind of" are words and stay.
const FILLERS: &[&str] = &["um", "umm", "uh", "uhh", "uhm", "erm", "hmm"];
/// Doubled on purpose ("what it had had", "I said that that was"): never collapsed.
const KEEP_DOUBLE: &[&str] = &["had", "that"];
const NUMBER_WORDS: &[&str] = &[
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve",
    "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen", "twenty", "thirty", "forty",
    "fifty", "sixty", "seventy", "eighty", "ninety", "hundred", "thousand", "million", "billion",
];
const ORDINALS: &[&str] = &["first", "firstly", "second", "secondly", "third", "thirdly", "fourth", "fourthly", "fifth", "fifthly"];
const SPOKEN_ORDINALS: &[&str] = &["one", "two", "three", "four", "five", "1", "2", "3", "4", "5"];

/// What `tidy_pieces` did, for the report on real transcripts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct TidyStats {
    paragraphs: usize,
    repeats: usize,
    fillers: usize,
}

/// A word lower-cased, with only its letters, digits and apostrophes.
fn norm(w: &str) -> String {
    w.chars()
        .map(|c| if c == '\u{2019}' { '\'' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_filler(w: &str) -> bool {
    FILLERS.contains(&norm(w).as_str())
}

fn is_number(n: &str) -> bool {
    n.chars().any(|c| c.is_ascii_digit()) || NUMBER_WORDS.contains(&n)
}

fn ends_sentence(w: &str) -> bool {
    w.trim_end_matches(['"', '\'', '\u{201d}', '\u{2019}', ')', ']']).ends_with(['.', '?', '!', '\u{2026}'])
}

fn starts_capital(w: &str) -> bool {
    w.chars().find(|c| c.is_alphanumeric()).is_some_and(char::is_uppercase)
}

/// The word with its first letter capitalised.
fn capitalize(w: &str) -> String {
    match w.char_indices().find(|(_, c)| c.is_alphanumeric()) {
        Some((i, c)) => format!("{}{}{}", &w[..i], c.to_uppercase(), &w[i + c.len_utf8()..]),
        None => w.to_string(),
    }
}

fn count_words(toks: &[Tok]) -> usize {
    toks.iter().filter(|t| t.word().is_some()).count()
}

fn capitalize_first(toks: &mut [Tok]) {
    if let Some(Tok::Word(w)) = toks.iter_mut().find(|t| t.word().is_some()) {
        *w = capitalize(w);
    }
}

/// The pieces as paragraphs: a new one at a pause of `PAUSE_MS` after a sentence end or before a capital, or at any
/// pause of `LONG_PAUSE_MS`; a fragment under `FRAGMENT_WORDS` words joins the paragraph before it; a long one splits
/// at a sentence end past `PARAGRAPH_WORDS`. A piece that is only a filler goes, as does a filler that opens a
/// sentence; an immediate repeat of one to three words is said once; each paragraph starts with a capital, and a run of
/// First / Second / Third sentences becomes a `- ` list. Markers stay where they are and never stand alone. Nothing
/// else is removed, reworded or summarized.
fn tidy_pieces(pieces: &[Piece]) -> (String, TidyStats) {
    let mut stats = TidyStats::default();
    // Paragraphs by pauses, each with the pause before it.
    let mut paras: Vec<(u64, Vec<Tok>)> = vec![];
    let mut last_end: Option<u64> = None;
    for p in pieces {
        let words: Vec<&str> = p.toks.iter().filter_map(Tok::word).collect();
        let toks: Vec<Tok> = if !words.is_empty() && words.iter().all(|w| is_filler(w)) {
            stats.fillers += words.len();
            p.toks.iter().filter(|t| t.word().is_none()).cloned().collect()
        } else {
            p.toks.clone()
        };
        if toks.is_empty() {
            continue;
        }
        let gap = last_end.map_or(0, |e| p.t_ms.saturating_sub(e));
        last_end = Some(p.end_ms);
        let new = match paras.last() {
            None => true,
            Some((_, cur)) => {
                let ends = cur.iter().rev().find_map(Tok::word).is_some_and(ends_sentence);
                let cap = toks.iter().find_map(Tok::word).is_some_and(starts_capital);
                count_words(&toks) > 0 && (gap >= LONG_PAUSE_MS || (gap >= PAUSE_MS && (ends || cap)))
            }
        };
        match paras.last_mut() {
            Some((_, cur)) if !new => cur.extend(toks),
            _ => paras.push((gap, toks)),
        }
    }
    // A fragment joins the paragraph before it; markers never stand alone.
    let mut merged: Vec<Vec<Tok>> = vec![];
    for (gap, toks) in paras {
        let n = count_words(&toks);
        match merged.last_mut() {
            Some(prev) if n == 0 || (n < FRAGMENT_WORDS && gap < LONG_PAUSE_MS) || count_words(prev) == 0 => {
                prev.extend(toks)
            }
            _ => merged.push(toks),
        }
    }
    // Long paragraphs split at a sentence end; a list paused between its items stays one list.
    let mut chunks: Vec<Vec<Tok>> = vec![];
    for toks in merged {
        for c in split_long(toks) {
            let c = collapse_repeats(drop_lead_fillers(c, &mut stats), &mut stats);
            if c.is_empty() {
                continue;
            }
            match chunks.last_mut() {
                Some(prev) if continues_list(prev, &c) => prev.extend(c),
                _ => chunks.push(c),
            }
        }
    }
    stats.paragraphs = chunks.len();
    let out: Vec<String> = chunks.into_iter().map(render).collect();
    (out.join("\n\n"), stats)
}

/// Cut after the first sentence end once `PARAGRAPH_WORDS` words have gone by, keeping the markers right after it with
/// it, and never leaving a tail under `FRAGMENT_WORDS` words.
fn split_long(toks: Vec<Tok>) -> Vec<Vec<Tok>> {
    let total = count_words(&toks);
    let mut out = vec![];
    let mut cur: Vec<Tok> = vec![];
    let (mut n, mut seen) = (0, 0);
    let mut it = toks.into_iter().peekable();
    while let Some(t) = it.next() {
        let end = t.word().is_some_and(ends_sentence);
        if t.word().is_some() {
            n += 1;
            seen += 1;
        }
        cur.push(t);
        if end && n >= PARAGRAPH_WORDS && total - seen >= FRAGMENT_WORDS {
            while let Some(m) = it.next_if(|t| t.word().is_none()) {
                cur.push(m);
            }
            out.push(std::mem::take(&mut cur));
            n = 0;
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Remove a filler that opens a sentence ("Um, so the header" is "So the header"); one inside a sentence stays.
fn drop_lead_fillers(toks: Vec<Tok>, stats: &mut TidyStats) -> Vec<Tok> {
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut cap = false;
    for t in toks {
        match t {
            Tok::Word(w) => {
                let start = out.iter().rev().find_map(Tok::word).is_none_or(ends_sentence);
                if start && is_filler(&w) {
                    stats.fillers += 1;
                    cap = true;
                    continue;
                }
                out.push(Tok::Word(if std::mem::take(&mut cap) { capitalize(&w) } else { w }));
            }
            m => out.push(m),
        }
    }
    out
}

/// Words `i..i + n` said again straight after, within one sentence, with no marker between and no number among them.
fn repeat_at(toks: &[Tok], i: usize, n: usize) -> bool {
    let (a, b) = (&toks[i..i + n], &toks[i + n..i + 2 * n]);
    let words = |s: &[Tok]| s.iter().map(|t| t.word().map(norm)).collect::<Option<Vec<String>>>();
    let (Some(x), Some(y)) = (words(a), words(b)) else { return false };
    x == y
        && x.iter().all(|w| !w.is_empty() && !is_number(w))
        && !(n == 1 && KEEP_DOUBLE.contains(&x[0].as_str()))
        && !a.iter().filter_map(Tok::word).any(ends_sentence)
        && !b[..n - 1].iter().filter_map(Tok::word).any(ends_sentence)
}

/// Say an immediate repeat of one to three words once ("kind of kind of" is "kind of"), keeping the second copy, which
/// carries the punctuation, with the first one's capital.
fn collapse_repeats(mut toks: Vec<Tok>, stats: &mut TidyStats) -> Vec<Tok> {
    loop {
        let mut changed = false;
        for n in (1..=3).rev() {
            let mut i = 0;
            while i + 2 * n <= toks.len() {
                if repeat_at(&toks, i, n) {
                    let cap = toks[i].word().is_some_and(starts_capital);
                    toks.drain(i..i + n);
                    if cap {
                        capitalize_first(&mut toks[i..i + 1]);
                    }
                    stats.repeats += 1;
                    changed = true;
                } else {
                    i += 1;
                }
            }
        }
        if !changed {
            return toks;
        }
    }
}

/// The sentences of a paragraph: each ends after a word that ends one, with the markers right after it.
fn sentences(toks: &[Tok]) -> Vec<&[Tok]> {
    let mut out = vec![];
    let (mut s, mut i) = (0, 0);
    while i < toks.len() {
        let end = toks[i].word().is_some_and(ends_sentence);
        i += 1;
        if end {
            while i < toks.len() && toks[i].word().is_none() {
                i += 1;
            }
            out.push(&toks[s..i]);
            s = i;
        }
    }
    if s < toks.len() {
        out.push(&toks[s..]);
    }
    out
}

/// The sentence opens with First / Second / Third.. or "number one" / "number two"..; with `later`, Finally and Lastly
/// count too (they only continue a list).
fn ordinal(sentence: &[Tok], later: bool) -> bool {
    let mut w = sentence.iter().filter_map(Tok::word).map(norm);
    match w.next().as_deref() {
        Some("number") => w.next().is_some_and(|n| SPOKEN_ORDINALS.contains(&n.as_str())),
        Some(f) => ORDINALS.contains(&f) || (later && ["finally", "lastly"].contains(&f)),
        None => false,
    }
}

/// `next` opens with a later item of a list `prev` has begun ("Second, .." after "First, ..").
fn continues_list(prev: &[Tok], next: &[Tok]) -> bool {
    let opens = sentences(next).first().is_some_and(|s| ordinal(s, true));
    let first = next.iter().find_map(Tok::word).map(norm);
    opens && !matches!(first.as_deref(), Some("first" | "firstly")) && sentences(prev).iter().any(|s| ordinal(s, false))
}

/// One paragraph as text: its first letter capitalised, and a run of First / Second / Third sentences one `- ` line
/// each, in their own words.
fn render(mut toks: Vec<Tok>) -> String {
    capitalize_first(&mut toks);
    let join = |s: &[Tok]| s.iter().map(Tok::text).collect::<Vec<_>>().join(" ");
    let sents = sentences(&toks);
    let strong: Vec<usize> = (0..sents.len()).filter(|&i| ordinal(sents[i], false)).collect();
    if strong.len() < 2 {
        return join(&toks);
    }
    let mut lines: Vec<String> = vec![];
    if strong[0] > 0 {
        lines.push(join(&toks[..sents[..strong[0]].iter().map(|s| s.len()).sum()]));
    }
    for s in &sents[strong[0]..] {
        let mut s = s.to_vec();
        if ordinal(&s, true) {
            capitalize_first(&mut s);
            lines.push(format!("- {}", join(&s)));
        } else if let Some(l) = lines.last_mut() {
            l.push(' ');
            l.push_str(&join(&s));
        }
    }
    lines.join("\n")
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

/// Markdown with images inlined at the capture moment, pointing at `images/NN-kind.png`; with `tidy`, in paragraphs.
pub fn assemble_markdown(items: &[Item], tidy: bool) -> (String, Vec<Capture>) {
    markdown_linked(items, &|c| format!("images/{}", filename(c.id, c.kind)), tidy)
}

/// Markdown with each picture inlined at its marker, linking to `link(capture)`.
fn markdown_linked(items: &[Item], link: &dyn Fn(&Capture) -> String, tidy: bool) -> (String, Vec<Capture>) {
    let (plain, captures) = compose_with(items, tidy);
    if plain.is_empty() {
        return (String::new(), captures);
    }
    let mut md = plain;
    for c in &captures {
        let needle = marker(c.id, c);
        let img = format!("![{}]({})", escape_md_alt(&needle), link(c));
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
    assemble_clipboard_files(items, &[], false)
}

/// The clipboard form whose plain text lists each picture's file; the HTML shows the pictures. `files[j]` is picture
/// `j + 1` as the prompt numbers it. With files, the markers are the bare `[image n]` and one list closes the prompt;
/// without (the setting is off, or nothing was saved), the markers carry the control and pins as before. With `tidy`,
/// the text is in paragraphs (`tidy_pieces`), each a `<p>` in the HTML.
pub fn assemble_clipboard_files(items: &[Item], files: &[String], tidy: bool) -> ClipboardPayload {
    let (marked, captures) = compose_with(items, tidy);
    let plain = if files.is_empty() || captures.is_empty() {
        marked.clone()
    } else {
        format!("{}\n\n{}", compose_marked(items, true, tidy).0, picture_list(&captures, files))
    };
    if plain.is_empty() {
        return ClipboardPayload {
            html: String::new(),
            plain: String::new(),
        };
    }
    let mut html_body = if tidy {
        marked.split("

").map(|p| format!("<p style=\"margin:0 0 0.75em;\">{}</p>", escape_html(p))).collect()
    } else {
        escape_html(&marked)
    };
    for c in &captures {
        let needle = escape_html(&marker(c.id, c));
        let img = format!(
            "<img src=\"{}\" alt=\"{}\" width=\"{}\" height=\"{}\" style=\"max-width:100%;height:auto;vertical-align:middle;border:1px solid #d4d4d8;border-radius:6px;\" />",
            png_data_url(&c.png),
            escape_html(&escape_md_alt(&marker(c.id, c))),
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

/// The list that closes a prompt for agents: the folder, then each picture's file in backticks (a folder with spaces
/// still reads as one path) and what it is.
///
/// ```text
/// Pictures (2), in `C:/Users/max/Pointcast/2026-11-09_1432/`:
/// [image 1] `C:/Users/max/Pointcast/2026-11-09_1432/01.png`  region 933x73 · pins: 1=too far left
/// [image 2] `C:/Users/max/Pointcast/2026-11-09_1432/02.png`  full 1813x989
/// ```
fn picture_list(captures: &[Capture], files: &[String]) -> String {
    let files: Vec<String> = files.iter().map(|f| f.replace(std::path::MAIN_SEPARATOR, "/")).collect();
    let rows: Vec<String> =
        captures.iter().zip(&files).map(|(c, f)| format!("[image {}] `{f}`  {}", c.id, caption(c))).collect();
    let dir = files.first().and_then(|f| f.rsplit_once('/')).map_or("", |(d, _)| d);
    format!("Pictures ({}), in `{dir}/`:\n{}", rows.len(), rows.join("\n"))
}

/// `region 933x73 · pointing at button "Save" · pins: 1=too far left`: the kind and pixel size, then the control pointed
/// at and the pins when the picture has them.
fn caption(c: &Capture) -> String {
    let mut bits = vec![format!("{} {}x{}", c.kind.as_str(), c.width, c.height)];
    bits.extend(c.element.iter().map(named).filter(|n| !n.is_empty()).map(|n| format!("pointing at {n}")));
    let p = pins(c);
    if !p.is_empty() {
        bits.push(format!("pins: {}", p.join("; ")));
    }
    bits.join(" · ")
}

/// Handoff folders kept: the newest this many, and none older than `HANDOFF_KEEP_DAYS`.
pub const HANDOFF_KEEP: usize = 30;
pub const HANDOFF_KEEP_DAYS: u64 = 7;
/// The `app` a handoff manifest carries; pruning touches only folders whose manifest says it.
const HANDOFF_APP: &str = "pointcast";

/// `manifest.json` in a handoff folder.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub app: String,
    /// The History entry the folder was copied from.
    pub id: String,
    pub created_ms: u64,
    /// `created_ms` in local time, RFC 3339.
    pub created: String,
    pub pictures: Vec<ManifestPicture>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManifestPicture {
    /// As `[image n]` numbers it.
    pub n: u32,
    pub file: String,
    pub kind: CaptureKind,
    pub width: u32,
    pub height: u32,
    pub t_ms: u64,
    #[serde(default)]
    pub labels: Vec<Label>,
    /// The control the person pointed at, as the marker names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointing_at: Option<String>,
    /// Picked from the recording by Pointcast.
    #[serde(default)]
    pub auto: bool,
}

/// A written handoff folder and its pictures' paths in prompt order, with forward slashes.
#[derive(Debug, Clone, PartialEq)]
pub struct Handoff {
    pub dir: std::path::PathBuf,
    pub files: Vec<String>,
}

/// Where handoff folders go: the setting, else `~/Pointcast`. Tests never write to the real home folder.
pub fn handoff_root(s: &crate::settings::Settings) -> std::path::PathBuf {
    if let Some(d) = s.handoff_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        return d.into();
    }
    if cfg!(test) {
        return std::env::temp_dir().join(format!("pointcast-test-handoff-{}", std::process::id()));
    }
    dirs::home_dir().unwrap_or_else(|| std::path::PathBuf::from(".")).join("Pointcast")
}

/// A folder name `YYYY-MM-DD_HHMM`, with a `-n` suffix for another recording in the same minute.
fn is_handoff_name(name: &str) -> bool {
    let (Some(stamp), Some(rest)) = (name.get(..15), name.get(15..)) else { return false };
    let shape = stamp
        .char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            10 => c == '_',
            _ => c.is_ascii_digit(),
        });
    shape && (rest.is_empty() || rest.strip_prefix('-').is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())))
}

/// A folder only its owner can open (0700 on Unix). Fails when it already exists.
fn create_private(p: &Path) -> std::io::Result<()> {
    #[allow(unused_mut)] // set only on Unix
    let mut b = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(p)
}

/// Copy a saved recording's pictures for agents: `<root>/<YYYY-MM-DD_HHMM>/` with `01.png`.. numbered as the prompt
/// numbers them, `prompt.md` linking to them and `manifest.json`. Then prune the root (`prune_handoff`). A recording
/// without pictures writes nothing. On failure the folder is removed and History's own copies stand.
pub fn write_handoff(root: &Path, id: &str, created_ms: u64, items: &[Item], now_ms: u64, tidy: bool) -> Result<Handoff, String> {
    let digits = |n: usize| if n >= 100 { 3 } else { 2 };
    let (_, captures) = compose(items);
    if captures.is_empty() {
        return Err("no pictures".into());
    }
    let width = digits(captures.len());
    let name = |n: u32| format!("{n:0width$}.png");
    use chrono::TimeZone;
    let local = chrono::Local.timestamp_millis_opt(created_ms as i64).single().unwrap_or_else(chrono::Local::now);
    let stamp = local.format("%Y-%m-%d_%H%M").to_string();
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut dir = None;
    for k in 1..1000 {
        let p = root.join(if k == 1 { stamp.clone() } else { format!("{stamp}-{k}") });
        match create_private(&p) {
            Ok(()) => {
                dir = Some(p);
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    }
    let dir = dir.ok_or("no free handoff folder name")?;
    let write = || -> Result<(), String> {
        for c in &captures {
            std::fs::write(dir.join(name(c.id)), &c.png).map_err(|e| e.to_string())?;
        }
        let (md, _) = markdown_linked(items, &|c| name(c.id), tidy);
        std::fs::write(dir.join("prompt.md"), format!("{md}\n")).map_err(|e| e.to_string())?;
        let manifest = Manifest {
            app: HANDOFF_APP.into(),
            id: id.into(),
            created_ms,
            created: local.to_rfc3339_opts(chrono::SecondsFormat::Secs, false),
            pictures: captures
                .iter()
                .map(|c| ManifestPicture {
                    n: c.id,
                    file: name(c.id),
                    kind: c.kind,
                    width: c.width,
                    height: c.height,
                    t_ms: c.t_ms,
                    labels: c.labels.clone(),
                    pointing_at: c.element.as_ref().map(named).filter(|n| !n.is_empty()),
                    auto: c.auto,
                })
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("manifest.json"), json).map_err(|e| e.to_string())
    };
    if let Err(e) = write() {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    prune_handoff(root, now_ms, Some(&dir));
    let base = dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
    let files = captures.iter().map(|c| format!("{base}/{}", name(c.id))).collect();
    Ok(Handoff { dir, files })
}

/// Remove handoff folders beyond the newest `HANDOFF_KEEP` or older than `HANDOFF_KEEP_DAYS`. Only folders named like
/// one whose `manifest.json` Pointcast wrote: anything else in the root is never touched, nor `spare` (the folder just
/// written, whatever its time says).
pub fn prune_handoff(root: &Path, now_ms: u64, spare: Option<&Path>) {
    let mut ours: Vec<(u64, String, std::path::PathBuf)> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_handoff_name(&name) {
                return None;
            }
            let m: Manifest = serde_json::from_slice(&std::fs::read(e.path().join("manifest.json")).ok()?).ok()?;
            (m.app == HANDOFF_APP).then(|| (m.created_ms, name, e.path()))
        })
        .collect();
    ours.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
    let oldest = now_ms.saturating_sub(HANDOFF_KEEP_DAYS * 86_400_000);
    for (i, (created, _, p)) in ours.iter().enumerate() {
        if (i >= HANDOFF_KEEP || *created < oldest) && spare != Some(p.as_path()) {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}

/// The newest handoff folder written for History entry `id`, with its manifest.
pub fn find_handoff(root: &Path, id: &str) -> Option<(std::path::PathBuf, Manifest)> {
    std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| {
            let m: Manifest = serde_json::from_slice(&std::fs::read(e.path().join("manifest.json")).ok()?).ok()?;
            (m.id == id).then(|| (e.path(), m))
        })
        .max_by(|a, b| a.1.created_ms.cmp(&b.1.created_ms).then(a.0.cmp(&b.0)))
}

/// Picture `c`'s file in the handoff folder `dir` (with manifest `m`) while it still holds the same picture, with
/// forward slashes.
pub fn handoff_copy(dir: &Path, m: &Manifest, c: &Capture) -> Option<String> {
    let f = dir.join(&m.pictures.iter().find(|p| p.n == c.id)?.file);
    (std::fs::read(&f).ok()? == c.png).then(|| f.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/"))
}

/// The files of the handoff folder for entry `id` whose pictures are exactly `items`' pictures, in prompt order; none
/// when there is no such folder, or a picture was edited, added or taken out since it was written.
pub fn handoff_files(root: &Path, id: &str, items: &[Item]) -> Option<Vec<String>> {
    let (_, captures) = compose(items);
    let (dir, m) = find_handoff(root, id)?;
    if captures.is_empty() || m.pictures.len() != captures.len() {
        return None;
    }
    captures.iter().map(|c| handoff_copy(&dir, &m, c)).collect()
}

/// Remove every handoff folder Pointcast wrote for entry `id` (named like one, with a manifest that says so).
pub fn remove_handoffs(root: &Path, id: &str) {
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !is_handoff_name(&name) || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(m) = std::fs::read(e.path().join("manifest.json")).ok().and_then(|b| serde_json::from_slice::<Manifest>(&b).ok()) else {
            continue;
        };
        if m.app == HANDOFF_APP && m.id == id {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// Write `prompt.md` and `images/NN-kind.png` under `dir`. Returns the markdown path.
pub fn export_folder(items: &[Item], dir: &Path, tidy: bool) -> Result<std::path::PathBuf, String> {
    let (md, captures) = assemble_markdown(items, tidy);
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
    /// Picked from the recording by Pointcast.
    pub auto: bool,
}

pub fn assemble_export(items: &[Item]) -> SessionExport {
    assemble_export_files(items, &[], false)
}

/// The export whose plain text lists each picture's file (`assemble_clipboard_files`; `files` in prompt order).
pub fn assemble_export_files(items: &[Item], files: &[String], tidy: bool) -> SessionExport {
    let clip = assemble_clipboard_files(items, files, tidy);
    let (markdown, captures) = assemble_markdown(items, tidy);
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
                auto: c.auto,
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
                pointer: None,
                trail: vec![],
                element: None,
                place: None,
                anchors: vec![],
                auto: false,
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
    fn hidden_areas_are_one_note_on_the_marker_counted_in_plain_words() {
        let mut labels = vec![Label { n: 1, text: "Save".into() }];
        note_hidden(&mut labels, 0);
        assert_eq!(labels.len(), 1, "no hidden area, no note");
        note_hidden(&mut labels, 1);
        assert_eq!(hidden_count(&labels), 1);
        let (text, _) = compose(&[said(0, 1_000, "this button"), shot(400, CaptureKind::Region, labels.clone())]);
        assert_eq!(text, "this [image 1: 1=Save; 1 area hidden] button");
        // Marking the picture again from History adds to the one note.
        note_hidden(&mut labels, 2);
        assert_eq!(labels.iter().filter(|l| l.n == 0).count(), 1);
        assert_eq!(labels.last().unwrap().text, "3 areas hidden");
        assert_eq!(hidden_count(&labels), 3);
    }

    fn pointed(t: u64, role: &str, name: &str, labels: Vec<Label>) -> Item {
        let mut it = shot(t, CaptureKind::Full, labels);
        if let Item::Image { capture, .. } = &mut it {
            capture.element = Some(button(role, name));
        }
        it
    }

    fn button(role: &str, name: &str) -> crate::element::ElementMark {
        let rect = crate::element::Bounds { x: 10, y: 20, w: 116, h: 41 };
        crate::element::ElementMark { role: role.into(), name: name.into(), rect }
    }

    #[test]
    fn the_control_pointed_at_is_named_first_on_the_marker() {
        let pin = vec![Label { n: 1, text: "too far left".into() }];
        let (text, _) = compose(&[said(0, 1_000, "this one"), pointed(400, "button", "Place order", pin.clone())]);
        assert_eq!(text, r#"this [image 1: button "Place order"; 1=too far left] one"#);
        let one = |it: Item| compose(&[it]).0;
        assert_eq!(one(pointed(0, "button", "Place order", vec![])), r#"[image 1: button "Place order"]"#);
        assert_eq!(one(shot(0, CaptureKind::Full, pin.clone())), "[image 1: 1=too far left]");
        assert_eq!(one(shot(0, CaptureKind::Full, vec![])), "[image 1]");
        assert_eq!(one(pointed(0, "button", "", vec![])), "[image 1: button]", "an unnamed control by its role");
        assert_eq!(one(pointed(0, "", "Total", vec![])), r#"[image 1: "Total"]"#);
        assert_eq!(one(pointed(0, " ", " ", pin.clone())), "[image 1: 1=too far left]");
        // The name stays one quoted line: its quotes become single, its breaks and runs of spaces one space.
        assert_eq!(named(&button("link", r#"Say "hi"  to   Max "#)), r#"link "Say 'hi' to Max""#);
        let long = named(&button("text field", &"a".repeat(100)));
        assert_eq!(long, format!(r#"text field "{}…""#, "a".repeat(NAME_MAX - 1)));
        assert_eq!(named(&button("text", &"b".repeat(NAME_MAX))), format!(r#"text "{}""#, "b".repeat(NAME_MAX)));
    }

    #[test]
    fn markdown_and_html_carry_the_named_control() {
        let items = [said(0, 1_000, "this one"), pointed(400, "button", "Place order", vec![Label { n: 1, text: "too far left".into() }])];
        let (md, _) = assemble_markdown(&items, false);
        assert_eq!(md, r#"this ![image 1: button "Place order"; 1=too far left](images/01-full.png) one"#);
        let clip = assemble_clipboard(&items);
        assert_eq!(clip.plain, r#"this [image 1: button "Place order"; 1=too far left] one"#);
        assert!(clip.html.contains(r#"alt="image 1: button &quot;Place order&quot;; 1=too far left""#), "{}", clip.html);
        assert!(!clip.html.contains("[image 1"), "the marker became the picture: {}", clip.html);
        assert_eq!(assemble_export(&items).plain, clip.plain);
    }

    fn sized(t: u64, kind: CaptureKind, w: u32, h: u32, labels: Vec<Label>) -> Item {
        let mut it = shot(t, kind, labels);
        if let Item::Image { capture, .. } = &mut it {
            (capture.width, capture.height) = (w, h);
        }
        it
    }

    #[test]
    fn the_plain_prompt_closes_with_one_list_of_pictures_and_the_html_shows_them() {
        let pin = vec![Label { n: 1, text: "too far left".into() }];
        let items = [
            said(0, 1_000, "the save button sits too far left"),
            sized(900, CaptureKind::Full, 1813, 989, vec![]),
            sized(300, CaptureKind::Region, 933, 73, pin),
        ];
        // `files[j]` is picture j + 1 as the prompt numbers it.
        let dir = "C:/Users/max/Pointcast/2026-11-09_1432";
        let files = [format!("{dir}/01.png"), format!("{dir}/02.png")];
        let clip = assemble_clipboard_files(&items, &files, false);
        assert_eq!(
            clip.plain,
            concat!(
                "the save [image 1] button sits too far [image 2] left\n",
                "\n",
                "Pictures (2), in `C:/Users/max/Pointcast/2026-11-09_1432/`:\n",
                "[image 1] `C:/Users/max/Pointcast/2026-11-09_1432/01.png`  region 933x73 · pins: 1=too far left\n",
                "[image 2] `C:/Users/max/Pointcast/2026-11-09_1432/02.png`  full 1813x989",
            )
        );
        assert_eq!(clip.html, assemble_clipboard(&items).html, "the HTML keeps the pictures inline, without paths");
        assert!(!clip.html.contains("Pointcast/2026"));
        let exp = assemble_export_files(&items, &files, false);
        assert_eq!(exp.plain, clip.plain, "agents read the list too");
        assert_eq!(exp.markdown, assemble_markdown(&items, false).0);
        // The control pointed at rides in the caption, between the size and the pins.
        let one = [pointed(0, "button", "Place order", vec![])];
        assert!(assemble_clipboard_files(&one, &files[..1], false).plain.ends_with(r#"`  full 1x1 · pointing at button "Place order""#));
        // No files (the setting is off, or History could not keep the recording): no list and no paths.
        let off = assemble_export_files(&items, &[], false).plain;
        assert_eq!(off, "the save [image 1: 1=too far left] button sits too far [image 2] left");
        assert!(!off.contains("Pictures") && !off.contains('/'), "{off}");
        // No pictures: no list.
        assert_eq!(assemble_clipboard_files(&[said(0, 1_000, "just words")], &files, false).plain, "just words");
    }

    const CREATED: u64 = 1_793_000_000_000;
    const MINUTE: u64 = 60_000;
    const DAY: u64 = 86_400_000;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-handoff-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn png_wide(w: u32) -> Vec<u8> {
        crate::image_ops::encode_png(&vec![7u8; (w * 4) as usize], w, 1).unwrap()
    }

    fn manifest(dir: &Path) -> Manifest {
        serde_json::from_slice(&std::fs::read(dir.join("manifest.json")).unwrap()).unwrap()
    }

    #[test]
    fn picture_n_is_file_0n_and_the_earlier_picture_when_a_key_frame_was_stored_first() {
        let root = tmp("numbering");
        // Key frames are added after the stop, so one taken later can sit before the person's picture in storage.
        let mut key = shot(800, CaptureKind::Full, vec![]);
        let mut mine = shot(200, CaptureKind::Region, vec![]);
        for (it, w, auto) in [(&mut key, 3, true), (&mut mine, 2, false)] {
            if let Item::Image { capture, .. } = it {
                (capture.png, capture.width, capture.auto) = (png_wide(w), w, auto);
            }
        }
        let items = vec![said(0, 1_000, "look here"), key, mine];
        assert_eq!(picture_order(&items), [1, 0]);
        let h = write_handoff(&root, "e1", CREATED, &items, CREATED, false).unwrap();
        assert!(h.files[0].ends_with("/01.png") && h.files[1].ends_with("/02.png"), "{:?}", h.files);
        let first = std::fs::read(h.dir.join("01.png")).unwrap();
        assert_eq!(crate::image_ops::png_size(&first), Some((2, 1)), "[image 1] is the picture taken at 0.2 s");
        assert_eq!(std::fs::read(h.dir.join("02.png")).unwrap(), png_wide(3));
        let m = manifest(&h.dir);
        let got: Vec<_> = m.pictures.iter().map(|p| (p.n, p.file.as_str(), p.t_ms, p.auto)).collect();
        assert_eq!(got, [(1, "01.png", 200, false), (2, "02.png", 800, true)]);
        let plain = assemble_clipboard_files(&items, &h.files, false).plain;
        assert!(plain.starts_with("[image 1] look here [image 2]\n\n"), "{plain}");
        assert!(plain.contains(&format!("[image 1] `{}`  region 2x1\n", h.files[0])), "{plain}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_handoff_folder_holds_numbered_pictures_a_linked_prompt_and_a_manifest() {
        let root = tmp("folder");
        let pin = vec![Label { n: 1, text: "too far left".into() }];
        let items = [
            said(0, 1_000, "the save button sits too far left"),
            sized(900, CaptureKind::Full, 1813, 989, vec![]),
            sized(300, CaptureKind::Region, 933, 73, pin),
        ];
        let h = write_handoff(&root, "entry-7", CREATED, &items, CREATED, false).unwrap();
        use chrono::TimeZone;
        let stamp = chrono::Local.timestamp_millis_opt(CREATED as i64).unwrap().format("%Y-%m-%d_%H%M").to_string();
        assert_eq!(h.dir, root.join(&stamp));
        let mut names: Vec<String> =
            std::fs::read_dir(&h.dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["01.png", "02.png", "manifest.json", "prompt.md"]);
        // Shown with --nocapture: what is pasted and what the folder holds.
        println!("{}\n---\n{}: {names:?}", assemble_clipboard_files(&items, &h.files, false).plain, h.dir.display());
        println!("{}", std::fs::read_to_string(h.dir.join("manifest.json")).unwrap());
        let base = h.dir.to_string_lossy().replace(std::path::MAIN_SEPARATOR, "/");
        assert_eq!(h.files, [format!("{base}/01.png"), format!("{base}/02.png")]);
        assert_eq!(
            std::fs::read_to_string(h.dir.join("prompt.md")).unwrap(),
            "the save ![image 1: 1=too far left](01.png) button sits too far ![image 2](02.png) left\n"
        );
        let m: serde_json::Value = serde_json::from_slice(&std::fs::read(h.dir.join("manifest.json")).unwrap()).unwrap();
        assert_eq!((m["app"].as_str(), m["id"].as_str(), m["created_ms"].as_u64()), (Some("pointcast"), Some("entry-7"), Some(CREATED)));
        assert!(m["created"].as_str().is_some_and(|c| c.starts_with(&stamp[..10])), "{m}");
        assert_eq!(
            m["pictures"],
            serde_json::json!([
                {"n": 1, "file": "01.png", "kind": "region", "width": 933, "height": 73, "t_ms": 300,
                 "labels": [{"n": 1, "text": "too far left"}], "auto": false},
                {"n": 2, "file": "02.png", "kind": "full", "width": 1813, "height": 989, "t_ms": 900,
                 "labels": [], "auto": false},
            ])
        );
        // Another recording in the same minute gets its own folder.
        let again = write_handoff(&root, "entry-8", CREATED + 1_000, &items, CREATED, false).unwrap();
        assert_eq!(again.dir, root.join(format!("{stamp}-2")));
        assert_eq!(write_handoff(&root, "entry-9", CREATED + 2_000, &items, CREATED, false).unwrap().dir, root.join(format!("{stamp}-3")));
        // No pictures: no folder.
        assert!(write_handoff(&root, "words", CREATED, &[said(0, 9, "hi")], CREATED, false).is_err());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 3);
        // A hundred pictures or more take three digits.
        let many: Vec<Item> = (0..100).map(|t| shot(t, CaptureKind::Full, vec![])).collect();
        let h = write_handoff(&root, "many", CREATED + DAY, &many, CREATED + DAY, false).unwrap();
        assert!(h.files[0].ends_with("/001.png") && h.files[99].ends_with("/100.png"), "{:?}", &h.files[..2]);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn pruning_keeps_the_newest_thirty_none_older_than_a_week_and_never_a_folder_it_did_not_write() {
        let root = tmp("prune");
        let items = [shot(0, CaptureKind::Full, vec![])];
        let now = CREATED + 10 * DAY;
        // Eight days old, written when it was new.
        let old = write_handoff(&root, "old", now - 8 * DAY, &items, now - 8 * DAY, false).unwrap().dir;
        // Things Pointcast did not write: a folder named like one, one with a manifest of someone else's, a file.
        let foreign = root.join("2020-01-01_0000");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("notes.txt"), "mine").unwrap();
        let other = root.join("2020-01-01_0001");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("manifest.json"), r#"{"app":"else","id":"x","created_ms":0,"created":"","pictures":[]}"#).unwrap();
        std::fs::write(root.join("keep.txt"), "mine").unwrap();
        let mut ours = vec![];
        for i in 0..32 {
            ours.push(write_handoff(&root, &format!("e{i}"), now - (32 - i) * MINUTE, &items, now, false).unwrap().dir);
            if i == 0 {
                assert!(!old.exists(), "a folder older than a week goes at the next write");
            }
        }
        assert!(!ours[0].exists() && !ours[1].exists(), "beyond the newest thirty");
        assert!(ours[2..].iter().all(|d| d.join("manifest.json").exists()), "the newest thirty stay");
        assert!(foreign.join("notes.txt").exists() && other.join("manifest.json").exists() && root.join("keep.txt").exists());
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 30 + 3);
        assert!(is_handoff_name("2026-11-09_1432") && is_handoff_name("2026-11-09_1432-12"));
        assert!(!is_handoff_name("2026-11-09_1432-") && !is_handoff_name("2026-11-09-1432") && !is_handoff_name("notes"));
        assert!(!is_handoff_name("2026-11-09_14é2"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn markdown_inlines_relative_pngs_at_the_marker() {
        let (md, caps) = assemble_markdown(&[
            said(0, 1_000, "look here"),
            shot(500, CaptureKind::Region, vec![]),
        ], false);
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
        let prompt = export_folder(&items, &dir, false).unwrap();
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

    // ---- Tidy the text: paragraphs at the pauses, no ums, no repeated words.

    /// The tidied text of `items`.
    fn tidy(items: &[Item]) -> String {
        compose_with(items, true).0
    }

    /// Pieces said back to back, `gap_ms` apart, each lasting 300 ms a word.
    fn spoken(parts: &[(u64, &str)]) -> Vec<Item> {
        let mut t = 0;
        let mut v = vec![];
        for (gap, text) in parts {
            t += gap;
            let end = t + 300 * text.split_whitespace().count().max(1) as u64;
            v.push(said(t, end, text));
            t = end;
        }
        v
    }

    const LONG_ENOUGH: &str = "the header is far too tall on this page";

    #[test]
    fn tidy_starts_a_paragraph_at_a_pause_after_a_sentence_end_or_before_a_capital_or_at_any_long_pause() {
        let a = "the header is far too tall on this page.";
        let b = "the footer has the same problem on every page.";
        // 1.2 s after a sentence end: a new paragraph, its first letter capitalised.
        assert_eq!(
            tidy(&spoken(&[(0, a), (1_200, b)])),
            "The header is far too tall on this page.\n\nThe footer has the same problem on every page."
        );
        // A shorter pause, or no sentence end and no capital: the same paragraph.
        assert_eq!(tidy(&spoken(&[(0, a), (1_100, b)])), format!("The header is far too tall on this page. {b}"));
        assert_eq!(tidy(&spoken(&[(0, LONG_ENOUGH), (2_000, b)])), format!("The header is far too tall on this page {b}"));
        // No sentence end but a capital after 1.2 s: a new paragraph.
        assert_eq!(
            tidy(&spoken(&[(0, LONG_ENOUGH), (1_300, "The footer has the same problem on every page.")])),
            "The header is far too tall on this page\n\nThe footer has the same problem on every page."
        );
        // 2.5 s, whatever the punctuation.
        assert_eq!(
            tidy(&spoken(&[(0, LONG_ENOUGH), (2_500, b)])),
            "The header is far too tall on this page\n\nThe footer has the same problem on every page."
        );
    }

    #[test]
    fn tidy_joins_a_short_fragment_to_the_paragraph_before_it_unless_a_long_pause_came_first() {
        let a = "the header is far too tall on this page.";
        assert_eq!(
            tidy(&spoken(&[(0, a), (1_500, "Okay, that one too.")])),
            "The header is far too tall on this page. Okay, that one too."
        );
        assert_eq!(
            tidy(&spoken(&[(0, a), (2_600, "Okay, that one too.")])),
            "The header is far too tall on this page.\n\nOkay, that one too."
        );
    }

    #[test]
    fn tidy_splits_a_paragraph_past_110_words_at_its_next_sentence_end() {
        let sentence = |k: usize| format!("sentence {} has exactly ten words in it right here.", NUMBER_WORDS[k % 20]);
        let text: Vec<String> = (0..13).map(sentence).collect();
        let got = tidy(&[said(0, 60_000, &text.join(" "))]);
        let paras: Vec<&str> = got.split("\n\n").collect();
        assert_eq!(paras.len(), 2, "{got}");
        assert_eq!(paras[0].split_whitespace().count(), 110, "cut at the sentence end that reached 110 words");
        assert_eq!(paras[1].split_whitespace().count(), 20);
        assert!(paras[0].ends_with("right here.") && paras[1].starts_with("Sentence"), "{got}");
        // 115 words: the tail would be a 5-word fragment, so no cut.
        let short = format!("{} five more words right here.", text[..11].join(" "));
        assert!(!tidy(&[said(0, 60_000, &short)]).contains("\n\n"));
    }

    #[test]
    fn tidy_says_an_immediate_repeat_of_one_to_three_words_once_but_never_numbers_or_across_a_sentence_end() {
        let one = |s: &str| tidy(&[said(0, 9_000, s)]);
        assert_eq!(one("it is kind of kind of slow"), "It is kind of slow");
        assert_eq!(one("click the the button"), "Click the button");
        assert_eq!(one("I I think the save button is gone"), "I think the save button is gone");
        assert_eq!(one("move it to the left to the left please"), "Move it to the left please");
        assert_eq!(one("the the header"), "The header");
        assert_eq!(one("The the header"), "The header");
        // Not numbers, not across a sentence end, not the doubles grammar wants.
        assert_eq!(one("press two two times"), "Press two two times");
        assert_eq!(one("it costs 5 5 dollars"), "It costs 5 5 dollars");
        assert_eq!(one("done. done. now the menu"), "Done. done. now the menu");
        assert_eq!(one("what it had had in it"), "What it had had in it");
        // A picture between the two keeps both: markers are never moved.
        let items = [said(0, 1_000, "the the"), shot(500, CaptureKind::Full, vec![])];
        assert_eq!(tidy(&items), "The [image 1] the");
    }

    #[test]
    fn tidy_drops_a_piece_that_is_only_a_filler_and_a_filler_that_opens_a_sentence_and_nothing_else() {
        let a = "the header is far too tall on this page.";
        for um in ["Um.", "Um...", "Uh.", "um", "Uh, um."] {
            assert_eq!(
                tidy(&spoken(&[(0, a), (300, um), (300, "and the footer is too.")])),
                "The header is far too tall on this page. and the footer is too.",
                "{um}"
            );
        }
        assert_eq!(tidy(&[said(0, 9_000, "Um, so the header is too tall")]), "So the header is too tall");
        assert_eq!(tidy(&[said(0, 9_000, "it is gone. uh, so the header")]), "It is gone. So the header");
        // Inside a sentence a filler stays, and the phrases that are words stay too.
        assert_eq!(tidy(&[said(0, 9_000, "the, um, header is tall")]), "The, um, header is tall");
        let words = "you know, it is like, I mean, kind of slow and basically broken";
        assert_eq!(tidy(&[said(0, 9_000, words)]), "You know, it is like, I mean, kind of slow and basically broken");
        // A filler-only piece with a picture in it keeps the picture.
        let items = [said(0, 3_000, a), said(3_100, 3_600, "Um."), shot(3_300, CaptureKind::Full, vec![])];
        assert_eq!(tidy(&items), "The header is far too tall on this page. [image 1]");
    }

    #[test]
    fn tidy_turns_a_run_of_first_second_third_sentences_into_a_list_in_their_own_words() {
        let got = tidy(&[said(0, 20_000, "we need three things. first, the header. Second, the footer is off. it overlaps. Third, the sidebar.")]);
        assert_eq!(got, "We need three things.\n- First, the header.\n- Second, the footer is off. it overlaps.\n- Third, the sidebar.");
        let spoken_numbers = tidy(&[said(0, 20_000, "Number one, the save button. Number two, the menu.")]);
        assert_eq!(spoken_numbers, "- Number one, the save button.\n- Number two, the menu.");
        // Items with pauses between them stay one list.
        let paused = tidy(&spoken(&[
            (0, "First, the header is far too tall on the page."),
            (1_500, "Second, the footer is off by a few pixels."),
            (1_500, "Finally, the menu opens on the wrong side."),
        ]));
        assert_eq!(
            paused,
            "- First, the header is far too tall on the page.\n- Second, the footer is off by a few pixels.\n- Finally, the menu opens on the wrong side."
        );
        // One "first" is not a list.
        assert_eq!(tidy(&[said(0, 9_000, "First I opened the menu. Then it closed.")]), "First I opened the menu. Then it closed.");
    }

    #[test]
    fn tidy_never_moves_a_marker_and_never_lets_one_stand_as_a_paragraph() {
        // A picture taken in the pause stays after the words before it, not alone between paragraphs.
        let items = [
            said(0, 3_000, "look at this part of the screen right here."),
            shot(4_000, CaptureKind::Full, vec![]),
            said(8_000, 11_000, "the next thing is the footer on the page."),
        ];
        assert_eq!(tidy(&items), "Look at this part of the screen right here. [image 1]\n\nThe next thing is the footer on the page.");
        // A picture as the piece starts leads its paragraph, attached to the words; the capital skips it.
        let items = [said(0, 3_000, "first piece of words goes right here now."), said(6_000, 9_000, "next piece of words goes here."), shot(6_000, CaptureKind::Zoom, vec![])];
        assert_eq!(tidy(&items), "First piece of words goes right here now.\n\n[image 1] Next piece of words goes here.");
        // Pictures only, or a picture before a long pause: one paragraph.
        assert_eq!(tidy(&[shot(10, CaptureKind::Full, vec![]), shot(20, CaptureKind::Zoom, vec![])]), "[image 1] [image 2]");
        let items = [shot(0, CaptureKind::Full, vec![]), said(5_000, 6_000, "hello there")];
        assert_eq!(tidy(&items), "[image 1] Hello there");
        // Named markers are copied as they are, mid-sentence.
        let items = [said(0, 1_000, "this one"), pointed(400, "button", "Place order", vec![])];
        assert_eq!(tidy(&items), r#"This [image 1: button "Place order"] one"#);
    }

    #[test]
    fn tidy_reaches_every_output_markdown_prompt_md_html_and_the_listed_plain_text() {
        let items = [
            said(0, 3_000, "the save button is far too tall on this page."),
            sized(1_500, CaptureKind::Region, 933, 73, vec![]),
            said(4_500, 7_500, "um, the footer has the same problem everywhere."),
        ];
        let want = "The save button is far [image 1] too tall on this page.\n\nThe footer has the same problem everywhere.";
        assert_eq!(tidy(&items), want);
        let (md, _) = assemble_markdown(&items, true);
        assert_eq!(md, want.replace("[image 1]", "![image 1](images/01-region.png)"));
        let clip = assemble_clipboard_files(&items, &[], true);
        assert_eq!(clip.plain, want);
        assert_eq!(clip.html.matches("<p ").count(), 2, "{}", clip.html);
        assert!(clip.html.contains("<p style=\"margin:0 0 0.75em;\">The save button") && clip.html.contains("<img"), "{}", clip.html);
        let files = ["C:/p/01.png".to_string()];
        let listed = assemble_clipboard_files(&items, &files, true).plain;
        assert!(listed.starts_with(&format!("{want}\n\nPictures (1)")), "{listed}");
        let exp = assemble_export_files(&items, &files, true);
        assert_eq!((exp.plain.as_str(), exp.markdown.as_str()), (listed.as_str(), md.as_str()));
        // Off: today's single-space joining, everywhere.
        let raw = "the save button is far [image 1] too tall on this page. um, the footer has the same problem everywhere.";
        assert_eq!(compose_with(&items, false).0, raw);
        assert_eq!(assemble_clipboard_files(&items, &[], false).plain, raw);
        assert!(!assemble_clipboard_files(&items, &[], false).html.contains("<p"));
        // The handoff prompt.md reads in paragraphs too.
        let root = tmp("tidy");
        let h = write_handoff(&root, "t", CREATED, &items, CREATED, true).unwrap();
        assert_eq!(std::fs::read_to_string(h.dir.join("prompt.md")).unwrap(), format!("{}\n", want.replace("[image 1]", "![image 1](01.png)")));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The markers in a text, in order.
    fn marks_in(s: &str) -> Vec<String> {
        let mut out = vec![];
        let mut rest = s;
        while let Some(i) = rest.find("[image ") {
            let j = i + rest[i..].find(']').expect("a closed marker");
            out.push(rest[i..=j].to_string());
            rest = &rest[j + 1..];
        }
        out
    }

    /// The words of a text without its markers, lower-cased with the punctuation stripped.
    fn words_in(s: &str) -> Vec<String> {
        let mut t = s.to_string();
        for m in marks_in(s) {
            t = t.replacen(&m, " ", 1);
        }
        t.split_whitespace().map(norm).filter(|w| !w.is_empty()).collect()
    }

    /// `out` is `raw` with only fillers and immediate repeats of one to three words taken out, the order kept and
    /// nothing added; and the markers are the same, in the same order.
    fn check_invariants(raw: &str, out: &str) -> Result<(), String> {
        if marks_in(raw) != marks_in(out) {
            return Err(format!("markers changed: {:?} -> {:?}", marks_in(raw), marks_in(out)));
        }
        let (r, o) = (words_in(raw), words_in(out));
        // Word `p` of `seq` is one copy of one to three words said twice in a row.
        let repeat = |seq: &[String], p: usize| {
            (1..=3).any(|n| {
                (p.saturating_sub(n - 1)..=p).any(|s| {
                    (s + 2 * n <= seq.len() && seq[s..s + n] == seq[s + n..s + 2 * n])
                        || (s >= n && s + n <= seq.len() && seq[s - n..s] == seq[s..s + n])
                })
            })
        };
        let mut j = 0;
        for (i, w) in r.iter().enumerate() {
            if j < o.len() && o[j] == *w {
                j += 1;
                continue;
            }
            // What is left once the words already taken out are gone: the output so far, then the rest of the input.
            let seq: Vec<String> = o[..j].iter().chain(&r[i..]).cloned().collect();
            if !(FILLERS.contains(&w.as_str()) || repeat(&seq, j)) {
                return Err(format!("word {i} {w:?} went missing"));
            }
        }
        if j != o.len() {
            return Err(format!("words added or moved from output word {j}"));
        }
        for p in out.split("\n\n") {
            if words_in(p).is_empty() && !o.is_empty() {
                return Err(format!("a paragraph without words: {p:?}"));
            }
        }
        Ok(())
    }

    #[test]
    fn tidy_keeps_every_content_word_in_order_and_every_marker_over_generated_dictations() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        let vocab = [
            "the", "header", "button", "is", "too", "tall", "so", "we", "kind", "of", "like", "you", "know,", "I", "mean,",
            "two", "three", "4", "Um,", "um", "uh.", "First,", "second,", "Third", "number", "one", "save", "page.",
            "done.", "right?", "okay!", "It's", "menu\u{2026}", "had", "that", "\u{201c}Save\u{201d}", "it", "here.",
        ];
        let gaps = [0, 100, 900, 1_200, 1_300, 2_000, 2_500, 4_000];
        for case in 0..400 {
            let mut items = vec![];
            let mut t = 0u64;
            for _ in 0..1 + rnd(14) {
                t += gaps[rnd(gaps.len())];
                let mut w: Vec<&str> = if rnd(6) == 0 {
                    vec![["Um.", "Um...", "Uh.", "um"][rnd(4)]]
                } else {
                    (0..1 + rnd(16)).map(|_| vocab[rnd(vocab.len())]).collect()
                };
                if rnd(3) == 0 && w.len() > 1 {
                    let (s, n) = (rnd(w.len()), 1 + rnd(3));
                    let span: Vec<&str> = w[s..(s + n).min(w.len())].to_vec();
                    let at = s + span.len();
                    w.splice(at..at, span);
                }
                let end = t + 250 * w.len() as u64;
                items.push(said(t, end, &w.join(" ")));
                t = end;
            }
            for _ in 0..rnd(5) {
                items.push(shot(rnd(t as usize + 3_000) as u64, CaptureKind::Full, vec![]));
            }
            let raw = compose(&items).0;
            let out = tidy(&items);
            if let Err(e) = check_invariants(&raw, &out) {
                panic!("case {case}: {e}\nraw: {raw}\nout: {out}");
            }
            assert_eq!(compose_with(&items, false).0, raw, "off is today's joining");
        }
    }

    /// The owner's real dictations: `POINTCAST_REAL_ITEMS` names a folder of `*.items.json`. Prints counts only.
    #[test]
    #[ignore]
    fn tidy_keeps_every_word_and_marker_of_real_transcripts() {
        let dir = std::env::var("POINTCAST_REAL_ITEMS").expect("POINTCAST_REAL_ITEMS names a folder of *.items.json");
        let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.to_string_lossy().ends_with(".items.json"))
            .collect();
        paths.sort();
        assert!(!paths.is_empty(), "no transcripts in {dir}");
        println!("| # | words in | words out | paragraphs | repeats collapsed | fillers dropped |");
        for (k, p) in paths.iter().enumerate() {
            let mut items: Vec<Item> = serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap();
            // The recordings had no pictures: plant them as compose places them (in a gap, and mid-piece).
            let ends: Vec<(u64, u64)> = items.iter().filter_map(|i| match i {
                Item::Speech { t_ms, end_ms, .. } => Some((*t_ms, *end_ms)),
                Item::Image { .. } => None,
            }).collect();
            let end = ends.iter().map(|e| e.1).max().unwrap_or(0);
            let mut times = vec![end * 3 / 10, end * 62 / 100, end * 9 / 10];
            if ends.len() > 1 {
                times.push((ends[0].1 + ends[1].0) / 2);
            }
            items.extend(times.into_iter().map(|t| shot(t, CaptureKind::Full, vec![])));
            let (pieces, _) = pieces_of(&items, false);
            let raw = compose(&items).0;
            let (out, st) = tidy_pieces(&pieces);
            assert_eq!(out, tidy(&items));
            check_invariants(&raw, &out).unwrap_or_else(|e| panic!("transcript {}: {e}", k + 1));
            println!(
                "| {} | {} | {} | {} | {} | {} |",
                k + 1,
                words_in(&raw).len(),
                words_in(&out).len(),
                st.paragraphs,
                st.repeats,
                st.fillers
            );
        }
    }
}
