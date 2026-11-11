# Live assistant plan (Pointcast 0.5)

Base: `program/lean-dictation` dee0184 (v0.4.8). Program branch: `program/live-assistant`. Written 2026-11-11
from two investigations: the provider layer (how to use the AI a person already pays for) and the live loop
(what Clicky, Cluely, Superwhisper, Wispr Flow, Granola and the vendors' desktop apps do, and what a study tool
needs that none of them gives).

## End state

Press Ask, say a question about what is on your screen, and the answer streams into a calm panel from the AI you
already pay for; the picture you pointed at, your marks and the whole conversation are kept as a session you can
resume, search and summarize with receipts, and a minute after you close it Pointcast is one small process again.

## Decisions

| Question | Decision | Why |
|---|---|---|
| First AI | The person's ChatGPT plan through OpenAI's own Codex CLI, driven over `codex app-server` (JSON lines on stdio) | The owner's pick. It is the documented surface for embedding: text deltas, a picture as a file path, cancel, sign-in state, models and plan limits, and Pointcast never touches a secret |
| Then | Claude through an unmodified Claude Code the person signed in to; then API keys and local models on one OpenAI-compatible adapter plus Anthropic's API | Same child-process spine for the two CLIs; keys cover OpenRouter, Gemini, Ollama, LM Studio and the offline case |
| Credentials | Pointcast never reads, stores or forwards another app's token, never proxies and never rotates accounts. Keys the person pastes live in the OS credential store | A test fails the build if provider source names a vendor credential file |
| Tools | Off. The assistant reads words and pictures and streams text. Every approval or tool request from a CLI is declined at once | A study answer needs no shell |
| Start of a question | The panel looks like listening at once; one frame is grabbed at the press, before the panel shows | Owner: show recording at once |
| Answers | Text first: markdown, code and math. Spoken answers are not in 0.5 | Owner: text first, voice later |
| What leaves the computer | The words of the question, the pictures of that question (after redaction, crop and fit), their captions. Never audio, History or a picture the person has not seen | First time Pointcast sends anything; README and onboarding copy change in the same release |
| Visibility | The panel is never content-protected: it shows in every screen share and recording. Nothing is built to hide Pointcast from other software | A visible study tool, test-locked |
| Capture | One deliberate picture per question, never a stream. The never-capture list applies because Ask uses `grab_frame` | Lean, and the person sees what is sent |
| Look | Wispr Flow and Superwhisper are the bar: calm, information-light, one quiet living shape for listening, thinking and answering; no spinner and text where a shape will do | Owner's direction; the motion and state spec is slice 5 |

One provider contract serves both reports:

```rust
pub enum Part { Text(String), Picture { path: PathBuf, w: u32, h: u32, mime: &'static str } }  // a file Pointcast wrote, already flattened
pub struct Turn { pub role: Role, pub parts: Vec<Part> }
pub enum Event { Delta(String), Usage(Usage), Limits(Limits), Done, Failed(Failure) }
pub enum Failure { NotInstalled, SignedOut, Limit { resets: Option<i64> }, Offline, Refused, NoPictures, Stopped, Broken(String) }
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn caps(&self) -> Caps;                 // images, max_images, max_edge, offline, uses_gpu
    fn health(&self) -> Health;             // spends nothing
    fn warm(&self);
    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure>;
}
pub trait Chat: Send { fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<(), Failure>; fn cancel(&self); fn close(&mut self); }
```

`open` takes the earlier turns so a session resumed from disk has its context whichever side keeps the thread;
`ask` takes only the new turn. A turn ends with exactly one `Done` or `Failed`.

## Slices

Each slice is one branch off the program branch, gated on its own, merged back when green. No slice sits behind a flag.

| # | Slice | Items | Files | Gate |
|---|---|---|---|---|
| 0 | Usage stats | `stats.json` beside History with running totals, days (400 kept), a 7 by 24 grid, streaks and the newest 100 entry ids, so Transcribe again replaces and never double counts. Shown: words, words a minute (words over speech minutes), time saved against typing at 40 wpm, recordings a day, pictures, streak. A Usage pane in Settings and one tray line, "Today: 1,240 words". Seeded once from History. Local only; one small write per recording; no timer | new `stats.rs`; `session.rs`, `lib.rs`, `tray.rs`, `Settings.tsx`, `api.ts`, `native.css` | `cargo test --lib stats::`: markers are not words; local midnight with an injected offset; a streak across a gap; the Transcribe-again delta; one id twice; a seed equals a direct count; a corrupt file is replaced. `npm run build`. The pane looked at light and dark, both skins |
| 1 | Provider spine and the Codex adapter | `assist/`: the contract above, a child-process spine (job object or process group, kill on drop, stderr tail, strays ended only when pid and start time both match), JSON-lines calls, the Codex adapter (ephemeral read-only thread in an empty folder Pointcast owns, features and MCP servers off, every server request declined), detection and health. A stand-in `fake_codex` drives every test | new `assist/{mod,child,jsonl,codex,detect}.rs`, `examples/fake_codex.rs` | `cargo test --lib assist::` on the stand-in: request order and shapes; deltas in order and the whole-message item not shown twice; cancel, and a child that ignores it ended in 2 s; a turn that fails before its reply; signed out sends no turn; a limit carries its reset; approvals declined and unknown requests answered; a model without pictures refused before sending; unknown notifications ignored; no child after idle or close; the stray rule; no credential file named in source |
| 2 | Ask about this | The Ask key (hold or tap as dictation does); one frame at the press (window under the pointer, else the monitor), fitted for the wire; local speech; a `live` panel docked at the screen edge, shown without taking focus, never protected; the answer streamed; saved as turn 1 of a session folder (`sessions/<id>/history/<turn>/` plus `answer.json` with the exact payload and six timestamps). A scripted provider stands in | new `live.rs`, `sessions.rs`; `history.rs` (`with_keep`), `session.rs` (a purpose on start and stop), `capture.rs` (`grab_ask`), `settings.rs`, `chord.rs`, `lib.rs`, `platform.rs`, `tauri.conf.json`; new `live.html`, `live-main.tsx`, `windows/Live.tsx`, `lib/live.ts` | `cargo test --lib live:: sessions::`: each state by one path; Esc while listening sends nothing; a turn folder round-trips; a session is not pruned at 101 turns; `live` is never protected. `node --test src/lib/live.test.mjs`. Each state screenshotted on Windows and macOS |
| 3 | Ask through your ChatGPT plan | Slices 1 and 2 joined: Settings › "Use the AI you already have" (found, signed in, plan), sign in through Codex's own flow, the first-use sheet per provider and destination, the model list from `model/list` with the lightest picture model as default, the limits line under an answer, plain failure messages, the child ended after 5 idle minutes, README and onboarding copy | `lib.rs`, `settings.rs`, `assist/consent.rs`, `Settings.tsx`, `Live.tsx`, README | Tests: no consent, zero spawns; a new destination asks again. On the real CLI: handshake, account and model list with no stray process; then 10 real asks with release to first words logged (target 1.1 s p50, 3.5 s p95 plus the CLI's start). Idle memory back to baseline a minute after closing |
| 4 | Conversation and disclosure | Follow-ups with context (earlier turns as text, pictures of the last 2 turns), a typed follow-up, barge-in on the Ask key, the Esc ladder (generating, then panel), a release frame only when the screen changed, `[Sent]` showing the exact payload, drop the picture, review before sending (setting), errors with Try again and Copy prompt, private-window titles added to the never-capture defaults | `live.rs`, `sessions.rs`, `privacy.rs`, `tray.rs`, `Live.tsx`, `lib/live.ts`, `Settings.tsx` | Tests: context shape; a superseded answer saved as stopped with its text; a never-capture window painted in the sent bytes; a redacted rectangle holds no original pixel; `sent` equals what the provider received. A five-turn conversation with two barge-ins in the real app |
| 5 | The look | A teardown of Wispr Flow's and Superwhisper's recording, loading and stats screens, then one motion and state spec for capsule and panel on both systems: listening, finishing, thinking, answering, done, error. Markdown (raw HTML off), code with copy, math loaded only when needed; per-OS material and type; move, resize, pin as a window, the collapsed pill; the shortcut map (dictate, capture, ask, open the panel, summarize, each rebindable) | `platform.rs`, `Live.tsx`, `Indicator.tsx`, new `lib/markdown.ts`, `native.css`, `package.json` | `node --test`: raw HTML renders as text; an unclosed fence renders; a remote image is never requested. Real app, light and dark, both systems: over a full-screen app, on a second monitor at another scale, a follow-up returns focus to the study app, a second capturer sees the panel, Pointcast's own picture does not contain it |
| 6 | Sessions | List, resume, rename, search (a scan, no index), delete, the joining rule (same session while the panel is open or closed under 10 minutes), export to a markdown folder, MCP tools `pointcast_sessions` and `pointcast_session` | `sessions.rs`, `prompt.rs`, `mcp.rs`, `lib.rs`, `Live.tsx`, `api.ts` | Tests: every exported link resolves; an export survives `prune_handoff`; search finds a word in a question, an answer and a pin label; the MCP tools stay within the byte budget. Resume yesterday's session in the real app |
| 7 | Ask about a marked region | The overlay's primary action becomes Ask when opened from Ask; pins, steps, redaction and crop reach the prompt as captions; the answer may place numbered marks on a copy of the picture | `overlay.rs`, `Overlay.tsx`, `live.rs`, `image_ops.rs`, `prompt.rs`, `Live.tsx` | Tests: the mark-tag parser's one accepted form and its rejects; a mark at a known pixel lands within 2 px on the stored picture; captions in order. Mark three steps on a diagram and read an answer that names them |
| 8 | Summarize and the study loop | Tutor (hints first, one check question) and Direct styles; Summarize as JSON where every point cites a turn and a quote, and a quote that is not a substring of that turn is dropped and counted; cards and a quiz; Anki and markdown export; Quiz me | new `summary.rs`; `sessions.rs`, `live.rs`, `Live.tsx`, `prompt.rs` | Tests on a scripted provider: an invented quote dropped; case and spacing forgiven; a wrong turn dropped; bad JSON retried once, then said; an empty session gives an empty summary. A 10-turn real session where the owner checks every quote |
| 9 | Claude through Claude Code | `claude -p` with stream-json both ways, tools off, no session files, pictures as base64 blocks, on the same spine | `assist/claude.rs`, a stand-in, `detect.rs`, `Settings.tsx` | The slice 1 cases on a Claude stand-in. Needs the owner's reading of Anthropic's conditions first |
| 10 | API keys and this computer | One OpenAI-compatible adapter (OpenAI, OpenRouter, Gemini key, Ollama, LM Studio, llama.cpp) and Anthropic's API; keys in the OS credential store; "Only on this computer" limits the list to local models | `assist/{http,openai,anthropic,vault}.rs`, `Settings.tsx` | Tests against a local stub server; with the offline switch on, no outbound client is built; `settings.json` never holds a key |
| 11 | Carried over | Live text and Transcribe again made identical; Remove also cleans `additionalDirectories`; the MCP switch's accessibility state; captions carry the app and window title | `session.rs`, `agents.rs`, `Settings.tsx`, `prompt.rs` | A locking test each |

## Order

0 and 1 share no file and start in the same session as this plan (0 is the first shippable slice). 2 follows 0 (both
change `session.rs` and `lib.rs`). 3 needs 1 and 2. 4, 5 and 6 need 3 and share `Live.tsx`, so they run in that
order, or with 5 owning the view and 4 the logic. 7 needs 4. 8 needs 6. 9 and 10 need only 1 and can run beside 4 to 8.
11 fits wherever a lane is free.

## Gates

A lane runs `cargo test --lib <module>::` and `npm run build` in its worktree when a build slot and 3 GB of RAM are
free; otherwise it pushes its branch and runs `ci.yml` there. Front-end logic is a pure reducer under
`node --test`. Anything on screen is looked at in the built app before it is called done. Latency and memory
figures in this plan are targets; each gate records the real number. The program branch runs CI before it goes to
`main`. A release is the owner's yes, and so are two steps in slice 3: the first handshake with his signed-in
Codex and the first live question, which spends his plan.

## The edge (not in this plan)

- Spoken answers (Kokoro through the speech runtime already downloaded is the candidate), open-mic conversation, wake words.
- System audio, meetings, other people's voices. Continuous screen watching. More than the pointer's monitor.
- A live pointer drawn over the screen, clicking, agents acting on the computer, any tool use by the model.
- Text of the screen by OCR or the accessibility tree beyond the one control's name.
- A spaced-repetition scheduler, notifications, streak nudges. Accounts, sync, sharing, mobile.
- Installing or updating a vendor's CLI; more than one account per vendor; "Sign in with ChatGPT" without a CLI (a 4 hour spike later).
- Any form of hiding from screen sharing, proctoring or interview software.
