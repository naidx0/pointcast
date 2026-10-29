//! A global shortcut: modifiers plus one key (`A`–`Z`, `0`–`9`, `F1`–`F24`, `Space`).
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Cmd on macOS, the Windows / Super key elsewhere.
    pub meta: bool,
    pub key: String,
}

const META: &str = if cfg!(target_os = "macos") {
    "Cmd"
} else {
    "Win"
};

/// Session start/stop: Ctrl+Shift+Space, Cmd+Shift+Space on macOS.
pub fn default_session() -> Chord {
    Chord {
        ctrl: !cfg!(target_os = "macos"),
        meta: cfg!(target_os = "macos"),
        shift: true,
        key: "Space".into(),
        ..Chord::default()
    }
}

/// Full screenshot during a session. Avoids macOS's Cmd+Shift+3.
pub fn default_full() -> Chord {
    Chord {
        ctrl: true,
        shift: true,
        key: "1".into(),
        ..Chord::default()
    }
}

/// Region / annotate overlay. Avoids macOS's Cmd+Shift+4.
pub fn default_region() -> Chord {
    Chord {
        ctrl: true,
        shift: true,
        key: "2".into(),
        ..Chord::default()
    }
}

/// Zoom crop around the pointer.
pub fn default_zoom() -> Chord {
    Chord {
        ctrl: true,
        shift: true,
        key: "Z".into(),
        ..Chord::default()
    }
}

fn norm_key(k: &str) -> Option<String> {
    let up = k.trim().to_ascii_uppercase();
    match up.as_str() {
        "SPACE" => return Some("Space".into()),
        s if s.len() == 1 && s.chars().all(|c| c.is_ascii_alphanumeric()) => return Some(up),
        _ => {}
    }
    let n: u8 = up.strip_prefix('F')?.parse().ok()?;
    (1..=24).contains(&n).then(|| format!("F{n}"))
}

impl Chord {
    /// `Ctrl+Shift+Space`, `cmd+shift+space`, `Control + Alt + F13`. Exactly one key.
    pub fn parse(s: &str) -> Result<Chord, String> {
        let mut c = Chord::default();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => c.ctrl = true,
                "alt" | "option" | "opt" => c.alt = true,
                "shift" => c.shift = true,
                "win" | "cmd" | "command" | "meta" | "super" => c.meta = true,
                _ => {
                    let k = norm_key(part).ok_or_else(|| format!("{part} cannot be bound"))?;
                    if !c.key.is_empty() {
                        return Err("One key only".into());
                    }
                    c.key = k;
                }
            }
        }
        if c.key.is_empty() {
            return Err("Needs a key".into());
        }
        Ok(c)
    }

    pub fn keys(&self) -> Vec<String> {
        let mut v: Vec<String> = [
            (self.ctrl, "Ctrl"),
            (self.alt, "Alt"),
            (self.shift, "Shift"),
            (self.meta, META),
        ]
        .into_iter()
        .filter(|(on, _)| *on)
        .map(|(_, k)| k.to_string())
        .collect();
        if !self.key.is_empty() {
            v.push(self.key.clone());
        }
        v
    }

    pub fn label(&self) -> String {
        self.keys().join("+")
    }

    fn mods(&self) -> usize {
        [self.ctrl, self.alt, self.shift, self.meta]
            .iter()
            .filter(|x| **x)
            .count()
    }

    /// Tauri global-shortcut accelerator (`Control+Shift+Space`, `Command+Shift+Space`).
    pub fn accelerator(&self) -> String {
        let mut v = vec![];
        if self.ctrl {
            v.push("Control");
        }
        if self.alt {
            v.push("Alt");
        }
        if self.shift {
            v.push("Shift");
        }
        if self.meta {
            v.push(if cfg!(target_os = "macos") {
                "Command"
            } else {
                "Super"
            });
        }
        v.push(self.key.as_str());
        v.join("+")
    }
}

/// Chords the picker refuses, with the reason shown in Settings.
pub fn reserved() -> Vec<(&'static str, &'static str)> {
    reserved_on(cfg!(target_os = "macos"))
}

fn reserved_on(mac: bool) -> Vec<(&'static str, &'static str)> {
    if mac {
        vec![
            ("Cmd+Shift+3", "macOS: screenshot"),
            ("Cmd+Shift+4", "macOS: screenshot"),
            ("Cmd+Shift+5", "macOS: screenshot"),
            ("Cmd+Space", "macOS: Spotlight"),
            ("Ctrl+Space", "macOS: input source"),
        ]
    } else {
        vec![
            ("Ctrl+Space", "Windows: input method"),
            ("Alt+Space", "Windows: window menu"),
            ("Alt+F4", "Windows: close window"),
            ("Ctrl+Shift+S", "Save as in most apps"),
        ]
    }
}

pub fn refusal(c: &Chord) -> Option<String> {
    refusal_on(c, cfg!(target_os = "macos"))
}

fn refusal_on(c: &Chord, mac: bool) -> Option<String> {
    let f_high = c
        .key
        .strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| n >= 13);
    if c.mods() == 0 && !f_high {
        return Some("Add Ctrl, Alt or Shift".into());
    }
    if c.mods() + 1 > 4 {
        return Some("Four keys at most".into());
    }
    if !mac {
        if c.ctrl && c.alt {
            return Some("Ctrl+Alt types AltGr letters".into());
        }
        if c.meta {
            return Some("Windows: Win shortcuts".into());
        }
    } else if c.alt && !c.ctrl && !c.meta {
        return Some("macOS 15 refuses Option or Option+Shift alone: add Ctrl or Cmd".into());
    }
    let l = c.label();
    reserved_on(mac)
        .into_iter()
        .find(|(k, _)| Chord::parse(k).is_ok_and(|r| r == *c) || *k == l)
        .map(|(_, why)| why.to_string())
}

/// Record-a-chord: feed it the keys held down at each poll.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Ctrl,
    Alt,
    Shift,
    Meta,
    Esc,
    Other(String),
}

#[derive(Debug, Default)]
pub struct Recorder {
    seen: Chord,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Step {
    Live(Vec<String>),
    Done(Chord),
    Cancel,
}

impl Recorder {
    pub fn feed(&mut self, down: &[Key]) -> Step {
        if down.contains(&Key::Esc) {
            return Step::Cancel;
        }
        if down.is_empty() {
            if !self.seen.key.is_empty() {
                return Step::Done(std::mem::take(&mut self.seen));
            }
            self.seen = Chord::default();
            return Step::Live(vec![]);
        }
        for k in down {
            match k {
                Key::Ctrl => self.seen.ctrl = true,
                Key::Alt => self.seen.alt = true,
                Key::Shift => self.seen.shift = true,
                Key::Meta => self.seen.meta = true,
                Key::Other(n) => {
                    if let Some(n) = norm_key(n) {
                        self.seen.key = n;
                    }
                }
                Key::Esc => {}
            }
        }
        Step::Live(self.seen.keys())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(s: &str) -> Chord {
        Chord::parse(s).unwrap()
    }

    #[test]
    fn chords_parse_in_any_spelling_and_print_one_way() {
        assert_eq!(c("ctrl + shift + space").label().contains("Space"), true);
        assert_eq!(c("Shift+Control+a").label(), "Ctrl+Shift+A");
        assert_eq!(c("Alt+f13").keys(), ["Alt", "F13"]);
        assert_eq!(c("ctrl+space").key, "Space");
        for bad in ["Ctrl+Shift", "", "Ctrl+1+2", "Ctrl+Tab", "Ctrl+F25", "Ctrl+é"] {
            assert!(Chord::parse(bad).is_err(), "{bad}");
        }
        let d = default_session();
        if cfg!(target_os = "macos") {
            assert_eq!(d.label(), "Shift+Cmd+Space");
        } else {
            assert_eq!(d.label(), "Ctrl+Shift+Space");
        }
        assert_eq!(refusal(&d), None);
        assert!(d.accelerator().contains("Space"));
    }

    #[test]
    fn the_picker_refuses_what_breaks_a_rule_or_is_already_used() {
        assert!(refusal(&c("A")).is_some());
        assert_eq!(refusal(&c("F13")), None);
        if !cfg!(target_os = "macos") {
            assert!(refusal(&c("Ctrl+Alt+S")).unwrap().contains("AltGr"));
            assert!(refusal(&c("Win+Shift+S")).is_some());
            assert_eq!(refusal(&c("Ctrl+Shift+1")), None);
            assert_eq!(refusal(&c("Ctrl+Shift+Space")), None);
        }
        for (k, why) in reserved() {
            assert_eq!(refusal(&c(k)).as_deref(), Some(why), "{k}");
        }
    }

    #[test]
    fn macos_refuses_option_and_option_shift_alone_up_front() {
        for k in ["Alt+K", "Option+Shift+K", "Alt+Shift+Space", "Alt+F13"] {
            let why = refusal_on(&c(k), true).unwrap_or_default();
            assert!(why.contains("Option"), "{k}: {why:?}");
            assert_eq!(refusal_on(&c(k), false), None, "{k} is fine on Windows");
        }
        for k in ["Ctrl+Alt+K", "Cmd+Alt+K", "Cmd+Shift+Alt+K", "Ctrl+Shift+K", "F13"] {
            assert_eq!(refusal_on(&c(k), true), None, "{k}");
        }
        assert!(refusal_on(&c("Ctrl+Alt+K"), false).is_some(), "Windows still refuses AltGr chords");
    }

    #[test]
    fn the_recorder_commits_on_release_and_cancels_on_esc() {
        use Key::*;
        let k = |s: &str| Other(s.into());
        let mut r = Recorder::default();
        assert_eq!(r.feed(&[Ctrl]), Step::Live(vec!["Ctrl".into()]));
        assert_eq!(
            r.feed(&[Ctrl, Shift, k("2")]),
            Step::Live(vec!["Ctrl".into(), "Shift".into(), "2".into()])
        );
        assert!(matches!(r.feed(&[Shift]), Step::Live(_)));
        assert_eq!(r.feed(&[]), Step::Done(c("Ctrl+Shift+2")));
        let mut r = Recorder::default();
        r.feed(&[Ctrl, k("4")]);
        assert_eq!(r.feed(&[Ctrl, Esc]), Step::Cancel);
    }
}
