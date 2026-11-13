# Ask panel polish and the assistant direction

Local only (owner, 2026-11-12): branch `local/assistant-polish`, never pushed. The wider plan is `docs/next-layer-plan.md`.

## Done on this branch

### The voice: the breathing wave (B+)

- **Why.** The old trace was one wiggly stroke with a red dot at its tip. It read as something swimming.
- **What the owner chose.** From the drafts in `docs/design/waves-draft.html`, the breathing wave (B), with "more waving, more action, and not just stopping at one certain point".
- **What it is now** (`src/lib/line.ts` `breathStep`, `breathAt`, `breathPath`; drawn by `BreathWave` in `src/components/Line.tsx`):
  - Three soft layered waves, with no head or tail. A mask fades both ends.
  - Each layer travels its own way at its own speed (+1, −1.37, +1.83).
  - The frequencies are unrelated (1, golden ratio, silver ratio) and drift slowly. The swell's centre and width wander along the line. No frame repeats and the swell never parks in one spot.
  - The voice sets the height (quick up, gentle down) and the speed (1.8 rad/s at silence, plus up to 4.2 with voice).
  - At silence it still breathes at 6 % height.
  - A soft gain (tanh) makes a normal voice read clearly and lets a shout round off below the edge.
  - With reduced motion it is still.
- **Where it is used.** The Ask panel only, both the listening row and a spoken follow-up. The dictation capsule keeps the pen trace for now.
- **Gate.** `src/lib/line.test.mjs`:
  - both ends sit on the middle and the wave stays inside its box;
  - silence keeps a little height;
  - a voice moves it faster;
  - the crest travels more than 30 % of the width over 30 s;
  - the layers travel both ways.

### Resizable Ask panel

- **The grip.** A corner grip at the bottom left; the panel stays docked at the top right, so dragging left widens it and dragging down makes it taller. The arrow keys move it in 20 px steps, and a double click returns to the usual size.
- **Limits.** Width 380–900 px, always inside the screen's margins. Once dragged, an answer shows at the chosen height (at least 200 px; it may pass the usual 60 %), and long answers scroll inside it. The listening row stays one row.
- **Saved.** The size is kept in page storage (`pointcast.panelSize`) across launches.
- **Rust.** `redock_panel` and `show_panel` keep the window's own width (`panel_width`) instead of the fixed 500.
- **Gate.** `src/lib/live.test.mjs` (`panelWidth`, `panelHeight` with a chosen height, `dragSize`, `parsePanelSize`) and `platform.rs` (`panel_width`).
- **Not yet seen in the real app.** A drag in the Tauri window is still to be checked on this Mac.

### Glacier look and the icon blue (owner, 2026-11-12)

- **The accent is the app icon's blue.** `#2b55f0` in light mode (white text on it 5.7:1), `#2f5bff` for dark-mode buttons (5.2:1) and `#8ab0ff` for dark-mode links (7:1). The same blue is in the sidebar tiles, the dictation capsule and the markup overlay.
- **Onboarding.** A framed Glacier band across the top: tall on the welcome step, with the app icon half on it, and a slim ribbon on the steps after.
  - The content sits on the plain window surface. No frosted card floats on the art (the owner found that unnatural).
  - On Windows the window is frameless, the band runs to the window's edges, and Minimize and Close sit on it.
- **Settings.** A soft Glacier wash from the window's top edge down, fading as it goes, under the sidebar and the page alike. There is no bar across the top. On Windows the caption buttons float on it, over a clear strip that drags the window and frosts once the page scrolls.
- **How it's built** (`src/lib/backdrop.ts`, `src/components/Backdrop.tsx`, `src/components/WinBar.tsx`):
  - The painting is one SVG image, built once, and moved only by transform.
  - It drifts slowly, rests while the window is in the background, and holds still under reduced motion. The wash goes away under Reduce Transparency.
- **Reference study.** `docs/design/reference-study.md` covers Cluely, Clicky, Wispr Flow, Superwhisper, Raycast and xAI. Rules adopted so far:
  - the dictation capsule breathes too (two layers), which answers the earlier open question;
  - a widened Ask panel keeps a 680 px text column;
  - every window is drawn before it is shown (already true: all windows start hidden).

## Next (the assistant direction)

The owner's vision is a Siri-like, all-in-one assistant on Mac and Windows: you talk and point, and it answers or acts, through whichever AI you already use.

1. **Sessions you come back to.** Every Ask session is saved already (`sessions/`).
   - The new parts: the panel offers to resume the last session when you log on or open it, and Home shows Ask and dictation on one timeline (next-layer S1).
   - Pick a session from the panel's session list or from Home.
2. **Continue in the AI app.** From an Ask session, open the same conversation in the AI's own app with its words and pictures carried over:
   - Claude: Claude Code via `claude --resume`, or the desktop app via a deep link where one exists;
   - ChatGPT: Codex `thread/resume`;
   - otherwise copy the transcript and pictures, and open the app.
   - The handoff carries the session summary (S3's cheap summariser) so the other app starts with the context.
3. **Model picking everywhere.** Model and Thinking rows for every AI, in onboarding and in the panel's header (next-layer S2).
4. **Then acting.** Read-only screen tools, then tiered actions, then multi-step tasks (next-layer S5–S7).

## Open for the owner

- On log-on, should the panel resume the last session automatically, or offer it with one click? Suggested: offer it.
