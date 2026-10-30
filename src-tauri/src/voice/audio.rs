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
    /// The earliest a piece still to come can start: what is before it is never cut again.
    pub fn keep_from(&self) -> usize {
        self.open.unwrap_or(self.seen.saturating_sub(PAD).max(self.done))
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

/// Below this rms (-80 dBFS) a frame has essentially no signal; voice below `QUIET` rms (-40 dBFS) is very low.
const SILENT: f32 = 0.0001;
const QUIET: f32 = 0.01;

/// What the HUD says about the microphone, from the last frames' rms (oldest first) against a room at `floor`:
/// "silent" when the last 5 s had essentially no signal, "quiet" when there was voice (half a second or more) but very
/// low, else nothing.
pub fn mic_note(recent: &[f32], floor: f32) -> Option<&'static str> {
    let five = RATE as usize * 5 / FRAME;
    if recent.len() >= five && recent[recent.len() - five..].iter().all(|r| *r < SILENT) {
        return Some("silent");
    }
    let on = (floor * ON).max(MIN_VOICE);
    let voice: Vec<f32> = recent.iter().copied().filter(|r| *r > on).collect();
    (voice.len() >= RATE as usize / 2 / FRAME && rms(&voice) < QUIET).then_some("quiet")
}

/// How long `x` is voice, frame by frame, against a room at `floor`.
pub fn voiced_ms(x: &[f32], floor: f32) -> u64 {
    let on = (floor * ON).max(MIN_VOICE);
    let n = x.chunks_exact(FRAME).filter(|f| rms(f) > on).count();
    (n * FRAME) as u64 * 1000 / RATE as u64
}

/// A second-order Butterworth high-pass at 16 kHz (the RBJ cookbook's), run sample by sample.
#[derive(Debug, Clone)]
struct HighPass {
    b: [f64; 3],
    a: [f64; 2],
    x: [f64; 2],
    y: [f64; 2],
}

impl HighPass {
    fn new(hz: f32) -> Self {
        let (s, c) = (2.0 * std::f64::consts::PI * hz as f64 / RATE as f64).sin_cos();
        let a0 = 1.0 + s / 2f64.sqrt();
        HighPass {
            b: [(1.0 + c) / 2.0 / a0, -(1.0 + c) / a0, (1.0 + c) / 2.0 / a0],
            a: [-2.0 * c / a0, (2.0 - a0) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn run(&mut self, v: f32) -> f32 {
        let v = v as f64;
        let y = self.b[0] * v + self.b[1] * self.x[0] + self.b[2] * self.x[1] - self.a[0] * self.y[0] - self.a[1] * self.y[1];
        (self.x, self.y) = ([v, self.x[0]], [y, self.y[0]]);
        y as f32
    }
}

/// The front end works in frames of 10 ms.
const BOOST_FRAME: usize = RATE as usize / 100;
/// Taps of the low-pass that takes 16 kHz audio to 48 kHz and back: a sinc cut at 7.6 kHz under a Blackman window.
const TAPS: usize = 144;
/// The share of the input kept under the denoised sound: noise is lowered by at most 12 dB, so the model hears a
/// cleaner voice without the denoiser's artefacts.
const NOISE_FLOOR: f32 = 0.25;

fn low_pass() -> [f32; TAPS] {
    let (pi, fc, m) = (std::f64::consts::PI, 7_600.0 / 48_000.0, (TAPS - 1) as f64 / 2.0);
    let mut h = [0f32; TAPS];
    for (i, t) in h.iter_mut().enumerate() {
        let n = i as f64 - m;
        let sinc = if n == 0.0 { 2.0 * fc } else { (2.0 * pi * fc * n).sin() / (pi * n) };
        let w = 2.0 * pi * i as f64 / (TAPS - 1) as f64;
        *t = (sinc * (0.42 - 0.5 * w.cos() + 0.08 * (2.0 * w).cos())) as f32;
    }
    let sum: f32 = h.iter().sum();
    h.map(|t| t / sum)
}

/// RNNoise (the nnnoiseless port) on 16 kHz audio: each frame goes up to the 48 kHz it was trained on, is denoised,
/// mixed with `NOISE_FLOOR` of itself and comes back down.
struct Denoiser {
    /// The low-pass, and its three phases for the way up, each turned to run oldest sample first.
    h: [f32; TAPS],
    phases: [[f32; TAPS / 3]; 3],
    /// The last inputs at 16 kHz that the way up still reads, then this frame's.
    up: Vec<f32>,
    /// The last samples at 48 kHz that the way down still reads, then this frame's.
    down: Vec<f32>,
    rnn: Box<nnnoiseless::DenoiseState<'static>>,
    /// This frame and the last at 48 kHz (RNNoise gives back the frame before the one it is given).
    wide: Vec<f32>,
    last: Vec<f32>,
    /// RNNoise's first frame only fades in: it is given back silent.
    started: bool,
}

impl Default for Denoiser {
    fn default() -> Self {
        let h = low_pass();
        let k = TAPS / 3;
        Denoiser {
            h,
            phases: std::array::from_fn(|p| std::array::from_fn(|m| h[3 * (k - 1 - m) + p])),
            up: vec![0.0; k - 1 + BOOST_FRAME],
            down: vec![0.0; TAPS - 3 + BOOST_FRAME * 3],
            rnn: nnnoiseless::DenoiseState::new(),
            wide: vec![0.0; BOOST_FRAME * 3],
            last: vec![0.0; BOOST_FRAME * 3],
            started: false,
        }
    }
}

impl Denoiser {
    /// One frame of `BOOST_FRAME` samples, denoised in place (a frame late). Returns how sure RNNoise is that it is voice.
    fn frame(&mut self, x: &mut [f32]) -> f32 {
        const K: usize = TAPS / 3;
        // RNNoise reads 16-bit sample values.
        const SCALE: f32 = 32_768.0;
        // Plain loops over indices: they stay quick in a debug build too.
        self.up[K - 1..].copy_from_slice(x);
        for i in 0..BOOST_FRAME {
            for p in 0..3 {
                let mut s = 0.0;
                for m in 0..K {
                    s += self.phases[p][m] * self.up[i + m];
                }
                self.wide[3 * i + p] = s * 3.0 * SCALE;
            }
        }
        self.up.copy_within(BOOST_FRAME.., 0);
        let fresh = &mut self.down[TAPS - 3..];
        let voice = self.rnn.process_frame(fresh, &self.wide);
        for (o, w) in fresh.iter_mut().zip(&self.last) {
            *o = (1.0 - NOISE_FLOOR) * *o + NOISE_FLOOR * w;
        }
        std::mem::swap(&mut self.last, &mut self.wide);
        for (i, v) in x.iter_mut().enumerate() {
            let mut s = 0.0;
            for m in 0..TAPS {
                s += self.h[m] * self.down[3 * i + m];
            }
            *v = s / SCALE;
        }
        self.down.copy_within(BOOST_FRAME * 3.., 0);
        if !std::mem::replace(&mut self.started, true) {
            x.fill(0.0);
        }
        voice
    }
}

/// The level of speech's louder moments is brought towards this rms (-15 dBFS, which puts speech as a whole at about
/// -20 dBFS), by at most `MAX_BOOST` (+36 dB), and never turned down.
const TARGET: f32 = 0.17;
const MAX_BOOST: f32 = 63.1;

/// Of a 10 ms frame's level, the share that moves a level tracked with time constant `ms`.
fn follow(ms: f32) -> f32 {
    1.0 - (-10.0 / ms).exp()
}

/// A slow automatic gain that moves only while someone speaks: the speech level follows a louder frame in about
/// 100 ms (attack) and a softer one in about 1.5 s (release); in pauses and over noise alone the gain stays where it
/// was. Speech is 30 ms or more of frames RNNoise takes for voice whose level (smoothed over about 30 ms) is 6 dB over the
/// room's; the room's level is where the smoothed level falls back to (in about 200 ms), creeping up over about 4 s.
/// Frames of digital silence are not heard at all.
#[derive(Debug, Clone)]
struct Agc {
    /// Powers: the smoothed frame level and the room's, once a frame with sound has been heard; the speech level, once
    /// speech has been heard.
    smooth: f32,
    room: Option<f32>,
    speech: Option<f32>,
    /// Frames in a row that sound like speech.
    run: u32,
    gain: f32,
}

impl Default for Agc {
    fn default() -> Self {
        Agc { smooth: 0.0, room: None, speech: None, run: 0, gain: 1.0 }
    }
}

impl Agc {
    /// One frame, in place; `voice` is how sure the denoiser is that it is voice.
    fn frame(&mut self, x: &mut [f32], voice: f32) {
        let p = x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32;
        let from = self.gain;
        if p > 1e-14 {
            let room = match self.room {
                None => {
                    self.smooth = p;
                    p
                }
                Some(r) => r,
            };
            self.smooth += (p - self.smooth) * follow(30.0);
            let s = self.smooth;
            self.room = Some(room + (s - room) * if s < room { follow(200.0) } else { follow(4_000.0) });
            self.run = if voice >= 0.5 && s > room * 4.0 { self.run + 1 } else { 0 };
            if self.run >= 3 {
                let l = self.speech.map_or(p, |l| l + (p - l) * if p > l { follow(100.0) } else { follow(1_500.0) });
                self.speech = Some(l);
                self.gain = (TARGET / l.sqrt()).clamp(1.0, MAX_BOOST);
            }
        }
        let n = x.len() as f32;
        for (i, v) in x.iter_mut().enumerate() {
            *v = limit(*v * (from + (self.gain - from) * (i + 1) as f32 / n));
        }
    }
}

/// A soft limiter: untouched up to half of full scale, then bent smoothly towards full scale.
fn limit(v: f32) -> f32 {
    if v.abs() <= 0.5 {
        v
    } else {
        v.signum() * (0.5 + 0.5 * ((v.abs() - 0.5) * 2.0).tanh())
    }
}

/// What the voice detector and the model hear instead of the raw microphone, so quieter and farther speech comes through:
/// an 80 Hz high-pass, RNNoise and the speech-gated gain. Streaming and cheap: each call gives back as many samples as it
/// is given, a frame and the filters' delay (about 23 ms) late, the same in any size of piece.
pub struct Boost {
    high: HighPass,
    noise: Denoiser,
    agc: Agc,
    pending: Vec<f32>,
    ready: std::collections::VecDeque<f32>,
    /// The power of the frame coming in, as the microphone gave it, and how many samples in a row up to the last whole
    /// frame had essentially no signal.
    power: f32,
    hush: usize,
}

impl Default for Boost {
    fn default() -> Self {
        Boost {
            high: HighPass::new(80.0),
            noise: Denoiser::default(),
            agc: Agc::default(),
            pending: Vec::with_capacity(BOOST_FRAME),
            ready: std::iter::repeat(0.0).take(BOOST_FRAME).collect(),
            power: 0.0,
            hush: 0,
        }
    }
}

impl Boost {
    /// `x` through the front end, in place.
    pub fn process(&mut self, x: &mut [f32]) {
        for v in x.iter_mut() {
            self.power += *v * *v;
            self.pending.push(self.high.run(*v));
            if self.pending.len() == BOOST_FRAME {
                let still = self.power / (BOOST_FRAME as f32) < SILENT * SILENT;
                (self.hush, self.power) = (if still { self.hush + BOOST_FRAME } else { 0 }, 0.0);
                let voice = self.noise.frame(&mut self.pending);
                self.agc.frame(&mut self.pending, voice);
                self.ready.extend(self.pending.drain(..));
            }
            *v = self.ready.pop_front().unwrap_or(0.0);
        }
    }
    /// The microphone itself gave essentially nothing for the last 5 s, whatever the gain makes of it.
    pub fn silent(&self) -> bool {
        self.hush >= RATE as usize * 5
    }
}

/// `mic_note` for a take heard through the front end: whether the microphone is silent is judged on what it gives
/// (`silent`), how quiet the voice is on what the voice detector and the model hear, after the gain (`recent`, `floor`).
pub fn boosted_note(recent: &[f32], floor: f32, silent: bool) -> Option<&'static str> {
    if silent {
        return Some("silent");
    }
    mic_note(recent, floor).filter(|n| *n != "silent")
}

/// The newest part of a stream, by position in it: the audio the pieces still to come are cut from.
#[derive(Debug, Default)]
pub struct Tail {
    from: usize,
    samples: std::collections::VecDeque<f32>,
}

impl Tail {
    pub fn push(&mut self, x: &[f32]) {
        self.samples.extend(x);
    }
    /// The samples of `r` still held.
    pub fn copy(&self, r: std::ops::Range<usize>) -> Vec<f32> {
        let end = self.from + self.samples.len();
        let (a, b) = (r.start.clamp(self.from, end) - self.from, r.end.clamp(self.from, end) - self.from);
        self.samples.range(a..b.max(a)).copied().collect()
    }
    /// Let go of everything before `at`.
    pub fn drop_before(&mut self, at: usize) {
        let n = at.saturating_sub(self.from).min(self.samples.len());
        self.samples.drain(..n);
        self.from += n;
    }
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
    if !crate::permissions::may_list_microphones() {
        return vec![];
    }
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
pub(crate) mod tests {
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
    fn the_hud_hears_a_silent_or_very_quiet_microphone_and_nothing_else() {
        let frames = |x: &[f32]| {
            let mut s = Segmenter::new(700, 25_000);
            s.feed_new(x);
            (s.recent(), s.floor())
        };
        let note = |x: &[f32]| {
            let (r, f) = frames(x);
            mic_note(&r, f)
        };
        assert_eq!(note(&vec![0.0; 16_000 * 6]), Some("silent"));
        assert_eq!(note(&vec![0.0; 16_000 * 4]), None, "not five seconds yet");
        // Words at about -44 dBFS: quiet. At -20 dBFS, or in a quiet room with no words: nothing to say.
        assert_eq!(note(&spoken(4, 0.6, 0.6, 0.009, 0.0003).0), Some("quiet"));
        assert_eq!(note(&spoken(4, 0.6, 0.6, 0.14, 0.0003).0), None);
        assert_eq!(note(&spoken(0, 0.0, 0.0, 0.0, 0.0003).0), None);
        // Words, then five seconds of nothing: the microphone went silent.
        let mut x = spoken(4, 0.6, 0.6, 0.14, 0.0003).0;
        x.extend(vec![0.0; 16_000 * 5 + FRAME]);
        assert_eq!(note(&x), Some("silent"));
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

    /// Noise at rms `r` for `secs`: white, or with `hum` half white and half mains hum (60 and 120 Hz) over rumble.
    pub(crate) fn noise(secs: f32, r: f32, seed: u32, hum: bool) -> Vec<f32> {
        let mut s = seed;
        let mut white = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s as f32 / u32::MAX as f32 * 2.0 - 1.0) * 3f32.sqrt()
        };
        let n = (RATE as f32 * secs) as usize;
        let w: Vec<f32> = (0..n).map(|_| white()).collect();
        if !hum {
            return w.iter().map(|v| v * r).collect();
        }
        let (mut lp, tau) = (0.0f32, 2.0 * std::f32::consts::PI / RATE as f32);
        let h: Vec<f32> = (0..n)
            .map(|i| {
                lp += (white() - lp) * 0.057;
                3.0 * lp + 0.5 * (tau * 60.0 * i as f32).sin() + 0.25 * (tau * 120.0 * i as f32).sin()
            })
            .collect();
        let k = 1.0 / rms(&h);
        w.iter().zip(&h).map(|(a, b)| (a + b * k) * r / 2f32.sqrt()).collect()
    }

    /// A voice-like sound at rms `amp`: the first 20 harmonics of a pitch gliding around 140 Hz, softer as they go up,
    /// opened and closed four times a second as syllables are.
    pub(crate) fn voice_like(secs: f32, amp: f32) -> Vec<f32> {
        let tau = 2.0 * std::f32::consts::PI;
        let mut phase = 0.0f32;
        let v: Vec<f32> = (0..(RATE as f32 * secs) as usize)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                phase += tau * (140.0 + 20.0 * (tau * 0.7 * t).sin()) / RATE as f32;
                let s: f32 = (1..=20).map(|k| (k as f32 * phase).sin() / k as f32).sum();
                s * (0.5 - 0.5 * (tau * 4.0 * t).cos()).sqrt()
            })
            .collect();
        let k = amp / rms(&v);
        v.iter().map(|x| x * k).collect()
    }

    fn db(a: &[f32], b: &[f32]) -> f32 {
        20.0 * (rms(a) / rms(b)).log10()
    }

    #[test]
    fn the_high_pass_takes_out_offset_and_rumble_and_keeps_the_voice_band() {
        let through = |x: &[f32]| {
            let mut h = HighPass::new(80.0);
            x.iter().map(|v| h.run(*v)).collect::<Vec<f32>>()
        };
        // Measured after the first half second, once the filter has settled.
        let settled = |x: &[f32]| x[8_000..].to_vec();
        assert!(rms(&settled(&through(&[0.1; 32_000]))) < 1e-4, "an offset is taken out");
        for (hz, most) in [(20.0, -20.0), (30.0, -15.0)] {
            let x = sine(16_000, hz, 2.0, 0.1);
            let d = db(&settled(&through(&x)), &settled(&x));
            assert!(d < most, "{hz} Hz: {d:.1} dB");
        }
        for hz in [300.0, 1_000.0, 3_000.0] {
            let x = sine(16_000, hz, 2.0, 0.1);
            let d = db(&settled(&through(&x)), &settled(&x));
            assert!(d.abs() < 0.5, "{hz} Hz: {d:.1} dB");
        }
    }

    /// `x` through the denoiser alone, frame by frame.
    fn denoised(x: &[f32]) -> Vec<f32> {
        let mut d = Denoiser::default();
        let mut y = x.to_vec();
        y.chunks_exact_mut(BOOST_FRAME).for_each(|f| {
            d.frame(f);
        });
        y
    }

    #[test]
    fn the_denoiser_lowers_noise_alone_by_10_db_and_leaves_a_clean_tone_within_1_5_db() {
        // Measured over the last 2 of 4 s.
        let last = |x: &[f32]| x[32_000..].to_vec();
        for r in [0.002, 0.01, 0.05] {
            let x = noise(4.0, r, 7, false);
            let d = db(&last(&denoised(&x)), &last(&x));
            assert!(d < -10.0, "noise at {r}: {d:.1} dB");
        }
        for (hz, amp) in [(300.0, 0.1), (1_000.0, 0.1), (1_000.0, 0.01)] {
            let x = sine(16_000, hz, 4.0, amp);
            let d = db(&last(&denoised(&x)), &last(&x));
            assert!(d.abs() < 1.5, "{hz} Hz at {amp}: {d:.1} dB");
        }
    }

    /// `x` through `agc` frame by frame, every frame taken for voice; and the gain after each frame.
    fn gained(x: &[f32], agc: &mut Agc) -> (Vec<f32>, Vec<f32>) {
        let mut y = x.to_vec();
        let g = y
            .chunks_exact_mut(BOOST_FRAME)
            .map(|f| {
                agc.frame(f, 1.0);
                agc.gain
            })
            .collect();
        (y, g)
    }

    #[test]
    fn the_gain_brings_a_quiet_sound_to_its_target_within_its_attack_and_never_past_its_cap() {
        let onset = 8_000;
        // A steady tone at rms 0.003 (-50 dBFS) reaches the target; one at 0.0001 (-80 dBFS) gets the most the cap
        // allows. Each comes after half a second of a quiet room.
        for (r, want) in [(0.003f32, TARGET), (0.0001, 0.0001 * MAX_BOOST)] {
            let mut x = noise(0.5, 0.00002, 3, false);
            x.extend(sine(16_000, 1_000.0, 2.0, r * 2f32.sqrt()));
            let (y, gains) = gained(&x, &mut Agc::default());
            // From 100 ms after the tone starts, every frame.
            for f in y[onset + 1_600..].chunks(BOOST_FRAME) {
                let d = 20.0 * (rms(f) / want).log10();
                assert!(d.abs() < 0.5, "tone at {r}: {d:.2} dB from {want}");
            }
            assert!(gains.iter().all(|g| (1.0..=MAX_BOOST).contains(g)), "tone at {r}: {gains:?}");
        }
    }

    #[test]
    fn the_gain_never_raises_noise_alone_holds_still_in_pauses_and_never_turns_a_voice_down() {
        // A room with nothing but noise, white or with hum and rumble, even when every frame is taken for voice.
        for hum in [false, true] {
            for r in [0.0005, 0.01, 0.05] {
                let x = noise(4.0, r, 11, hum);
                let (y, gains) = gained(&x, &mut Agc::default());
                assert!(gains.iter().all(|g| *g == 1.0) && y == x, "noise at {r}, hum {hum}");
            }
        }
        // A quiet word raises the gain; the room after it raises it no further, however long.
        let mut agc = Agc::default();
        let mut x = noise(0.5, 0.0005, 5, true);
        x.extend(sine(16_000, 300.0, 1.0, 0.005));
        gained(&x, &mut agc);
        let after = agc.gain;
        assert!(after > 10.0, "{after}");
        let (_, gains) = gained(&noise(6.0, 0.0005, 6, true), &mut agc);
        assert!(gains.iter().all(|g| *g == after), "{after} then {gains:?}");
        // A loud voice is left as it is.
        let mut x = noise(0.5, 0.0005, 5, false);
        x.extend(sine(16_000, 300.0, 1.0, 0.4));
        let (y, gains) = gained(&x, &mut Agc::default());
        assert!(gains.iter().all(|g| *g == 1.0) && y == x);
    }

    #[test]
    fn the_front_end_gives_the_same_sound_in_any_size_of_piece_and_raises_a_far_voice() {
        let mut x = noise(0.5, 0.002, 9, true);
        let words: Vec<f32> = voice_like(3.0, 0.01).iter().zip(noise(3.0, 0.002, 10, true)).map(|(a, b)| a + b).collect();
        x.extend(&words);
        let mut whole = x.clone();
        Boost::default().process(&mut whole);
        let d = db(&whole[8_000 + 4_800..], &x[8_000..]);
        assert!(d > 10.0, "a voice at -40 dBFS over a room at -54 dBFS is raised: {d:.1} dB");
        for size in [1usize, 37, 159, 160, 161, 1_600, 48_000] {
            let mut b = Boost::default();
            let mut got = x.clone();
            got.chunks_mut(size).for_each(|p| b.process(p));
            let worst = got.iter().zip(&whole).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            assert!(worst < 1e-6, "pieces of {size}: {worst}");
        }
    }

    #[test]
    fn the_front_end_costs_well_under_a_twentieth_of_a_core() {
        let mut x = voice_like(20.0, 0.01);
        let t = std::time::Instant::now();
        Boost::default().process(&mut x);
        let spent = t.elapsed().as_secs_f32();
        // 5% of a core would be 1 s for these 20 s; a debug build of the tests is allowed four times that.
        let most = if cfg!(debug_assertions) { 4.0 } else { 1.0 };
        assert!(spent < most, "{spent:.2} s for 20 s of audio");
    }

    #[test]
    fn the_hud_judges_silence_on_the_microphone_and_quietness_on_what_the_model_hears() {
        let heard = |x: &[f32]| {
            let mut b = Boost::default();
            let mut y = x.to_vec();
            b.process(&mut y);
            let mut s = Segmenter::new(700, 25_000);
            s.feed_new(&y);
            boosted_note(&s.recent(), s.floor(), b.silent())
        };
        let words = |amp: f32, room: f32| {
            let mut x = noise(1.0, room, 4, false);
            for _ in 0..4 {
                x.extend(voice_like(0.6, amp).iter().zip(noise(0.6, room, 5, false)).map(|(a, b)| a + b));
                x.extend(noise(0.6, room, 6, false));
            }
            x
        };
        // Words at -44 dBFS are "quiet" as the microphone gives them; raised, they are not. At -80 dBFS even the most
        // gain leaves them quiet.
        let quiet = words(0.006, 0.0003);
        let mut s = Segmenter::new(700, 25_000);
        s.feed_new(&quiet);
        assert_eq!(mic_note(&s.recent(), s.floor()), Some("quiet"));
        assert_eq!(heard(&quiet), None);
        assert_eq!(heard(&words(0.0001, 0.000003)), Some("quiet"));
        // Words, then 6 s of a microphone that gives nothing, or only a hiss at -90 dBFS that the gain lifts: silent.
        for hiss in [0.0, 0.00003] {
            let mut x = words(0.006, 0.0003);
            x.extend(noise(6.0, hiss, 8, false));
            assert_eq!(heard(&x), Some("silent"), "hiss at {hiss}");
        }
        // What the model hears can be next to nothing while the microphone still gives a sound: that is not silent.
        assert_eq!(boosted_note(&[0.00005; 600], 0.00005, false), None);
        assert_eq!(boosted_note(&[0.00005; 600], 0.00005, true), Some("silent"));
    }

    #[test]
    fn a_piece_never_starts_before_where_the_segmenter_said_to_keep_audio_from() {
        let (x, _) = spoken(60, 0.3, 0.12, 0.2, 0.0005);
        let x = [x, take(&[(1.0, 1.0), (2.0, 0.2), (1.0, 1.0)])].concat();
        let mut s = Segmenter::new(700, 3_000);
        let mut held = Tail::default();
        let (mut most, mut pieces) = (0, 0);
        for piece in x.chunks(1_920) {
            let keep = s.keep_from();
            held.push(piece);
            for r in s.feed_new(piece) {
                assert!(r.start >= keep, "{r:?} starts before {keep}");
                assert_eq!(held.copy(r.clone()), x[r].to_vec());
                pieces += 1;
            }
            held.drop_before(s.keep_from());
            most = most.max(held.samples.len());
        }
        for r in s.close() {
            assert_eq!(held.copy(r.clone()), x[r].to_vec());
        }
        assert!(pieces >= 6, "{pieces}");
        // No more than a piece at the cap, its padding and a feed's worth is ever held.
        assert!(most <= 3 * 16_000 + FRAME + PAD + 1_920, "{most}");
    }
}
