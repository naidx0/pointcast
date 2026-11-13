//! The call as one small state machine: it listens, the person's turn ends, it thinks, it speaks, it listens again,
//! until the call ends. Pure: it is told what happened and when (ms), and says what to do (`Do`); everything else is
//! the caller's (`super::Running`).

use super::turn::Heard;
use serde::Serialize;

/// Nobody talking for this long while it listens ends the call.
pub const QUIET_END_MS: u64 = 45_000;
/// Esc twice within this long ends the call.
pub const DOUBLE_ESC_MS: u64 = 800;

/// Who has the call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// It listens for the person's turn.
    Listening,
    /// The turn was asked and nothing is said yet (or the next sentence is still coming).
    Thinking,
    /// It reads its answer aloud.
    Speaking,
}

/// Why the call ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    Toggle,
    Esc,
    Closed,
    Quiet,
    Hangup,
    Failed,
}

/// Where the Ask panel is, as the call sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// It records a question (the call's, or one the Ask key started).
    Listening,
    /// A question is being finished, sent or answered.
    Asking,
    /// No question is under way: an answer shows, or an error, or nothing.
    Done,
    /// The panel closed.
    Closed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The Call switch (the panel's button).
    Toggle,
    Esc,
    /// The turn just asked said only "end call" or "hang up".
    Hangup,
    /// Listening could not start.
    Failed,
    /// What the turn detector heard in a window of audio.
    Heard(Heard),
    /// How the Ask side stands now: the panel's stage, and whether an answer is being read aloud.
    Ask { stage: Stage, speaking: bool },
    /// Time passed.
    Tick,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Do {
    /// Start recording the next turn, from `from` (samples into the call's audio; none: from now).
    Listen { from: Option<u64> },
    /// The turn ended: finish its words and ask.
    Ask,
    /// Stop the answer on its way.
    StopAnswer,
    /// Stop reading aloud, at once.
    StopSpeech,
    /// Throw away the turn being recorded.
    Discard,
    /// The call is over.
    End(Why),
}

#[derive(Debug, Clone)]
pub struct Call {
    side: Side,
    /// The Ask side as last seen.
    stage: Stage,
    /// The person is in the middle of a turn.
    talking: bool,
    /// Since when nobody talked while it listened (ms).
    quiet_from: u64,
    esc_at: Option<u64>,
    over: Option<Why>,
}

impl Call {
    /// A call starts at `now` (ms): it listens.
    pub fn start(now: u64) -> (Call, Vec<Do>) {
        let c = Call { side: Side::Listening, stage: Stage::Done, talking: false, quiet_from: now, esc_at: None, over: None };
        (c, vec![Do::Listen { from: None }])
    }

    pub fn side(&self) -> Side {
        self.side
    }

    pub fn over(&self) -> Option<Why> {
        self.over
    }

    /// The person is in the middle of a turn.
    #[cfg(test)]
    pub fn talking(&self) -> bool {
        self.talking
    }

    /// `ev` happened at `now` (ms): what to do, in order.
    pub fn step(&mut self, ev: Event, now: u64) -> Vec<Do> {
        if self.over.is_some() {
            return vec![];
        }
        match ev {
            Event::Toggle => self.end(Why::Toggle),
            Event::Hangup => self.end(Why::Hangup),
            Event::Failed => self.end(Why::Failed),
            Event::Esc => {
                if self.esc_at.is_some_and(|t| now.saturating_sub(t) <= DOUBLE_ESC_MS) {
                    return self.end(Why::Esc);
                }
                self.esc_at = Some(now);
                match self.side {
                    // The first Esc silences it, and it listens again.
                    Side::Speaking => self.listen(None, now, true),
                    Side::Thinking => vec![Do::StopAnswer],
                    Side::Listening => vec![],
                }
            }
            Event::Heard(h) => self.heard(h, now),
            Event::Ask { stage, speaking } => self.ask(stage, speaking, now),
            Event::Tick => {
                let quiet = self.side == Side::Listening && !self.talking && now.saturating_sub(self.quiet_from) >= QUIET_END_MS;
                if quiet {
                    self.end(Why::Quiet)
                } else {
                    vec![]
                }
            }
        }
    }

    fn heard(&mut self, h: Heard, now: u64) -> Vec<Do> {
        match (self.side, h) {
            (Side::Listening, Heard::Started { .. }) => {
                self.talking = true;
                vec![]
            }
            // The person talked over the answer: it stops at once, and what they say is the next turn, from just
            // before they began.
            (Side::Speaking, Heard::Started { at }) => {
                let out = self.listen(Some(at), now, true);
                self.talking = true;
                out
            }
            (Side::Listening, Heard::Ended { .. }) if self.talking => {
                (self.side, self.talking) = (Side::Thinking, false);
                vec![Do::Ask]
            }
            (Side::Listening, Heard::Noise | Heard::Ended { .. }) => {
                self.talking = false;
                self.quiet_from = now;
                vec![]
            }
            _ => vec![],
        }
    }

    fn ask(&mut self, stage: Stage, speaking: bool, now: u64) -> Vec<Do> {
        let before = std::mem::replace(&mut self.stage, stage);
        match (self.side, stage) {
            (_, Stage::Closed) => self.end(Why::Closed),
            // The turn being recorded was thrown away from the panel (its Stop): it listens again.
            (Side::Listening, Stage::Done) if before == Stage::Listening => self.listen(None, now, false),
            // The Ask key finished the turn by hand, or started one: the call follows.
            (Side::Listening, Stage::Asking) => {
                (self.side, self.talking) = (Side::Thinking, false);
                vec![]
            }
            (Side::Thinking | Side::Speaking, Stage::Listening) => {
                (self.side, self.quiet_from) = (Side::Listening, now);
                vec![]
            }
            (Side::Thinking | Side::Speaking, _) if speaking => {
                self.side = Side::Speaking;
                vec![]
            }
            (Side::Thinking | Side::Speaking, Stage::Done) => self.listen(None, now, false),
            (Side::Speaking, Stage::Asking) => {
                self.side = Side::Thinking;
                vec![]
            }
            _ => vec![],
        }
    }

    /// Listen again from `from`, stopping speech (`hush`) and an answer still on its way.
    fn listen(&mut self, from: Option<u64>, now: u64, hush: bool) -> Vec<Do> {
        let mut out = Vec::new();
        if hush {
            out.push(Do::StopSpeech);
        }
        if self.stage == Stage::Asking {
            out.push(Do::StopAnswer);
        }
        out.push(Do::Listen { from });
        (self.side, self.talking, self.quiet_from) = (Side::Listening, false, now);
        out
    }

    fn end(&mut self, why: Why) -> Vec<Do> {
        let mut out = vec![Do::StopSpeech];
        if self.side == Side::Listening && self.stage != Stage::Closed {
            out.push(Do::Discard);
        }
        out.push(Do::End(why));
        self.over = Some(why);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 16; // samples per ms

    fn ended() -> Event {
        Event::Heard(Heard::Ended { from: 0, to: 1000 * S })
    }
    fn started(at: u64) -> Event {
        Event::Heard(Heard::Started { at })
    }
    fn ask(stage: Stage, speaking: bool) -> Event {
        Event::Ask { stage, speaking }
    }

    #[test]
    fn listen_think_speak_listen_with_no_key_pressed() {
        let (mut c, first) = Call::start(0);
        assert_eq!((first, c.side()), (vec![Do::Listen { from: None }], Side::Listening));
        assert_eq!(c.step(ask(Stage::Listening, false), 10), []);
        assert_eq!(c.step(started(100 * S), 400), []);
        assert!(c.talking());
        assert_eq!(c.step(ended(), 2000), [Do::Ask], "the turn's end asks");
        assert_eq!(c.side(), Side::Thinking);
        assert_eq!(c.step(ask(Stage::Asking, false), 2100), []);
        assert_eq!(c.side(), Side::Thinking);
        assert_eq!(c.step(ask(Stage::Asking, true), 3000), []);
        assert_eq!(c.side(), Side::Speaking, "the answer is read aloud as it streams");
        // The voice caught up with the stream: thinking again until the next sentence.
        assert_eq!(c.step(ask(Stage::Asking, false), 3500), []);
        assert_eq!(c.side(), Side::Thinking);
        assert_eq!(c.step(ask(Stage::Done, true), 4000), []);
        assert_eq!(c.side(), Side::Speaking, "the answer shows; the rest is still read");
        assert_eq!(c.step(ask(Stage::Done, false), 6000), [Do::Listen { from: None }], "spoken to the end: it listens again");
        assert_eq!(c.side(), Side::Listening);
        assert_eq!(c.over(), None);
    }

    #[test]
    fn a_turn_with_nothing_said_or_an_error_listens_again() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        assert_eq!(c.step(ask(Stage::Done, false), 1600), [Do::Listen { from: None }]);
        assert_eq!(c.side(), Side::Listening);
    }

    #[test]
    fn talking_over_the_answer_stops_it_at_once_and_becomes_the_next_turn_from_where_it_began() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        c.step(ask(Stage::Done, true), 3000);
        assert_eq!(c.step(started(48_000), 3200), [Do::StopSpeech, Do::Listen { from: Some(48_000) }]);
        assert_eq!(c.side(), Side::Listening);
        assert!(c.talking(), "the barge-in is a turn under way");
        assert_eq!(c.step(ended(), 5000), [Do::Ask]);
    }

    #[test]
    fn talking_over_an_answer_still_streaming_stops_the_answer_too() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        c.step(ask(Stage::Asking, true), 2000);
        assert_eq!(c.step(started(40_000), 2600), [Do::StopSpeech, Do::StopAnswer, Do::Listen { from: Some(40_000) }]);
    }

    #[test]
    fn noise_while_it_speaks_or_thinks_and_a_second_turn_while_it_thinks_are_ignored() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        assert_eq!(c.step(started(30_000), 1800), [], "thinking: not a barge-in");
        assert_eq!(c.step(Event::Heard(Heard::Noise), 1900), []);
        c.step(ask(Stage::Done, true), 2000);
        assert_eq!(c.step(Event::Heard(Heard::Noise), 2100), []);
        assert_eq!(c.step(ended(), 2200), [], "only a start barges in");
        assert_eq!(c.side(), Side::Speaking);
    }

    #[test]
    fn the_toggle_ends_the_call_and_throws_away_a_turn_being_said() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        assert_eq!(c.step(Event::Toggle, 500), [Do::StopSpeech, Do::Discard, Do::End(Why::Toggle)]);
        assert_eq!(c.over(), Some(Why::Toggle));
        assert_eq!(c.step(ended(), 2000), [], "nothing after the end");
        // While it speaks: the voice stops, nothing to discard.
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        c.step(ask(Stage::Done, true), 2000);
        assert_eq!(c.step(Event::Toggle, 2500), [Do::StopSpeech, Do::End(Why::Toggle)]);
    }

    #[test]
    fn esc_once_silences_the_voice_and_listens_esc_twice_ends_the_call() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        c.step(ask(Stage::Done, true), 2000);
        assert_eq!(c.step(Event::Esc, 3000), [Do::StopSpeech, Do::Listen { from: None }]);
        assert_eq!(c.side(), Side::Listening);
        assert_eq!(c.step(Event::Esc, 3000 + DOUBLE_ESC_MS), [Do::StopSpeech, Do::Discard, Do::End(Why::Esc)]);
        // Two far apart are two single ones.
        let (mut c, _) = Call::start(0);
        assert_eq!(c.step(Event::Esc, 100), []);
        assert_eq!(c.step(Event::Esc, 100 + DOUBLE_ESC_MS + 1), []);
        assert_eq!(c.over(), None);
        // While it thinks, the first stops the answer.
        c.step(started(0), 2000);
        c.step(ended(), 3000);
        assert_eq!(c.step(Event::Esc, 5000), [Do::StopAnswer]);
    }

    #[test]
    fn forty_five_seconds_with_nobody_talking_ends_the_call_but_never_mid_turn_or_while_it_answers() {
        let (mut c, _) = Call::start(0);
        assert_eq!(c.step(Event::Tick, QUIET_END_MS - 1), []);
        assert_eq!(c.step(Event::Tick, QUIET_END_MS), [Do::StopSpeech, Do::Discard, Do::End(Why::Quiet)]);
        // A long turn and a long answer do not count.
        let (mut c, _) = Call::start(0);
        c.step(started(0), 1000);
        assert_eq!(c.step(Event::Tick, 60_000), []);
        c.step(ended(), 61_000);
        c.step(ask(Stage::Asking, true), 62_000);
        assert_eq!(c.step(Event::Tick, 200_000), []);
        // The clock starts again when it listens again, and noise does not stop it from ending.
        c.step(ask(Stage::Done, false), 200_000);
        c.step(Event::Heard(Heard::Noise), 220_000);
        assert_eq!(c.step(Event::Tick, 220_000 + QUIET_END_MS - 1), []);
        assert_eq!(c.step(Event::Tick, 265_000).last(), Some(&Do::End(Why::Quiet)));
    }

    #[test]
    fn saying_end_call_alone_or_closing_the_panel_or_a_failed_start_ends_it() {
        let (mut c, _) = Call::start(0);
        c.step(started(0), 300);
        c.step(ended(), 1500);
        assert_eq!(c.step(Event::Hangup, 1600), [Do::StopSpeech, Do::End(Why::Hangup)]);
        let (mut c, _) = Call::start(0);
        assert_eq!(c.step(ask(Stage::Closed, false), 100), [Do::StopSpeech, Do::End(Why::Closed)], "a closed panel discarded its turn itself");
        let (mut c, _) = Call::start(0);
        assert_eq!(c.step(Event::Failed, 5).last(), Some(&Do::End(Why::Failed)));
    }

    #[test]
    fn the_ask_key_used_during_a_call_is_followed() {
        let (mut c, _) = Call::start(0);
        // The key finished the turn by hand.
        assert_eq!(c.step(ask(Stage::Asking, false), 500), []);
        assert_eq!(c.side(), Side::Thinking);
        // The key started listening while it spoke.
        c.step(ask(Stage::Done, true), 1000);
        assert_eq!(c.step(ask(Stage::Listening, false), 1500), []);
        assert_eq!(c.side(), Side::Listening);
        assert_eq!(c.step(Event::Tick, 1500 + QUIET_END_MS - 1), [], "the quiet clock started there");
        // The panel's Stop threw the turn away: it listens again.
        assert_eq!(c.step(ask(Stage::Done, false), 2000), [Do::Listen { from: None }]);
        assert_eq!(c.step(ask(Stage::Done, false), 2100), [], "once");
    }
}
