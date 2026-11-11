# Roadmap

What I plan to build next in Pointcast, roughly in order. Hours are my own rough estimates. Total: about 70 hours.

## 1. Land the lean dictation program (draft PR #5): about 14 h
- Finish and merge `program/lean-dictation`: deterministic GPU decoding (no flash attention), "Tidy the text", the faster mic start and the idle memory release (4 h for review fixes and a cross-platform test pass).
- Slice 9: decode short pieces in context so one-word takes stop coming out wrong (5 h).
- Slice 11: an opt-in local tidy step that runs only on this computer, off by default (5 h).

## 2. Linux support: about 16 h
- CI already builds and tests on Ubuntu, and the pointer code already survives having no display. The work left is the tray icon, global shortcuts on X11 and Wayland (portal), screen capture through PipeWire, and an AppImage or .deb in the release workflow (12 h).
- Add Linux to install.sh and the updater, and document it in README and LOCAL_SETUP.md (4 h).

## 3. Signing and trust: about 8 h
- Apple Developer ID signing and notarization in `scripts/release-mac.sh`, so the Open Anyway step goes away (4 h).
- Windows code signing in `scripts/release-windows.ps1` and the release workflow (4 h).

## 4. Capture and mark-up: about 12 h
- Multi-monitor captures with mixed DPI: an overlay per display and a correct HiDPI rasterizer (`overlay.rs`, `capture.rs`) (5 h).
- Mark-up: a blur or redact tool for sensitive regions, a numbered step marker and a crop-after-capture step (`Overlay.tsx`, `image_ops.rs`) (5 h).
- Extend the "Never capture" list in `privacy.rs` with window-title rules (2 h).

## 5. Agents over MCP: about 8 h
- Ship `pointcast_list`, `pointcast_get` and `pointcast_shot` from PR #5 with a byte budget, and add an MCP conformance test in CI (4 h).
- One-click agent setup in Settings: write the user-scope config for Claude Code, Cursor and Codex, with a check that it worked (4 h).

## 6. Tests, docs and bugs: about 12 h
- End-to-end smoke tests on macOS and Windows runners: record a fake audio file, capture, check the pasted prompt (`prompt.rs`, `session.rs`) (6 h).
- Make the opt-in `real_install` and `every_model` model tests run nightly (2 h).
- Settings › Speech: microphone level preview and clearer errors for denied permissions (2 h).
- Docs: troubleshooting for Wayland and multi-monitor setups, plus an architecture page (2 h).
