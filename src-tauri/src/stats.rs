//! What the person has dictated, as running counters in `<data dir>/stats.json`. History keeps only its newest
//! entries, so totals cannot be counted from it later. The file is read and rewritten once per saved recording and
//! read when the Usage pane opens; no timer, no thread, and between those only today's word count stays in memory,
//! for the tray's line.
use crate::history::Meta;
use crate::prompt::Item;
use chrono::{Datelike, NaiveDate, Offset, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Days kept one by one; older ones stay in the totals only.
pub const KEEP_DAYS: usize = 400;

/// Entries remembered by id, so one saved twice counts once and one transcribed again replaces its words.
pub const KEEP_RECENT: usize = crate::history::KEEP;

const FILE: &str = "stats.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Total {
    pub recordings: u64,
    pub words: u64,
    pub speech_ms: u64,
    pub wall_ms: u64,
    pub pictures: u64,
}

/// One local day: recordings, words, time speaking, time recording, pictures.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    pub r: u64,
    pub w: u64,
    pub speech_ms: u64,
    pub wall_ms: u64,
    pub p: u64,
}

/// Recordings and words in one hour of the day, or on one day of the week.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Slot {
    pub r: u64,
    pub w: u64,
}

/// Consecutive local days with at least one recording. `current` is the run that ends on `last_day`: whoever shows it
/// checks that day is today or yesterday.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Streak {
    pub current: u64,
    pub longest: u64,
    pub last_day: String,
}

/// What an entry was counted as, and the local day and hour it was counted on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub w: u64,
    pub speech_ms: u64,
    pub p: u64,
    pub day: String,
    #[serde(default)]
    pub h: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Data {
    pub v: u32,
    /// Counting began here: the oldest entry History held at the first count, or the last reset.
    pub since_ms: u64,
    pub total: Total,
    pub days: BTreeMap<String, Day>,
    /// By local hour, 0 to 23.
    pub hours: Vec<Slot>,
    /// By local weekday, Monday first.
    pub weekdays: Vec<Slot>,
    /// Recordings by weekday (Monday first), then by hour: the two lists above cannot say which hours of which day.
    #[serde(default)]
    pub grid: Vec<Vec<u64>>,
    pub streak: Streak,
    pub recent: BTreeMap<String, Recent>,
}

/// What one recording adds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Counts {
    pub words: u64,
    pub speech_ms: u64,
    pub pictures: u64,
}

/// The words said, the time they took and the pictures taken. Only speech has words: a picture's marker, labels and
/// control name are not counted, nor is the placeholder for speech that could not be made out. Speech with no words in
/// it (only that placeholder) takes no time either: Speed is words over the time they took.
pub fn counts(items: &[Item]) -> Counts {
    let mut c = Counts::default();
    for it in items {
        match it {
            Item::Speech { t_ms, end_ms, text } => {
                let n = text.split_whitespace().filter(|w| *w != crate::voice::stt::UNCLEAR).count() as u64;
                c.words += n;
                if n > 0 {
                    c.speech_ms += end_ms.saturating_sub(*t_ms);
                }
            }
            Item::Image { .. } => c.pictures += 1,
        }
    }
    c
}

/// The local day ("2026-11-11"), hour and weekday (Monday 0) of a time, `offset_s` seconds east of UTC.
fn local(ms: u64, offset_s: i32) -> (String, usize, usize) {
    let t = chrono::DateTime::from_timestamp_millis(ms as i64 + offset_s as i64 * 1000).unwrap_or_default().naive_utc();
    (t.format("%Y-%m-%d").to_string(), t.hour() as usize, t.weekday().num_days_from_monday() as usize)
}

/// This computer's offset from UTC at a time, in seconds: summer time counts on the days it was in force.
fn offset_at(ms: u64) -> i32 {
    chrono::DateTime::from_timestamp_millis(ms as i64).map_or(0, |d| d.with_timezone(&chrono::Local).offset().fix().local_minus_utc())
}

fn date(day: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").ok()
}

/// Put `new` in place of `old` inside a sum.
fn swap(sum: &mut u64, old: u64, new: u64) {
    *sum = sum.saturating_sub(old) + new;
}

impl Data {
    fn new(since_ms: u64) -> Self {
        Data { v: 1, since_ms, hours: vec![Slot::default(); 24], weekdays: vec![Slot::default(); 7], grid: vec![vec![0; 24]; 7], ..Default::default() }
    }

    /// A file of this version with every list at its length.
    fn sound(&self) -> bool {
        self.v == 1 && self.hours.len() == 24 && self.weekdays.len() == 7 && self.grid.len() == 7 && self.grid.iter().all(|d| d.len() == 24)
    }

    /// Count a saved recording made at `created_ms`, `offset_s` seconds east of UTC. One already counted (the same id)
    /// changes nothing.
    fn add(&mut self, id: &str, c: Counts, created_ms: u64, wall_ms: u64, offset_s: i32) {
        if self.recent.contains_key(id) {
            return;
        }
        let (day, h, wd) = local(created_ms, offset_s);
        let t = &mut self.total;
        (t.recordings, t.words, t.speech_ms, t.wall_ms, t.pictures) =
            (t.recordings + 1, t.words + c.words, t.speech_ms + c.speech_ms, t.wall_ms + wall_ms, t.pictures + c.pictures);
        let d = self.days.entry(day.clone()).or_default();
        (d.r, d.w, d.speech_ms, d.wall_ms, d.p) = (d.r + 1, d.w + c.words, d.speech_ms + c.speech_ms, d.wall_ms + wall_ms, d.p + c.pictures);
        for s in [&mut self.hours[h], &mut self.weekdays[wd]] {
            (s.r, s.w) = (s.r + 1, s.w + c.words);
        }
        self.grid[wd][h] += 1;
        self.streak_on(&day);
        self.recent.insert(id.into(), Recent { w: c.words, speech_ms: c.speech_ms, p: c.pictures, day, h: h as u32 });
        // Ids are time stamps and days are dates: the first key is the oldest.
        while self.recent.len() > KEEP_RECENT {
            self.recent.pop_first();
        }
        while self.days.len() > KEEP_DAYS {
            self.days.pop_first();
        }
    }

    /// Transcribe again: the entry's words and speech time take the place of what it was counted as, wherever it was
    /// counted. Its recording, its day and its pictures are as they were. An entry no longer remembered changes nothing.
    fn replace(&mut self, id: &str, c: Counts) {
        let Some(r) = self.recent.get_mut(id) else { return };
        let (w, s) = (r.w, r.speech_ms);
        (r.w, r.speech_ms) = (c.words, c.speech_ms);
        let (day, h) = (r.day.clone(), r.h as usize % 24);
        swap(&mut self.total.words, w, c.words);
        swap(&mut self.total.speech_ms, s, c.speech_ms);
        if let Some(d) = self.days.get_mut(&day) {
            swap(&mut d.w, w, c.words);
            swap(&mut d.speech_ms, s, c.speech_ms);
        }
        swap(&mut self.hours[h].w, w, c.words);
        if let Some(wd) = date(&day).map(|d| d.weekday().num_days_from_monday() as usize) {
            swap(&mut self.weekdays[wd].w, w, c.words);
        }
    }

    /// A recording was made on `day`, which is already among the days.
    fn streak_on(&mut self, day: &str) {
        let gap = date(&self.streak.last_day).zip(date(day)).map(|(a, b)| (b - a).num_days());
        match gap {
            Some(0) => {}
            Some(1) => self.streak.current += 1,
            // An earlier day arriving late (a recording recovered after a crash) can join two runs: count them again.
            Some(n) if n < 0 => {
                let (run, longest) = self.runs();
                self.streak.current = self.streak.current.max(run);
                self.streak.longest = self.streak.longest.max(longest);
            }
            _ => self.streak.current = 1,
        }
        if !matches!(gap, Some(n) if n <= 0) {
            self.streak.last_day = day.into();
        }
        self.streak.longest = self.streak.longest.max(self.streak.current);
    }

    /// The run of consecutive days that ends on the last day kept, and the longest run among the days kept.
    fn runs(&self) -> (u64, u64) {
        let (mut run, mut longest, mut prev) = (0u64, 0u64, None::<NaiveDate>);
        for d in self.days.keys().filter_map(|k| date(k)) {
            run = if prev.is_some_and(|p| (d - p).num_days() == 1) { run + 1 } else { 1 };
            longest = longest.max(run);
            prev = Some(d);
        }
        (run, longest)
    }
}

pub struct Stats {
    root: PathBuf,
    /// One read and write at a time: a stop and a Transcribe again can end together.
    lock: Mutex<()>,
    /// Today's words as last counted, with the day they are for.
    today: Mutex<Option<(String, u64)>>,
    /// Seconds east of UTC at a time. This computer's; a test names its own.
    offset: fn(u64) -> i32,
}

impl Stats {
    pub fn new(data_dir: &Path) -> Self {
        Stats { root: data_dir.to_path_buf(), lock: Mutex::default(), today: Mutex::default(), offset: offset_at }
    }

    fn path(&self) -> PathBuf {
        self.root.join(FILE)
    }

    /// Count a recording History just saved. Never an error: a recording is not lost or held up over its statistics.
    pub fn count(&self, m: &Meta) {
        let _one = self.lock.lock().unwrap();
        let mut d = self.load().0;
        d.add(&m.id, self.entry(&m.id), m.created_ms, m.duration_ms, (self.offset)(m.created_ms));
        self.write(&d);
    }

    /// An entry was transcribed again: its new words replace the ones counted for it.
    pub fn again(&self, id: &str) {
        let _one = self.lock.lock().unwrap();
        let mut d = self.load().0;
        d.replace(id, self.entry(id));
        self.write(&d);
    }

    /// The counters, for the Usage pane.
    pub fn get(&self) -> Data {
        let _one = self.lock.lock().unwrap();
        self.settled()
    }

    /// Start counting again from now. History is left as it is, and is not counted in again.
    pub fn reset(&self) -> Data {
        let _one = self.lock.lock().unwrap();
        let d = Data::new(crate::session::now_ms());
        self.write(&d);
        d
    }

    /// Words dictated today, for the tray's line. Read from the file the first time, then kept in memory as each
    /// recording is counted.
    pub fn today_words(&self) -> u64 {
        if self.today.lock().unwrap().is_none() {
            let _one = self.lock.lock().unwrap();
            self.settled();
        }
        let day = self.day_now();
        match &*self.today.lock().unwrap() {
            Some((d, w)) if *d == day => *w,
            _ => 0,
        }
    }

    fn day_now(&self) -> String {
        let now = crate::session::now_ms();
        local(now, (self.offset)(now)).0
    }

    /// The counters as kept, a first count written down so it is made once.
    fn settled(&self) -> Data {
        let (d, fresh) = self.load();
        if fresh {
            self.write(&d);
        } else {
            self.note_today(&d);
        }
        d
    }

    /// The counters as kept and false; with no file, or one that does not read, a fresh count of what History holds
    /// and true.
    fn load(&self) -> (Data, bool) {
        match std::fs::read(self.path()).ok().and_then(|b| serde_json::from_slice::<Data>(&b).ok()).filter(Data::sound) {
            Some(d) => (d, false),
            None => (self.seed(), true),
        }
    }

    /// Every entry History holds, counted oldest first; counting begins at the oldest.
    fn seed(&self) -> Data {
        let mut entries = crate::history::Store::new(&self.root).list();
        entries.reverse();
        let mut d = Data::new(entries.first().map_or_else(crate::session::now_ms, |m| m.created_ms));
        for m in &entries {
            d.add(&m.id, self.entry(&m.id), m.created_ms, m.duration_ms, (self.offset)(m.created_ms));
        }
        d
    }

    /// What an entry holds now, from its session as History wrote it. Picture files are not read.
    fn entry(&self, id: &str) -> Counts {
        std::fs::read(self.root.join("history").join(id).join("items.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Vec<Item>>(&b).ok())
            .map_or_else(Counts::default, |v| counts(&v))
    }

    /// Write to a temp file, then rename it into place. A write that fails leaves the old file and is not an error.
    fn write(&self, d: &Data) {
        self.note_today(d);
        let Ok(b) = serde_json::to_vec(d) else { return };
        let _ = std::fs::create_dir_all(&self.root);
        let tmp = self.path().with_extension("tmp");
        if std::fs::write(&tmp, b).and_then(|_| std::fs::rename(&tmp, self.path())).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }

    fn note_today(&self, d: &Data) {
        let day = self.day_now();
        let words = d.days.get(&day).map_or(0, |x| x.w);
        *self.today.lock().unwrap() = Some((day, words));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::Store;
    use crate::prompt::{Capture, CaptureKind, Label};

    /// A test's own data folder: never the real app's, never another test's.
    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pc-stats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn said(t: u64, end: u64, text: &str) -> Item {
        Item::Speech { t_ms: t, end_ms: end, text: text.into() }
    }

    fn picture(t: u64) -> Item {
        Item::Image {
            t_ms: t,
            capture: Capture {
                id: 0,
                kind: CaptureKind::Region,
                t_ms: t,
                png: vec![137, 80, 78, 71],
                width: 1,
                height: 1,
                labels: vec![Label { n: 1, text: "this one here".into(), ..Default::default() }],
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
        }
    }

    /// Midnight UTC on a day of November 2026, in ms. The 9th is a Monday.
    fn nov(day: u32) -> u64 {
        NaiveDate::from_ymd_opt(2026, 11, day).unwrap().and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis() as u64
    }

    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;

    fn words(n: u64) -> Counts {
        Counts { words: n, speech_ms: n * 400, pictures: 0 }
    }

    #[test]
    fn markers_and_the_unclear_placeholder_are_not_words() {
        let unclear = crate::voice::stt::UNCLEAR;
        let items = vec![
            said(0, 900, "the button is off"),
            picture(1_000),
            said(1_200, 3_200, unclear),
            said(3_300, 4_000, &format!("move it {unclear} left")),
        ];
        // The unclear item's 2 s is not time spent saying words; the item with words and a placeholder keeps its time.
        assert_eq!(counts(&items), Counts { words: 7, speech_ms: 900 + 700, pictures: 1 });
        // The prompt has a marker with the picture's labels and control in it; none of that was said.
        let plain = crate::prompt::assemble_export(&items).plain;
        assert!(plain.contains("[image 1") && plain.contains("this one here"), "{plain}");
        assert_eq!(counts(&[picture(0)]), Counts { words: 0, speech_ms: 0, pictures: 1 });
        assert_eq!(counts(&[said(0, 5_000, unclear)]), Counts { words: 0, speech_ms: 0, pictures: 0 });
        assert_eq!(counts(&[said(900, 100, "  two   words ")]).words, 2);
    }

    #[test]
    fn unclear_speech_does_not_lower_speed_when_counted_transcribed_again_or_counted_from_history() {
        let d = dir("unclear-speed");
        let store = Store::new(&d);
        let stats = Stats { offset: |_| 0, ..Stats::new(&d) };
        let unclear = crate::voice::stt::UNCLEAR;
        let wpm = |t: &Total| t.words * 60_000 / t.speech_ms.max(1);
        assert_eq!(stats.get().total, Total::default(), "counting begins on an empty History");
        // 60 words over 30 s, then 30 s nobody could make out: 120 words a minute, not 60.
        let first = vec![said(0, 30_000, &"word ".repeat(60)), said(30_000, 60_000, unclear)];
        let m = store.save(&first, nov(9) + 10 * HOUR, 61_000).unwrap();
        stats.count(&m);
        let counted = stats.get().total;
        assert_eq!((counted.words, counted.speech_ms, wpm(&counted)), (60, 30_000, 120), "counted at the stop");
        // Transcribed again: 90 words over 45 s and 15 s unclear, still 120 words a minute.
        let better = vec![said(0, 45_000, &"word ".repeat(90)), said(45_000, 60_000, unclear)];
        store.replace_speech(&m.id, &better).unwrap();
        stats.again(&m.id);
        let again = stats.get().total;
        assert_eq!((again.recordings, again.words, again.speech_ms, wpm(&again)), (1, 90, 45_000, 120), "transcribed again");
        // A first count from History says the same.
        std::fs::remove_file(d.join(FILE)).unwrap();
        assert_eq!(stats.get().total, again, "counted from History");
    }

    #[test]
    fn a_recording_belongs_to_the_local_day_and_hour_it_was_made_in() {
        // Wednesday 11 November, 23:30 UTC.
        let at = nov(11) + 23 * HOUR + HOUR / 2;
        assert_eq!(local(at, 0), ("2026-11-11".to_string(), 23, 2));
        assert_eq!(local(at, 3_600), ("2026-11-12".to_string(), 0, 3), "an hour east it is already Thursday");
        assert_eq!(local(nov(11) + HOUR, -8 * 3_600), ("2026-11-10".to_string(), 17, 1), "eight hours west it is still Tuesday");
        // Two recordings half an hour apart, either side of local midnight: two days, a streak of two.
        let mut d = Data::new(0);
        d.add("a", words(10), at - HOUR, 9_000, 3_600);
        d.add("b", words(5), at, 4_000, 3_600);
        assert_eq!(d.days.keys().collect::<Vec<_>>(), ["2026-11-11", "2026-11-12"]);
        assert_eq!((d.days["2026-11-11"].w, d.days["2026-11-12"].w), (10, 5));
        assert_eq!((d.hours[23].clone(), d.hours[0].clone()), (Slot { r: 1, w: 10 }, Slot { r: 1, w: 5 }));
        assert_eq!((d.weekdays[2].w, d.weekdays[3].w, d.grid[2][23], d.grid[3][0]), (10, 5, 1, 1));
        assert_eq!(d.streak, Streak { current: 2, longest: 2, last_day: "2026-11-12".into() });
        // The same two with no offset fall on one day.
        let mut u = Data::new(0);
        u.add("a", words(10), at - HOUR, 9_000, 0);
        u.add("b", words(5), at, 4_000, 0);
        assert_eq!((u.days.len(), u.days["2026-11-11"].r, u.streak.current), (1, 2, 1));
        assert_eq!(u.total, d.total, "the totals do not depend on the clock");
    }

    #[test]
    fn a_streak_across_a_gap_starts_again_and_the_longest_is_kept() {
        let mut d = Data::new(0);
        for (i, day) in [2, 3, 4].into_iter().enumerate() {
            d.add(&format!("e{i}"), words(3), nov(day) + 9 * HOUR, 1_000, 0);
        }
        d.add("again-that-day", words(3), nov(4) + 15 * HOUR, 1_000, 0);
        assert_eq!(d.streak, Streak { current: 3, longest: 3, last_day: "2026-11-04".into() });
        // Nothing on the 5th and 6th.
        d.add("f", words(3), nov(7) + 9 * HOUR, 1_000, 0);
        assert_eq!(d.streak, Streak { current: 1, longest: 3, last_day: "2026-11-07".into() });
        d.add("g", words(3), nov(8) + 9 * HOUR, 1_000, 0);
        assert_eq!(d.streak, Streak { current: 2, longest: 3, last_day: "2026-11-08".into() });
        // A recording from the 6th turns up late (recovered after a crash): the run now starts there.
        d.add("late", words(3), nov(6) + 9 * HOUR, 1_000, 0);
        assert_eq!(d.streak, Streak { current: 3, longest: 3, last_day: "2026-11-08".into() });
        // And one from the 5th joins the two runs.
        d.add("later", words(3), nov(5) + 9 * HOUR, 1_000, 0);
        assert_eq!(d.streak, Streak { current: 7, longest: 7, last_day: "2026-11-08".into() });
    }

    #[test]
    fn transcribing_again_replaces_an_entrys_words_and_never_counts_them_twice() {
        let mut d = Data::new(0);
        // Monday 9 November at 10 and at 14.
        d.add("a", Counts { words: 5, speech_ms: 2_000, pictures: 2 }, nov(9) + 10 * HOUR, 6_000, 0);
        d.add("b", Counts { words: 3, speech_ms: 1_000, pictures: 0 }, nov(9) + 14 * HOUR, 2_000, 0);
        let before = d.clone();
        let better = Counts { words: 9, speech_ms: 2_500, pictures: 2 };
        d.replace("a", better);
        assert_eq!(d.total, Total { recordings: 2, words: 12, speech_ms: 3_500, wall_ms: 8_000, pictures: 2 });
        assert_eq!(d.days["2026-11-09"], Day { r: 2, w: 12, speech_ms: 3_500, wall_ms: 8_000, p: 2 });
        assert_eq!((d.hours[10].clone(), d.hours[14].clone()), (Slot { r: 1, w: 9 }, Slot { r: 1, w: 3 }));
        assert_eq!((d.weekdays[0].clone(), d.grid[0][10], d.grid[0][14]), (Slot { r: 2, w: 12 }, 1, 1));
        assert_eq!(d.recent["a"], Recent { w: 9, speech_ms: 2_500, p: 2, day: "2026-11-09".into(), h: 10 });
        assert_eq!(d.streak, before.streak);
        let once = d.clone();
        d.replace("a", better);
        assert_eq!(d, once, "the same words again change nothing");
        // Fewer words than before (the first pass heard noise as speech) come off.
        d.replace("a", Counts { words: 0, speech_ms: 0, pictures: 2 });
        assert_eq!((d.total.words, d.total.speech_ms, d.hours[10].clone(), d.grid[0][10]), (3, 1_000, Slot { r: 1, w: 0 }, 1));
        let left = d.clone();
        d.replace("not-remembered", better);
        assert_eq!(d, left, "an entry that is not remembered changes nothing");
    }

    #[test]
    fn the_same_id_saved_twice_counts_once() {
        let mut d = Data::new(0);
        d.add("a", words(5), nov(9) + 10 * HOUR, 6_000, 0);
        let once = d.clone();
        d.add("a", words(5), nov(9) + 10 * HOUR, 6_000, 0);
        d.add("a", words(50), nov(10), 60_000, 0);
        assert_eq!(d, once);
        assert_eq!((d.total.recordings, d.total.words, d.recent.len()), (1, 5, 1));
    }

    /// Five recordings over four days, with and without pictures and unclear speech.
    fn fixture() -> Vec<(Vec<Item>, u64, u64)> {
        let unclear = crate::voice::stt::UNCLEAR;
        vec![
            (vec![said(0, 900, "the button is off by a few pixels"), picture(1_000), said(1_200, 2_000, "move it left")], nov(3) + 9 * HOUR, 2_400),
            (vec![said(0, 4_000, unclear)], nov(3) + 22 * HOUR, 4_100),
            (vec![picture(10), picture(20)], nov(4) + 8 * HOUR, 700),
            (vec![said(100, 61_000, &"word ".repeat(140))], nov(6) + 13 * HOUR, 62_000),
            (vec![said(0, 800, "and one more"), picture(900)], nov(6) + 13 * HOUR + 5_000, 1_500),
        ]
    }

    #[test]
    fn a_first_count_from_history_equals_a_direct_count_of_its_entries() {
        let d = dir("seed");
        let store = Store::new(&d);
        let mut direct = Total::default();
        for (items, at, wall) in fixture() {
            store.save(&items, at, wall).unwrap();
            let c = counts(&items);
            direct.recordings += 1;
            direct.words += c.words;
            direct.speech_ms += c.speech_ms;
            direct.wall_ms += wall;
            direct.pictures += c.pictures;
        }
        assert_eq!((direct.recordings, direct.words, direct.pictures), (5, 11 + 140 + 3, 4), "the fixture is what it says");
        assert!(!d.join(FILE).exists());
        let stats = Stats { offset: |_| 0, ..Stats::new(&d) };
        let got = stats.get();
        assert_eq!(got.total, direct);
        assert_eq!(got.since_ms, nov(3) + 9 * HOUR, "counting begins at the oldest entry");
        assert_eq!(got.days.keys().collect::<Vec<_>>(), ["2026-11-03", "2026-11-04", "2026-11-06"]);
        let sum = |f: fn(&Day) -> u64| got.days.values().map(f).sum::<u64>();
        assert_eq!((sum(|x| x.r), sum(|x| x.w), sum(|x| x.speech_ms), sum(|x| x.wall_ms), sum(|x| x.p)), (5, direct.words, direct.speech_ms, direct.wall_ms, 4));
        assert_eq!(got.hours.iter().map(|s| s.w).sum::<u64>(), direct.words);
        assert_eq!(got.weekdays.iter().map(|s| s.r).sum::<u64>(), 5);
        assert_eq!(got.grid.iter().flatten().sum::<u64>(), 5);
        assert_eq!(got.streak, Streak { current: 1, longest: 2, last_day: "2026-11-06".into() });
        let ids: Vec<String> = store.list().into_iter().map(|m| m.id).collect();
        assert!(ids.len() == 5 && ids.iter().all(|id| got.recent.contains_key(id)), "{ids:?}");
        // Written down once: the file reads back as it was returned, and a later save of an entry it holds adds nothing.
        let kept: Data = serde_json::from_slice(&std::fs::read(d.join(FILE)).unwrap()).unwrap();
        assert_eq!(kept, got);
        stats.count(&store.list()[0]);
        assert_eq!(stats.get(), got);
        // An offset moves the days, never the totals.
        std::fs::remove_file(d.join(FILE)).unwrap();
        let east = Stats { offset: |_| 3 * 3_600, ..Stats::new(&d) }.get();
        assert_eq!(east.total, direct);
        assert_eq!(east.days.keys().collect::<Vec<_>>(), ["2026-11-03", "2026-11-04", "2026-11-06"]);
        assert_eq!((east.days["2026-11-04"].r, east.days["2026-11-03"].r), (2, 1), "22:00 UTC is the next day three hours east");
    }

    #[test]
    fn a_stats_file_that_does_not_read_is_replaced_and_the_recording_still_saves() {
        let d = dir("corrupt");
        let store = Store::new(&d);
        let unread: [&[u8]; 4] = [b"{ not json", b"", br#"{"v":1}"#, br#"{"v":7,"since_ms":0,"total":{"recordings":9,"words":9,"speech_ms":9,"wall_ms":9,"pictures":9},"days":{},"hours":[],"weekdays":[],"streak":{"current":0,"longest":0,"last_day":""},"recent":{}}"#];
        for junk in unread {
            let _ = std::fs::remove_dir_all(d.join("history"));
            std::fs::write(d.join(FILE), junk).unwrap();
            let stats = Stats { offset: |_| 0, ..Stats::new(&d) };
            let (items, at, wall) = fixture().remove(0);
            let m = store.save(&items, at, wall).unwrap();
            stats.count(&m);
            assert_eq!(store.items(&m.id).unwrap().len(), 3, "the words and the picture are in History");
            let kept: Data = serde_json::from_slice(&std::fs::read(d.join(FILE)).unwrap()).unwrap();
            assert!(kept.sound());
            assert_eq!(kept.total, Total { recordings: 1, words: 11, speech_ms: 1_700, wall_ms: wall, pictures: 1 });
            assert_eq!(stats.get(), kept);
        }
        assert!(!d.join("stats.tmp").exists());
    }

    #[test]
    fn a_stats_file_that_cannot_be_written_never_costs_the_recording() {
        let d = dir("unwritable");
        // A folder is in the file's place: it cannot be read, replaced or renamed over.
        std::fs::create_dir_all(d.join(FILE).join("in-the-way")).unwrap();
        let store = Store::new(&d);
        let stats = Stats { offset: |_| 0, ..Stats::new(&d) };
        let now = crate::session::now_ms();
        let m = store.save(&[said(0, 900, "still here after that")], now, 1_000).unwrap();
        stats.count(&m);
        stats.again(&m.id);
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.items(&m.id).unwrap(), [said(0, 900, "still here after that")]);
        assert!(d.join(FILE).is_dir() && !d.join("stats.tmp").exists(), "nothing is left half written");
        assert_eq!(stats.get().total.words, 4, "and what History holds is still counted, from History");
        // The tray's line needs no file: it is the count just made, unless the day (UTC here) turned since the save.
        let today = local(now, 0).0;
        if stats.day_now() == today {
            assert_eq!(stats.today_words(), 4);
        }
    }

    #[test]
    fn days_past_four_hundred_fold_into_the_totals_and_the_newest_hundred_ids_are_remembered() {
        let mut d = Data::new(0);
        let n = KEEP_DAYS as u64 + 5;
        let start = nov(1);
        for i in 0..n {
            d.add(&chrono::DateTime::from_timestamp_millis((start + i * DAY) as i64).unwrap().format("%Y%m%d-090000-000").to_string(), Counts { words: 10, speech_ms: 4_000, pictures: 1 }, start + i * DAY + 9 * HOUR, 5_000, 0);
            if i + 1 == KEEP_DAYS as u64 {
                assert_eq!(d.days.len(), KEEP_DAYS, "every day is still there at four hundred");
                assert!(d.days.contains_key("2026-11-01"));
            }
        }
        assert_eq!(d.days.len(), KEEP_DAYS);
        assert_eq!(d.days.keys().next().unwrap(), "2026-11-06", "the five oldest days are gone");
        assert_eq!(d.total, Total { recordings: n, words: n * 10, speech_ms: n * 4_000, wall_ms: n * 5_000, pictures: n });
        assert_eq!(d.days.values().map(|x| x.w).sum::<u64>(), KEEP_DAYS as u64 * 10);
        assert_eq!(d.hours[9], Slot { r: n, w: n * 10 });
        assert_eq!(d.streak.current, n, "a streak longer than the days kept is still counted");
        assert_eq!(d.streak.longest, n);
        assert_eq!(d.recent.len(), KEEP_RECENT);
        let last = chrono::DateTime::from_timestamp_millis((start + (n - 1) * DAY) as i64).unwrap().format("%Y%m%d-090000-000").to_string();
        let first = chrono::DateTime::from_timestamp_millis((start + (n - KEEP_RECENT as u64) * DAY) as i64).unwrap().format("%Y%m%d-090000-000").to_string();
        assert_eq!((d.recent.keys().next().unwrap(), d.recent.keys().last().unwrap()), (&first, &last));
        assert!(d.sound());
    }

    #[test]
    fn a_reset_starts_from_nothing_and_history_is_not_counted_in_again() {
        let d = dir("reset");
        let store = Store::new(&d);
        for (items, at, wall) in fixture() {
            store.save(&items, at, wall).unwrap();
        }
        let stats = Stats { offset: |_| 0, ..Stats::new(&d) };
        assert_eq!(stats.get().total.recordings, 5);
        let before = crate::session::now_ms();
        let fresh = stats.reset();
        assert_eq!(fresh.total, Total::default());
        assert!(fresh.since_ms >= before && fresh.days.is_empty() && fresh.recent.is_empty() && fresh.sound());
        assert_eq!(stats.get(), fresh, "the entries History still holds stay out");
        assert_eq!(store.list().len(), 5, "and History itself is untouched");
        assert_eq!(stats.today_words(), 0);
        // Transcribing one of them again adds nothing either; a new recording counts.
        stats.again(&store.list()[0].id);
        assert_eq!(stats.get().total, Total::default());
        let m = store.save(&[said(0, 500, "a new one")], nov(8), 600).unwrap();
        stats.count(&m);
        assert_eq!(stats.get().total, Total { recordings: 1, words: 3, speech_ms: 500, wall_ms: 600, pictures: 0 });
    }
}
