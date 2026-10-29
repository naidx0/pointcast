# Pointcast v4: native on macOS and Windows

**End state.** A first-time user installs Pointcast, finishes onboarding in under two minutes
(permissions, on-device speech model, shortcuts tested live, one practice capture), and from then
on presses one shortcut anywhere to talk, mark the screen and paste one prompt into any AI tool.
The app reads as a native macOS product on a Mac and as a native Windows 11 product on Windows.

## Slices (each gated on its own)

1. **Rename** — GitHub repo `naidx0/claudecode-extrawork` → `naidx0/pointcast`; docs point at it.
   Gate: `gh repo view naidx0/pointcast`.
2. **Native chrome** — system type (SF Pro / Segoe UI Variable), platform tokens (Apple / Fluent),
   real window materials (macOS `sidebar`/`hudWindow` vibrancy, Windows 11 Mica), no fake traffic
   lights, content-protected Pointcast windows (never in a capture), monochrome template tray icon
   on macOS, tray labels that name the real shortcut. Gate: `npm run build`, cargo tests, Windows
   screenshots, macOS CI build.
3. **Onboarding** — its own window on first run: Welcome → Permissions (Microphone, Screen
   Recording, Accessibility for auto-paste on macOS) → Speech model (on-device download with
   progress) → Shortcuts (record your own chord, then press it and watch it light up) → Practice
   (a real session: talk, mark, stop, see the prompt; nothing is pasted anywhere) → Done.
   Gate: walk it end to end on Windows in the real app; `?preview=1` walk in the browser.
4. **Settings** — macOS-style preferences: sidebar panes (General, Shortcuts, Speech, Capture,
   Output, Privacy, About) with grouped rows. Gate: screenshots in both themes.
5. **Recording HUD** — compact capsule at the top of the active screen with a live level meter,
   timer, shot count, region / full / zoom / stop. Gate: real session on Windows.
6. **Annotate** — overlay placed on the pointer's monitor (no macOS fullscreen Space), tools:
   box, pin, arrow, rectangle, highlighter, pen, text, magnify; undo; colours; Enter with no box
   takes the whole screen. Gate: browser preview walk plus a real capture on Windows.
7. **Ship check** — `cargo test`, `tsc`, `npm run build`, `tauri build` on Windows (MSI/NSIS),
   macOS universal build in CI.

Order: 1 → 2 → (3, 4, 5 share the design system from 2) → 6 runs in parallel (owns
`src/windows/Overlay.tsx` and `src/styles/overlay.css` only) → 7.

## Stated edge (out of this program)

- Marketing material, launch posts, the demo video (owner will brief).
- Flipping the repo public, signing and notarising builds (owner decisions; need certificates).
- Cloud speech providers and API keys; accessibility-tree element context (roadmap).
- Live verification on macOS hardware: this machine is Windows. macOS is proven by the CI build
  only, and every macOS-only path is listed as unverified until someone runs it on a Mac.
