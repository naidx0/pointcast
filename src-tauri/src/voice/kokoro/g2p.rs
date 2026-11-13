//! Text to Kokoro's phonemes, on this computer. sherpa-onnx 2.0 has no espeak-ng (GPL), and its own lexicon path
//! drops every word it does not know, so Pointcast reads the model's lexicon itself and says what is not in it: numbers
//! as words, "GitHub" as "git hub", "API" letter by letter, and an unknown word by its spelling's usual sounds.

/// The model's word list: each word with its phonemes, in one block of text with an index sorted by word.
pub struct Lexicon {
    text: String,
    /// (start of the word, end of the word, end of the line) in `text`, sorted by word.
    index: Vec<(u32, u32, u32)>,
}

impl Lexicon {
    /// Lines of `word p h o n e m e s`, as the model's lexicon files have them.
    pub fn parse(text: String) -> Lexicon {
        let mut index = Vec::new();
        let mut at = 0usize;
        for line in text.split_inclusive('\n') {
            let end = at + line.trim_end().len();
            if let Some(sp) = line.find(' ') {
                index.push((at as u32, (at + sp) as u32, end as u32));
            }
            at += line.len();
        }
        index.sort_by(|a, b| text[a.0 as usize..a.1 as usize].cmp(&text[b.0 as usize..b.1 as usize]));
        index.dedup_by(|a, b| text[a.0 as usize..a.1 as usize] == text[b.0 as usize..b.1 as usize]);
        Lexicon { text, index }
    }

    /// The phonemes of `word` (lower case), joined.
    pub fn get(&self, word: &str) -> Option<String> {
        let key = |e: &(u32, u32, u32)| &self.text[e.0 as usize..e.1 as usize];
        let i = self.index.binary_search_by(|e| key(e).cmp(word)).ok()?;
        let (_, k, end) = self.index[i];
        Some(self.text[k as usize..end as usize].chars().filter(|c| *c != ' ').collect())
    }

    pub fn has(&self, word: &str) -> bool {
        self.get(word).is_some()
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.index.len()
    }
}

/// Punctuation the model has a token for, said as it is.
const MARKS: &[char] = &[',', '.', '!', '?', ';', ':', '—', '…', '"', '(', ')', '“', '”'];
/// Marks that open: a space before them, none after.
const OPENING: &[char] = &['(', '“'];

/// One piece of a sentence: a word's phonemes, or a mark.
#[derive(Debug, PartialEq)]
enum Piece {
    Word(String),
    Mark(char),
}

/// `text` as Kokoro's phonemes: words apart by spaces, marks against the word before them.
pub fn phonemes(lex: &Lexicon, text: &str) -> String {
    let pieces = pieces(lex, &tidy(text));
    let mut out = String::new();
    let mut open = true;
    for p in pieces {
        match p {
            Piece::Word(w) if !w.is_empty() => {
                if !open {
                    out.push(' ');
                }
                out.push_str(&w);
                open = false;
            }
            Piece::Word(_) => {}
            Piece::Mark(m) if OPENING.contains(&m) || (m == '"' && open_quote(&out)) => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push(m);
                open = true;
            }
            Piece::Mark(m) => {
                out.push(m);
                open = false;
            }
        }
    }
    out.trim().to_string()
}

/// A straight double quote opens when an even number came before it.
fn open_quote(before: &str) -> bool {
    before.matches('"').count().is_multiple_of(2)
}

/// Curly apostrophes straight, three dots as one mark, dashes as the model's dash.
fn tidy(text: &str) -> String {
    text.replace(['’', '‘'], "'").replace("...", "…").replace(" - ", " — ").replace(['–', '―'], "—")
}

fn pieces(lex: &Lexicon, text: &str) -> Vec<Piece> {
    let chunks: Vec<&str> = text.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, chunk) in chunks.iter().enumerate() {
        let lead: String = chunk.chars().take_while(|c| MARKS.contains(c) && *c != '.').collect();
        let rest = &chunk[lead.len()..];
        let core_end = rest.trim_end_matches(|c: char| MARKS.contains(&c)).len();
        let (mut core, mut tail) = (&rest[..core_end], &rest[core_end..]);
        // "Dr." and "etc." keep their dot when the lexicon has them so.
        if tail.starts_with('.') && !core.is_empty() && lex.has(&format!("{}.", core.to_lowercase())) {
            core = &rest[..core_end + 1];
            tail = &rest[core_end + 1..];
        }
        out.extend(lead.chars().map(Piece::Mark));
        if !core.is_empty() {
            let next = chunks.get(i + 1).map(|n| n.trim_start_matches(|c: char| !c.is_alphanumeric())).unwrap_or("");
            out.push(Piece::Word(word(lex, core, next)));
        }
        let tail = tail.replace("..", "…");
        let mut last = None;
        for m in tail.chars() {
            // "?!" is said once each; a run of the same mark once.
            if last != Some(m) {
                out.push(Piece::Mark(m));
            }
            last = Some(m);
        }
    }
    out
}

/// One whitespace-free chunk (marks peeled off) as phonemes.
fn word(lex: &Lexicon, w: &str, next: &str) -> String {
    let lower = w.to_lowercase();
    // The two short words the lexicon gives their stressed form: "a" as "uh", "the" as "thuh" before a consonant.
    match lower.as_str() {
        "a" if w == "a" || next.chars().next().is_some_and(char::is_lowercase) => return "ɐ".into(),
        "the" => return if next.chars().next().is_some_and(|c| "aeiouAEIOU".contains(c)) { "ði".into() } else { "ðə".into() },
        _ => {}
    }
    if let Some(p) = lex.get(&lower) {
        // "AI", "IT", "US" in capitals are letters; "NO", "STOP" and "NASA" are words.
        if !(shouted(w) && w.chars().count() <= 3 && !COMMON.contains(&lower.as_str())) {
            return p;
        }
    }
    if let Some(said) = written(w) {
        return said.split(' ').map(|x| word(lex, x, "")).collect::<Vec<_>>().join(" ");
    }
    let parts = split(w);
    if parts.len() > 1 {
        return parts.iter().map(|x| word(lex, x, "")).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" ");
    }
    unknown(lex, w)
}

/// Short words said as words even in capitals.
const COMMON: &[&str] = &[
    "ok", "no", "not", "yes", "the", "and", "but", "or", "so", "do", "go", "we", "he", "me", "my", "up", "to", "of", "in", "on",
    "is", "be", "an", "as", "at", "by", "if", "all", "you", "can", "now", "new", "get", "use", "why", "how", "who", "has",
    "had", "was", "are", "did", "see", "say", "let", "its", "our", "out", "off", "one", "two", "six", "ten", "yet", "may",
    "any", "few", "i'm", "her", "him", "his", "she",
];

/// All capitals and at least two letters: an acronym.
fn shouted(w: &str) -> bool {
    w.chars().filter(|c| c.is_alphabetic()).count() >= 2 && w.chars().all(|c| !c.is_alphabetic() || c.is_uppercase())
}

/// A word with no letters of its own that stands for words: a number, an amount, a symbol.
fn written(w: &str) -> Option<String> {
    let symbol = match w {
        "&" => Some("and"),
        "+" => Some("plus"),
        "=" => Some("equals"),
        "@" => Some("at"),
        "%" => Some("percent"),
        "#" => Some("number"),
        "e.g" | "e.g." => Some("for example"),
        "i.e" | "i.e." => Some("that is"),
        "vs" | "vs." => Some("versus"),
        _ => None,
    };
    if let Some(s) = symbol {
        return Some(s.into());
    }
    let (sign, body) = match w.strip_prefix(['-', '−']) {
        Some(b) if b.starts_with(|c: char| c.is_ascii_digit()) => ("minus ", b),
        _ => ("", w),
    };
    let (unit, body) = match body.chars().next() {
        Some('$') => (Some(("dollar", "dollars")), &body[1..]),
        Some('€') => (Some(("euro", "euros")), &body['€'.len_utf8()..]),
        Some('£') => (Some(("pound", "pounds")), &body['£'.len_utf8()..]),
        _ => (None, body),
    };
    let (body, percent) = match body.strip_suffix('%') {
        Some(b) => (b, " percent"),
        None => (body, ""),
    };
    if !body.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let digits: String = body.chars().filter(|c| *c != ',').collect();
    // An ordinal: 1st, 22nd, 103rd, 4th.
    for suffix in ["st", "nd", "rd", "th"] {
        if let Some(n) = digits.strip_suffix(suffix).filter(|n| n.chars().all(|c| c.is_ascii_digit())) {
            if n.len() > 15 {
                return None;
            }
            return Some(format!("{sign}{}", ordinal(&cardinal(n.parse().ok()?))));
        }
    }
    let (whole, frac) = match digits.split_once('.') {
        Some((a, b)) if !b.is_empty() && b.chars().all(|c| c.is_ascii_digit()) => (a, Some(b)),
        _ => (digits.as_str(), None),
    };
    if !whole.chars().all(|c| c.is_ascii_digit()) || whole.len() > 15 {
        return None;
    }
    let n: u64 = whole.parse().ok()?;
    let plain = !body.contains(',') && unit.is_none() && percent.is_empty() && frac.is_none() && sign.is_empty();
    let mut said = if plain && whole.len() == 4 { year(n) } else { cardinal(n) };
    match (unit, frac) {
        (Some((one, many)), Some(c)) if c.len() == 2 => {
            said = format!("{said} {}", if n == 1 { one } else { many });
            let cents: u64 = c.parse().ok()?;
            if cents > 0 {
                said = format!("{said} {} {}", cardinal(cents), if cents == 1 { "cent" } else { "cents" });
            }
            return Some(format!("{sign}{said}"));
        }
        (_, Some(f)) => said = format!("{said} point {}", f.chars().map(|d| ONES[d as usize - '0' as usize]).collect::<Vec<_>>().join(" ")),
        _ => {}
    }
    if let Some((one, many)) = unit {
        said = format!("{said} {}", if n == 1 && frac.is_none() { one } else { many });
    }
    Some(format!("{sign}{said}{percent}"))
}

const ONES: [&str; 20] = [
    "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve", "thirteen", "fourteen",
    "fifteen", "sixteen", "seventeen", "eighteen", "nineteen",
];
const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];

/// A whole number in words: 1,204 is "one thousand two hundred four".
pub fn cardinal(n: u64) -> String {
    fn under_1000(n: u64) -> String {
        let mut w = Vec::new();
        if n >= 100 {
            w.push(format!("{} hundred", ONES[(n / 100) as usize]));
        }
        let r = n % 100;
        if r >= 20 {
            w.push(match r % 10 {
                0 => TENS[(r / 10) as usize].to_string(),
                d => format!("{} {}", TENS[(r / 10) as usize], ONES[d as usize]),
            });
        } else if r > 0 || n == 0 {
            w.push(ONES[r as usize].to_string());
        }
        w.join(" ")
    }
    if n == 0 {
        return "zero".into();
    }
    // Up to quintillions: every u64 is said (u64::MAX is about 18 quintillion).
    let scales = [(1_000_000_000_000_000_000, "quintillion"), (1_000_000_000_000_000, "quadrillion"), (1_000_000_000_000, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand")];
    let (mut n, mut w) = (n, Vec::new());
    for (size, name) in scales {
        if n >= size {
            w.push(format!("{} {name}", under_1000(n / size)));
            n %= size;
        }
    }
    if n > 0 {
        w.push(under_1000(n));
    }
    w.join(" ")
}

/// A four-digit number said as a year: 1999 "nineteen ninety nine", 2026 "twenty twenty six", 2005 "two thousand five".
fn year(n: u64) -> String {
    let (hi, lo) = (n / 100, n % 100);
    if !(1100..2100).contains(&n) || (hi == 20 && lo < 10) {
        return cardinal(n);
    }
    match lo {
        0 => format!("{} hundred", cardinal(hi)),
        1..=9 => format!("{} oh {}", cardinal(hi), ONES[lo as usize]),
        _ => format!("{} {}", cardinal(hi), cardinal(lo)),
    }
}

/// The ordinal of a number in words: "twenty one" is "twenty first".
fn ordinal(words: &str) -> String {
    let (head, last) = words.rsplit_once(' ').map_or(("", words), |(h, l)| (h, l));
    let last = match last {
        "one" => "first".into(),
        "two" => "second".into(),
        "three" => "third".into(),
        "five" => "fifth".into(),
        "eight" => "eighth".into(),
        "nine" => "ninth".into(),
        "twelve" => "twelfth".into(),
        l if l.ends_with('y') => format!("{}ieth", &l[..l.len() - 1]),
        l => format!("{l}th"),
    };
    if head.is_empty() {
        last
    } else {
        format!("{head} {last}")
    }
}

/// A chunk that holds more than one word: "GitHub", "snake_case", "read/write", "mp3", "main.rs", "U.S.".
fn split(w: &str) -> Vec<String> {
    // Single letters between dots are spelled: "U.S." is "U S".
    let dotted: Vec<&str> = w.split('.').filter(|p| !p.is_empty()).collect();
    if dotted.len() > 1 && dotted.iter().all(|p| p.chars().count() == 1) {
        return dotted.iter().map(|p| p.to_uppercase()).collect();
    }
    let mut parts = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    let chars: Vec<char> = w.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        let next = chars.get(i + 1).copied();
        let boundary = match prev {
            None => false,
            Some(p) => {
                (p.is_ascii_digit() != c.is_ascii_digit() && p.is_alphanumeric() && c.is_alphanumeric())
                    || (p.is_lowercase() && c.is_uppercase())
                    // "HTMLParser" is "HTML" "Parser", but "APIs" is one word.
                    || (p.is_uppercase() && c.is_uppercase() && next.is_some_and(char::is_lowercase) && chars.get(i + 2).is_some_and(|c| c.is_lowercase()))
            }
        };
        if c == '.' && prev.is_some_and(|p| p.is_ascii_digit()) && next.is_some_and(|n| n.is_ascii_digit()) {
            cur.push(c);
        } else if c.is_alphanumeric() || c == '\'' || c == '%' || c == '$' || c == ',' && next.is_some_and(|n| n.is_ascii_digit()) {
            if boundary && !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            cur.push(c);
        } else {
            if !cur.is_empty() {
                parts.push(std::mem::take(&mut cur));
            }
            match c {
                '.' if next.is_some() => parts.push("dot".into()),
                '&' | '+' | '=' | '@' | '#' => parts.push(c.to_string()),
                _ => {}
            }
        }
        prev = Some(c);
    }
    if !cur.is_empty() {
        parts.push(cur);
    }
    parts
}

/// A single word the lexicon does not have.
fn unknown(lex: &Lexicon, w: &str) -> String {
    let lower = w.to_lowercase();
    let letters: String = lower.chars().filter(|c| c.is_alphabetic() || *c == '\'').collect();
    if letters.is_empty() {
        return String::new();
    }
    // "Pointcast's", "APIs": the word, then its s.
    for (end, keep) in [("'s", 2), ("s", 1)] {
        if let Some(stem) = w.strip_suffix(end).or_else(|| w.strip_suffix(&end.to_uppercase())) {
            if stem.chars().count() >= 2 && (keep == 2 || shouted(stem)) {
                let base = word(lex, stem, "");
                if !base.is_empty() {
                    return format!("{base}{}", s_sound(&base));
                }
            }
        }
    }
    if shouted(w) || !letters.chars().any(|c| "aeiouy".contains(c)) || letters.chars().count() <= 2 {
        return spell(lex, &letters);
    }
    if let Some(parts) = compound(lex, &letters) {
        return parts.iter().filter_map(|p| lex.get(p)).collect::<Vec<_>>().join(" ");
    }
    sounds(&letters)
}

/// The plural or possessive s after `base`'s last sound.
fn s_sound(base: &str) -> &'static str {
    match base.chars().last() {
        Some('s' | 'z' | 'ʃ' | 'ʒ' | 'ʧ' | 'ʤ') => "ɪz",
        Some('p' | 't' | 'k' | 'f' | 'θ') => "s",
        _ => "z",
    }
}

/// Letter by letter, each by its name.
fn spell(lex: &Lexicon, letters: &str) -> String {
    letters.chars().filter(|c| c.is_alphabetic()).filter_map(|c| lex.get(&c.to_string())).collect::<Vec<_>>().join(" ")
}

/// `w` as two or three words the lexicon has, each of three letters or more, fewest parts first: "pointcast" is
/// "point" "cast".
fn compound(lex: &Lexicon, w: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    // best[i]: the fewest parts that make up the first i letters, and where the last part starts.
    let mut best: Vec<Option<(usize, usize)>> = vec![None; n + 1];
    best[0] = Some((0, 0));
    for end in 3..=n {
        for start in 0..=end - 3 {
            let Some((parts, _)) = best[start] else { continue };
            let piece: String = chars[start..end].iter().collect();
            if lex.has(&piece) && best[end].is_none_or(|(p, _)| parts + 1 < p) {
                best[end] = Some((parts + 1, start));
            }
        }
    }
    let (parts, _) = best[n]?;
    if !(2..=3).contains(&parts) {
        return None;
    }
    let mut out = Vec::new();
    let mut end = n;
    while end > 0 {
        let (_, start) = best[end]?;
        out.push(chars[start..end].iter().collect());
        end = start;
    }
    out.reverse();
    Some(out)
}

/// A word by its spelling's usual sounds, stressed on its first vowel: rough, but every word is said.
fn sounds(w: &str) -> String {
    const PAIRS: &[(&str, &str)] = &[
        ("tch", "ʧ"), ("sch", "sk"), ("igh", "I"), ("sh", "ʃ"), ("ch", "ʧ"), ("th", "θ"), ("ph", "f"), ("wh", "w"), ("ck", "k"),
        ("ng", "ŋ"), ("qu", "kw"), ("ee", "i"), ("ea", "i"), ("oo", "u"), ("ou", "W"), ("ow", "O"), ("oa", "O"), ("ai", "A"),
        ("ay", "A"), ("oi", "Y"), ("oy", "Y"), ("au", "ɔ"), ("aw", "ɔ"), ("ie", "i"), ("ei", "A"), ("er", "ɚ"), ("ar", "ɑɹ"),
        ("or", "ɔɹ"), ("ur", "ɜɹ"), ("ir", "ɜɹ"),
    ];
    let single = |c: char| match c {
        'a' => "æ",
        'e' => "ɛ",
        'i' => "ɪ",
        'o' => "ɑ",
        'u' => "ʌ",
        'y' => "i",
        'c' => "k",
        'g' => "ɡ",
        'j' => "ʤ",
        'q' => "k",
        'r' => "ɹ",
        'x' => "ks",
        _ => "",
    };
    let vowel = |p: &str| p.chars().any(|c| "æɛɪɑʌiuWOAYɔɚɜI".contains(c));
    let chars: Vec<char> = w.chars().filter(|c| c.is_ascii_alphabetic()).collect();
    let mut out = String::new();
    let mut stressed = false;
    let mut i = 0;
    while i < chars.len() {
        let rest: String = chars[i..].iter().collect();
        let (said, used) = match PAIRS.iter().find(|(s, _)| rest.starts_with(s)) {
            Some((s, p)) => (p.to_string(), s.len()),
            None => {
                let c = chars[i];
                // A silent e at the end.
                if c == 'e' && i == chars.len() - 1 && i > 1 {
                    (String::new(), 1)
                } else {
                    let s = single(c);
                    (if s.is_empty() { c.to_string() } else { s.to_string() }, 1)
                }
            }
        };
        if !stressed && vowel(&said) {
            out.push('ˈ');
            stressed = true;
        }
        // A doubled consonant is said once.
        if !(used == 1 && i > 0 && chars[i - 1] == chars[i] && !vowel(&said)) {
            out.push_str(&said);
        }
        i += used;
    }
    out
}

/// Phonemes cut into pieces of at most `max` characters, at a mark where one is near, else at a space: Kokoro reads
/// at most a few hundred at once.
pub fn chunks(phonemes: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest: Vec<char> = phonemes.chars().collect();
    while rest.len() > max {
        let window = &rest[..max];
        let at = window
            .iter()
            .rposition(|c| [',', '.', '!', '?', ';', ':', '—'].contains(c))
            .filter(|&i| i > max / 2)
            .map(|i| i + 1)
            .or_else(|| window.iter().rposition(|c| *c == ' ').filter(|&i| i > 0))
            .unwrap_or(max);
        out.push(rest[..at].iter().collect::<String>().trim().to_string());
        rest.drain(..at);
    }
    let last: String = rest.into_iter().collect();
    if !last.trim().is_empty() {
        out.push(last.trim().to_string());
    }
    out
}

/// Phonemes as the pieces the model makes one after another: the first clause alone, at most `first` characters, so
/// its sound starts soon; then clauses together until five words or more, at most `max` each.
pub fn parts(phonemes: &str, first: usize, max: usize) -> Vec<String> {
    let mut clauses: Vec<String> = Vec::new();
    let mut cur = String::new();
    for c in phonemes.chars() {
        cur.push(c);
        if [',', '.', '!', '?', ';', ':', '—', '…'].contains(&c) {
            clauses.push(std::mem::take(&mut cur));
        }
    }
    clauses.push(cur);
    let mut clauses = clauses.into_iter().map(|c| c.trim().to_string()).filter(|c| !c.is_empty());
    let Some(head) = clauses.next() else { return Vec::new() };
    let mut out = Vec::new();
    let mut rest = String::new();
    if let Some((one, more)) = chunks(&head, first).split_first() {
        out.push(one.clone());
        rest = more.join(" ");
    }
    let words = |s: &str| s.split(' ').filter(|w| !w.is_empty()).count();
    for c in clauses {
        if !rest.is_empty() {
            rest.push(' ');
        }
        rest.push_str(&c);
        if words(&rest) >= 5 {
            out.extend(chunks(&std::mem::take(&mut rest), max));
        }
    }
    if !rest.is_empty() {
        out.extend(chunks(&rest, max));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A few words as the model's lexicon writes them.
    fn lex() -> Lexicon {
        Lexicon::parse(
            "hello h ə l ˈ O\nworld w ˈ ɜ ɹ l d\npoint p ˈ Y n t\ncast k ˈ æ s t\ngit ɡ ˈ ɪ t\nhub h ˈ ʌ b\nthe ð i\na A\nis ɪ z\n\
             dr. d ˈ ɑ k t ə ɹ\nsmith s m ˈ ɪ θ\nok ˌ O k ˈ A\nit ɪ t\nit's ɪ t s\np p ˈ i\ni ˈ I\nb b ˈ i\nc s ˈ i\nu j ˈ u\ns ˈ ɛ s\n\
             twenty t w ˈ ɛ n t i\none w ˈ ʌ n\nfirst f ˈ ɜ ɹ s t\nfive f ˈ I v\npercent p ə ɹ s ˈ ɛ n t\ndollars d ˈ ɑ l ə ɹ z\n\
             dot d ˈ ɑ t\nmain m ˈ A n\nr ˈ ɑ ɹ\ncat k ˈ æ t\napple ˈ æ p ə l\nnineteen n ˌ I n t ˈ i n\nninety n ˈ I n t i\n\
             nine n ˈ I n\nthree θ ɹ ˈ i\npoint p ˈ Y n t\nfour f ˈ ɔ ɹ\n"
                .into(),
        )
    }

    #[test]
    fn known_words_are_looked_up_whatever_their_case_and_marks_stay_against_them() {
        let l = lex();
        assert_eq!(l.len(), 36, "one entry per word, the doubled one once");
        assert_eq!(phonemes(&l, "Hello, world!"), "həlˈO, wˈɜɹld!");
        assert_eq!(phonemes(&l, "  hello   WORLD...  "), "həlˈO wˈɜɹld…");
        assert_eq!(phonemes(&l, "(hello) “world”"), "(həlˈO) “wˈɜɹld”");
        assert_eq!(phonemes(&l, "Dr. Smith"), "dˈɑktəɹ smˈɪθ", "an abbreviation the lexicon has keeps its dot");
        assert_eq!(phonemes(&l, "It’s ok?!"), "ɪts ˌOkˈA?!");
    }

    #[test]
    fn a_and_the_are_said_unstressed_the_way_people_say_them() {
        let l = lex();
        assert_eq!(phonemes(&l, "a cat"), "ɐ kˈæt");
        assert_eq!(phonemes(&l, "the cat"), "ðə kˈæt");
        assert_eq!(phonemes(&l, "The apple"), "ði ˈæpəl");
    }

    #[test]
    fn a_word_not_in_the_lexicon_is_never_dropped() {
        let l = lex();
        assert_eq!(phonemes(&l, "Pointcast"), "pˈYnt kˈæst", "two words it has");
        assert_eq!(phonemes(&l, "GitHub"), "ɡˈɪt hˈʌb", "camel case");
        assert_eq!(phonemes(&l, "API"), "A pˈi ˈI", "an acronym, letter by letter");
        assert_eq!(phonemes(&l, "APIs"), "A pˈi ˈIz");
        assert_eq!(split("HTMLParser"), ["HTML", "Parser"]);
        assert_eq!(phonemes(&l, "Pointcast's"), "pˈYnt kˈæsts");
        assert_eq!(phonemes(&l, "main.rs"), "mˈAn dˈɑt ˈɑɹ ˈɛs");
        assert_eq!(phonemes(&l, "U.S."), "jˈu ˈɛs.");
        let made_up = phonemes(&l, "Blorvex");
        assert!(made_up.starts_with("bl") && made_up.contains('ˈ') && made_up.len() > 4, "{made_up}");
    }

    #[test]
    fn numbers_amounts_and_years_are_said_in_words() {
        assert_eq!(written("1204").unwrap(), "twelve oh four");
        assert_eq!(written("1,204").unwrap(), "one thousand two hundred four");
        assert_eq!(written("1999").unwrap(), "nineteen ninety nine");
        assert_eq!(written("2026").unwrap(), "twenty twenty six");
        assert_eq!(written("2005").unwrap(), "two thousand five");
        assert_eq!(written("1900").unwrap(), "nineteen hundred");
        assert_eq!(written("42").unwrap(), "forty two");
        assert_eq!(written("0").unwrap(), "zero");
        assert_eq!(written("3.14").unwrap(), "three point one four");
        assert_eq!(written("-7").unwrap(), "minus seven");
        assert_eq!(written("50%").unwrap(), "fifty percent");
        assert_eq!(written("$1").unwrap(), "one dollar");
        assert_eq!(written("$5.99").unwrap(), "five dollars ninety nine cents");
        assert_eq!(written("$2,000,000").unwrap(), "two million dollars");
        assert_eq!(written("1st").unwrap(), "first");
        assert_eq!(written("22nd").unwrap(), "twenty second");
        assert_eq!(written("40th").unwrap(), "fortieth");
        assert_eq!(written("12th").unwrap(), "twelfth");
        assert_eq!(cardinal(1_000_000_000_001), "one trillion one");
        assert_eq!(written("&").unwrap(), "and");
        assert_eq!(written("abc"), None);
        let l = lex();
        assert_eq!(phonemes(&l, "21st, 5%."), "twˈɛnti fˈɜɹst, fˈIv pəɹsˈɛnt.");
    }

    #[test]
    fn huge_numbers_and_ordinals_never_panic() {
        assert_eq!(cardinal(2_000_000_000_000_000), "two quadrillion");
        assert!(cardinal(u64::MAX).starts_with("eighteen quintillion "), "{}", cardinal(u64::MAX));
        assert_eq!(written("2000000000000000th"), None, "past what is said in words, like a long plain number");
        assert_eq!(written("99999999999999999999th"), None);
        assert_eq!(written("999999999999999th").unwrap(), "nine hundred ninety nine trillion nine hundred ninety nine billion nine hundred ninety nine million nine hundred ninety nine thousand nine hundred ninety ninth");
        let l = lex();
        assert!(!phonemes(&l, "the 2000000000000000th time").is_empty());
    }

    #[test]
    fn long_phonemes_are_cut_at_a_mark_or_a_space_never_inside_a_word() {
        let p = "aaaa bbbb, cccc dddd eeee. ffff";
        assert_eq!(chunks(p, 100), [p]);
        assert_eq!(chunks(p, 14), ["aaaa bbbb,", "cccc dddd", "eeee. ffff"]);
        for c in chunks(&"word ".repeat(200), 50) {
            assert!(c.chars().count() <= 50 && !c.starts_with(' ') && c.split(' ').all(|w| w == "word"), "{c}");
        }
        assert!(chunks("   ", 10).is_empty());
    }

    #[test]
    fn a_sentence_is_made_in_pieces_its_first_clause_alone_and_short() {
        // "Sure, here is how. You open it, then save."
        let p = "ʃˈʊɹ, hˈɪɹ ɪz hˈW. jˈu ˈOpən ɪt, ðˈɛn sˈAv.";
        assert_eq!(parts(p, 48, 380), ["ʃˈʊɹ,", "hˈɪɹ ɪz hˈW. jˈu ˈOpən ɪt,", "ðˈɛn sˈAv."]);
        // A long first clause is cut at a space, near the start.
        let long = "wˈʌn tˈu θɹˈi fˈɔɹ fˈIv sˈɪks sˈɛvən ˈAt nˈIn tˈɛn ɪlˈɛvən.";
        let got = parts(long, 30, 380);
        assert!(got[0].chars().count() <= 30 && got.len() == 2 && got.join(" ") == long, "{got:?}");
        assert!(parts("", 48, 380).is_empty());
        assert_eq!(parts("hˈI", 48, 380), ["hˈI"]);
    }
}
