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

## Next

1. Rebuild the app (all of the above is in). It is at `src-tauri/target/release/bundle/macos/Pointcast.app`, ad-hoc signed.
2. The owner tries a real call, a computer-use task and Kokoro on this Mac — now also: the bigger Ask panel, the Call key, restored window spots, the pointer ring.
3. On the owner's yes: push to main.
   - The Windows code has never been compiled; CI is the first compile.
   - Then read CI.

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
