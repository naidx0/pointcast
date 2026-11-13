//! The Mac's own voices, through AVSpeechSynthesizer: the standard voices and the Premium and Enhanced ones the person
//! downloaded in System Settings › Accessibility › Spoken Content. Nothing leaves the computer.

use super::{Done, How, Speaker, VoiceInfo};
use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::{AnyObject, NSObject};
use objc2::{class, define_class, msg_send, AnyThread, DefinedClass};
use std::ffi::{c_char, CStr, CString};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// AVSpeechSynthesisVoiceQuality: 2 is Enhanced, 3 Premium.
const ENHANCED: isize = 2;
/// AVSpeechBoundaryImmediate.
const IMMEDIATE: isize = 0;

/// The rate AVSpeechSynthesizer takes for `rate` (1 is normal). Its own scale runs from 0 to 1 with 0.5 normal, and
/// above that it speeds up far faster than it reads, so 2 is 0.7.
fn av_rate(rate: f32) -> f32 {
    let r = rate.clamp(0.5, 2.0);
    if r <= 1.0 {
        0.5 * r
    } else {
        0.5 + (r - 1.0) * 0.2
    }
}

/// How often the watcher looks whether the synthesizer is still speaking.
const LOOK: Duration = Duration::from_millis(15);
/// A sentence that has not started to sound by then never will.
const NEVER: Duration = Duration::from_secs(5);

struct Ivars {
    /// The sentence being said: its number, its utterance (by address), and what to call when it ends.
    now: Mutex<Option<(u64, usize, Done)>>,
    /// The number of the last sentence that started to sound, and when; and how many have.
    began: Mutex<(u64, Option<Instant>)>,
    begun: AtomicU64,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and Listener does not implement Drop.
    #[unsafe(super(NSObject))]
    #[ivars = Ivars]
    struct Listener;

    impl Listener {
        #[unsafe(method(speechSynthesizer:didStartSpeechUtterance:))]
        fn started(&self, _synth: &AnyObject, utterance: &AnyObject) {
            if let Some(n) = self.number_of(utterance) {
                self.start(n);
            }
        }

        #[unsafe(method(speechSynthesizer:didFinishSpeechUtterance:))]
        fn finished(&self, _synth: &AnyObject, utterance: &AnyObject) {
            if let Some(n) = self.number_of(utterance) {
                self.end(n);
            }
        }

        #[unsafe(method(speechSynthesizer:didCancelSpeechUtterance:))]
        fn cancelled(&self, _synth: &AnyObject, utterance: &AnyObject) {
            if let Some(n) = self.number_of(utterance) {
                self.end(n);
            }
        }
    }
);

/// A synthesizer tells its listener on the main queue, which runs in the app but not on a test's thread; a watcher
/// that looks at `isSpeaking` ends a sentence too, whichever comes first.
impl Listener {
    fn new() -> Retained<Self> {
        let ivars = Ivars { now: Mutex::new(None), began: Mutex::new((0, None)), begun: AtomicU64::new(0) };
        let this = Self::alloc().set_ivars(ivars);
        unsafe { msg_send![super(this), init] }
    }

    /// The number of the sentence being said, when `utterance` is its.
    fn number_of(&self, utterance: &AnyObject) -> Option<u64> {
        let at = utterance as *const AnyObject as usize;
        self.ivars().now.lock().unwrap().as_ref().filter(|n| n.1 == at).map(|n| n.0)
    }

    fn saying(&self, n: u64) -> bool {
        self.ivars().now.lock().unwrap().as_ref().is_some_and(|x| x.0 == n)
    }

    /// Sentence `n` started to sound; told once.
    fn start(&self, n: u64) {
        let mut b = self.ivars().began.lock().unwrap();
        if b.0 != n {
            *b = (n, Some(Instant::now()));
            self.ivars().begun.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Sentence `n` ended or was cut: its `done` is called, once.
    fn end(&self, n: u64) {
        let done = {
            let mut now = self.ivars().now.lock().unwrap();
            if now.as_ref().is_some_and(|x| x.0 == n) {
                now.take().map(|x| x.2)
            } else {
                None
            }
        };
        if let Some(done) = done {
            done();
        }
    }
}

/// A sentence's synthesizer, and the listener every synthesizer tells when a sentence starts and ends. A synthesizer
/// moves on to its next sentence only through the main queue, so each sentence gets a fresh one: it starts in about
/// 50 ms on any thread, and the main thread being busy never holds speech up.
#[derive(Clone)]
struct Held {
    synth: Retained<AnyObject>,
    listener: Retained<Listener>,
}

// SAFETY: AVSpeechSynthesizer takes its messages from any thread, and the listener keeps its state behind locks.
unsafe impl Send for Held {}

impl Held {
    fn new(listener: Retained<Listener>) -> Held {
        // SAFETY: AVFoundation is linked (permissions.rs), and `setDelegate:` keeps a weak reference that `Held` outlives.
        let synth: Retained<AnyObject> = unsafe { msg_send![class!(AVSpeechSynthesizer), new] };
        unsafe {
            let _: () = msg_send![&*synth, setDelegate: &*listener];
        }
        Held { synth, listener }
    }

    /// Watch sentence `n` until it ends: started once `isSpeaking` turns on, ended once it turns off again (or when it
    /// never turned on).
    fn watch(self, n: u64) {
        let from = Instant::now();
        let mut heard = false;
        while self.listener.saying(n) {
            std::thread::sleep(LOOK);
            // SAFETY: a plain property read, from any thread.
            let speaking: bool = unsafe { msg_send![&*self.synth, isSpeaking] };
            if speaking && !heard {
                heard = true;
                self.listener.start(n);
            } else if !speaking && (heard || from.elapsed() >= NEVER) {
                self.listener.end(n);
            }
        }
    }
}

#[derive(Default)]
pub struct Mac {
    /// The sentence being said, or the last one.
    held: Mutex<Option<Held>>,
    /// Numbers the sentences.
    said: AtomicU64,
}

impl Mac {
    /// How many sentences have started to sound, and when the last one did.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn begun(&self) -> (u64, Option<Instant>) {
        let g = self.held.lock().unwrap();
        g.as_ref().map_or((0, None), |h| {
            let i = h.listener.ivars();
            (i.begun.load(Ordering::SeqCst), i.began.lock().unwrap().1)
        })
    }
}

impl Speaker for Mac {
    fn speak(&self, sentence: &str, how: &How, done: Done) -> Result<(), String> {
        autoreleasepool(|_| {
            let text = ns_string(sentence)?;
            // SAFETY: plain AVSpeechUtterance and AVSpeechSynthesisVoice messages with their documented types.
            let utterance: Option<Retained<AnyObject>> = unsafe { msg_send![class!(AVSpeechUtterance), speechUtteranceWithString: &*text] };
            let utterance = utterance.ok_or_else(|| "The sentence could not be made ready to say.".to_string())?;
            unsafe {
                if let Some(id) = how.voice.as_deref() {
                    let id = ns_string(id)?;
                    let voice: *mut AnyObject = msg_send![class!(AVSpeechSynthesisVoice), voiceWithIdentifier: &*id];
                    if !voice.is_null() {
                        let _: () = msg_send![&*utterance, setVoice: voice];
                    }
                }
                let _: () = msg_send![&*utterance, setRate: av_rate(how.rate)];
            }
            let n = self.said.fetch_add(1, Ordering::SeqCst) + 1;
            let mut g = self.held.lock().unwrap();
            let h = Held::new(g.as_ref().map_or_else(Listener::new, |h| h.listener.clone()));
            *h.listener.ivars().now.lock().unwrap() = Some((n, Retained::as_ptr(&utterance) as usize, done));
            unsafe {
                let _: () = msg_send![&*h.synth, speakUtterance: &*utterance];
            }
            *g = Some(h.clone());
            drop(g);
            std::thread::Builder::new().name("speak-watch".into()).spawn(move || h.watch(n)).map_err(|e| e.to_string())?;
            Ok(())
        })
    }

    fn stop(&self) {
        let g = self.held.lock().unwrap();
        if let Some(h) = g.as_ref() {
            autoreleasepool(|_| unsafe {
                let _: bool = msg_send![&*h.synth, stopSpeakingAtBoundary: IMMEDIATE];
            });
        }
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        autoreleasepool(|_| unsafe {
            let all: Option<Retained<AnyObject>> = msg_send![class!(AVSpeechSynthesisVoice), speechVoices];
            let all = all.ok_or_else(|| "The Mac listed no voices.".to_string())?;
            let n: usize = msg_send![&*all, count];
            let mut found = Vec::with_capacity(n);
            for i in 0..n {
                let v: *mut AnyObject = msg_send![&*all, objectAtIndex: i];
                if v.is_null() {
                    continue;
                }
                let quality: isize = msg_send![v, quality];
                found.push(VoiceInfo {
                    id: string(msg_send![v, identifier]),
                    name: string(msg_send![v, name]),
                    lang: string(msg_send![v, language]),
                    natural: quality >= ENHANCED,
                });
            }
            let here = string(msg_send![class!(AVSpeechSynthesisVoice), currentLanguageCode]);
            Ok(super::ranked(found, &here))
        })
    }
}

fn ns_string(s: &str) -> Result<Retained<AnyObject>, String> {
    let c = CString::new(s.replace('\0', " ")).map_err(|e| e.to_string())?;
    // SAFETY: `c` is a valid NUL-terminated UTF-8 string for the length of the call.
    let ns: Option<Retained<AnyObject>> = unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] };
    ns.ok_or_else(|| "The text could not be passed to the voice.".to_string())
}

/// An NSString's text; empty for none.
unsafe fn string(ns: *mut AnyObject) -> String {
    if ns.is_null() {
        return String::new();
    }
    let p: *const c_char = msg_send![ns, UTF8String];
    if p.is_null() {
        String::new()
    } else {
        CStr::from_ptr(p).to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::av_rate;

    #[test]
    fn the_rate_maps_to_avspeech_with_normal_at_its_middle_and_never_past_its_ends() {
        assert_eq!(av_rate(1.0), 0.5);
        assert_eq!(av_rate(0.5), 0.25);
        assert!((av_rate(2.0) - 0.7).abs() < 1e-6);
        assert_eq!(av_rate(0.1), av_rate(0.5));
        assert_eq!(av_rate(9.0), av_rate(2.0));
        assert!(av_rate(1.25) > 0.5 && av_rate(1.25) < av_rate(1.5));
    }
}

