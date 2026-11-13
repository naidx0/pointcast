//! An answer cut into sentences as it streams in, each one made fit to be read aloud. A sentence is given out the
//! moment it is complete (one character after its full stop, to tell "Dr. Smith" from "done. Next"), and `finish`
//! gives out the rest. Markdown marks go, a link says its text, inline code is read as it is, and a fenced code block
//! is not read at all: "(code)" is said once in its place. A heading or a list item is a sentence of its own. What a
//! model thinks between `<think>` tags is never given out.

/// Once a sentence is this many characters long it is cut at its next comma or semicolon, so speech starts sooner.
pub const LONG: usize = 220;
/// With no comma or semicolon, it is cut at its next space once it is this long.
pub const HARD: usize = 400;
/// Said once in place of each fenced code block.
pub const CODE: &str = "(code)";

/// Titles and short words that end in a full stop without ending the sentence: lowercase, without the stop.
const TITLES: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "mt", "ft", "lt", "col", "gen", "gov", "sen", "rep", "rev", "capt", "sgt", "vs", "cf",
    "approx", "dept",
];
/// Words that end in a full stop without ending the sentence when a number follows ("No. 5", "p. 12", "Jan. 3").
const BEFORE_NUMBERS: &[&str] = &[
    "no", "nos", "p", "pp", "vol", "fig", "figs", "ch", "sec", "art", "jan", "feb", "mar", "apr", "jun", "jul", "aug", "sep", "sept", "oct",
    "nov", "dec",
];
/// What may close a sentence after its stop and still belong to it: quotes, brackets, and the ends of emphasis.
const CLOSERS: &[char] = &['"', '\'', '\u{201d}', '\u{2019}', ')', ']', '*', '_'];

#[derive(Debug, Default)]
pub struct Splitter {
    unthink: Unthink,
    line: Line,
    /// A line's first characters, held until they say what kind of line it is.
    head: String,
    /// A table row, held whole.
    row: String,
    /// Inside a fenced code block: its fence character and how many of them opened it.
    fence: Option<(char, usize)>,
    /// The sentence so far, as Markdown, and its length in characters.
    pending: String,
    count: usize,
    stop: Option<Stop>,
    /// The run of backticks that opened the code span the text is in; 0 outside one.
    tick: usize,
    /// Backticks just seen, not yet counted as an opening or a close.
    run: usize,
    /// Inside a link's text, and inside its address.
    bracket: usize,
    url: bool,
    /// The last character was a link text's closing bracket.
    closed: bool,
    out: Vec<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Line {
    /// Its first characters are held in `head`.
    #[default]
    Start,
    Para,
    /// A heading or a list item: a sentence of its own.
    Own,
    Table,
    /// Code, a fence or a rule: nothing of it is read.
    Skip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Blank,
    FenceOpen(char, usize),
    FenceClose,
    CodeLine,
    Rule,
    /// A heading or a list item; the marker ends at this byte of the line.
    Own(usize),
    /// A quote; its marker ends at this byte.
    Quote(usize),
    Table,
    Para,
}

/// A possible end of the sentence, waiting for what comes next.
#[derive(Debug, Clone)]
struct Stop {
    /// Where the sentence would end in `pending` (after the stop and its closers).
    end: usize,
    how: Ends,
    /// Whitespace came after it.
    spaced: bool,
    /// The word before a full stop, to tell an abbreviation; empty after `!` or `?`.
    word: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ends {
    /// `.`, `!`, `?` or `…`: a sentence ends when the next word does not start in lowercase.
    Full,
    /// `。`, `！` or `？`: a sentence ends here whatever follows.
    Wide,
    /// A comma or semicolon in a long sentence: cut at the space after it.
    Comma,
}

impl Splitter {
    /// Take the next piece of the answer and give out the sentences it completed.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        let mut shown = String::new();
        self.unthink.feed(text, &mut shown);
        for c in shown.chars() {
            self.char(c);
        }
        std::mem::take(&mut self.out)
    }

    /// The answer ended: give out what is left, and start over.
    pub fn finish(&mut self) -> Vec<String> {
        let rest = self.unthink.finish();
        for c in rest.chars() {
            self.char(c);
        }
        if self.line != Line::Start || !self.head.is_empty() {
            self.end_line();
        }
        if self.fence.is_none() {
            self.flush();
        }
        let out = std::mem::take(&mut self.out);
        *self = Splitter::default();
        out
    }

    fn char(&mut self, c: char) {
        match c {
            '\r' => {}
            '\n' => self.end_line(),
            _ => match self.line {
                Line::Start => {
                    self.head.push(c);
                    if let Some(kind) = self.classify(false) {
                        self.begin(kind);
                    }
                }
                Line::Para | Line::Own => self.inline(c),
                Line::Table => self.row.push(c),
                Line::Skip => {}
            },
        }
    }

    /// What kind of line `head` starts; none while it could still be more than one. `whole` when the line ended.
    fn classify(&self, whole: bool) -> Option<Kind> {
        let t = self.head.trim_start_matches([' ', '\t']);
        let lead = self.head.len() - t.len();
        let Some(first) = t.chars().next() else {
            return whole.then_some(if self.fence.is_some() { Kind::CodeLine } else { Kind::Blank });
        };
        let same = t.chars().take_while(|&c| c == first).count();
        let only = same == t.chars().count();
        if let Some((f, n)) = self.fence {
            if first != f {
                return Some(Kind::CodeLine);
            }
            if only && !whole && same < n {
                return None;
            }
            return Some(if same >= n { Kind::FenceClose } else { Kind::CodeLine });
        }
        let spaced_at = |i: usize| t[i..].starts_with([' ', '\t']);
        match first {
            '`' | '~' => {
                if only && !whole {
                    return None;
                }
                Some(if same >= 3 { Kind::FenceOpen(first, same) } else { Kind::Para })
            }
            '#' => {
                if only && !whole {
                    return None;
                }
                Some(if same <= 6 && (only || spaced_at(same)) { Kind::Own(lead + same) } else { Kind::Para })
            }
            '-' | '*' | '+' | '_' => {
                let rule = t.chars().all(|c| c == first || c == ' ' || c == '\t');
                if rule && !whole {
                    return None;
                }
                if rule && t.chars().filter(|&c| c == first).count() >= 3 {
                    return Some(Kind::Rule);
                }
                Some(if first != '_' && t.len() > 1 && spaced_at(1) { Kind::Own(lead + 1) } else { Kind::Para })
            }
            '0'..='9' => {
                let d = t.chars().take_while(char::is_ascii_digit).count();
                let rest = &t[d..];
                let marked = d <= 9 && rest.starts_with(['.', ')']);
                if !whole && (rest.is_empty() || (marked && rest.len() == 1)) {
                    return None;
                }
                Some(if marked && rest.len() > 1 && rest[1..].starts_with([' ', '\t']) { Kind::Own(lead + d + 1) } else { Kind::Para })
            }
            '>' => Some(Kind::Quote(lead + 1)),
            '|' => Some(Kind::Table),
            _ => Some(Kind::Para),
        }
    }

    fn begin(&mut self, kind: Kind) {
        let head = std::mem::take(&mut self.head);
        let replay = |s: &mut Self, from: usize| {
            for c in head[from..].chars() {
                s.inline(c);
            }
        };
        match kind {
            Kind::Blank => self.flush(),
            Kind::FenceOpen(c, n) => {
                self.flush();
                self.out.push(CODE.to_string());
                self.fence = Some((c, n));
                self.line = Line::Skip;
            }
            Kind::FenceClose => {
                self.fence = None;
                self.line = Line::Skip;
            }
            Kind::CodeLine => self.line = Line::Skip,
            Kind::Rule => {
                self.flush();
                self.line = Line::Skip;
            }
            Kind::Own(at) => {
                self.flush();
                self.line = Line::Own;
                replay(self, at);
            }
            Kind::Quote(at) => {
                self.line = Line::Para;
                replay(self, at);
            }
            Kind::Table => {
                self.flush();
                self.line = Line::Table;
                self.row = head;
            }
            Kind::Para => {
                self.line = Line::Para;
                replay(self, 0);
            }
        }
    }

    fn end_line(&mut self) {
        if self.line == Line::Start {
            match self.classify(true) {
                Some(Kind::Blank) | None => {
                    self.head.clear();
                    if self.fence.is_none() {
                        self.flush();
                    }
                    return;
                }
                Some(kind) => self.begin(kind),
            }
        }
        match self.line {
            Line::Para => {
                // One line break inside a paragraph is a space; a stop at the end of the line ends the sentence.
                self.settle_run();
                if self.stop.is_some() {
                    self.cut();
                }
                if !self.pending.trim().is_empty() {
                    self.pending.push(' ');
                    self.count += 1;
                }
                (self.tick, self.bracket, self.url, self.closed) = (0, 0, false, false);
            }
            Line::Own => self.flush(),
            Line::Table => {
                let row = std::mem::take(&mut self.row);
                if !row.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t')) {
                    let cells: Vec<&str> = row.split('|').map(str::trim).filter(|c| !c.is_empty()).collect();
                    self.say(&cells.join(", "));
                }
            }
            Line::Start | Line::Skip => {}
        }
        self.line = Line::Start;
    }

    fn inline(&mut self, c: char) {
        if self.stop.is_some() && self.lookahead(c) {
            return;
        }
        if c == '`' {
            self.run += 1;
            self.add(c);
            return;
        }
        self.settle_run();
        let free = self.tick == 0 && self.bracket == 0 && !self.url;
        if free {
            let how = match c {
                '.' | '!' | '?' | '\u{2026}' => Some(Ends::Full),
                '\u{3002}' | '\u{ff01}' | '\u{ff1f}' => Some(Ends::Wide),
                ',' | ';' if self.count >= LONG => Some(Ends::Comma),
                _ => None,
            };
            if let Some(how) = how {
                let word = if c == '.' { self.word_before() } else { String::new() };
                self.add(c);
                self.stop = Some(Stop { end: self.pending.len(), how, spaced: false, word });
                return;
            }
            if c.is_whitespace() && self.count >= HARD {
                let end = self.pending.len();
                self.cut_at(end);
                return;
            }
        }
        if self.tick == 0 {
            match c {
                '[' if !self.url => self.bracket += 1,
                ']' if self.bracket > 0 => {
                    self.bracket -= 1;
                    self.add(c);
                    self.closed = true;
                    return;
                }
                '(' if self.closed => self.url = true,
                ')' if self.url => self.url = false,
                _ => {}
            }
        }
        self.closed = false;
        self.add(c);
    }

    /// What comes after a possible end of the sentence. True when `c` was taken here (a closer, another stop or a
    /// space); false when it is read as usual, after the sentence was cut or the stop found to be none.
    fn lookahead(&mut self, c: char) -> bool {
        let Some(stop) = self.stop.as_mut() else { return false };
        if !stop.spaced {
            let more = (stop.how == Ends::Full && matches!(c, '.' | '!' | '?' | '\u{2026}')) || (stop.how != Ends::Comma && CLOSERS.contains(&c));
            if more {
                self.pending.push(c);
                self.count += 1;
                stop.end = self.pending.len();
                return true;
            }
            if c.is_whitespace() {
                if stop.how == Ends::Comma {
                    self.cut();
                } else {
                    stop.spaced = true;
                    self.add(c);
                }
                return true;
            }
            // A character right after the stop: "3.14", "example.com", "1,000".
            if stop.how == Ends::Wide {
                self.cut();
            } else {
                self.stop = None;
            }
            return false;
        }
        if c.is_whitespace() {
            self.add(c);
            return true;
        }
        let ends = match stop.how {
            Ends::Full => !c.is_lowercase() && !abbreviation(&stop.word, c),
            Ends::Wide | Ends::Comma => true,
        };
        if ends {
            self.cut();
        } else {
            self.stop = None;
        }
        false
    }

    fn add(&mut self, c: char) {
        self.pending.push(c);
        self.count += 1;
    }

    /// Backticks just seen open a code span, or close the one they match.
    fn settle_run(&mut self) {
        if self.run == 0 {
            return;
        }
        if self.tick == 0 {
            self.tick = self.run;
        } else if self.tick == self.run {
            self.tick = 0;
        }
        self.run = 0;
    }

    /// The word before the full stop about to be added: "Dr" in "(Dr.", "U.S" in "the U.S.".
    fn word_before(&self) -> String {
        let from = self.pending.rfind(|c: char| c.is_whitespace() || "([{\"'\u{201c}\u{2018}*_".contains(c)).map_or(0, |i| {
            i + self.pending[i..].chars().next().map_or(1, char::len_utf8)
        });
        self.pending[from..].to_string()
    }

    /// The sentence ends at the stop: give it out and keep what came after.
    fn cut(&mut self) {
        if let Some(stop) = self.stop.take() {
            self.cut_at(stop.end);
        }
    }

    fn cut_at(&mut self, end: usize) {
        self.stop = None;
        let rest = self.pending.split_off(end);
        let sentence = std::mem::take(&mut self.pending);
        self.say(&sentence);
        self.pending = rest.trim_start().to_string();
        self.count = self.pending.chars().count();
    }

    /// The sentence ends here, with whatever it holds.
    fn flush(&mut self) {
        self.settle_run();
        let sentence = std::mem::take(&mut self.pending);
        self.say(&sentence);
        self.stop = None;
        self.count = 0;
        (self.tick, self.run, self.bracket, self.url, self.closed) = (0, 0, 0, false, false);
    }

    fn say(&mut self, raw: &str) {
        let s = clean(raw);
        if s.chars().any(char::is_alphanumeric) {
            self.out.push(s);
        }
    }
}

/// Whether `word`, then a full stop, then `next` is an abbreviation and not the end of a sentence.
fn abbreviation(word: &str, next: char) -> bool {
    let w = word.to_lowercase();
    if TITLES.contains(&w.as_str()) || (next.is_ascii_digit() && BEFORE_NUMBERS.contains(&w.as_str())) {
        return true;
    }
    // An initial ("J. R. R. Tolkien"), though not "I".
    let mut cs = word.chars();
    if let (Some(c), None) = (cs.next(), cs.next()) {
        return c.is_uppercase() && c != 'I';
    }
    // Letters with stops between them: "U.S", "e.g", "i.e", "Ph.D", "a.m".
    w.contains('.') && w.split('.').all(|p| !p.is_empty() && p.chars().count() <= 2 && p.chars().all(char::is_alphabetic))
}

/// One sentence of Markdown as it is read aloud: a link or a picture says its text, inline code its contents, and
/// emphasis marks and tags go. A `*` between spaces ("2 * 3") and a `_` inside a word ("snake_case") stay.
pub fn clean(raw: &str) -> String {
    let cs: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    let run_of = |i: usize| cs[i..].iter().take_while(|&&c| c == cs[i]).count();
    while i < cs.len() {
        let c = cs[i];
        match c {
            '`' => {
                let n = run_of(i);
                let mut j = i + n;
                let mut close = None;
                while j < cs.len() {
                    if cs[j] == '`' {
                        let m = run_of(j);
                        if m == n {
                            close = Some(j);
                            break;
                        }
                        j += m;
                    } else {
                        j += 1;
                    }
                }
                match close {
                    Some(end) => {
                        out.extend(&cs[i + n..end]);
                        i = end + n;
                    }
                    None => i += n,
                }
            }
            '!' if cs.get(i + 1) == Some(&'[') => i += 1,
            '[' => {
                let close = find_close(&cs, i + 1, '[', ']');
                match close {
                    Some(end) => {
                        let text: String = cs[i + 1..end].iter().collect();
                        out.push_str(&clean(&text));
                        i = end + 1;
                        // Its address, or the name of a reference, is not read.
                        let after = match cs.get(i) {
                            Some('(') => find_close(&cs, i + 1, '(', ')'),
                            Some('[') => find_close(&cs, i + 1, '[', ']'),
                            _ => None,
                        };
                        if let Some(end) = after {
                            i = end + 1;
                        }
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            '*' | '_' | '~' => {
                let n = run_of(i);
                let before = i.checked_sub(1).map(|j| cs[j]);
                let after = cs.get(i + n).copied();
                let word = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
                let space = |c: Option<char>| c.is_none_or(char::is_whitespace);
                let mark = match c {
                    '~' => n >= 2,
                    '_' => !(word(before) && word(after)),
                    _ => !(space(before) && space(after)),
                };
                if !mark {
                    out.extend(&cs[i..i + n]);
                }
                i += n;
            }
            '<' => {
                let end = cs[i..].iter().position(|&c| c == '>').map(|p| i + p);
                let tag = end.filter(|&e| {
                    let inner: String = cs[i + 1..e].iter().collect();
                    let name = inner.trim_start_matches('/').trim_end_matches('/').trim();
                    name.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && name.chars().all(|c| c.is_ascii_alphanumeric())
                });
                match tag {
                    Some(e) => {
                        out.push(' ');
                        i = e + 1;
                    }
                    None => {
                        out.push(c);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where the bracket opened before `from` closes, counting brackets opened inside it.
fn find_close(cs: &[char], from: usize, open: char, close: char) -> Option<usize> {
    let mut depth = 0;
    for (j, &c) in cs.iter().enumerate().skip(from) {
        if c == open {
            depth += 1;
        } else if c == close {
            if depth == 0 {
                return Some(j);
            }
            depth -= 1;
        }
    }
    None
}

/// Drops what a model thinks out loud between `<think>` and `</think>` (some models on this computer put it in the
/// answer itself), however the tags are split across pieces.
#[derive(Debug, Default)]
struct Unthink {
    held: String,
    inside: bool,
}

const OPEN: &[&str] = &["<think>", "<thinking>"];
const CLOSE: &[&str] = &["</think>", "</thinking>"];

impl Unthink {
    fn feed(&mut self, text: &str, out: &mut String) {
        for c in text.chars() {
            self.one(c, out);
        }
    }

    fn one(&mut self, c: char, out: &mut String) {
        if self.held.is_empty() && c != '<' {
            if !self.inside {
                out.push(c);
            }
            return;
        }
        self.held.push(c);
        let tags = if self.inside { CLOSE } else { OPEN };
        if tags.contains(&self.held.as_str()) {
            self.inside = !self.inside;
            self.held.clear();
            return;
        }
        if tags.iter().any(|t| t.starts_with(self.held.as_str())) {
            return;
        }
        // Not a tag after all: what was held is text, and its last character may start a tag of its own.
        let mut held = std::mem::take(&mut self.held);
        held.pop();
        if !self.inside {
            out.push_str(&held);
        }
        if c == '<' {
            self.one(c, out);
        } else if !self.inside {
            out.push(c);
        }
    }

    /// What was held as a possible tag, now known to be text; nothing of an unfinished thought.
    fn finish(&mut self) -> String {
        let held = std::mem::take(&mut self.held);
        let inside = std::mem::take(&mut self.inside);
        if inside {
            String::new()
        } else {
            held
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(text: &str) -> Vec<String> {
        let mut s = Splitter::default();
        let mut out = s.push(text);
        out.extend(s.finish());
        out
    }

    fn v(x: &[&str]) -> Vec<String> {
        x.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_sentence_is_given_out_as_soon_as_the_next_one_starts_and_the_rest_at_the_end() {
        let mut s = Splitter::default();
        assert!(s.push("Hello there.").is_empty(), "a full stop alone could still be an abbreviation");
        assert!(s.push(" ").is_empty());
        assert_eq!(s.push("How"), v(&["Hello there."]));
        assert!(s.push(" are you?").is_empty());
        assert_eq!(s.finish(), v(&["How are you?"]));
        assert!(s.finish().is_empty(), "finish starts over");
    }

    #[test]
    fn abbreviations_initials_and_dotted_short_forms_do_not_end_a_sentence() {
        assert_eq!(
            all("Dr. Smith met Mr. Jones in the U.S. last May. They talked, e.g. about J. R. R. Tolkien. It went well."),
            v(&["Dr. Smith met Mr. Jones in the U.S. last May.", "They talked, e.g. about J. R. R. Tolkien.", "It went well."])
        );
        assert_eq!(all("See No. 5 and p. 12. No. That is wrong."), v(&["See No. 5 and p. 12.", "No.", "That is wrong."]));
        assert_eq!(all("So did I. Then we left."), v(&["So did I.", "Then we left."]), "I is not an initial");
        assert_eq!(all("He has a Ph.D. in physics. Wow!"), v(&["He has a Ph.D. in physics.", "Wow!"]));
    }

    #[test]
    fn decimals_versions_addresses_and_ellipses_inside_a_sentence_do_not_end_it() {
        assert_eq!(
            all("Pi is 3.14 and the version is 1.2.3. Visit example.com now... or later. Done!? Yes."),
            v(&["Pi is 3.14 and the version is 1.2.3.", "Visit example.com now... or later.", "Done!?", "Yes."])
        );
        assert_eq!(all("It costs $5. 10 people came."), v(&["It costs $5.", "10 people came."]));
    }

    #[test]
    fn quotes_brackets_and_emphasis_after_the_stop_stay_with_their_sentence() {
        assert_eq!(all("He said \"stop.\" Then (quietly.) **Go now.** Fine."), v(&["He said \"stop.\"", "Then (quietly.)", "Go now.", "Fine."]));
    }

    #[test]
    fn list_items_and_headings_are_sentences_of_their_own() {
        assert_eq!(
            all("## Steps\nDo this:\n1. Open the file\n2) Click **Save**.\n- Done\n* Also this\n+ And that\n\nAfter the list."),
            v(&["Steps", "Do this:", "Open the file", "Click Save.", "Done", "Also this", "And that", "After the list."])
        );
    }

    #[test]
    fn a_line_break_inside_a_paragraph_is_a_space_and_a_blank_line_ends_it() {
        assert_eq!(all("This sentence wraps\nonto the next line. Next\none.\n\nNew paragraph"), v(&[
            "This sentence wraps onto the next line.",
            "Next one.",
            "New paragraph"
        ]));
    }

    #[test]
    fn markdown_marks_go_and_a_link_says_its_text() {
        assert_eq!(
            all("# Title\nSome **bold**, *italic*, _under_ and ~~gone~~ text with a [link to docs](https://x.com/a.b?c=d. e) here."),
            v(&["Title", "Some bold, italic, under and gone text with a link to docs here."])
        );
        assert_eq!(all("A picture ![a cat](cat.png) and a [ref][1] link."), v(&["A picture a cat and a ref link."]));
        assert_eq!(all("Keep 2 * 3 and snake_case.<br>Next."), v(&["Keep 2 * 3 and snake_case. Next."]));
        assert_eq!(all("> A quote. Here.\n---\nAfter"), v(&["A quote.", "Here.", "After"]));
    }

    #[test]
    fn fenced_code_is_not_read_and_code_is_said_once_but_inline_code_is_read_plainly() {
        assert_eq!(
            all("Run this:\n```bash\nnpm i. Then this.\n\nnpm test\n```\nThen `npm test. Now` runs."),
            v(&["Run this:", CODE, "Then npm test. Now runs."]),
        );
        assert_eq!(all("Before.\n~~~\nx = 1\n~~~\n```py\nprint(1)\n```\nAfter."), v(&["Before.", CODE, CODE, "After."]));
        assert_eq!(all("Look:\n````\n```\nstill code\n````\nOut."), v(&["Look:", CODE, "Out."]), "a longer fence closes only on its own length");
        assert_eq!(all("Unclosed:\n```\nlet x = 1;"), v(&["Unclosed:", CODE]), "an answer that ends inside code says nothing more");
    }

    #[test]
    fn a_table_reads_row_by_row_without_its_rule() {
        assert_eq!(all("| Name | Age |\n|---|:-:|\n| Ann | 30 |"), v(&["Name, Age", "Ann, 30"]));
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_comma_or_semicolon_after_about_220_characters() {
        let part = "this clause goes on for a while without stopping";
        let long = format!("{part}, {part}, {part}, {part}, {part}; {part}, {part}. Short one.");
        let got = all(&long);
        assert!(got.len() >= 3, "{got:?}");
        assert!(got[0].chars().count() >= LONG && got[0].chars().count() < LONG + part.len() + 3, "{}", got[0]);
        assert!(got[0].ends_with(',') || got[0].ends_with(';'), "{}", got[0]);
        assert_eq!(got.last().unwrap(), "Short one.");
        assert_eq!(got.join(" "), long);
        // No comma at all: cut at a space once it is very long.
        let words = "word ".repeat(120);
        let got = all(&words);
        assert!(got.len() >= 2 && got[0].chars().count() < HARD + 8, "{got:?}");
        assert_eq!(got.join(" "), words.trim());
    }

    #[test]
    fn thinking_between_tags_is_never_given_out() {
        assert_eq!(all("<think>I should greet. Then explain.</think>Hello there. Bye."), v(&["Hello there.", "Bye."]));
        assert_eq!(all("<thinking>Hmm.</thinking>Ok."), v(&["Ok."]));
        assert_eq!(all("a < b and <b>bold</b> stay. <thin air."), v(&["a < b and bold stay.", "<thin air."]));
        assert_eq!(all("<think>never closed. More."), Vec::<String>::new());
    }

    #[test]
    fn streamed_text_in_random_chunk_sizes_gives_the_same_sentences_as_one_chunk() {
        let samples = [
            "Dr. Smith met Mr. Jones in the U.S. last May. They talked, e.g. about 3.14 and v1.2.3. It went well!",
            "## Steps\nDo this:\n1. Open the file\n2. Click **Save**.\n- Done\n\nAfter the [list](http://a.b/c).",
            "Run this:\n```bash\nnpm i\n```\nThen `npm test` runs. | a | b |\n|---|---|\n| 1 | 2 |\nEnd.",
            "<think>Plan it. Then go.</think>Héllo wörld — ça va? 日本語の文です。次の文！Done…\nNext line.",
            &format!("{}; and the end. Small.", "a long run of words, ".repeat(20)),
        ];
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for text in samples {
            let whole = all(text);
            assert!(!whole.is_empty());
            let cs: Vec<char> = text.chars().collect();
            for _ in 0..300 {
                let mut s = Splitter::default();
                let mut got = Vec::new();
                let mut i = 0;
                while i < cs.len() {
                    let n = 1 + next(12);
                    let piece: String = cs[i..(i + n).min(cs.len())].iter().collect();
                    got.extend(s.push(&piece));
                    i += n;
                }
                got.extend(s.finish());
                assert_eq!(got, whole, "{text:?}");
            }
        }
    }

    #[test]
    fn wide_stops_end_a_sentence_whatever_follows() {
        assert_eq!(all("日本語の文です。次の文！最後？"), v(&["日本語の文です。", "次の文！", "最後？"]));
    }
}
