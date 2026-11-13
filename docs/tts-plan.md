# Pointcast talks back

Local only (owner, 2026-11-12). Free: no ElevenLabs, no paid voice API, nothing leaves the computer for speech.

**End state.** Ask can answer out loud in a natural voice, sentence by sentence as the answer streams in, and it stops
the moment you start talking or press the Ask key. With Call mode on, it feels like a phone call: you talk, it answers,
it listens again, with no key presses. It runs free and offline on macOS and Windows.

## How it works

- **Speak as it streams.** Answer text is cut into sentences as it arrives (no waiting for the whole answer). Each
  sentence is spoken as soon as it is complete, and Markdown, code blocks and links are read sensibly or skipped.
- **Two voices, both free and local.**
  - **System voice (first):** macOS `AVSpeechSynthesizer` (the Siri and Premium voices the person has downloaded),
    Windows `SpeechSynthesizer` (WinRT). Nothing to download, so it works on day one.
  - **Natural voice (next):** Kokoro-82M (Apache-2.0), run on the computer through sherpa-onnx, downloaded once like
    the speech model is today. Near paid quality, fast on a CPU.
- **Barge-in.** Speech stops at once when the microphone hears the person (the existing level meter), when the Ask key
  is pressed, or with Esc. Nothing already spoken is repeated.
- **Call mode.** Talk, it answers aloud, then it listens again with voice-activity detection to know when you've
  finished. One switch; off by default.

## Slices

| Slice | What | Gate |
|---|---|---|
| T1. System voice | `voice/speak/`: a sentence splitter (pure, tested), a speaker trait with macOS and Windows system voices, "Read answers aloud" in Settings with a voice and rate choice, speaking streamed Ask answers, and stop on key, Esc or the person's voice | Tests for the splitter (streaming, abbreviations, code, lists) and for barge-in with a fake speaker; a hand-run on this Mac that speaks a streamed answer and stops when talked over |
| T2. Natural voice | Kokoro through sherpa-onnx, downloaded on demand with a checksum, played through cpal, as a choice beside the system voice | A probe that times the first sentence's audio (target under 400 ms on this Mac); voice choice tests |
| T3. Call mode | Voice-activity detection ends your turn; it answers aloud and listens again; a visible "in a call" state in the capsule | A hand-run 3-turn call on this Mac with barge-in |

**First slice:** T1, now.

**T2 runtime (2026-11-13).** Kokoro runs in the app through the sherpa-onnx 2.0 C API that speech already downloads and pins per OS (macOS arm64, Windows x64): no new engine, no native build step, no helper process, nothing installed system-wide, Apache-2.0 throughout.
Not `sherpa-rs` (links a second sherpa into every build, ~30 MB, beside the downloaded one); not the prebuilt CLI (a process per sentence reloads the 326 MB model); not `ort` with espeak-ng (GPL-3.0, which the MIT app cannot link; sherpa-onnx 2.0 dropped it for that reason).
Without espeak, words become phonemes in Pointcast from the model's own lexicon (`voice/kokoro/g2p.rs`): numbers, acronyms, compounds and unknown words are said, never dropped.
Model: `kokoro-multi-lang-v1_0` at full precision, pinned by URL and SHA-256: a 350 MB download (plus the 9 MB runtime if speech has not fetched it), 366 MB kept. Its int8 version (132 MB) made audio slower than it plays on an Apple M5.
First audio on this Mac (M5, warm voice, speakers awake): 394 ms from a sentence to the device; a sentence plays while the next is made, and a long one starts after its first clause.

**T3 runtime (2026-11-13).** End of turn is Silero VAD v5 (MIT, 2.3 MB, pinned by URL, size and SHA-256) run by the
same sherpa-onnx 2.0 runtime through its VAD C API, downloaded at the first call; until it is here (or with no runtime)
a level-only detector stands in. Each 32 ms window is speech or not; `voice/call/turn.rs` makes turns of it: 700 ms of
silence ends a turn, under 250 ms of speech is none. One microphone stays open for the call (`audio::Feed`, 4 s kept) and
each turn records from it, so a turn starts 150 ms before its first word. While it speaks, the person barges in after
160 ms only when 10 dB over its own voice through the speakers (learnt from its first 250 ms of sound, then slowly).
Hand-run on recorded voices (`say -o`, nothing played): three turns and a barge-in, the barge-in's recording starting
153 ms before its first sound, its own voice at -24 dB never stopping it.
Follow-ups: macOS voice processing (echo cancellation) on the call's microphone; an optional Call shortcut needs a
setting and a Settings field.

## The stated edge

- Out of scope: cloud voices, voice cloning, phone numbers or real telephony, and singing or sound effects.
- Windows code is compiled under `cfg(windows)` and unverified until it runs on a real PC.
