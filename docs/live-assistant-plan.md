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
| Look | Decided by the owner on two rendered concepts (2026-11-11): one continuous hand-drawn line carries listening, thinking and answering (your voice draws it, it loops while it waits, it straightens into the margin of the answer and its end is the caret); everything around it keeps the app's existing native look, macOS-native on a Mac and Windows-native on Windows, with less on screen than the concept had. The restraint of xAI's product UI is the reference for cleanliness. Red means the microphone is live; the accent colour means going to or coming from the provider, and nothing else. No spinner, no state words: text appears only when a wait runs long (600 ms microphone, 2 s finishing, 8 s and 20 s provider) | "Longhand is the coolest one so far but our current one is still better, so use Longhand for the things that I liked." He rejected the concept's home and session screens and the cell-based direction |
| Shortcuts | Every global chord is Ctrl+Shift plus a key: Space is your voice (dictate), the question-mark key asks (Ctrl+Shift+/), digits are pictures. Asking with nothing said opens the panel. Summarize, sessions and the follow-up field are panel keys, not global ones. All rebindable | One new global chord; two voice keys with one sentence each |

One provider contract serves both reports (amended after an independent review, 2026-11-11):

```rust
pub enum Mime { Png, Jpeg }
pub enum Part { Text(String), Picture { path: PathBuf, w: u32, h: u32, mime: Mime } }   // a file Pointcast wrote, already flattened
pub struct Turn { pub role: Role, pub parts: Vec<Part> }
pub struct Event { pub turn: TurnId, pub kind: Kind }
pub enum Kind { Thinking, Delta(String), Usage(Usage), Limits(Limits), Done, Failed(Failure) }
pub enum Failure { NotInstalled, SignedOut, Limit { resets: Option<i64> }, Offline, Refused, NoPictures, ContextFull, Stopped, Broken(String) }
pub trait Provider: Send + Sync {
    fn id(&self) -> &'static str;
    fn caps(&self, model: Option<&str>) -> Caps;   // per model: pictures, max_images, max_edge, offline
    fn health(&self) -> Health;                    // spends nothing
    fn warm(&self);                                // called when the Ask key goes down
    fn open(&self, system: &str, earlier: &[Turn]) -> Result<Box<dyn Chat>, Failure>;
    fn once(&self, system: &str, turns: &[Turn]) -> Result<Box<dyn Chat>, Failure>;   // shares nothing with an open Chat (Summarize)
}
pub trait Chat: Send {
    fn ask(&mut self, turn: Turn, out: Sender<Event>) -> Result<TurnId, Failure>;
    fn stopper(&self) -> Arc<dyn Fn() + Send + Sync>;   // cancel from another thread while a turn runs
    fn close(&mut self);
}
pub fn replay(earlier: &[Turn], pictures_of_last: usize) -> Vec<Part>;   // one transcript plus the last pictures; returns exactly what is sent
```

- `ask` takes only the new turn and every event carries its turn id, so text that arrives after an interrupt never
  lands in the next answer. A turn ends with exactly one `Done` or `Failed`.
- Pointcast owns the context. A resumed session, and every adapter that keeps no thread, sends `replay(earlier, 2)`;
  what `replay` returned is what `[Sent]` shows. A resumed picture is the same bytes that were sent the first time,
  kept in the session folder, never a History original or a handoff file.
- Keeping Codex to question and answer is more than declining approvals: the thread config also turns off the
  shell and exec tools, `view_image`, web search, apps, browser and computer use, hooks, image generation and
  multi-agent, empties the MCP table, and overrides the person's own Codex settings that would change where the
  words go or run a program (model provider, notify, hooks, profiles, project instructions). A command, file,
  search or tool item that starts anyway ends the turn as `Refused`. If the effective provider is not OpenAI the
  chat does not open, so the consent line is never false. One list holds every method the adapter may send, and a
  test fails on any other or on a parameter that asks for a token.

## The first real handshake (2026-11-11, owner approved)

Codex 0.164.0, the owner's signed-in account, no turn started, nothing spent. `account/read`: a
ChatGPT plan, which works. `model/list`: `gpt-6-luna` (default), `gpt-5.6-terra`, `gpt-5.6-luna`, all text and image.
`thread/start` with the fence answered in about 0.2 s with `modelProvider: openai`, a read-only sandbox without
network, `approvalPolicy: untrusted`; every config override was accepted without error (whether each takes effect is
still unproven). Two findings change the adapter: a thread started without a model took the person's configured
default (`gpt-6.1-sol`), which this plan does not offer, so Pointcast always names a listed model; and Codex always
adds the person's own `~/.codex/AGENTS.md` (`instructionSources`), which no setting removed, so Pointcast says so
where the AI is chosen and records it in what was sent rather than hiding it. No Codex process outlived the
handshake. Script and results are kept outside the repository.

## The first live question (2026-11-11, owner approved one)

One turn through the real Codex with the adapter's exact fence, one picture, effort low, on the plan's default
model. The picture asked a sum and also told the reader to open a file in the working folder, where a canary file
sat. Result: the sum was answered from the picture; first words 3.0 s after sending, done at 3.3 s, 8 deltas, then
the whole message once more (as the adapter expects); no command, file, search or tool item started; Codex made no
request; the canary's text was not in the answer; the limits read back; no Codex process was left. One gap: an
`mcpServer/startupStatus/updated` notification arrived during the turn, and the person's Codex has MCP servers of
its own configured, so the empty `mcp_servers` override does not keep them from starting. No tool was called, and a
tool call ends the turn as refused, but the servers should not start at all: next slice.

## What Hermes and T3 Code do (2026-11-11, read at the owner's request)

T3 Code connects ChatGPT plans through OpenAI's own "Sign in with ChatGPT" for open-source apps (its own registered
client, a loopback callback, plan sharing), with `codex app-server` on the person's login as the fallback, Claude
through the Agent SDK, and other agents through ACP. Hermes connects every subscription, but its Claude, ChatGPT and
Copilot sign-ins reuse other apps' stored tokens or client ids and call private endpoints, which those vendors do not
allow; its `codex app-server` and `copilot --acp --stdio` routes are sanctioned. Consequences for this plan: a
"Sign in with ChatGPT" route (no Codex install, no `~/.codex/AGENTS.md` added) becomes the preferred ChatGPT route
for Plus and Pro plans once a spike on a paid account confirms pictures and streaming; Codex stays the route for Free
plans; Copilot goes through `copilot --acp --stdio`; Hermes's handling of an expired sign-in (never
retried in a loop, one plain "sign in again") is copied, its routes are not. Report kept outside the repository.

## Slices

Each slice is one branch off the program branch, gated on its own, merged back when green. No slice sits behind a flag.

| # | Slice | Items | Files | Gate |
|---|---|---|---|---|
| 0 | Usage stats | `stats.json` beside History with running totals, days (400 kept), a 7 by 24 grid, streaks and the newest 100 entry ids, so Transcribe again replaces and never double counts. Shown: words, words a minute (words over speech minutes), time saved against typing at 40 wpm, recordings a day, pictures, streak. A Usage pane in Settings and one tray line, "Today: 1,240 words". Seeded once from History. Local only; one small write per recording; no timer | new `stats.rs`; `session.rs`, `lib.rs`, `tray.rs`, `Settings.tsx`, `api.ts`, `native.css` | `cargo test --lib stats::`: markers are not words; local midnight with an injected offset; a streak across a gap; the Transcribe-again delta; one id twice; a seed equals a direct count; a corrupt file is replaced. `npm run build`. The pane looked at light and dark, both skins |
| 1 | Provider spine and the Codex adapter | `assist/`: the contract above, a child-process spine (job object or process group, kill on drop, stderr tail, strays ended only when pid and start time both match), JSON-lines calls, the Codex adapter (ephemeral read-only thread in an empty folder Pointcast owns, features and MCP servers off, every server request declined), detection (the native executable behind an npm shim preferred) and health, `effort: low` by default. The stand-in is a mode of the test binary itself, since `cargo test --lib` builds no example | new `assist/{mod,child,jsonl,codex,detect}.rs`; one `mod` line in `lib.rs` | `cargo test --lib assist::` on the stand-in: request order and shapes; deltas in order and the whole-message item not shown twice; cancel, and a child that ignores it ended in 2 s; a turn that fails before its reply; signed out sends no turn; a limit carries its reset; approvals declined and unknown requests answered; a model without pictures refused before sending; unknown notifications ignored; no child after idle or close; the stray rule; late deltas after an interrupt dropped; the next turn waits for the interrupted one to end; `once` uses another thread; `replay` shape; a tool item that starts ends the turn; a foreign provider refuses to open; the method allowlist; no credential file named in source. The gate reports the count of tests run, above zero |
| 2 | Ask about this | The Ask key (hold or tap as dictation does); one frame at the press (window under the pointer, else the monitor), fitted for the wire; the thumbnail of exactly that picture in the panel from the press, with a key to drop it before release; `grab_frame` paints out Pointcast's own unprotected windows, so an earlier answer is never in a later picture; private-window titles added to the never-capture defaults; `warm()` at key down; local speech; a `live` panel docked at the screen edge, shown without taking focus, never protected; the answer streamed with the five-bar shape in its three behaviours and minimal markdown and code rendering (raw HTML off); saved as turn 1 of a session folder (`sessions/<id>/history/<turn>/` plus `answer.json` with the exact payload and six timestamps). A scripted provider stands in | new `live.rs`, `sessions.rs`; `history.rs` (`with_keep`), `session.rs` (a purpose on start and stop), `capture.rs` (`grab_ask`), `privacy.rs`, `settings.rs`, `chord.rs`, `lib.rs`, `platform.rs`, `tauri.conf.json`; new `live.html`, `live-main.tsx`, `windows/Live.tsx`, `lib/live.ts`, `lib/markdown.ts` | `cargo test --lib live:: sessions::`: each state by one path; Esc while listening sends nothing; a turn folder round-trips; a session is not pruned at 101 turns; `live` is never protected; a frame taken with the panel open does not contain it; a dropped picture is neither stored nor sent; an Ask with nothing said stores and sends nothing. `node --test` on `live` and `markdown` (raw HTML renders as text, an unclosed fence renders, a remote image is never requested). Each state screenshotted on Windows and macOS |
| 3 | Ask through your ChatGPT plan | Slices 1 and 2 joined: Settings › "Use the AI you already have", a provider list drawn from what Rust reports (so later adapters add no Settings edit): found, signed in, plan. When Codex is missing, a "Get Codex" step with OpenAI's own install command and live re-detection, no restart. Sign in through Codex's own flow, the first-use sheet per provider and destination, the model list from `model/list` with the lightest picture model as default, the limits line under an answer, plain failure messages, the child ended after 5 idle minutes, README and onboarding copy | `lib.rs`, `settings.rs`, `assist/consent.rs`, `Settings.tsx`, `Live.tsx`, README | Tests: no consent, zero spawns; a new destination asks again. On the real CLI: no Codex, a signed-out Codex and an unconfigured Windows sandbox each end in a plain next step; handshake, account and model list with no stray process; a canary file plus an on-screen instruction to read it, where no command item starts and the canary text never appears in the answer; then 10 real asks with release to first words logged (target 3 s p50 and 8 s p95 through Codex, started at the press; 1.1 s and 3.5 s stays the target for a direct key). Idle memory back to baseline a minute after closing |
| 4 | Conversation and disclosure | Follow-ups with context (`replay`: earlier turns as text, pictures of the last 2 turns), a typed follow-up, Esc stops generating and then closes the panel, `[Sent]` showing the exact payload, a plain error line that keeps the question | `live.rs`, `sessions.rs`, `tray.rs`, `Live.tsx`, `lib/live.ts` | Tests: context shape; a stopped answer saved with its text; a never-capture window painted in the sent bytes; a redacted rectangle holds no original pixel; `sent` equals what the provider received, replay included; a resumed picture is the same bytes as first sent. A five-turn conversation in the real app |
| 5 | The panel, finished | The full motion and state spec on the panel with reduced-motion variants; code with copy, math loaded only when needed; per-OS material and type; move, resize, pin as a window, the collapsed pill; barge-in on the Ask key (the old answer saved as stopped), a release frame only when the masked screen changed, Try again and Copy prompt; the three shortcut groups in Settings (Voice, Pictures, Assistant) | `platform.rs`, `live.rs`, `Live.tsx`, `lib/markdown.ts`, `native.css`, `package.json`, `Settings.tsx` | Tests: a superseded answer saved as stopped; a formula renders. Real app, light and dark, both systems: over a full-screen app, on a second monitor at another scale, a follow-up returns focus to the study app, a second capturer sees the panel, two barge-ins in one conversation |
| 6 | Sessions | List, resume, rename, search (a scan, no index), delete, the joining rule (same session while the panel is open or closed under 10 minutes), export to a markdown folder, MCP tools `pointcast_sessions` and `pointcast_session` | `sessions.rs`, `prompt.rs`, `mcp.rs`, `lib.rs`, `Live.tsx`, `api.ts` | Tests: every exported link resolves; an export survives `prune_handoff`; search finds a word in a question, an answer and a pin label; the MCP tools stay within the byte budget. Resume yesterday's session in the real app |
| 7 | Ask about a marked region | The overlay's primary action becomes Ask when opened from Ask; pins, steps, redaction and crop reach the prompt as captions; the answer may place numbered marks on a copy of the picture | `overlay.rs`, `Overlay.tsx`, `live.rs`, `image_ops.rs`, `prompt.rs`, `Live.tsx` | Tests: the mark-tag parser's one accepted form and its rejects; a mark at a known pixel lands within 2 px on the stored picture; captions in order. Mark three steps on a diagram and read an answer that names them |
| 8 | Summarize | Tutor (hints first, one check question) and Direct styles; Summarize as JSON where every point cites a turn and a quote, and a quote that is not a substring of that turn is dropped and counted; key points and open questions, each linked to its moment and picture; markdown export. Cards and a quiz are a later phase (owner, 2026-11-11) | new `summary.rs`; `sessions.rs`, `live.rs`, `Live.tsx`, `prompt.rs` | Tests on a scripted provider: an invented quote dropped; case and spacing forgiven; a wrong turn dropped; bad JSON retried once, then said; an empty session gives an empty summary. A 10-turn real session where the owner checks every quote |
| 9 | Claude through Claude Code | `claude -p` with stream-json both ways, tools off, no session files, pictures as base64 blocks, on the same spine | `assist/claude.rs`, a stand-in, `detect.rs` | The slice 1 cases on a Claude stand-in. Needs the owner's reading of Anthropic's conditions first |
| 10 | API keys and this computer | One OpenAI-compatible adapter (OpenAI, OpenRouter, Gemini key, Ollama, LM Studio, llama.cpp) and Anthropic's API; keys in the OS credential store; "Only on this computer" limits the list to local models | `assist/{http,openai,anthropic,vault}.rs`; a key field in `Settings.tsx` | Tests against a local stub server; with the offline switch on, no outbound client is built; `settings.json` never holds a key |
| 11 | Carried over | Live text and Transcribe again made identical; Remove also cleans `additionalDirectories`; the MCP switch's accessibility state; captions carry the app and window title | `session.rs`, `agents.rs`, `Settings.tsx`, `prompt.rs` | A locking test each |
| 12 | Home, and the capsule | A Home section, first in the main window, in the existing native style: the usage numbers and the when-you-dictate grid (moved from the Usage pane), recent sessions as a plain list, and the three ways in with their shortcuts. The dictation capsule keeps its current design; only two things change: the line replaces the spinner while finishing, and there is no "Pasted" line since the pasted text is its own confirmation | `Settings.tsx` (Home), `Indicator.tsx`, `lib/capsule.ts`, `native.css` | `node --test` on `capsule`: no line for pasted, one for each other outcome. Home and each capsule state screenshotted, both skins, light and dark, and with reduced motion |
| M | Mac updates in place (ships ahead of 0.5, off `main`) | One self-signed certificate for every release build, so macOS keeps Screen Recording and Accessibility across updates; the installer resets them only when the signer changed; the app quits, swaps and reopens by itself; the update check reads a static `latest.json` from the release first, so it works on a network that has used up GitHub's API allowance, and a failed check never reads as up to date | `install.sh`, `update.rs`, `release.yml`, `update-test.yml`, `scripts/`, `docs/SIGNING.md` | On a macOS runner with a throwaway certificate: build N+1 satisfies build N's requirement, an ad-hoc copy does not; the real script over N leaves N+1 running with no permission reset; `cargo test --lib update::`. Switching the real certificate on is the owner's yes |

## Order

0 and 1 start in the same session as this plan (0 is the first shippable slice); they share one line of `lib.rs`.
M runs beside them off `main`. 2 follows 0 (both change `session.rs` and `lib.rs`). 3 needs 1 and 2, and with 4 it
is the first usable release: stats, Ask through ChatGPT, follow-ups. Then 5, 6, 7 and 8 in that order (they share
`Live.tsx`, and 7 shares `live.rs` and `prompt.rs`). 9 and 10 need only 3 and touch no file of 4 to 8 beyond one
key field. 12 touches only the capsule and runs once 0 has landed (`native.css`). 11 follows 8.

## Gates

A lane runs `cargo test --lib <module>::` and `npm run build` in its worktree when a build slot and 3 GB of RAM are
free; otherwise it pushes its branch and runs `ci.yml` there. Front-end logic is a pure reducer under
`node --test`. Anything on screen is looked at in the built app before it is called done. Latency and memory
figures in this plan are targets; each gate records the real number. The program branch runs CI before it goes to
`main`. A release is the owner's yes, and so are two steps in slice 3: the first handshake with his signed-in
Codex and the first live question, which spends his plan.

## The edge (not in this plan)

- Spoken answers (Kokoro through the speech runtime already downloaded is the candidate), open-mic conversation, wake words.
- Study cards, a quiz and Anki export: after 0.5.
- System audio, meetings, other people's voices. Continuous screen watching. More than the pointer's monitor.
- A live pointer drawn over the screen, clicking, agents acting on the computer, any tool use by the model.
- Text of the screen by OCR or the accessibility tree beyond the one control's name.
- A spaced-repetition scheduler, notifications, streak nudges. Accounts, sync, sharing, mobile.
- Installing or updating a vendor's CLI; more than one account per vendor; "Sign in with ChatGPT" without a CLI (a 4 hour spike later).
- Any form of hiding from screen sharing, proctoring or interview software.
