# Lean dictation plan

Base: `main` ca3e772 (v0.4.7). Program branch: `program/lean-dictation`. Written 2026-11-09 from five
investigations on the owner's PC (idle footprint, word audit on 8 real recordings, screenshot handoff,
UX review, formatting on 10 real transcripts).

## End state

Press, talk, point, paste: every word spoken after the press is in the prompt, the same audio always
gives the same text, the pictures arrive where an agent can open them, and a minute after the last
recording Pointcast is one 25 MB process again.

## What was measured

| Area | Finding | Evidence |
|---|---|---|
| Dictation | The GPU helper gives a different transcript on each run of the same audio: 133 of 1,342 words (9.9%) differ between two identical runs; 0 with flash attention off (`-nfa`), at 11-12x real time against 16-17x | 8 recordings, 7.56 min |
| Dictation | The microphone opens 1.0 to 2.3 s after the press in 4 of 10 recordings, with speech already under way; those words are never recorded | leading zeros in kept audio |
| Dictation | No gross loss elsewhere: the 600 ms gate dropped no utterance, the tail at stop lost nothing, piece cuts cost about 5 words a minute | word alignment |
| Dictation | Made-up pieces: "Thank you." 13 times and "Um..." 18 times in 100 recordings; `*sniff*`, `*sad music*` pass the filter; a few short pieces come out in another language | History scan |
| Lean | Idle with no recording is already the floor: 1 process, 22-35 MB, no WebView2 | process snapshots |
| Lean | For 10 minutes after a recording: the helper (0.9 GB VRAM, 1.5 GB committed) and the hidden capsule (6 WebView2 processes, 340 MB) stay | code + helper measured alone |
| Lean | A load outside a recording (Transcribe again, model change, GPU setup) is never freed until the next recording | code reading, not reproduced |
| Lean | Two loops wake forever: 400 ms (idle) and 5 s (GPU setup, already final on a set-up PC) | code |
| Handoff | Over MCP the model never sees the pictures: one 3.5 MB text block with base64 twice | mcp.rs, client limits |
| Handoff | Picture files are numbered in storage order, the prompt in time order; they live deep in app data (a path with a space on every Mac); sandboxed agents cannot read there | prompt.rs, history.rs |
| Formatting | Every piece is joined with one space, so a 15-minute dictation pastes as one block; rules kept 100% of words and every marker; small models (1.2B and under) summarise, repeat or answer the text | 10 transcripts, 5 models |
| UX | Setup blocks on the model download; the capsule never says the engine is loading; end notices cannot be clicked; Delete has no undo; credits miss Whisper, whisper.cpp, Parakeet | preview walk |

## Slices

Each slice is one branch off the program branch, gated on its own, merged back when green.

| # | Slice | Items | Files | Gate |
|---|---|---|---|---|
| 1 | Same audio, same words | Start the helper with `-nfa`. Every `*...*` is not speech; `(...)` holding a sound word is dropped | `voice/whisper_server.rs` | A test with a fake server proves the argument is passed; unit tests on `clean()`; on this PC, the same piece posted 5 times returns one text |
| 2 | Free what is not in use | Track when the engine was last used and free it when loaded, idle for N and nothing busy. Default N: 1 minute (a stored 10 becomes 1 once). Options 1 / 5 / 10 / 30 / Never | `lib.rs` (idle loop, `free_due`), `session.rs` (load/unload), `settings.rs`, `Settings.tsx` | Tests: a load outside a recording is freed after N; a recording in between keeps it; the migration runs once |
| 3 | Nothing wakes when idle | Capsule and overlay destroyed 60 s after they hide. The GPU-setup loop ends when its answer is final. The idle loop sleeps until a phase change or the next deadline | `lib.rs`, `platform.rs`, `voice/upgrade.rs` | Tests on the deadline logic with an injected clock; measured on this PC: 1 process and 0 helper 70 s after a recording |
| 4 | Every word after the press | The microphone opens first, on its own thread, before anything else at the press; the capsule says "Getting ready… keep talking" until audio is flowing and never shows a recording state before that; the first frame of a cold capsule is the true phase | `session.rs` (begin), `voice/audio.rs`, `Indicator.tsx`, `platform.rs` | Test: `t0` to first audio under 150 ms with a fake device that opens slowly elsewhere; on this PC, 10 recordings with leading silence under 250 ms |
| 5 | Pictures an agent can open | Handoff folder `~/Pointcast/<date_time>/01.png…` numbered in prompt order, with `prompt.md` and `manifest.json`; short `[image n]` markers in the text and one picture list at the end; folders pruned after 7 days or beyond the newest 30 | `prompt.rs`, `session.rs`, `history.rs`, `settings.rs` | Unit tests (numbering, list, pruning); one real recording pasted into Claude Code where the agent opens both pictures |
| 6 | Pictures over MCP | Protocol 2025-06-18; `pointcast_latest`, `pointcast_get`, `pointcast_list`, `pointcast_shot`; text, then per picture a caption and an image block within a token budget; `copy_to` drops the pictures into the agent's project folder | `mcp.rs`, agent setup in `lib.rs`, README | A Claude Code tool call shows the pictures inline under 25k tokens |
| 7 | First run without a wait | The recommended model starts downloading at Get Started; Continue is never disabled; keys and practice become one "Try it" step; credits complete | `Onboarding.tsx`, `Models.tsx`, `Settings.tsx` (About) | Preview walk on both skins; a fresh-machine install test reaches the practice step while the download runs |
| 8 | Notices you can act on | End notices open History on that entry and stay while hovered; "Paste what's done" from the start of transcribing; Delete has a 5 s Undo; Discard asks once | `Indicator.tsx`, `Result.tsx`, `lib.rs` (hide timing) | Preview walk; tests on the undo window |
| 9 | Short pieces heard in context | A piece under 2.5 s, or one that is only "Thank you." / "you" / "Um...", is decoded joined to the audio before it and only its new words kept; language is detected once per recording | `session.rs` (decode_ranges), `voice/stt.rs`, `voice/engine.rs` | The 11 suspect pieces from the audit: 2 of 2 made-up gone, 7 of 7 real kept; no new differences on the 8 recordings |
| 10 | Paragraphs, not one block | A rule-based formatter where the prompt is composed: a new paragraph at a pause of 1.2 s after a sentence end or any pause of 2.5 s, repeats of 1 to 3 words collapsed, a short filler list removed, first/second/third runs as a list; markers never touched; on by default | `prompt.rs`, `settings.rs`, `Settings.tsx` | On the 10 real transcripts: every content word kept in order, none added, every marker kept; unit tests on each rule |
| 11 | Tidy with a local model (opt-in) | A History action, never before the paste: Qwen3.5-2B (1.19 GiB, Apache-2.0) through a CI-built `llama-server` helper started on demand and killed after; markers stripped before the model and put back; each chunk checked (words kept 0.90 or more, at most 2 new words, length 0.85 to 1.05) or the rule text stays | `voice/` (helper), `models.json`, `Result.tsx`, a helper workflow | The 26 measured chunks pass the check; a summarising model is rejected; nothing of it runs when the action is not used |

### Next loop (from ROADMAP.md items 4 to 6 and nine real recordings on the program build)

Real use on the owner's PC, 9 recordings: audio starts 91 to 284 ms after the press (was 1.0 to 2.3 s in 4 of 10),
no stray "Thank you." or "Um..." piece, 10 to 16x real time.

| # | Slice | Items | Files | Gate |
|---|---|---|---|---|
| 12 | Mark-up without friction | A redact tool (a solid block over a region, burned into the picture, never recoverable from the file); a Cancel button beside Capture; dragging the box always moves it, so the Move tool goes | `Overlay.tsx`, the picture rasterizer | Unit test: the redacted pixels in the saved PNG are one colour; preview walk on both skins |
| 13 | Step markers and crop | A numbered step marker that counts up by itself (1, 2, 3) and is named in the picture list; crop after capture | `Overlay.tsx`, `image_ops.rs`, `prompt.rs` | Tests on numbering and the caption; the crop keeps mark positions |
| 14 | The app knows this computer | Settings › Speech shows the microphone in use with a live level and says when it is silent or slow to open, with the device named; each recording keeps how long its microphone took to open; one line for the graphics card, engine and measured speed | `Settings.tsx`, `session.rs`, `voice/audio.rs`, `history.rs` | Tests on the meta fields; preview walk; a real recording shows the fields |
| 15 | Agent setup in one click | Settings writes the user-scope MCP config for Claude Code, Cursor and Codex and checks it answers; an MCP conformance test in CI | `lib.rs` (agent setup), `Settings.tsx`, `mcp.rs` tests | The check passes against the running server; a wrong token is reported |
| 16 | Release 0.4.8 | Version, notes, README (size budget, handoff folder, tidy), a cross-platform pass: install tests on fresh macOS and Windows runners | release files | Draft release green; the owner's yes to publish |

## Order

1, 2, 3 touch different files and land first (1 is the smallest and the largest gain for dictation).
4 follows 3 (both change the capsule's lifecycle). 5 then 6. 7 and 8 are front-end and can run beside
5 and 6. 9 after 1 (it is measured against repeatable decoding). 10 after 5 (both change how the prompt is
composed). 11 last: models of 1.2B and under were worse than the rules alone, and the 2B model's gain is modest.

First shippable slice: **1**, started in the same session as this plan.

## Gates

A lane runs `cargo test --lib` and `npm run build` in its worktree when a build slot and 3 GB of RAM
are free; otherwise it pushes its branch and runs `ci.yml` there (Linux tests, macOS and Windows
builds). The program branch runs CI before it goes to `main`. A release is the owner's yes.

## The edge (not in this plan)

- A native (non-WebView) capsule window. It would mean dictation never starts WebView2; revisit after slice 3 is measured.
- A `pointcast` command-line tool: a GUI executable cannot print to a Windows console cleanly, and it has the same sandbox limits as the agent.
- Pasting pictures one by one into chat apps ("attach train"): waits on a manual paste test of what ChatGPT, Claude, Gemini and Grok do with the HTML images today.
- A previous-text prompt for each piece (177 to 152 differing words in one run, one inserted phrase): measure again after slice 1.
- Driver-level voice processing, code signing, a larger speech model.
- Capture's advanced settings moving under a disclosure, sentence-case labels on Windows, the overlay toolbar: listed in the UX review, scheduled after slice 8.
