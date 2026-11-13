# Handoff — 2026-11-13

**Branch:** `local/assistant-polish` in `~/dev/pointcast`, head `ea0f25d`, 63 commits ahead of `origin/main` (merge-base `1926a749`).
**Local only:** nothing is pushed. `pushRemote` is set to `no-push-local-only`. Push to main only on the owner's explicit yes.

## Done on this branch (all gates green at ea0f25d: 807 Rust, 278 front-end, tsc, vite build)

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
  - Keep history (100, 500, 1,000 or Forever).
- **Fixes:**
  - stale macOS permissions after an update ("Reset and ask again");
  - one drag rule for every window, and windows stay where they were put.

## Voice review fixes (merged)

Four of the five review findings are fixed, test-locked and merged; the last item, the call-end race, was folded into the first fix. 813 Rust tests pass.

- A second Call click while the microphone opens cancels the start. A call's end, even after a panic, only clears its own state.
- The speaker closes its stream with the buffer unlocked, so closing never deadlocks.
- Huge ordinals and numbers never panic the natural voice.
- A sentence that panics while it is being made is skipped, and the voice goes on to the next one.
- **Not finished:** the guard against a duplicate Silero download, and the cap on the VAD buffer. The work in progress is saved as `docs/voice-fixes-wip-vad.patch` (not applied, not tested).

## Next

1. Rebuild the app (the voice fixes are merged). It is at `src-tauri/target/release/bundle/macos/Pointcast.app`, ad-hoc signed.
2. The owner tries a real call, a computer-use task and Kokoro on this Mac.
3. On the owner's yes: push to main.
   - The Windows code has never been compiled; CI is the first compile.
   - Then read CI.

## Not verified

- Windows: none of it has been compiled.
- In the running app:
  - a real call (echo margin, speaker volume, Bluetooth);
  - the confirm card and the plan card;
  - the physical Esc key while acting (may need Input Monitoring);
  - ACP agents acting (Gemini not signed in; Copilot reports "auto" only).
- Signing: an ad-hoc signature means every build resets macOS permissions. A Developer ID is needed (owner's money and identity).

## Follow-ups (not started)

- A Call shortcut.
- Window positions remembered across launches.
- Echo cancellation through macOS voice processing.
- A pointer ring while acting.
- A warning when the model that answered differs from the one chosen.
- `config_option_update` handling.
