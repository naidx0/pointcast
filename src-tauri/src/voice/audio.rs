//! The microphone (cpal) and the signal work around it: downmix to mono, resample to 16 kHz,
//! a level for the meter, and cutting a long take at its pauses so the model never sees more
//! than a few seconds at once. Samples live in memory only.
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 16_000;

/// Streaming resampler from `from` Hz to 16 kHz.
#[derive(Debug, Clone)]
pub struct Resampler {
    step: f64,
    pos: f64,
    acc: f64,
    n: f64,
    last: f32,
}

impl Resampler {
    pub fn new(from: u32) -> Self {
        Resampler {
            step: from as f64 / RATE as f64,
            pos: 0.0,
            acc: 0.0,
            n: 0.0,
            last: 0.0,
        }
    }
    pub fn push(&mut self, mono: &[f32], out: &mut Vec<f32>) {
        if self.step >= 1.0 {
            for &x in mono {
                self.acc += x as f64;
                self.n += 1.0;
                self.pos += 1.0;
                if self.pos >= self.step {
                    self.pos -= self.step;
                    out.push((self.acc / self.n) as f32);
                    self.acc = 0.0;
                    self.n = 0.0;
                }
            }
        } else {
            for &x in mono {
                while self.pos < 1.0 {
                    out.push(self.last + (x - self.last) * self.pos as f32);
                    self.pos += self.step;
                }
                self.pos -= 1.0;
                self.last = x;
            }
        }
    }
}

pub fn downmix(data: &[f32], channels: usize, out: &mut Vec<f32>) {
    let c = channels.max(1);
    for f in data.chunks(c) {
        out.push(f.iter().sum::<f32>() / f.len() as f32);
    }
}

pub fn rms(x: &[f32]) -> f32 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

pub fn meter(rms: f32) -> f32 {
    let db = 20.0 * rms.max(1e-6).log10();
    ((db + 60.0) / 50.0).clamp(0.0, 1.0)
}

/// A frame is voice only above this rms (about -54 dBFS), however quiet the room.
pub const MIN_VOICE: f32 = 0.002;
/// Voice starts this many times above the room's noise floor, and once started holds down to `HOLD` times it.
const ON: f32 = 2.5;
const HOLD: f32 = 1.8;
/// Frames voice is held after the level drops (the end of a word trails off).
const HANG: u32 = 4;
/// The noise floor is this low share of the last `HISTORY` frames (30 s).
const FLOOR_SHARE: f32 = 0.1;
const HISTORY: usize = 1_000;
/// Audio kept before a piece's first voice and after its last (400 ms).
const PAD: usize = RATE as usize * 4 / 10;

/// Cuts a take into pieces of speech for the model. A frame is voice when it stands out from the room: above `ON`
/// times the noise floor (a low percentile of the recent frames, so a noisy room or a quiet microphone moves it) and
/// above `MIN_VOICE`. A piece ends after `gap_ms` without voice, or at `max_ms`, cut at the quietest frame of its last
/// seconds so no word is split, and carries `PAD` of audio on each side of its voice.
#[derive(Debug, Clone)]
pub struct Segmenter {
    pub gap_ms: u32,
    pub max_ms: u32,
    floor: f32,
    /// The last frames' rms, oldest first.
    recent: std::collections::VecDeque<f32>,
    /// Where the open piece starts; none while no voice is waiting.
    open: Option<usize>,
    /// The end of the last frame of voice.
    last_voice: usize,
    voiced: bool,
    hang: u32,
    seen: usize,
    done: usize,
    /// The samples after the last whole frame, waiting for the next feed.
    pending: Vec<f32>,
}

pub const FRAME: usize = 480;

impl Segmenter {
    pub fn new(gap_ms: u32, max_ms: u32) -> Self {
        Segmenter {
            gap_ms,
            max_ms,
            floor: 0.0,
            recent: Default::default(),
            open: None,
            last_voice: 0,
            voiced: false,
            hang: 0,
            seen: 0,
            done: 0,
            pending: vec![],
        }
    }
    fn samples(ms: u32) -> usize {
        ms as usize * RATE as usize / 1000
    }
    /// The room's level: the rms most frames of the last 30 s rise above.
    pub fn floor(&self) -> f32 {
        self.floor
    }
    /// The last frames' rms, oldest first.
    pub fn recent(&self) -> Vec<f32> {
        self.recent.iter().copied().collect()
    }
    /// Cut the whole take so far; only the part not seen before is read.
    pub fn feed(&mut self, all: &[f32]) -> Vec<std::ops::Range<usize>> {
        let from = (self.seen + self.pending.len()).min(all.len());
        self.feed_new(&all[from..])
    }
    /// Cut what arrived since the last feed.
    pub fn feed_new(&mut self, new: &[f32]) -> Vec<std::ops::Range<usize>> {
        self.pending.extend_from_slice(new);
        let mut out = vec![];
        let mut at = 0;
        while at + FRAME <= self.pending.len() {
            let r = rms(&self.pending[at..at + FRAME]);
            at += FRAME;
            self.frame(r, &mut out);
        }
        self.pending.drain(..at);
        out
    }
    fn frame(&mut self, r: f32, out: &mut Vec<std::ops::Range<usize>>) {
        self.recent.push_back(r);
        if self.recent.len() > HISTORY {
            self.recent.pop_front();
        }
        if (self.seen / FRAME) % 10 == 0 || self.recent.len() < HISTORY {
            let mut v: Vec<f32> = self.recent.iter().copied().collect();
            let k = ((v.len() as f32 * FLOOR_SHARE) as usize).min(v.len() - 1);
            self.floor = *v.select_nth_unstable_by(k, |a, b| a.total_cmp(b)).1;
        }
        let at = self.seen;
        self.seen += FRAME;
        let over = |k: f32| r > (self.floor * k).max(MIN_VOICE);
        if over(ON) || (self.voiced && over(HOLD)) {
            self.voiced = true;
            self.hang = HANG;
        } else if self.hang > 0 {
            self.hang -= 1;
        } else {
            self.voiced = false;
        }
        if self.voiced {
            self.open.get_or_insert(at.saturating_sub(PAD).max(self.done));
            self.last_voice = self.seen;
        }
        let Some(start) = self.open else { return };
        if self.seen - self.last_voice >= Self::samples(self.gap_ms) {
            let end = (self.last_voice + PAD).min(self.seen);
            out.push(start..end);
            (self.done, self.open) = (end, None);
        } else if self.seen - start >= Self::samples(self.max_ms) {
            // Cut at the quietest frame of the last few seconds (at most a third of the piece).
            let look = (Self::samples(4_000).min(Self::samples(self.max_ms) / 3) / FRAME).max(1);
            let cut = (0..look.min(self.recent.len()))
                .map(|k| (self.seen - (k + 1) * FRAME + FRAME / 2, self.recent[self.recent.len() - 1 - k]))
                .filter(|(c, _)| *c > start)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(self.seen, |(c, _)| c);
            out.push(start..cut);
            self.done = cut;
            self.open = (self.last_voice > cut).then_some(cut);
        }
    }
    pub fn open_start(&self) -> Option<usize> {
        self.open
    }
    pub fn finish(&mut self, all: &[f32]) -> Vec<std::ops::Range<usize>> {
        let mut out = self.feed(all);
        out.extend(self.close());
        out
    }
    /// The take ended: what is still open is a piece too.
    pub fn close(&mut self) -> Vec<std::ops::Range<usize>> {
        let end = self.seen + self.pending.len();
        let out = self.open.take().map(|s| s..end).into_iter().collect();
        self.done = end;
        out
    }
}

/// How long `x` is voice, frame by frame, against a room at `floor`.
pub fn voiced_ms(x: &[f32], floor: f32) -> u64 {
    let on = (floor * ON).max(MIN_VOICE);
    let n = x.chunks_exact(FRAME).filter(|f| rms(f) > on).count();
    (n * FRAME) as u64 * 1000 / RATE as u64
}

/// A piece whose peak is below this is raised before decoding: to `PEAK`, by at most 30 dB.
const LOW_PEAK: f32 = 0.25;
const PEAK: f32 = 0.5;
const MAX_GAIN: f32 = 31.6;

/// `x` raised so its peak reaches `PEAK`: when its peak is low, or `always`.
pub fn louder(x: &[f32], always: bool) -> Vec<f32> {
    let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
    let g = if peak > 0.0 && (always || peak < LOW_PEAK) { (PEAK / peak).min(MAX_GAIN) } else { 1.0 };
    if g <= 1.0 {
        return x.to_vec();
    }
    x.iter().map(|v| v * g).collect()
}

#[derive(Default)]
pub struct Take {
    pub samples: Mutex<Vec<f32>>,
    pub level: AtomicU32,
    pub open: AtomicBool,
    pub error: Mutex<Option<String>>,
    /// Bytes copied out of the take so far, so the cost of reading a long one stays in view.
    pub copied: AtomicU64,
}

impl Take {
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }
    pub fn len(&self) -> usize {
        self.samples.lock().unwrap().len()
    }
    pub fn copy(&self, r: std::ops::Range<usize>) -> Vec<f32> {
        let s = self.samples.lock().unwrap();
        let v = s[r.start.min(s.len())..r.end.min(s.len())].to_vec();
        self.copied.fetch_add(v.len() as u64 * 4, Ordering::Relaxed);
        v
    }
    pub fn all(&self) -> Vec<f32> {
        let v = self.samples.lock().unwrap().clone();
        self.copied.fetch_add(v.len() as u64 * 4, Ordering::Relaxed);
        v
    }
    /// The samples from `pos` on: what arrived since a reader last looked.
    pub fn read_from(&self, pos: usize) -> Vec<f32> {
        let s = self.samples.lock().unwrap();
        let v = s[pos.min(s.len())..].to_vec();
        self.copied.fetch_add(v.len() as u64 * 4, Ordering::Relaxed);
        v
    }
}

/// cpal keeps one Windows device enumerator for the whole process, in the multithreaded COM
/// apartment, and Windows tears that apartment down when the last thread in it ends. A listing from
/// a thread that starts later then calls into a dead object and the process crashes. Keep the
/// apartment for the life of the app.
fn keep_com_apartment() {
    #[cfg(windows)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            let _ = unsafe { windows::Win32::System::Com::CoIncrementMTAUsage() };
        });
    }
}

pub fn devices() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    keep_com_apartment();
    let host = cpal::default_host();
    let def = host.default_input_device().and_then(|d| d.name().ok());
    let mut v: Vec<String> = host
        .input_devices()
        .map(|it| it.filter_map(|d| d.name().ok()).collect())
        .unwrap_or_default();
    if let Some(d) = def {
        v.retain(|x| *x != d);
        v.insert(0, d);
    }
    v
}

pub struct Mic {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Mic {
    pub fn open(device: Option<String>, take: Arc<Take>) -> Result<Mic, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<Result<(), String>>();
        let s2 = stop.clone();
        let thread = std::thread::Builder::new()
            .name("pointcast-mic".into())
            .spawn(move || match build(device, take.clone()) {
                Ok(stream) => {
                    take.open.store(true, Ordering::SeqCst);
                    let _ = tx.send(Ok(()));
                    while !s2.load(Ordering::SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    drop(stream);
                    take.open.store(false, Ordering::SeqCst);
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        match rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Mic {
                stop,
                thread: Some(thread),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err("The microphone did not open".into())
            }
        }
    }

    /// A 16-bit PCM WAV played into `take` at its own pace, as if spoken.
    pub fn from_wav(path: &std::path::Path, take: Arc<Take>) -> Result<Mic, String> {
        let (rate, channels, pcm) = read_wav(&std::fs::read(path).map_err(|e| e.to_string())?)?;
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = stop.clone();
        take.open.store(true, Ordering::SeqCst);
        let thread = std::thread::spawn(move || {
            let mut rs = Resampler::new(rate);
            let step = (rate as usize / 50) * channels.max(1);
            for chunk in pcm.chunks(step.max(1)) {
                if s2.load(Ordering::SeqCst) {
                    break;
                }
                let mut mono = vec![];
                downmix(chunk, channels, &mut mono);
                let mut out = vec![];
                rs.push(&mono, &mut out);
                take.level
                    .store(meter(rms(&out)).to_bits(), Ordering::Relaxed);
                take.samples.lock().unwrap().extend_from_slice(&out);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            while !s2.load(Ordering::SeqCst) {
                take.samples
                    .lock()
                    .unwrap()
                    .extend(std::iter::repeat(0.0).take(320));
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            take.open.store(false, Ordering::SeqCst);
        });
        Ok(Mic {
            stop,
            thread: Some(thread),
        })
    }

    fn close(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Mic {
    fn drop(&mut self) {
        self.close();
    }
}

pub fn read_wav(b: &[u8]) -> Result<(u32, usize, Vec<f32>), String> {
    if b.len() < 12 || &b[0..4] != b"RIFF" || &b[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let (mut i, mut fmt) = (12usize, None);
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        let body = &b[i + 8..(i + 8 + n).min(b.len())];
        if id == b"fmt " && body.len() >= 16 {
            let ch = u16::from_le_bytes([body[2], body[3]]) as usize;
            let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if bits != 16 {
                return Err("16-bit WAV only".into());
            }
            fmt = Some((rate, ch));
        } else if id == b"data" {
            let (rate, ch) = fmt.ok_or("no fmt chunk")?;
            let pcm = body
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0)
                .collect();
            return Ok((rate, ch, pcm));
        }
        i += 8 + n + (n & 1);
    }
    Err("no data chunk".into())
}

fn build(device: Option<String>, take: Arc<Take>) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    keep_com_apartment();
    let host = cpal::default_host();
    let dev = match device {
        Some(name) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().ok().as_deref() == Some(name.as_str()))
            .or_else(|| host.default_input_device()),
        None => host.default_input_device(),
    }
    .ok_or("No microphone")?;
    let cfg = dev.default_input_config().map_err(|e| e.to_string())?;
    let channels = cfg.channels() as usize;
    let rate = cfg.sample_rate().0;
    let fmt = cfg.sample_format();
    let config: cpal::StreamConfig = cfg.into();
    let rs = Mutex::new(Resampler::new(rate));
    let t2 = take.clone();
    let on_err = move || {
        let t3 = t2.clone();
        move |e: cpal::StreamError| {
            *t3.error.lock().unwrap() = Some(e.to_string());
        }
    };
    let sink = move |data: &[f32]| {
        let mut mono = Vec::with_capacity(data.len() / channels.max(1));
        downmix(data, channels, &mut mono);
        let mut out = Vec::with_capacity(mono.len() / 2);
        rs.lock().unwrap().push(&mono, &mut out);
        take.level
            .store(meter(rms(&out)).to_bits(), Ordering::Relaxed);
        take.samples.lock().unwrap().extend_from_slice(&out);
    };
    let stream = match fmt {
        cpal::SampleFormat::F32 => {
            dev.build_input_stream(&config, move |d: &[f32], _: &_| sink(d), on_err(), None)
        }
        cpal::SampleFormat::I16 => dev.build_input_stream(
            &config,
            move |d: &[i16], _: &_| {
                let f: Vec<f32> = d.iter().map(|x| *x as f32 / 32768.0).collect();
                sink(&f)
            },
            on_err(),
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_input_stream(
            &config,
            move |d: &[u16], _: &_| {
                let f: Vec<f32> = d.iter().map(|x| (*x as f32 / 32768.0) - 1.0).collect();
                sink(&f)
            },
            on_err(),
            None,
        ),
        other => return Err(format!("{other:?} microphones are not supported")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

#[cfg(test)]
mod tests {
    #[test]
    fn devices_can_be_listed_from_one_thread_after_another() {
        // Each listing on a fresh thread that starts after the last one ended, as Settings and the
        // setup assistant do from the async runtime.
        for _ in 0..3 {
            std::thread::spawn(|| super::devices()).join().unwrap();
        }
    }

    use super::*;

    fn sine(rate: u32, hz: f32, secs: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| amp * (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin())
            .collect()
    }
    fn crossings(x: &[f32]) -> usize {
        x.windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count()
    }

    #[test]
    fn resampling_keeps_length_and_pitch_from_any_rate() {
        for rate in [48_000u32, 44_100, 16_000, 8_000] {
            let x = sine(rate, 440.0, 1.0, 0.5);
            let mut r = Resampler::new(rate);
            let mut out = vec![];
            for c in x.chunks(317) {
                r.push(c, &mut out);
            }
            assert!(
                (out.len() as i64 - 16_000).abs() <= 2,
                "{rate}: {}",
                out.len()
            );
            let z = crossings(&out) as i64;
            assert!((z - 880).abs() <= 4, "{rate}: {z}");
        }
        let mut m = vec![];
        downmix(&[1.0, 0.0, 0.5, 0.5], 2, &mut m);
        assert_eq!(m, [0.5, 0.5]);
    }

    #[test]
    fn the_meter_is_empty_in_silence_and_moves_with_speech_level() {
        assert_eq!(meter(rms(&[0.0; 480])), 0.0);
        let quiet = meter(rms(&sine(16_000, 200.0, 0.03, 0.01)));
        let loud = meter(rms(&sine(16_000, 200.0, 0.03, 0.3)));
        assert!(quiet > 0.0 && loud > quiet && loud <= 1.0, "{quiet} {loud}");
    }

    fn take(parts: &[(f32, f32)]) -> Vec<f32> {
        let mut v = vec![0.0; 8_000];
        for (on, off) in parts {
            v.extend(sine(16_000, 300.0, *on, 0.2));
            v.extend(std::iter::repeat(0.0005).take((*off * 16_000.0) as usize));
        }
        v
    }

    #[test]
    fn a_take_is_cut_at_its_pauses_and_never_longer_than_the_cap() {
        let x = take(&[(1.0, 1.0), (2.0, 0.2), (1.0, 1.0)]);
        let mut s = Segmenter::new(700, 12_000);
        let mut got = vec![];
        let mut n = 0;
        while n < x.len() {
            n = (n + 1_600).min(x.len());
            got.extend(s.feed(&x[..n]));
        }
        got.extend(s.finish(&x));
        assert_eq!(got.len(), 2, "{got:?}: the 200 ms pause does not cut");
        assert!(got[0].start < 8_000 && got[0].end > 24_000, "{:?}", got[0]);
        assert!(got.windows(2).all(|w| w[0].end <= w[1].start));
        let mut s = Segmenter::new(700, 12_000);
        let q = vec![0.0005; 48_000];
        assert!(s.feed(&q).is_empty() && s.finish(&q).is_empty());
        let mut s = Segmenter::new(700, 12_000);
        let early = take(&[(1.0, 1.0), (1.5, 0.0)]);
        let got = s.finish(&early);
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[1].end == early.len());
        let mut s = Segmenter::new(700, 3_000);
        let long = take(&[(7.0, 0.0)]);
        let pieces = s.feed(&long);
        assert!(pieces.len() >= 2, "{pieces:?}");
        assert!(
            pieces.iter().all(|r| r.len() <= 3 * 16_000 + FRAME + 4_800),
            "{pieces:?}"
        );
    }

    /// `n` words of `on` s each, `off` s apart, at amplitude `amp`, over a room at rms `room` (white noise), after 2 s of
    /// room alone. Returns the take and where each word is.
    fn spoken(n: usize, on: f32, off: f32, amp: f32, room: f32) -> (Vec<f32>, Vec<std::ops::Range<usize>>) {
        let sec = |s: f32| (s * 16_000.0) as usize;
        let mut v = vec![0.0; sec(2.0)];
        let mut words = vec![];
        for _ in 0..n {
            let w = sine(16_000, 220.0, on, amp);
            words.push(v.len()..v.len() + w.len());
            v.extend(w);
            v.extend(std::iter::repeat(0.0).take(sec(off)));
        }
        v.extend(std::iter::repeat(0.0).take(sec(2.0)));
        let mut seed = 7u32;
        for x in v.iter_mut() {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            *x += (seed as f32 / u32::MAX as f32 * 2.0 - 1.0) * room * 3f32.sqrt();
        }
        (v, words)
    }

    fn cut(x: &[f32], max_ms: u32) -> Vec<std::ops::Range<usize>> {
        let mut s = Segmenter::new(700, max_ms);
        let mut got = vec![];
        for piece in x.chunks(1_600) {
            got.extend(s.feed_new(piece));
        }
        got.extend(s.close());
        got
    }

    /// Every sample of every word is in exactly one piece, and the pieces do not overlap.
    fn each_word_once(got: &[std::ops::Range<usize>], words: &[std::ops::Range<usize>]) {
        assert!(got.windows(2).all(|w| w[0].end <= w[1].start), "{got:?}");
        for w in words {
            for i in [w.start, w.start + w.len() / 2, w.end - 1] {
                assert_eq!(got.iter().filter(|r| r.contains(&i)).count(), 1, "sample {i} of word {w:?} in {got:?}");
            }
        }
    }

    #[test]
    fn a_quiet_microphone_or_a_noisy_room_loses_no_words() {
        // Speech at 6% of full level (about -42 dBFS) in a quiet room.
        let (x, words) = spoken(6, 0.6, 1.2, 0.012, 0.0003);
        let got = cut(&x, 25_000);
        assert_eq!(got.len(), 6, "{got:?}");
        each_word_once(&got, &words);
        // Speech over a noisy room (rms 0.01, about -40 dBFS).
        let (x, words) = spoken(6, 0.6, 1.2, 0.1, 0.01);
        let got = cut(&x, 25_000);
        assert_eq!(got.len(), 6, "{got:?}");
        each_word_once(&got, &words);
        // The room alone, however loud, is no speech.
        for room in [0.0003, 0.01, 0.05] {
            let (x, _) = spoken(0, 0.0, 0.0, 0.0, room);
            assert!(cut(&[x.clone(), x.clone(), x].concat(), 25_000).is_empty(), "{room}");
        }
        // A piece carries 400 ms before its first word and after its last.
        let (x, words) = spoken(1, 1.0, 0.0, 0.2, 0.0003);
        let got = cut(&x, 25_000);
        assert!(got[0].start + PAD <= words[0].start + FRAME && got[0].end >= words[0].end + PAD - FRAME, "{got:?} {words:?}");
    }

    #[test]
    fn long_speech_is_cut_at_its_quiet_moments_and_never_past_the_cap() {
        // A minute of words with only 120 ms between them: no pause long enough to end a piece.
        let (x, words) = spoken(140, 0.3, 0.12, 0.2, 0.0005);
        let got = cut(&x, 25_000);
        assert!(got.len() >= 3, "{got:?}");
        assert!(got.iter().all(|r| r.len() <= 25 * 16_000 + FRAME), "{got:?}");
        each_word_once(&got, &words);
        // Each cut falls between words, never inside one.
        for r in &got[..got.len() - 1] {
            assert!(!words.iter().any(|w| w.start < r.end && r.end < w.end), "cut at {} inside a word", r.end);
        }
        let short = cut(&x, 3_000);
        assert!(short.iter().all(|r| r.len() <= 3 * 16_000 + FRAME), "{short:?}");
        each_word_once(&short, &words);
    }

    #[test]
    fn voice_holds_through_a_softer_moment_but_a_soft_sound_alone_never_starts_it() {
        let sec = |s: f32| (s * 16_000.0) as usize;
        // A word, then 1.2 s at a level between the hold and start thresholds over a room at about 0.002.
        let soft = 0.002 * 2.1 * 2f32.sqrt();
        let mut x = vec![0.002 * 2f32.sqrt(); sec(3.0)].iter().enumerate().map(|(i, a)| a * (i as f32 * 0.9).sin()).collect::<Vec<_>>();
        let word = x.len();
        x.extend(sine(16_000, 220.0, 0.5, 0.2));
        let tail = x.len();
        x.extend(sine(16_000, 220.0, 1.2, soft));
        x.extend(std::iter::repeat(0.0).take(sec(2.0)));
        let got = cut(&x, 25_000);
        assert_eq!(got.len(), 1, "{got:?}");
        assert!(got[0].start < word && got[0].end >= tail + sec(1.2), "the softer part stays in: {got:?}");
        // The same soft sound with no word before it is not voice.
        let mut alone = x[..word].to_vec();
        alone.extend(sine(16_000, 220.0, 1.2, soft));
        alone.extend(std::iter::repeat(0.0).take(sec(2.0)));
        assert!(cut(&alone, 25_000).is_empty());
    }

    #[test]
    fn a_quiet_piece_is_raised_before_decoding_by_at_most_30_db() {
        let peak = |x: &[f32]| x.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak(&louder(&sine(16_000, 220.0, 0.1, 0.05), false)) - 0.5).abs() < 0.01);
        assert!((peak(&louder(&sine(16_000, 220.0, 0.1, 0.001), false)) - 0.0316).abs() < 0.001, "30 dB at most");
        assert_eq!(louder(&[0.4, -0.3], false), vec![0.4, -0.3], "a loud piece is left alone");
        assert!((peak(&louder(&[0.4, -0.3], true)) - 0.5).abs() < 1e-6);
        assert_eq!(louder(&[0.0; 4], true), vec![0.0; 4]);
        assert_eq!(voiced_ms(&sine(16_000, 220.0, 1.5, 0.2), 0.0), 1_500);
        assert_eq!(voiced_ms(&vec![0.001; 16_000], 0.0), 0);
    }

    #[test]
    fn new_samples_in_any_size_of_piece_are_cut_as_the_whole_take_is() {
        let x = take(&[(1.0, 1.0), (2.0, 0.2), (1.0, 1.0), (0.5, 0.0)]);
        let mut whole = Segmenter::new(700, 12_000);
        let want = whole.finish(&x);
        for size in [1usize, 37, 479, 480, 481, 1_600, 50_000] {
            let mut s = Segmenter::new(700, 12_000);
            let mut got = vec![];
            for piece in x.chunks(size) {
                got.extend(s.feed_new(piece));
            }
            got.extend(s.close());
            assert_eq!(got, want, "pieces of {size}");
        }
    }
}
