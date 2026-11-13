# Handoff — 2026-11-13 (evening)

**Branch:** `local/assistant-polish` in `~/dev/pointcast`, head `66fc098`, 74 commits ahead of `origin/main` (merge-base `1926a749`).
**Local only:** nothing is pushed. `pushRemote` is set to `no-push-local-only`. Push to main only on the owner's explicit yes.

## Done on this branch (all gates green at 66fc098: 818 Rust, 278 front-end, tsc, vite build)

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
- `docs/next-layer-plan.md` (untracked, scoped): S1 one history timeline on Home + resume-last-session; S3 cheap summariser per AI; S4 memory you can see (`memory.md`, accept-proposed, AI page view, off switch); S8 native integrations.
- A "continue in the AI's own app" handoff (`claude --resume`, Codex thread resume, transcript/picture copy fallback).
