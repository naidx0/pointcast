# Handoff — 2026-11-13 (evening)

**Branch:** `local/assistant-polish` in `~/dev/pointcast`, head `dc9dfd4`, 80 commits ahead of `origin/main` (merge-base `1926a749`).
**Local only:** nothing is pushed. `pushRemote` is set to `no-push-local-only`. Push to main only on the owner's explicit yes.

## Done on this branch (all gates green at d939219: 820 Rust, 282 front-end, tsc, vite build)

- **Model and effort on every AI:** Settings › AI. Claude gets `--model`/`--effort`; Codex gets per-model efforts and a speed tier; ACP agents get `configOptions`. The choice is kept per route.
- **Computer use, slices A–D** (`src-tauri/src/computer/`, docs/cua-plan.md):
  - see/act MCP tools behind a per-turn grant with its own secret;
  - per-app trust tiers, the never-list, and Do it / Cancel cards for irreversible actions;
  - Esc and the Ask key stop it;
  - a journal listed under each answer;
  - a plan checklist with Start before multi-step work;
  - keys and typing only reach an app that is really in front.
  - Works with Claude and Codex; ACP agents only when they support MCP over HTTP.
- **Talk back** (`voice/speak/`, `voice/kokoro/`): answers read aloud sentence by sentence, with system voices or the Kokoro natural voice (350 MB, downloaded on demand). Stops on Esc, the Ask key, or when you talk over it.
- **Call mode** (`voice/call/`): Silero VAD ends a turn; it answers aloud and listens again; barge-in with pre-roll.
- **UI:**
  - the Aurora Glow wave (Settings › General › Appearance: Classic, Strands, Aurora);
  - the Glacier wash, now visible in light mode;
  - Keep history (100, 500, 1,000 or Forever);
  - the Ask panel opens at 640 px and reads at 15/22 px (resizable 380–900).

## This session's work (committed, tested, not yet human-verified)

- **VAD tail merged:** a duplicate Silero download is refused (`fetch_once`), and one stretch of speech is capped at 30 s so a false-positive voice can't grow the buffer without bound (`voice/call/vad.rs`). Was `docs/voice-fixes-wip-vad.patch`; now applied and test-locked.
- **Call shortcut:** Ctrl+Shift+C by default, settable in Settings; starts a call, cancels one still opening, ends one in progress (`chord.rs`, `lib.rs`).
- **Window positions persist:** the HUD and the Ask panel come back where they were put (`data_dir/spots.json`; a damaged file forgets only what it cannot read).
- **Pointer ring while acting:** a small accent ring follows the pointer for as long as a turn holds the computer tools and pulses at each click (`computer/ring.rs`, `ring.html`). Click-through, and on the protected list so the AI never sees it in its own screenshots.
- **Answered-model warning:** Claude (result `modelUsage`), Codex (completed turn) and ACP (what the session offers) report the model that answered; when it is not the one picked — an alias resolving to its own id doesn't count — the answer carries one quiet line (`Answered by x, not the y you picked.`). Kept turns carry it; Sent names the model asked.
- **ACP `config_option_update`:** a model changed in the agent's own window updates what the session offers and names what answered.

## Second session (committed, tested, not yet human-verified)

- **S1, one history on Home:** "Recent" is now one timeline of dictations and Ask sessions, newest first, each row marked "Dictated" or "Ask · provider" and opening its own store (`src/lib/timeline.ts`). It refreshes on `history://changed`, on live state settling, and on focus. Stats count Ask too — "Questions" and "Answers read" figures, kept per day and in totals; they touch no dictation figure, streak or grid cell.
- **S3, a cheap summariser:** `catalog::summary_pick` sends summaries through a cheaper pick of the same AI — Claude's Haiku at low effort, Codex's listed model that thinks least (minimal, then low), other routes' own model — through a provider of its own, so the held Ask provider stays warm and the summary can never act.

## Third session (committed, tested, not yet human-verified; gates: 829+ Rust, 282 front-end, tsc, vite)

- **S4, memory you can see** (`memory.rs`, AI page › Memory): `memory.md` in the data folder, ~200 lines, hand-editable. Accepted facts go to every AI as untrusted reference text; "remember…" asks land as proposals until accepted on a click; off switch in settings.
- **S4, session memory:** reopening a session replays its `summary.json` plus the last 4 turns instead of every turn it ever had (`live.rs::earlier`, `RECENT = 4`); older pictures stay reachable for Again.
- **"Summarise with" picker** (rest of S3): Automatic or a named model, kept in settings, on the AI page.
- **"Copy transcript"** in session controls: the session markdown goes on the clipboard so a conversation continues in the AI's own app; a real `claude --resume` handoff still needs the provider session id plumbed up.
- **`pointcast_files`** (S8, first native integration): list/reveal/move/mkdir/trash straight on the filesystem — paths inside ~ only, canonicalized against symlink escapes; credential/config folders, `.git` internals, Pointcast's own data and home itself off limits; the file manager's tier and plan rules apply; trash waits for the person (`trash` crate).
- **`docs/assistant-landscape.md`:** competitor scan — dictation modes (Superwhisper), system-audio sidecar (Cluely clones), ghost-cursor narration (Clicky), on-demand skill/MCP registry (rea/Raycast), same-stack reference (Handy), background acting without focus steal (axcli).

## Fourth session — live-test batch + install/signing (committed; gates: 832+ Rust, 281 front-end, tsc, vite)

- **Stable signing identity** (the install factor): self-signed `maxnaid` cert (`~/pointcast-signing/`, never in git) imported into a dedicated keychain with a stored password + codesign partition list — a plain login-keychain import leaves the key unpaired. Designated requirement is now `identifier "dev.pointcast.app" and certificate leaf`: macOS keeps Screen Recording + Accessibility across signed updates (ad-hoc reset them every build). Recipe in `docs/SIGNING.md`.
- **Real update path exercised:** `POINTCAST_DMG=… bash install.sh` quit the old app, installed the cert-signed build, relaunched — it correctly detected the signer change and reset permissions this once; every cert-signed update after stays quiet.
- **Live-test fixes:** Strands removed (Classic + Aurora; a saved "Strands" reads as Aurora); Aurora speed 0.5→0.8; the capsule clock no longer appears on its own after a minute (hover/focus/mic-warning only — it was covering the wave); the onboarding "Try it" hides once a question has been asked.
- **Ask header:** speaker switch (toggles `speak_answers`; while speaking, cuts the current sentence), pointer switch (toggles `computer_use` — it defaults OFF, which is why "the computer never acted" in the test), a New-ask button (was only in the Sessions sheet), and a Copy button on the opened question.
- **TTS proven, not broken:** two tests speak through the real `AVSpeechSynthesizer` — one through `Mac`, one a full streamed answer through `Reader`. Silence in the app was a stale installed build / disabled setting, never the pipeline.

## Fifth session — providers audit, onboarding TTS, browser tool

- **Providers verified complete** (the T3-Chat aggregation the owner asked for exists): plan routes ChatGPT (Codex), Claude (Code), Gemini CLI, Copilot, OpenCode; key routes OpenAI API, Claude API, **Grok (xAI)**, Gemini API, Groq, Mistral, DeepSeek, OpenRouter, plus "Other OpenAI-compatible" (any base URL) and local Ollama/LM Studio. Keys go to the vault via `assist_key_set`; consent sheet before any new host. The owner simply hadn't opened Settings › AI far enough.
- **Devin doesn't fit Ask**: its API dispatches async agent sessions (minutes, no streaming deltas) — a "send this task to Devin" action surface maybe later, not an Ask route. Noted, not built.
- **Onboarding**: "Read answers aloud" switch on the Done step (`speak_answers`; the voice itself stays in Settings).
- **`pointcast_browser`** (S8): the front browser's front tab or every tab, via JXA — Safari + Chrome family; other apps told they can't be read; See-tier, grant-gated, journaled; app name injected as a JSON string (no code carries).
- **`pointcast_mail`** (S8): newest inbox messages — sender, subject, unread dot — via JXA; reads only, never sends/moves/marks; Mail needn't have a window open.
- **`pointcast_calendar`** (S8): events now→N days (default 7) — ISO time, calendar, title, soonest first — via JXA; reads only; first use asks macOS for Calendar automation.

## Next

1. Owner re-tests in the new install: speaker + act switches on the panel, New button, copy a question, hover-only timer, faster Aurora; then Settings › AI — pick Grok/OpenRouter/etc. with a key.
2. On the owner's yes: push to main. GitHub secrets `MAC_SIGN_P12`/`MAC_SIGN_PASSWORD` needed for CI signing; a published release is what the Updates pane checks against.
3. Bigger still-open asks: remaining S8 integrations (mail/calendar via osascript next); dictation modes / system-audio / ghost-cursor / on-demand MCP registry from `docs/assistant-landscape.md`; echo cancellation; Windows compile; Developer ID for other machines.
   - The Windows code has never been compiled; CI is the first compile.
   - Then read CI.

## Lane 4 (2026-11-14 night) — decisions, base `19e3aa7`, `cargo test --lib` 835 passed, 0 failed

- **S4:** already built and gated (memory, session memory, summariser picker). Nothing left.
- **Echo cancellation:** not built, on purpose. Call mode already has an adaptive echo gate (`voice/call/turn.rs::Echo`). A reference-signal canceller cannot work for system voices (`AVSpeechSynthesizer` gives no samples). The only real fix is macOS voice-processing IO (AUVoiceIO) replacing the cpal mic path, which can only be judged on a live call (speaker volume, Bluetooth). It waits for the owner's real call: if the gate lets Pointcast's own voice through, that is the signal to build it.
- **"Continue in the AI app":** not buildable as a resume. Claude runs with `--no-session-persistence` and Codex threads live in Pointcast's fenced config, so no provider session id exists to `--resume`. Persisting sessions would break the isolation fence. "Copy transcript" stays the supported path.

## Not verified

- Windows: none of it has been compiled.
- In the running app:
  - a real call (echo margin, speaker volume, Bluetooth);
  - the confirm card and the plan card;
  - the physical Esc key while acting (may need Input Monitoring);
  - ACP agents acting (Gemini not signed in; Copilot reports "auto" only);
  - the pointer ring on a real acting turn, and the answered-model line on a real mismatch.
- Signing: an ad-hoc signature means every build resets macOS permissions. A Developer ID is needed (owner's money and identity).

## Follow-ups (not started)

- Echo cancellation through macOS voice processing.
- `docs/next-layer-plan.md`: S1–S4 built, S5–S7 exist in substance, S8 started (`pointcast_files`); mail/calendar/browser-CDP integrations remain — `docs/assistant-landscape.md` has the mechanisms to steal (osascript, Swift sidecar for EventKit, CDP attach, on-demand MCP registry).
- Provider session ids for a real "continue in the AI's own app" (`claude --resume`, Codex thread resume); Copy transcript is the working fallback.
- Human verification and a Windows compile, as ever.
