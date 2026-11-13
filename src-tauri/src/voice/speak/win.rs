//! Windows' own voices, through Windows.Media.SpeechSynthesis, played with a MediaPlayer. Nothing leaves the computer.
//! Compiled for Windows only, and unverified until it runs on a real PC.

use super::{Done, How, Speaker, VoiceInfo};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{IInspectable, HSTRING};
use windows::Foundation::TypedEventHandler;
use windows::Media::Core::MediaSource;
use windows::Media::Playback::{MediaPlayer, MediaPlayerFailedEventArgs};
use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

/// What to call when the sentence ends, taken by whichever of the end, a failure or a stop comes first.
type Slot = Arc<Mutex<Option<Done>>>;

fn call(slot: &Slot) {
    let done = slot.lock().unwrap().take();
    if let Some(done) = done {
        done();
    }
}

/// The player of the sentence being said.
struct Playing {
    player: MediaPlayer,
    slot: Slot,
}

#[derive(Default)]
pub struct Win {
    now: Mutex<Option<Playing>>,
    /// Counts stops: a sentence still being made when one comes is never played.
    stops: AtomicU64,
}

impl Speaker for Win {
    fn speak(&self, sentence: &str, how: &How, done: Done) -> Result<(), String> {
        // SAFETY: joins this thread to the multithreaded apartment; a thread already in one keeps it.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let stops = self.stops.load(Ordering::SeqCst);
        let slot: Slot = Arc::new(Mutex::new(Some(done)));
        let made = (|| -> windows::core::Result<MediaPlayer> {
            let synth = SpeechSynthesizer::new()?;
            if let Some(v) = how.voice.as_deref().and_then(find) {
                synth.SetVoice(&v)?;
            }
            synth.Options()?.SetSpeakingRate(how.rate.clamp(0.5, 2.0) as f64)?;
            let stream = synth.SynthesizeTextToStreamAsync(&HSTRING::from(sentence))?.get()?;
            let source = MediaSource::CreateFromStream(&stream, &stream.ContentType()?)?;
            let player = MediaPlayer::new()?;
            player.SetAutoPlay(false)?;
            player.SetSource(&source)?;
            let s = slot.clone();
            player.MediaEnded(&TypedEventHandler::<MediaPlayer, IInspectable>::new(move |_, _| {
                call(&s);
                Ok(())
            }))?;
            let s = slot.clone();
            player.MediaFailed(&TypedEventHandler::<MediaPlayer, MediaPlayerFailedEventArgs>::new(move |_, _| {
                call(&s);
                Ok(())
            }))?;
            Ok(player)
        })();
        let player = made.map_err(|e| e.to_string())?;
        let mut now = self.now.lock().unwrap();
        if self.stops.load(Ordering::SeqCst) != stops {
            call(&slot);
            return Ok(());
        }
        player.Play().map_err(|e| e.to_string())?;
        *now = Some(Playing { player, slot });
        Ok(())
    }

    fn stop(&self) {
        self.stops.fetch_add(1, Ordering::SeqCst);
        let playing = self.now.lock().unwrap().take();
        if let Some(p) = playing {
            let _ = p.player.Pause();
            let _ = p.player.Close();
            call(&p.slot);
        }
    }

    fn voices(&self) -> Result<Vec<VoiceInfo>, String> {
        // SAFETY: as in `speak`.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let listed = (|| -> windows::core::Result<(Vec<VoiceInfo>, String)> {
            let all = SpeechSynthesizer::AllVoices()?;
            let mut found = Vec::new();
            for i in 0..all.Size()? {
                let v = all.GetAt(i)?;
                let name = v.DisplayName()?.to_string();
                found.push(VoiceInfo { id: v.Id()?.to_string(), natural: name.contains("Natural"), name, lang: v.Language()?.to_string(), download: None });
            }
            Ok((found, SpeechSynthesizer::DefaultVoice()?.Language()?.to_string()))
        })();
        let (found, here) = listed.map_err(|e| e.to_string())?;
        Ok(super::ranked(found, &here))
    }
}

/// The installed voice with this id.
fn find(id: &str) -> Option<VoiceInformation> {
    let all = SpeechSynthesizer::AllVoices().ok()?;
    let want = HSTRING::from(id);
    (0..all.Size().ok()?).filter_map(|i| all.GetAt(i).ok()).find(|v| v.Id().is_ok_and(|x| x == want))
}
