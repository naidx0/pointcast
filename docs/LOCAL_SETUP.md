# Local setup — Pointcast

Build and run Pointcast on your machine. Speech models download on first use and stay local.

## Prerequisites

### All platforms

- **Rust** 1.90+ (`rustup` stable)
- **Node.js** 20+ (22 recommended) and npm
- **Git**

```bash
rustc --version
node --version
```

### macOS

1. Install Xcode Command Line Tools: `xcode-select --install`
2. See [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
3. Grant **Microphone** and **Screen Recording** when Pointcast asks (System Settings → Privacy & Security). Quit and relaunch after enabling Screen Recording.

### Windows

1. Visual Studio Build Tools with **Desktop development with C++** (MSVC + Windows SDK)
2. [WebView2 Evergreen](https://developer.microsoft.com/microsoft-edge/webview2/) (usually already installed)

### Linux (nice-to-have)

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libssl-dev libasound2-dev libxdo-dev \
  pkg-config libpipewire-0.3-dev clang libclang-dev libx11-dev
```

## Clone and install

```bash
git clone <repo-url> pointcast
cd pointcast
npm install
```

## One-click setup (recommended)

```bash
npm run tauri dev
```

1. Open **Settings** from the tray (or click the menu-bar / tray icon).
2. Click **Set up Pointcast** — downloads the speech runtime + recommended Moonshine model and requests mic/screen permissions.
3. Optional: if **SuperWhisper** is installed, use **Use existing SuperWhisper** instead of (or alongside) local STT.
4. Advanced options (hotkeys, model pick, export folder, hide-list) stay collapsed until you need them. Stock hotkeys are fine.

Models land under the OS local-data directory (`Pointcast/speech`).

## Run / test

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
npm run tauri dev
```

Browser UI preview (no capture):

```bash
npm run dev
# http://127.0.0.1:1420/?theme=light
# http://127.0.0.1:1420/indicator.html?preview=1
# http://127.0.0.1:1420/overlay.html?preview=1
# http://127.0.0.1:1420/result.html?preview=1
```

Default hotkeys: **Ctrl/Cmd+Shift+Space** session · **Ctrl+Shift+1** full · **Ctrl+Shift+2** region · **Ctrl+Shift+Z** zoom.

## Build installers

```bash
npm run tauri build
```

CI (`.github/workflows/ci.yml`) builds macOS universal and Windows artifacts.

## Recording a demo (30–60s)

1. Open `demo/buggy-shop.html` (or any buggy UI).
2. Start Pointcast; run one-click setup if models are not installed.
3. Session hotkey → talk → Region (box + pin) → optional Zoom → Stop.
4. Show the assembled prompt; click **Copy** / **Paste** / send to a detected tool (ChatGPT, Claude, Cursor).
5. Capture with OBS, QuickTime, Game Bar, or `scripts/record-demo.sh` on Linux X11.
6. Save to `docs/demo/pointcast-demo.mp4`.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| Black captures on macOS | Enable Screen Recording, then quit and relaunch |
| Mic device errors | Pick another mic in Advanced, or restart audio |
| Hotkeys do nothing | Change the chord in Advanced if another app owns it |
| Origin/CI Rust edition errors | `rustup update stable` (needs 1.90+) |
