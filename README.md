# Pointcast

**Talk, point, paste.** Press one shortcut, say what you see, mark up the screen, and Pointcast
hands you one prompt (your words with the annotated screenshots inline, where you took them) to
paste into ChatGPT, Claude, Cursor, Claude Code, Codex, Slack or any chat box.

It is a small native app for **macOS** (menu bar) and **Windows 11** (system tray). Speech is
transcribed on your computer. Nothing is uploaded.

![Pointcast demo placeholder](docs/demo/placeholder.gif)

## Why

Cursor's Annotate lets you point at a page and talk to the agent, but only inside Cursor's browser,
and only for Cursor. Pointcast does the same for the whole desktop and for every AI tool: a
native Settings window, a terminal, a Figma file, a buggy checkout page in any browser.

## First run

Pointcast opens a short setup assistant the first time it starts:

1. **Welcome.**
2. **Allow access.** Microphone, plus Screen Recording and Accessibility on macOS (Accessibility
   lets it paste for you; it is optional). A live meter shows the mic works.
3. **Speech model.** Pick Moonshine Base (111 MB, most accurate) or Tiny (30 MB, fastest). It
   downloads once and works offline.
4. **Shortcuts.** Press each one; it lights up when it works. Click one and type new keys to change it.
5. **Practice run.** A real recording, start to finish. The prompt appears in the window, nothing
   is pasted anywhere.
6. **Done.** Choose whether to paste automatically and whether to open at login.

You can reopen it any time from the menu: **Setup Assistant…**

## Using it

| Shortcut | macOS | Windows |
| --- | --- | --- |
| Start / stop | ⇧⌘Space | Ctrl+Shift+Space |
| Mark up a region | ⌃⇧2 | Ctrl+Shift+2 |
| Capture the screen | ⌃⇧1 | Ctrl+Shift+1 |
| Capture around the pointer | ⌃⇧Z | Ctrl+Shift+Z |

Any capture shortcut starts a recording if none is running. While you record, a small capsule at
the bottom of the screen shows the time, your voice level and the capture buttons; it never takes
focus from the app you are talking about.

**Mark up a region** freezes the screen. Drag a box (or click once for the whole screen), then:

| Tool | Key | |
| --- | --- | --- |
| Move | V | move or resize the box |
| Pin | P | numbered pin with a label; labels go into the prompt |
| Arrow | A | |
| Rectangle | R | |
| Highlighter | H | |
| Pen | D | |
| Text | T | |
| Magnifier | M | a 2× loupe of what is under it |

Five colours, undo (⌘Z / Ctrl+Z) and redo. **Return** captures, **Esc** steps back one layer.

When you stop, the prompt is pasted into the app you were using (and kept on the clipboard), or
copied, or saved as `prompt.md` with PNGs: your choice in **Settings → Output**. Rich HTML goes
on the clipboard so chat apps show the pictures; plain text with `[image n]` markers is the fallback.

Prefer to hold a key while you talk? **Settings → Shortcuts → Start or stop by: Holding.** A quick
tap still toggles.

## For coding agents (MCP)

A local MCP server on `127.0.0.1:47821` serves the latest prompt, so Claude Code, Cursor and Codex
can fetch it themselves:

```bash
curl http://127.0.0.1:47821/v1/session
# JSON-RPC: POST /mcp with tools/list, then tools/call get_latest_session
```

## Privacy

- Audio stays in memory and is dropped once it is transcribed.
- Pointcast's own windows are excluded from every capture by the OS.
- Password managers (1Password, Bitwarden, KeePass, Dashlane, LastPass, Keeper, NordPass, Enpass,
  RoboForm, Proton Pass, Keychain Access, Credential Manager, Windows Security) are painted out
  before a picture is kept. Add your own in **Settings → Capture → Never capture**.

## Build from source

Prerequisites: Rust 1.90+, Node 20+, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS. Details per platform, troubleshooting and demo tips: [docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md).

```bash
git clone https://github.com/naidx0/pointcast
cd pointcast
npm install
npm run tauri dev          # run
npm run tauri build        # .dmg/.app on macOS, .msi/.exe on Windows
```

Tests:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
```

## Architecture

```
src-tauri/src/
  lib.rs          commands, shortcuts (hold or tap), session flow
  platform.rs     where floating windows go, window levels, start/stop sounds
  permissions.rs  macOS AVFoundation / Screen Recording / Accessibility, Windows mic privacy
  tray.rs         menu bar extra / tray icon and menu
  capture.rs      screen grab, close-up around the pointer, privacy paint-out
  prompt.rs       transcript + [image n] markers → plain, HTML, markdown, folder
  session.rs      a recording: mic, speech worker, captures, export
  voice/          mic, resampler, segmenter, sherpa-onnx Moonshine
  clipboard.rs    HTML + plain clipboard, paste keystroke
  mcp.rs          localhost JSON-RPC + REST
src/windows/      Onboarding, Settings, Indicator (HUD), Overlay (annotate), Result
src/styles/       native.css (Apple and Fluent skins), overlay.css
```

The UI follows each platform: SF Pro, system blue, vibrancy (Liquid Glass on macOS 26) and a
System Settings layout on macOS; Segoe UI Variable, Mica and a Windows 11 Settings layout on
Windows. Light and dark follow the system.

## License

MIT. See [LICENSE](LICENSE) and [CONTRIBUTING](CONTRIBUTING.md). Moonshine (MIT) and sherpa-onnx
(Apache-2.0) are downloaded at setup, not bundled.
