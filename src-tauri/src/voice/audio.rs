//! The microphone (cpal) and the signal work around it: downmix to mono, resample to 16 kHz,
//! a level for the meter, and cutting a long take at its pauses so the model never sees more
//! than a few seconds at once. Samples live in memory only.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
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

#[derive(Debug, Clone)]
pub struct Segmenter {
    pub gap_ms: u32,
    pub max_ms: u32,
    floor: f32,
    start: usize,
    last_voice: Option<usize>,
    seen: usize,
    done: usize,
}

pub const FRAME: usize = 480;

impl Segmenter {
    pub fn new(gap_ms: u32, max_ms: u32) -> Self {
        Segmenter {
            gap_ms,
            max_ms,
            floor: 0.003,
            start: 0,
            last_voice: None,
            seen: 0,
            done: 0,
        }
    }
    fn ms(n: usize) -> u32 {
        (n as u64 * 1000 / RATE as u64) as u32
    }
    pub fn feed(&mut self, all: &[f32]) -> Vec<std::ops::Range<usize>> {
        let mut out = vec![];
        while self.seen + FRAME <= all.len() {
            let f = &all[self.seen..self.seen + FRAME];
            let r = rms(f);
            let voice = r > (self.floor * 3.0).max(0.01);
            if !voice {
                self.floor = self.floor * 0.95 + r * 0.05;
            }
            self.seen += FRAME;
            if voice {
                self.last_voice = Some(self.seen);
            }
            match self.last_voice {
                Some(v) => {
                    let quiet = Self::ms(self.seen - v);
                    let long = Self::ms(self.seen - self.start);
                    if quiet >= self.gap_ms || long >= self.max_ms {
                        out.push(self.start..self.seen);
                        self.start = self.seen;
                        self.done = self.seen;
                        self.last_voice = None;
                    }
                }
                None => {
                    let keep = (RATE as usize * 3 / 10).min(self.seen);
                    self.start = (self.seen - keep).max(self.done);
                }
            }
        }
        out
    }
    pub fn open_start(&self) -> Option<usize> {
        self.last_voice.map(|_| self.start)
    }
    pub fn finish(&mut self, all: &[f32]) -> Vec<std::ops::Range<usize>> {
        let mut out = self.feed(all);
        if self.last_voice.is_some() {
            out.push(self.start..all.len());
        }
        self.start = all.len();
        self.last_voice = None;
        out
    }
}

#[derive(Default)]
pub struct Take {
    pub samples: Mutex<Vec<f32>>,
    pub level: AtomicU32,
    pub open: AtomicBool,
    pub error: Mutex<Option<String>>,
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
        s[r.start.min(s.len())..r.end.min(s.len())].to_vec()
    }
    pub fn all(&self) -> Vec<f32> {
        self.samples.lock().unwrap().clone()
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
}
