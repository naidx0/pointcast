//! When the person's turn starts and ends, from one speech-or-not answer per window of audio (the VAD's), and what
//! counts as their voice while Pointcast speaks (`Echo`). Pure: no clock, no audio device.

use crate::voice::audio;

/// A turn ends after this much silence following speech...
pub const END_SILENCE_MS: u32 = 700;
/// ...and is a turn only with at least this much speech in it: a click, a cough or a bump is not.
pub const MIN_SPEECH_MS: u32 = 250;
/// Speech not yet long enough to be a turn is forgotten after this much silence.
pub const NOISE_GAP_MS: u32 = 200;
/// While Pointcast speaks, the person talking over it this long stops it (a turn needs `MIN_SPEECH_MS`; a barge-in is
/// heard sooner, so the voice stops while they are still on their first word).
pub const BARGE_MS: u32 = 160;
/// Audio kept before the first window heard as speech, so the start of the first word is never cut.
pub const PREROLL_MS: u32 = 150;

/// What a window of audio changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heard {
    Nothing,
    /// Speech went on long enough to be a turn. `at` is where it began, in samples since the stream began.
    Started { at: u64 },
    /// The turn ended: its speech ran from `from` to `to` (samples).
    Ended { from: u64, to: u64 },
    /// Speech too short to be a turn ended: it is forgotten.
    Noise,
}

/// Follows the person's turns from one speech-or-not answer per window.
#[derive(Debug, Clone)]
pub struct Turn {
    /// Speech is needed this long before a turn starts (`MIN_SPEECH_MS`, or `BARGE_MS` over Pointcast's voice).
    min_ms: u32,
    /// Samples heard so far.
    pos: u64,
    /// Where the speech under way began; none in silence.
    onset: Option<u64>,
    /// Speech heard since `onset`, and silence since the last speech.
    voiced_ms: u32,
    silent_ms: u32,
    /// The speech under way is a turn (`Started` was told).
    started: bool,
}

impl Default for Turn {
    fn default() -> Self {
        Turn::new(MIN_SPEECH_MS)
    }
}

impl Turn {
    pub fn new(min_ms: u32) -> Turn {
        Turn { min_ms, pos: 0, onset: None, voiced_ms: 0, silent_ms: 0, started: false }
    }

    /// From now on a turn starts after `ms` of speech; speech under way keeps what it has.
    pub fn need(&mut self, ms: u32) {
        self.min_ms = ms;
    }

    /// A turn is under way (started and not ended).
    #[cfg(test)]
    pub fn talking(&self) -> bool {
        self.started
    }

    /// Hear one window of `samples` (16 kHz) that the VAD called `speech` or not.
    pub fn hear(&mut self, speech: bool, samples: usize) -> Heard {
        let start = self.pos;
        self.pos += samples as u64;
        let ms = (samples as u64 * 1000 / audio::RATE as u64) as u32;
        if speech {
            if self.onset.is_none() {
                (self.onset, self.voiced_ms) = (Some(start), 0);
            }
            self.voiced_ms += ms;
            self.silent_ms = 0;
            if !self.started && self.voiced_ms >= self.min_ms {
                self.started = true;
                return Heard::Started { at: self.onset.unwrap_or(start) };
            }
            return Heard::Nothing;
        }
        let Some(from) = self.onset else { return Heard::Nothing };
        self.silent_ms += ms;
        let gap = if self.started { END_SILENCE_MS } else { NOISE_GAP_MS };
        if self.silent_ms < gap {
            return Heard::Nothing;
        }
        let (was, voiced, to) = (self.started, self.voiced_ms, self.pos - self.silent_ms as u64 * audio::RATE as u64 / 1000);
        (self.onset, self.started, self.voiced_ms, self.silent_ms) = (None, false, 0, 0);
        // A barge-in heard sooner than a turn needs that stopped at once is still too short to be one.
        if was && voiced >= MIN_SPEECH_MS {
            Heard::Ended { from, to }
        } else {
            Heard::Noise
        }
    }
}

/// How far above its own voice through the speakers the microphone must hear the person while Pointcast speaks: 10 dB.
pub const ECHO_MARGIN_DB: f32 = 10.0;
/// Its voice through the speakers is learnt rising over about this long (the few quieter windows a person's first
/// syllable rises through move it little)...
pub const ECHO_RISE_MS: f32 = 100.0;
/// ...and as last heard falls by this much a second (it is heard again each time it speaks).
pub const ECHO_FALL_DB_S: f32 = 3.0;
/// Before Pointcast has been heard through the speakers in a call, its voice is taken to be this loud (dBFS)...
pub const ECHO_START_DB: f32 = -36.0;
/// ...and each time it starts speaking, its first this-much of sound (over the barge-in level) is taken as its own
/// voice, loudest first: a voice louder than last time is learnt before it could stop itself. Silence before the
/// sound starts (a natural voice takes a moment) does not count.
pub const ECHO_GRACE_MS: u32 = 250;
/// After it stops speaking, its voice is still coming back through the speakers (latency, a room's reverb) for about
/// this long; in that time only what is over its learnt level by the margin counts as the person.
pub const ECHO_TAIL_MS: u32 = 300;

/// What counts as the person talking while Pointcast speaks: louder than its own voice coming back through the speakers
/// (the level heard while it spoke and the person did not, learnt rising over `ECHO_RISE_MS` and falling slowly) by
/// `ECHO_MARGIN_DB`, and never
/// quieter than the barge-in level of a normal answer (`speak::BARGE_RMS`, about -40 dBFS).
#[derive(Debug, Clone)]
pub struct Echo {
    /// Pointcast's own voice as the microphone hears it, in dBFS.
    db: f32,
    /// Sound still taken as its own voice since it started speaking (`ECHO_GRACE_MS`).
    grace_ms: u32,
    /// Time left of the tail after it stopped speaking (`ECHO_TAIL_MS`).
    tail_ms: u32,
}

impl Default for Echo {
    fn default() -> Self {
        Echo { db: ECHO_START_DB, grace_ms: 0, tail_ms: 0 }
    }
}

/// An rms in dBFS.
pub fn db_of(rms: f32) -> f32 {
    20.0 * rms.max(1e-6).log10()
}

impl Echo {
    /// Pointcast starts speaking.
    pub fn begin(&mut self) {
        self.grace_ms = ECHO_GRACE_MS;
    }

    /// Pointcast has stopped speaking: its voice still arrives for `ECHO_TAIL_MS`.
    pub fn end(&mut self) {
        self.tail_ms = ECHO_TAIL_MS;
    }

    /// Whether the tail of its voice is still arriving.
    pub fn tailing(&self) -> bool {
        self.tail_ms > 0
    }

    /// Hear one window at `rms` lasting `ms` in the tail: true when it is over its learnt level by the margin. Nothing is
    /// learnt here (the tail is quieter and ends soon).
    pub fn hear_tail(&mut self, rms: f32, ms: u32) -> bool {
        self.tail_ms = self.tail_ms.saturating_sub(ms);
        db_of(rms) >= self.threshold_db()
    }

    /// The level, in dBFS, the person must be over.
    pub fn threshold_db(&self) -> f32 {
        (self.db + ECHO_MARGIN_DB).max(db_of(crate::voice::speak::BARGE_RMS))
    }

    /// Hear one window at `rms` lasting `ms` while Pointcast speaks: true when it is loud enough to be the person.
    /// Anything quieter is Pointcast's own voice (or the room) and teaches the level.
    pub fn hear(&mut self, rms: f32, ms: u32) -> bool {
        let db = db_of(rms);
        if self.grace_ms > 0 {
            if db >= db_of(crate::voice::speak::BARGE_RMS) {
                self.grace_ms = self.grace_ms.saturating_sub(ms);
                self.db = self.db.max(db);
            }
            return false;
        }
        let person = db >= self.threshold_db();
        self.db = (self.db - ECHO_FALL_DB_S * ms as f32 / 1000.0).max(ECHO_START_DB - 24.0);
        if !person && db > self.db {
            self.db += (db - self.db) * (1.0 - (-(ms as f32) / ECHO_RISE_MS).exp());
        }
        person
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One 32 ms window, as Silero hears it.
    const W: usize = 512;
    const W_MS: u32 = 32;

    fn feed(t: &mut Turn, speech: bool, ms: u32) -> Vec<Heard> {
        (0..ms / W_MS).map(|_| t.hear(speech, W)).filter(|h| *h != Heard::Nothing).collect()
    }

    #[test]
    fn speech_starts_a_turn_after_its_minimum_and_seven_hundred_ms_of_silence_ends_it() {
        let mut t = Turn::default();
        assert!(feed(&mut t, false, 960).is_empty(), "silence is nothing");
        let started = feed(&mut t, true, 320);
        assert_eq!(started, [Heard::Started { at: 30 * W as u64 }], "told once, from where the speech began");
        assert!(t.talking());
        assert!(feed(&mut t, true, 1280).is_empty());
        // Silence under 700 ms is a pause inside the turn.
        assert!(feed(&mut t, false, 640).is_empty());
        assert!(feed(&mut t, true, 320).is_empty(), "the turn goes on after a pause");
        let ended = feed(&mut t, false, 704);
        assert_eq!(ended.len(), 1);
        let Heard::Ended { from, to } = ended[0] else { panic!("{ended:?}") };
        assert_eq!(from, 30 * W as u64);
        assert_eq!(to, (30 + 10 + 40 + 20 + 10) * W as u64, "it ends where the speech did, not 700 ms later");
        assert!(!t.talking());
    }

    #[test]
    fn the_hangover_is_seven_hundred_ms_not_one_window_less() {
        let mut t = Turn::default();
        feed(&mut t, true, 320);
        // 21 windows of 32 ms = 672 ms: not yet.
        assert!(feed(&mut t, false, 21 * W_MS).is_empty());
        assert!(matches!(t.hear(false, W), Heard::Ended { .. }), "the 22nd (704 ms) ends it");
    }

    #[test]
    fn a_short_noise_is_not_a_turn_and_is_forgotten() {
        let mut t = Turn::default();
        // A click: two windows (64 ms), then quiet.
        assert!(feed(&mut t, true, 64).is_empty());
        assert_eq!(feed(&mut t, false, 224), [Heard::Noise]);
        assert!(!t.talking());
        // 224 ms of speech (7 windows): still under 250.
        assert!(feed(&mut t, true, 224).is_empty());
        assert_eq!(feed(&mut t, false, 224), [Heard::Noise]);
        // Two short noises with a long gap between do not add up.
        feed(&mut t, true, 160);
        feed(&mut t, false, 320);
        assert!(feed(&mut t, true, 160).is_empty());
    }

    #[test]
    fn a_barge_in_is_heard_sooner_but_a_short_one_that_stops_is_still_not_a_turn() {
        let mut t = Turn::new(BARGE_MS);
        assert_eq!(feed(&mut t, true, 160), [Heard::Started { at: 0 }]);
        t.need(MIN_SPEECH_MS);
        // It stops at 192 ms of speech: the voice was stopped, but nothing is asked.
        feed(&mut t, true, 32);
        assert_eq!(feed(&mut t, false, 704), [Heard::Noise]);
        // A real one goes on and ends as a turn.
        let mut t = Turn::new(BARGE_MS);
        assert_eq!(feed(&mut t, true, 160).len(), 1);
        t.need(MIN_SPEECH_MS);
        feed(&mut t, true, 640);
        assert!(matches!(feed(&mut t, false, 704)[..], [Heard::Ended { from: 0, .. }]));
    }

    #[test]
    fn the_preroll_reaches_back_a_hundred_and_fifty_ms_before_the_first_word() {
        assert_eq!(PREROLL_MS, 150);
        assert!(BARGE_MS < MIN_SPEECH_MS && MIN_SPEECH_MS < END_SILENCE_MS);
    }

    #[test]
    fn its_own_voice_through_the_speakers_never_counts_as_the_person_but_their_voice_over_it_does() {
        let mut e = Echo::default();
        let rms = |db: f32| 10f32.powf(db / 20.0);
        // Pointcast's voice comes back at -30 dBFS, its words rising and falling 6 dB, for 5 s.
        for i in 0..160 {
            let db = -30.0 - (i % 8) as f32 * 0.75 - if i % 16 < 2 { 12.0 } else { 0.0 };
            assert!(!e.hear(rms(db), 32), "window {i} at {db} dBFS");
        }
        assert!(!e.hear(rms(-23.0), 32), "a peak 7 dB over its loudest is still its own");
        // The person starting to talk over it, rising to -18 dBFS (12 dB over it): heard once over the bar, at once.
        let heard: Vec<bool> = [-40.0, -30.0, -24.0, -18.0].iter().map(|&d| e.hear(rms(d), 32)).collect();
        assert_eq!(heard, [false, false, false, true], "its first quieter windows do not raise the bar past it");
        // Hearing the person does not raise the bar: they stay heard.
        for _ in 0..10 {
            assert!(e.hear(rms(-18.0), 32));
        }
        // In a quiet room the bar is never under the normal barge-in level.
        let mut quiet = Echo::default();
        for _ in 0..1000 {
            quiet.hear(rms(-70.0), 32);
        }
        assert!((quiet.threshold_db() - db_of(crate::voice::speak::BARGE_RMS)).abs() < 0.01);
        assert!(!quiet.hear(rms(-45.0), 32) && quiet.hear(rms(-38.0), 32));
    }

    #[test]
    fn a_voice_louder_than_the_last_is_learnt_from_its_first_sound_and_silence_before_it_does_not_count() {
        let rms = |db: f32| 10f32.powf(db / 20.0);
        let mut e = Echo::default();
        e.begin();
        // The natural voice takes 400 ms to sound: silence, then its voice at -22 dBFS, far over the starting bar.
        for _ in 0..12 {
            assert!(!e.hear(rms(-60.0), 32));
        }
        for i in 0..200 {
            let db = -22.0 - (i % 6) as f32;
            assert!(!e.hear(rms(db), 32), "window {i}: its own voice");
        }
        assert!(e.hear(rms(-10.0), 32), "the person, 12 dB over it");
        // The next answer starts where this one left the bar.
        e.begin();
        for _ in 0..8 {
            e.hear(rms(-30.0), 32);
        }
        assert!(!e.hear(rms(-23.0), 32));
    }

    #[test]
    fn its_tail_after_it_stops_is_not_the_person_but_a_clearly_louder_voice_is_and_the_tail_ends() {
        let mut e = Echo::default();
        e.begin();
        // It spoke at about -24 dBFS.
        for _ in 0..30 {
            e.hear(db_to_rms(-24.0), 32);
        }
        assert!(!e.tailing());
        e.end();
        assert!(e.tailing());
        assert!(!e.hear_tail(db_to_rms(-24.0), 32), "its own tail");
        assert!(e.hear_tail(db_to_rms(-8.0), 32), "a clearly louder voice");
        for _ in 0..ECHO_TAIL_MS / 32 + 1 {
            e.hear_tail(db_to_rms(-24.0), 32);
        }
        assert!(!e.tailing(), "the tail ends after ECHO_TAIL_MS");
    }

    fn db_to_rms(db: f32) -> f32 {
        10f32.powf(db / 20.0)
    }
}
