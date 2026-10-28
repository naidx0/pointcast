# Pointcast

**Voice plus annotated screenshots → one prompt you can paste into any AI tool.**

Pointcast lives in the system tray. Hold a hotkey, talk, mark the screen, stop. You get a single prompt — transcript with `[image n]` markers and the pictures inlined at the moment you took them — on the clipboard as rich HTML (so ChatGPT, Claude, Slack show the images) plus a plain-text fallback. Optional auto-paste and a `prompt.md` + PNG folder for CLI agents.

Think SuperWhisper meeting Cursor’s design-mode Annotate, but desktop-wide and tool-agnostic.

![Pointcast demo placeholder](docs/demo/placeholder.gif)

A 48-second Linux VM recording is at [`docs/demo/pointcast-demo.mp4`](docs/demo/pointcast-demo.mp4): the Northwind checkout sample, the floating recording indicator (mic + screen beads), Full / Region / Zoom / Stop, and the assembled prompt with an inline screenshot. Live speech-to-text and the freeze-frame overlay did not appear in that capture (no microphone on the VM; region overlay did not clearly show).

## Why

Cursor Annotate is excellent *inside Cursor’s browser*: click or shift-click, drag a box, draw, dictate a note, Esc backs out a layer, a hint bar names the gestures. It does not work in the rest of the OS, and it only feeds Cursor.

Pointcast fills that gap. Record a walkthrough of a buggy checkout page in Chrome, a native Settings window, or a terminal; paste the result into ChatGPT, Claude, Claude Code, Codex, Cursor, Slack, or a markdown file.

## Install

Step-by-step prerequisites (macOS / Windows / Linux), model download, permissions, and demo tips: **[docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md)**.

### From source

```bash
# prerequisites: Rust 1.90+, Node 20+, Tauri 2 system deps
git clone https://github.com/you/pointcast
cd pointcast
npm install
npm run tauri build
```

CI publishes **macOS universal** (`.dmg` / `.app`) and **Windows** (`.msi` / NSIS) artifacts. Linux is a nice-to-have: `deb` is produced on Ubuntu runners when WebKit is present.

Speech models (Moonshine v2 via sherpa-onnx 2.0.0) download on first use. They stay on disk under the OS local-data directory (`Pointcast/speech`). Nothing is uploaded.

### Permissions

- **macOS:** Microphone + Screen Recording. Pointcast is a menu-bar extra (`LSUIElement`). The first capture and the first listen trigger the system prompts. Copy: *Pointcast transcribes your voice locally. Audio never leaves this computer.*
- **Windows:** tray icon, global hotkeys, WASAPI mic, xcap capture. Password-manager windows are painted out.
- **Linux:** X11 (Wayland is best-effort via xcap). PulseAudio / PipeWire for the mic.

## Usage

1. Launch Pointcast. It sits in the tray / menu bar. Open **Settings** from the icon.
2. Click **Set up Pointcast** (one click: runtime + recommended model + permissions). Advanced options stay collapsed. If SuperWhisper is installed, you can use that instead for dictation.
3. Press **Ctrl+Shift+Space** (Windows/Linux) or **Cmd+Shift+Space** (macOS) to start a session. A compact glass indicator shows the timer and icon actions.
4. While recording:
   - **Ctrl+Shift+1** — full screenshot of the monitor under the pointer
   - **Ctrl+Shift+2** — region mode: freeze-frame overlay, drag a box, click numbered pins, pen / highlight. **Esc** backs out one layer at a time
   - **Ctrl+Shift+Z** — zoom crop of N×N pixels around the pointer (N is in Settings)
5. Press the session hotkey again (or **Stop**). Pointcast copies an HTML clipboard with inline images, writes `Documents/Pointcast/<timestamp>/prompt.md` plus PNGs, and can auto-paste into the focused app.

A tiny local MCP server listens on `127.0.0.1:47821`:

```bash
curl http://127.0.0.1:47821/v1/session
# JSON-RPC tools/list and tools/call get_latest_session on POST /mcp
```

Claude Code, Cursor, and Codex can fetch the latest session from there.

## Default hotkeys

| Action | Windows / Linux | macOS |
| --- | --- | --- |
| Start / stop session | Ctrl+Shift+Space | Cmd+Shift+Space |
| Full screenshot | Ctrl+Shift+1 | Ctrl+Shift+1 |
| Region / annotate | Ctrl+Shift+2 | Ctrl+Shift+2 |
| Zoom crop | Ctrl+Shift+Z | Ctrl+Shift+Z |

macOS system screenshot chords (Cmd+Shift+3/4/5) are refused. All four are configurable.

## Architecture

```
src-tauri/src/
  prompt.rs       assemble [image n] into plain / HTML / markdown / folder
  capture.rs      xcap grab, hide-list paint-over, zoom crop
  overlay.rs      Esc layer stack (select → annotate → pin/draw)
  session.rs      recording session, STT worker, export
  voice/          cpal mic, resampler, segmenter, sherpa-onnx Moonshine v2
  clipboard.rs    HTML + plain via arboard; Ctrl/Cmd+V auto-paste
  mcp.rs          localhost JSON-RPC + REST
  tray.rs         always-on tray / menu bar
  settings.rs     hotkeys, mic, model, output, start-at-login
```

The frontend is four Tauri windows (Vite + React): **settings**, **indicator**, **overlay**, **result**. Visual language follows Sequence Nova tokens (SF system fonts, vibrancy panels, pill controls, system light/dark) with the Sequence four-square mark as the app/tray icon.

Privacy: the hide-list defaults to 1Password, Bitwarden, KeePass, Dashlane, LastPass, Keeper, NordPass, Enpass, RoboForm, Proton Pass, Credential Manager, Windows Security, Keychain Access. Those windows are painted a flat charcoal before the PNG is kept. Audio stays in memory and is dropped after transcription.

## Development

```bash
npm install
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Prompt-assembly tests (transcript + markers + HTML + folder export) live next to `prompt.rs` and are required to stay green.

## Roadmap

- Accessibility-tree element context via **macOS AX** / **Windows UIA** (click a control, attach its role, name, and frame to the note — Cursor Annotate’s missing desktop equivalent)
- Batch mode: queue several notes, then flush one prompt
- Wayland portal capture without painting over our own windows
- Streaming STT partials that are fast enough on Moonshine Tiny
- Signed macOS / Windows releases

## License

MIT. See [LICENSE](LICENSE) and [CONTRIBUTING](CONTRIBUTING.md).

Speech models: Moonshine (Useful Sensors / Moonshine AI) is MIT. sherpa-onnx is Apache-2.0. They are downloaded, not vendored.
