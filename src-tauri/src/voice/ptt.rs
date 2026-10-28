//! Push to talk, one mode at a time: Hold talks while the chord is down and sends on release;
//! Tap starts on one press and sends on the next.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalkMode {
    Hold,
    #[default]
    #[serde(alias = "hold_or_tap")]
    Tap,
}

/// A held key whose release never arrives (an elevated window took the focus) stops here.
pub const WATCHDOG_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Start,
    Send,
    Nothing,
}

#[derive(Debug, Clone, Default)]
pub struct Ptt {
    pub mode: TalkMode,
    since: Option<u64>,
}

impl Ptt {
    pub fn new(mode: TalkMode) -> Self {
        Ptt { mode, since: None }
    }
    pub fn set_mode(&mut self, mode: TalkMode) {
        self.mode = mode;
    }
    pub fn listening(&self) -> bool {
        self.since.is_some()
    }
    pub fn press(&mut self, at: u64) -> Act {
        match self.since {
            None => {
                self.since = Some(at);
                Act::Start
            }
            Some(_) => match self.mode {
                TalkMode::Tap => {
                    self.since = None;
                    Act::Send
                }
                TalkMode::Hold => Act::Nothing,
            },
        }
    }
    pub fn release(&mut self, _at: u64) -> Act {
        match (self.mode, self.since) {
            (TalkMode::Hold, Some(_)) => {
                self.since = None;
                Act::Send
            }
            _ => Act::Nothing,
        }
    }
    pub fn tick(&mut self, at: u64) -> Act {
        match self.since {
            Some(t0) if self.mode == TalkMode::Hold && at.saturating_sub(t0) >= WATCHDOG_MS => {
                self.since = None;
                Act::Send
            }
            _ => Act::Nothing,
        }
    }
    pub fn cancel(&mut self) {
        self.since = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_talks_while_down_and_sends_on_release_even_when_short() {
        let mut h = Ptt::new(TalkMode::Hold);
        assert_eq!(h.press(0), Act::Start);
        assert_eq!(h.press(10), Act::Nothing, "a repeat press while held");
        assert_eq!(h.release(50), Act::Send, "even a short hold sends in Hold");
        assert!(!h.listening());
        assert_eq!(h.release(60), Act::Nothing, "a stray release");
    }

    #[test]
    fn tap_starts_on_one_press_and_sends_on_the_next_whatever_the_release() {
        let mut t = Ptt::new(TalkMode::Tap);
        assert_eq!(t.press(0), Act::Start);
        assert_eq!(t.release(40), Act::Nothing, "a short press stays on");
        assert!(t.listening());
        assert_eq!(
            t.release(2_000),
            Act::Nothing,
            "a long press stays on too: there is no hold in Tap"
        );
        assert_eq!(t.press(3_000), Act::Send);
        assert_eq!(t.release(3_050), Act::Nothing);
        assert!(!t.listening());
        let mut h = Ptt::new(TalkMode::Hold);
        h.press(0);
        assert_eq!(h.release(40), Act::Send);
    }

    #[test]
    fn the_watchdog_ends_a_lost_hold_and_leaves_a_tap_alone() {
        let mut w = Ptt::new(TalkMode::Hold);
        w.press(0);
        assert_eq!(w.tick(WATCHDOG_MS - 1), Act::Nothing);
        assert_eq!(w.tick(WATCHDOG_MS), Act::Send);
        let mut l = Ptt::new(TalkMode::Tap);
        l.press(0);
        assert_eq!(l.tick(WATCHDOG_MS * 2), Act::Nothing);
        l.cancel();
        assert!(!l.listening());
        assert_eq!(
            l.press(1),
            Act::Start,
            "after a cancel the next press starts afresh"
        );
    }

    #[test]
    fn there_is_one_mode_and_the_old_both_mode_loads_as_tap() {
        assert_eq!(TalkMode::default(), TalkMode::Tap);
        assert_eq!(
            serde_json::from_str::<TalkMode>("\"hold_or_tap\"").unwrap(),
            TalkMode::Tap
        );
        assert_eq!(
            serde_json::from_str::<TalkMode>("\"hold\"").unwrap(),
            TalkMode::Hold
        );
        assert_eq!(serde_json::to_string(&TalkMode::Tap).unwrap(), "\"tap\"");
        assert!(serde_json::from_str::<TalkMode>("\"both\"").is_err());
    }
}
