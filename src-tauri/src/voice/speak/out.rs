//! The speakers, through cpal: an output stream opened when there is something to play and closed after a while of
//! quiet. Sounds play one after another; a stop fades the one playing out over a few milliseconds (no click) and drops
//! the rest. All the work happens on a thread of its own; the audio callback only copies samples.

use super::natural::Sink;
use super::Done;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// After this long with nothing to play, the stream is closed.
const QUIET: Duration = Duration::from_secs(30);
/// A stop fades out over this long.
const FADE: Duration = Duration::from_millis(6);

#[derive(Default)]
struct Buf {
    /// Sounds handed over and not yet in `samples`: their samples, rate, and who hears their end.
    incoming: VecDeque<(Vec<f32>, u32, Done)>,
    /// Mono samples at the stream's rate, waiting for the device.
    samples: VecDeque<f32>,
    /// Samples the device has taken, and samples ever queued, since the stream opened.
    played: u64,
    queued: u64,
    /// Where each sound ends (in `queued` terms), with who hears it.
    ends: VecDeque<(u64, Done)>,
    /// The stream's sample rate, when one is open.
    rate: Option<u32>,
    /// What went wrong with the stream; it is closed and opened again for the next sound.
    broke: Option<String>,
    /// Open the stream now: something will play soon.
    wake: bool,
    closed: bool,
    /// When sound started (true) and stopped (false) reaching the device, for the hand-run probe.
    #[cfg(test)]
    heard: Vec<(Instant, bool)>,
}

impl Buf {
    /// The device asks for `frames` samples (of `channels` each): what is queued, then silence.
    fn fill(&mut self, out: &mut [f32], channels: usize) {
        #[cfg(test)]
        {
            let sounding = self.heard.last().is_some_and(|h| h.1);
            if sounding != !self.samples.is_empty() {
                self.heard.push((Instant::now(), !sounding));
            }
        }
        for frame in out.chunks_mut(channels.max(1)) {
            let s = match self.samples.pop_front() {
                Some(s) => {
                    self.played += 1;
                    s
                }
                None => 0.0,
            };
            frame.fill(s);
        }
    }

    /// Queue a sound already at the stream's rate.
    fn push(&mut self, samples: Vec<f32>, ended: Done) {
        self.queued += samples.len() as u64;
        self.samples.extend(samples);
        self.ends.push_back((self.queued, ended));
    }

    /// The sounds whose end the device has passed.
    fn ended(&mut self) -> Vec<Done> {
        let mut out = Vec::new();
        while self.ends.front().is_some_and(|(at, _)| *at <= self.played) {
            out.push(self.ends.pop_front().unwrap().1);
        }
        out
    }

    /// Cut everything: what plays fades out over `fade` samples, the rest is dropped. Returns every listener, all of
    /// them ended now.
    fn cut(&mut self, fade: usize) -> Vec<Done> {
        let keep = fade.min(self.samples.len());
        self.samples.truncate(keep);
        for (i, s) in self.samples.iter_mut().enumerate() {
            *s *= 1.0 - (i + 1) as f32 / keep as f32;
        }
        self.queued = self.played + keep as u64;
        let mut out: Vec<Done> = self.incoming.drain(..).map(|(_, _, d)| d).collect();
        out.extend(self.ends.drain(..).map(|(_, d)| d));
        out
    }
}

/// `x` from `from` Hz to `to` Hz, by straight lines between samples.
pub fn resample(x: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || x.is_empty() || from == 0 || to == 0 {
        return x.to_vec();
    }
    let n = (x.len() as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    (0..n)
        .map(|i| {
            let t = i as f64 * step;
            let j = t as usize;
            let (a, b) = (x[j.min(x.len() - 1)], x[(j + 1).min(x.len() - 1)]);
            a + (b - a) * (t - j as f64) as f32
        })
        .collect()
}

struct Shared {
    buf: Mutex<Buf>,
    moved: Condvar,
}

/// This computer's speakers, as a `Sink`.
pub struct Speakers {
    shared: Arc<Shared>,
}

impl Default for Speakers {
    fn default() -> Self {
        let shared = Arc::new(Shared { buf: Mutex::default(), moved: Condvar::new() });
        let worker = shared.clone();
        std::thread::Builder::new().name("speak-out".into()).spawn(move || worker.run()).expect("the speaker thread starts");
        Speakers { shared }
    }
}

impl Drop for Speakers {
    fn drop(&mut self) {
        self.stop();
        self.shared.buf.lock().unwrap().closed = true;
        self.shared.moved.notify_all();
    }
}

impl Sink for Speakers {
    fn wake(&self) {
        self.shared.buf.lock().unwrap().wake = true;
        self.shared.moved.notify_all();
    }

    fn play(&self, samples: Vec<f32>, rate: u32, ended: Done) -> Result<(), String> {
        self.shared.buf.lock().unwrap().incoming.push_back((samples, rate, ended));
        self.shared.moved.notify_all();
        Ok(())
    }

    fn stop(&self) {
        let ended = {
            let mut b = self.shared.buf.lock().unwrap();
            let fade = b.rate.map_or(0, |r| (r as f64 * FADE.as_secs_f64()) as usize);
            b.cut(fade)
        };
        for done in ended {
            done();
        }
    }
}

#[cfg(test)]
impl Speakers {
    /// Whether a stream is open (the device is awake).
    pub fn awake(&self) -> bool {
        self.shared.buf.lock().unwrap().rate.is_some()
    }

    /// When sound first reached the device after `from`, waiting up to `limit`.
    pub fn first_sound_after(&self, from: Instant, limit: Duration) -> Option<Instant> {
        while from.elapsed() < limit {
            if let Some(t) = self.shared.buf.lock().unwrap().heard.iter().find(|h| h.1 && h.0 >= from).map(|h| h.0) {
                return Some(t);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        None
    }

    /// The silences between sounds, as the device had them.
    pub fn gaps(&self) -> Vec<Duration> {
        let b = self.shared.buf.lock().unwrap();
        b.heard.windows(2).filter(|w| !w[0].1 && w[1].1).map(|w| w[1].0.duration_since(w[0].0)).collect()
    }
}

impl Shared {
    fn run(self: Arc<Self>) {
        let mut stream: Option<cpal::Stream> = None;
        let mut quiet_since = Instant::now();
        let mut b = self.buf.lock().unwrap();
        loop {
            if b.closed {
                return;
            }
            if let Some(why) = b.broke.take() {
                eprintln!("[speak] the speakers stopped: {why}");
                stream = None;
                b.rate = None;
                b.samples.clear();
                let ended: Vec<Done> = b.ends.drain(..).map(|(_, d)| d).collect();
                drop(b);
                ended.into_iter().for_each(|d| d());
                b = self.buf.lock().unwrap();
                continue;
            }
            if (b.wake || !b.incoming.is_empty()) && stream.is_none() {
                b.wake = false;
                quiet_since = Instant::now();
                drop(b);
                let opened = open(self.clone());
                b = self.buf.lock().unwrap();
                match opened {
                    Ok((s, rate)) => {
                        stream = Some(s);
                        b.wake = false;
                        (b.rate, b.played, b.queued) = (Some(rate), 0, b.samples.len() as u64);
                    }
                    Err(why) => {
                        eprintln!("[speak] {why}");
                        let ended: Vec<Done> = b.incoming.drain(..).map(|(_, _, d)| d).collect();
                        drop(b);
                        ended.into_iter().for_each(|d| d());
                        b = self.buf.lock().unwrap();
                        continue;
                    }
                }
            }
            if let (Some(rate), Some((samples, from, ended))) = (b.rate, b.incoming.pop_front()) {
                drop(b);
                let at_rate = resample(&samples, from, rate);
                b = self.buf.lock().unwrap();
                b.push(at_rate, ended);
                continue;
            }
            let ended = b.ended();
            if !ended.is_empty() {
                drop(b);
                ended.into_iter().for_each(|d| d());
                b = self.buf.lock().unwrap();
                continue;
            }
            let busy = !b.samples.is_empty() || !b.ends.is_empty();
            if busy {
                quiet_since = Instant::now();
            } else if stream.is_some() && quiet_since.elapsed() >= QUIET {
                stream = None;
                b.rate = None;
            }
            let wait = if busy { Duration::from_millis(5) } else { QUIET };
            b = self.moved.wait_timeout(b, wait).unwrap().0;
        }
    }
}

/// The default output device's stream, filled from `shared`, and its sample rate.
fn open(shared: Arc<Shared>) -> Result<(cpal::Stream, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let dev = cpal::default_host().default_output_device().ok_or("No speakers or headphones found.")?;
    let cfg = dev.default_output_config().map_err(|e| e.to_string())?;
    let (channels, rate, fmt) = (cfg.channels() as usize, cfg.sample_rate().0, cfg.sample_format());
    let config: cpal::StreamConfig = cfg.into();
    let broke = {
        let shared = shared.clone();
        move |e: cpal::StreamError| {
            shared.buf.lock().unwrap().broke = Some(e.to_string());
            shared.moved.notify_all();
        }
    };
    let mut scratch: Vec<f32> = Vec::new();
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream(&config, move |d: &mut [f32], _: &_| shared.buf.lock().unwrap().fill(d, channels), broke, None),
        cpal::SampleFormat::I16 => dev.build_output_stream(
            &config,
            move |d: &mut [i16], _: &_| {
                scratch.resize(d.len(), 0.0);
                shared.buf.lock().unwrap().fill(&mut scratch, channels);
                for (o, s) in d.iter_mut().zip(&scratch) {
                    *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
                }
            },
            broke,
            None,
        ),
        cpal::SampleFormat::U16 => dev.build_output_stream(
            &config,
            move |d: &mut [u16], _: &_| {
                scratch.resize(d.len(), 0.0);
                shared.buf.lock().unwrap().fill(&mut scratch, channels);
                for (o, s) in d.iter_mut().zip(&scratch) {
                    *o = ((s.clamp(-1.0, 1.0) + 1.0) * 32767.5) as u16;
                }
            },
            broke,
            None,
        ),
        other => return Err(format!("{other:?} speakers are not supported")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn counter() -> (Arc<AtomicUsize>, impl Fn() -> Done) {
        let n = Arc::new(AtomicUsize::new(0));
        let m = n.clone();
        (n, move || {
            let m = m.clone();
            Box::new(move || {
                m.fetch_add(1, Ordering::SeqCst);
            })
        })
    }

    #[test]
    fn sounds_play_one_after_another_and_each_end_is_heard_when_the_device_passes_it() {
        let (ended, done) = counter();
        let mut b = Buf::default();
        b.push(vec![0.5; 4], done());
        b.push(vec![0.25; 4], done());
        let mut out = [9.0; 12];
        // Two channels: each sample goes to both.
        b.fill(&mut out[..6], 2);
        assert_eq!(out[..6], [0.5; 6]);
        assert!(b.ended().is_empty(), "three of four samples played");
        b.fill(&mut out[6..], 2);
        assert_eq!(out[6..], [0.5, 0.5, 0.25, 0.25, 0.25, 0.25]);
        b.ended().into_iter().for_each(|d| d());
        assert_eq!(ended.load(Ordering::SeqCst), 1, "the first ended, the second not yet");
        let mut rest = [9.0; 8];
        b.fill(&mut rest, 2);
        assert_eq!(rest, [0.25, 0.25, 0.25, 0.25, 0.0, 0.0, 0.0, 0.0], "then silence");
        b.ended().into_iter().for_each(|d| d());
        assert_eq!(ended.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_cut_fades_out_over_a_few_samples_drops_the_rest_and_ends_everything_at_once() {
        let (ended, done) = counter();
        let mut b = Buf::default();
        b.push(vec![1.0; 1000], done());
        b.push(vec![1.0; 1000], done());
        b.incoming.push_back((vec![1.0; 10], 24_000, done()));
        let mut out = [0.0; 100];
        b.fill(&mut out, 1);
        b.cut(4).into_iter().for_each(|d| d());
        assert_eq!(ended.load(Ordering::SeqCst), 3, "every sound, playing, queued or still coming, has ended");
        let mut out = [9.0; 8];
        b.fill(&mut out, 1);
        assert_eq!(out, [0.75, 0.5, 0.25, 0.0, 0.0, 0.0, 0.0, 0.0], "a short fade, then silence");
        // It plays again after a cut, its end counted from where the device is.
        b.push(vec![0.5; 2], done());
        let mut out = [9.0; 2];
        b.fill(&mut out, 1);
        assert_eq!(b.ended().len(), 1);
    }

    /// Hand-run, silent (it plays only zeros): how long the speakers take from a sound handed over to the device
    /// taking it, with the device asleep and then awake. `cargo test --lib speakers_latency -- --ignored --nocapture`
    #[test]
    #[ignore = "opens this computer's speakers"]
    fn speakers_latency() {
        let s = Speakers::default();
        let heard = |s: &Speakers| {
            let from = Instant::now();
            s.play(vec![0.0; 2400], 24_000, Box::new(|| {})).unwrap();
            s.first_sound_after(from, Duration::from_secs(5)).expect("the device takes it").duration_since(from)
        };
        println!("asleep: {} ms from a sound handed over to the device taking it", heard(&s).as_millis());
        std::thread::sleep(Duration::from_millis(300));
        println!("awake: {} ms", heard(&s).as_millis());
        let s2 = Speakers::default();
        let t = Instant::now();
        s2.wake();
        while !s2.awake() && t.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(1));
        }
        println!("waking the device took {} ms", t.elapsed().as_millis());
    }

    #[test]
    fn resampling_keeps_the_length_in_time_and_the_shape() {
        let x: Vec<f32> = (0..240).map(|i| (i as f32 / 240.0 * std::f32::consts::TAU).sin()).collect();
        let up = resample(&x, 24_000, 48_000);
        assert_eq!(up.len(), 480);
        assert!((up[120] - x[60]).abs() < 1e-4 && (up[121] - (x[60] + x[61]) / 2.0).abs() < 1e-4);
        assert_eq!(resample(&x, 24_000, 44_100).len(), 441);
        assert_eq!(resample(&x, 24_000, 24_000), x);
        assert!(resample(&[], 24_000, 48_000).is_empty());
    }
}
