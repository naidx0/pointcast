# Smarter captures: zoom, outlines, key frames

**End state.** Every capture shows what the person pointed at without extra work: the spot they
paused on is ringed, close-ups centre on it and stay readable, the control under the pointer is
outlined and named in the prompt, and (when they opt in) the moments they talked about are
picked from the recording as key frames. Each behaviour has a setting.

## How it works today

One clock from session start. Speech pieces are `{t_ms, end_ms, text}`, cut at 0.9 s pauses.
Captures are `{t_ms, png, labels}`. `prompt::compose` puts `[image n]` at the word that far
through the piece; Moonshine gives no per-word times, so the position is a linear estimate.

## Slices

| | Slice | Gate |
| --- | --- | --- |
| S0 | A region capture is stamped at the freeze, not at Enter (`commit_region` uses `t_ms()` now, so 8 s of annotating puts the marker 8 s late). | Unit test: freeze at t, commit at t+8 s, capture `t_ms == t`. |
| S1 | Pointer trail: while recording, sample the cursor at ~30 Hz into a ring buffer (`t_ms, x, y, monitor`; positions only). Pure functions `dwell(trail)` (within ~12 px for ≥ 500 ms) and `to_image(point, monitor, scale)`. `Capture` gains `pointer: Option<Point>` and `trail: Vec<Point>` (last ~3 s), `serde(default)` so old history loads. | Unit tests: dwell found / not found / last one wins; mapping at scale 1, 1.5, 2 and on a negative-origin second monitor; old history JSON still loads. |
| S2 | Zoom and auto outline: a full capture rings the last dwell; region mode gets the dwell as an editable pre-placed shape; close-up centres on the dwell of the last 1.5 s, else the pointer. Options: sharpen 2× (`imageops::resize`), close-up centres on Pointer / Where I paused / Window under pointer. | Unit tests for centring and window pick; live Windows screenshot of a ring and a close-up on a dwell. |
| S5 | Element outlines: at capture, ask the OS for the element under the dwell or pointer (macOS `AXUIElementCopyElementAtPosition`; Windows UIA `ElementFromPoint`, bounding rect, name, control type). Outline it; marker reads `[image 2: button "Place order"; 1=too far left]`. | Unit test for marker text; one ignored real UIA test on Windows; live screenshot. macOS unverified until hardware or CI. |
| S3 | Spoken anchors: after stop, estimate word times (check sherpa-onnx token timestamps for Moonshine first, else linear); for pointing words (this, here, that, these, those) ring the trail position on the nearest capture. Option: Off / Ring / Ring + close-up. | Unit tests for word timing and anchor pick. |
| S4 | Key frames (opt-in, memory only): 1 fps buffer of the pointer's monitor, ≤ 1600 px, PNG, last 120 s, hide-list masked, dropped at stop unless kept. At stop pick frames at pointing words, dwell ends and big changes (> ~12 % pixels), ≥ 2 s apart, at most 6, each with its outline and trail; marked `auto: true` so History can drop one and re-copy. | Unit tests for selection on synthetic diffs; memory bound test. |
| S6 | Re-annotate from History: clicking a picture reopens it in the overlay; committing replaces it in the entry and re-copies. | Live walk on Windows. |

**Order:** S0 → S1 → S2 → S5 → S3 → S4 → S6. **First shippable slice:** S0 + S1.

## Settings (Capture pane)

Close-up size (exists) · Sharpen close-ups 2× · Close-up centres on: Pointer / Where I paused /
Window · Mark where I point: Off / Ring / Ring + trail · Outline the control under the pointer ·
Pick key frames for me (off, with a privacy note).

## Open for Max

- **Key frames default.** Recommended OFF (it keeps a rolling picture of the screen in memory
  while recording). Max decides.

## Stated edge

- No audio or screen frames are written to disk beyond what a finished prompt already keeps.
- No cloud vision or OCR; outlines come from the OS accessibility tree only.
- macOS paths are compile-checked here and verified only on a Mac or in CI.
