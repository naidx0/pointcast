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

## The stated edge

- Out of scope: cloud voices, voice cloning, phone numbers or real telephony, and singing or sound effects.
- Windows code is compiled under `cfg(windows)` and unverified until it runs on a real PC.
