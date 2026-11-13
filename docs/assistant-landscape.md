# Assistant landscape — what to steal

Field notes from a competitor scan (Superwhisper, Clicky, Cluely, rea/Raycast, Handy, axcli). Each item names the mechanism, not the marketing. Ordered by how soon Pointcast could use it.

## Dictation modes (Superwhisper)

Each "mode" bundles `{STT choice, language, LLM post-processing prompt, auto-activation rules by frontmost app/URL, delivery behavior}`. Pro snapshots **selected text + clipboard at record-start** into the prompt, and ducks other audio while recording.

Steal: named dictation presets (chat / email / note / code) = post-processing instruction + optional per-app auto-mode. Context snapshot at record-start is cheap: AX selected text + clipboard, marked untrusted.

## System audio capture (Cluely clones)

Cluely streams mic + **system** audio + screenshots to an LLM. On macOS the clones ship a tiny Swift binary (`SystemAudioDump`, ScreenCaptureKit `capturesAudio`) spawned as a child; stdout PCM → resample 16 kHz → STT/LLM. Overlay invisibility uses `setContentProtection`/`NSWindowSharingNone` (Tauri: `set_content_protected`) — but macOS 15's ScreenCaptureKit can ignore it, so don't promise invisibility.

Steal: the SCK `capturesAudio` sidecar → "listen to a meeting"/call mode where Pointcast hears the other person. Repos to read: `github.com/sohzm/cheating-daddy` (VAD+PCM resample), `github.com/pickle-com/glass` (GPL; SummaryService rolling summaries — mirrors our summary.json), `github.com/iamsrikanthnani/pluely` (a Tauri Cluely; older tags readable).

## Ghost-cursor narration (Clicky)

`github.com/farzaa/clicky` (MIT, documented AGENTS.md): menu-bar `LSUIElement` app, two NSPanels, full-screen transparent per-monitor overlay at `.screenSaver` level. Claude replies carry `[POINT:x,y:label:screenN]` tags; the overlay animates a cursor along a bezier to the element.

Steal: we already have the pointer ring + element rects. Let providers emit a labeled "pointing at" act (or reuse element ids) so the ring can show what the AI is about to touch before it touches it — a visible pre-act affordance on top of the journal.

## On-demand skills/MCP registry (rea, Raycast)

`github.com/morluto/rea` ("Reverse Engineer Anything") is an MCP server + agent *skill* installed on demand via `npx rea-agents setup` into Claude Code/Codex/Cursor — literally the "download the plugin when the task needs it" pattern. `github.com/raycast/extensions` shows the integration mechanics for native apps: `run-applescript`/JXA for Mail and browsers; bundled Swift helpers for EventKit/Contacts; per-extension `tools/*.ts` that the AI calls.

Steal: (a) an integration registry where a provider session gets extra MCP servers only when the task names them (mail, calendar, browser-CDP) instead of always-on tools; (b) `osascript` for Mail draft/send (confirm-gated) and Reminders; a small Swift sidecar only when EventKit/Contacts are really needed.

## Same-stack reference: Handy

`github.com/cjpais/Handy` (MIT) is Tauri 2 + Rust + React — our exact stack. Notable pieces: `transcribe-cpp` Whisper/Parakeet/Moonshine STT, `earshot` Silero VAD, `enigo` typing, `rdev` global hotkeys, `tauri-plugin-macos-permissions`, `tauri-plugin-store`, sqlite history, and an LLM post-processing prompt with **prompt-injection defense** — worth copying the prompt's framing (we already mark tool/screen text untrusted; dictation polish should too).

## Background acting without focus steal (axcli / computer_use)

`github.com/andelf/axcli` and the `computer_use` crate: indexed AX tree + SCK per-window screenshots + `CGEventPostToPid` — click/type in an app **without bringing it forward**, plus Playwright-style locators and occlusion-proof window captures.

Steal: today every act needs the app in front. A "background app" act tier would let the AI work a non-front window (screenshot per window, post events to pid) while the person keeps working. Same guard, new target kind.
