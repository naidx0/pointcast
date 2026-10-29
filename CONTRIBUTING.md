# Contributing to Pointcast

Thanks for helping. Pointcast is MIT-licensed; by opening a pull request you agree to license your
work under the MIT License.

Bugs and ideas: [open an issue](https://github.com/naidx0/pointcast/issues/new/choose).

## Build

You need Rust 1.90+, Node 20+, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS. Platform notes and troubleshooting: [docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md).

```bash
git clone https://github.com/naidx0/pointcast
cd pointcast
npm install
npm run tauri dev          # run
npm run tauri build        # .dmg/.app on macOS, setup .exe on Windows
```

Speech models download on first use into the OS local-data folder (`dev.pointcast.app/speech`);
they are never committed. To try a different model while developing, pick it in Settings → Speech.

## Tests

```bash
cd src-tauri && cargo test --lib
npm run build
```

`npm run build` type-checks the frontend and builds it. Two tests are opt-in because they download
models or need local archives: `real_install` and `every_model` in `src-tauri/src/voice/models.rs`.

## Where things live

```
src-tauri/src/
  lib.rs          commands, shortcuts (hold or tap), session flow
  chord.rs        shortcut parsing, defaults per OS, reserved keys
  platform.rs     where floating windows go, window levels, start/stop sounds
  permissions.rs  macOS Microphone / Screen Recording / Accessibility, Windows mic privacy
  tray.rs         menu bar extra / tray icon and menu
  update.rs       checks GitHub for a newer release, runs the installer
  capture.rs      screen grab, close-up around the pointer
  privacy.rs      paint-out of password managers and apps you exclude
  pointer.rs      where the pointer went during a recording
  element.rs      the control under the pointer (UI Automation / Accessibility)
  prompt.rs       transcript + [image n] markers → plain, HTML, markdown, folder
  session.rs      a recording: mic, speech worker, captures, export
  history.rs      finished prompts on disk, and recovery of a recording in progress
  voice/          mic, resampler, segmenter, model catalog and installer, sherpa-onnx
  clipboard.rs    HTML + plain clipboard, paste keystroke
  mcp.rs          localhost MCP (JSON-RPC) and REST server
src/windows/      Onboarding, Settings, Indicator (capsule), Overlay (mark-up), Result (History)
src/components/   shared pieces
src/styles/       native.css (Apple and Fluent skins), overlay.css
```

- Prompt assembly is in `prompt.rs`. Keep it covered by unit tests.
- The speech runtime (sherpa-onnx) is loaded at run time, never linked. New models go in
  `src-tauri/models.json` with their size and SHA-256 for every archive and file.
- The UI follows each platform: SF Pro, system blue, vibrancy (Liquid Glass on macOS 26) and a System
  Settings layout on macOS; Segoe UI Variable, Mica and a Windows 11 Settings layout on Windows.

## The MCP server

`mcp.rs` serves the latest prompt on `127.0.0.1:47821`. Every request needs `Authorization: Bearer <key>`,
and requests from web pages are refused. The key is made on first run and kept in `mcp-token` in the
data folder (`%LOCALAPPDATA%/dev.pointcast.app` on Windows, `~/Library/Application Support/dev.pointcast.app`
on macOS). It belongs in user settings only, never in a project file such as `.mcp.json`.

```bash
claude mcp add --transport http --scope user pointcast http://127.0.0.1:47821/mcp \
  --header "Authorization: Bearer $(cat ~/Library/Application\ Support/dev.pointcast.app/mcp-token)"
curl -H "Authorization: Bearer <key>" http://127.0.0.1:47821/v1/session
# JSON-RPC: POST /mcp with tools/list, then tools/call get_latest_session
```

A Cursor entry for `mcpServers` in `~/.cursor/mcp.json`:

```json
"pointcast": {
  "type": "http",
  "url": "http://127.0.0.1:47821/mcp",
  "headers": { "Authorization": "Bearer <key>" }
}
```

## Releases

Bump the version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json` (all three
must match), commit, and create a draft GitHub release tagged `v<version>`. Then build and upload on each
OS with the [GitHub CLI](https://cli.github.com) logged in (full steps in
[docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md#making-a-release)):

- macOS: `scripts/release-mac.sh` builds the universal `.dmg` and uploads it.
- Windows: `scripts/release-windows.ps1` builds `Pointcast_<version>_x64-setup.exe`, prints its SHA-256,
  uploads it and checks GitHub's digest against the local one.

Publish the draft once every platform is uploaded. Pass `--no-upload` / `-NoUpload` to build only.
The install scripts and the in-app updater check each download against the SHA-256 GitHub publishes
for it.

## Style

- Rust: `cargo fmt`, small modules, plain-English test names that state the behaviour.
- TypeScript: strict. No new UI library; follow the platform skins in `native.css`.
- Copy: short, plain sentences in the app's voice. Say what happens, not how clever it is.
- Commits: one change each, with a message that says what the person using Pointcast sees, for example
  `Setup assistant: the practice run says when the microphone is silent`.

## Pull requests

1. One concern per pull request.
2. Include tests for behaviour changes.
3. Say which OS you ran it on.
