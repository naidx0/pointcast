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

### Linux (X11)

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libssl-dev libasound2-dev libxdo-dev \
  pkg-config libpipewire-0.3-dev clang libclang-dev libx11-dev \
  libfuse2
```

Wayland: tray + global hotkeys + paste injection are best-effort; prefer an X11 session for the full flow.

## Clone and install

```bash
git clone https://github.com/naidx0/pointcast
cd pointcast
npm install
npm run tauri dev
```

## First run

The setup assistant opens on its own the first time (reopen it from the menu: **Setup Assistant…**):

1. **Allow access**: Microphone everywhere; Screen Recording and Accessibility on macOS. After
   turning Screen Recording on, use **Reopen Pointcast** (macOS applies it to a fresh process).
2. **Speech model**: Moonshine Base (111 MB), Tiny (30 MB), Parakeet v2 (482 MB), Whisper Turbo
   (564 MB) or Parakeet v3 (487 MB), plus the 7 to 10 MB sherpa-onnx
   runtime, saved under the OS local-data folder (`dev.pointcast.app/speech`).
3. **Shortcuts**: press each one until it shows a check; click one to record new keys.
4. **Practice run**: a real recording whose prompt stays in the window.

Settings live in `dev.pointcast.app/settings.json` in the same folder. Delete it to see the assistant again.

## UI stack notes

- **Type:** the system face (SF Pro on macOS, Segoe UI Variable on Windows); no web fonts.
- **Materials:** `windowEffects` in `tauri.conf.json` (vibrancy or Liquid Glass on macOS, Mica on
  Windows 11). The page only goes transparent when `platform_info` says the material draws.
- **Icons:** `lucide-react`; app and tray icons are drawn by `scripts/make-icon.py`.

## Run / test

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
npm run tauri dev
```

Browser UI preview (no capture). Add `os=mac` or `os=win` to see either skin, `theme=dark` for dark:

```bash
npm run dev
# http://127.0.0.1:1420/onboarding.html?os=mac&step=keys
# http://127.0.0.1:1420/?os=win&pane=shortcuts
# http://127.0.0.1:1420/indicator.html?preview=1
# http://127.0.0.1:1420/overlay.html?preview=1
# http://127.0.0.1:1420/result.html?preview=1
```

## Build installers

```bash
npm run tauri build
```

Artifacts land under `src-tauri/target/release/bundle/`:

| Platform | Typical outputs |
| --- | --- |
| Linux | `.deb`, AppImage (needs `libfuse2` to run the AppImage) |
| macOS | `.app`, `.dmg` (CI universal) |
| Windows | `.msi` / `.exe` (CI) |

Copy a Linux release into `/opt/cursor/artifacts/` when packaging demos.

## Recording a demo (30–60s)

1. Run `npm run dev` and open `http://127.0.0.1:1420/buggy-shop.html` (or any buggy UI).
2. Start Pointcast and finish the setup assistant.
3. Start/stop shortcut → talk → region shortcut (box, pin, arrow, magnifier) → Return → stop.
4. The prompt pastes into the focused app, or use the menu → **Paste Last Prompt**.
5. Capture with QuickTime, Screen Studio, OBS or Game Bar. Turn on Settings → Capture → Show
   Pointcast in captures first, or the recording capsule and overlay stay out of the video.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| Black captures on macOS | Enable Screen Recording, then quit and relaunch |
| Mic device errors | Pick another mic in Settings → Speech, or restart audio |
| Hotkeys do nothing | Settings → Shortcuts names any chord another app owns; click it and pick another |
| Nothing pastes on macOS | Allow Accessibility (Settings → General → Permissions), or choose Copy in Output |
| AppImage fails to run | Install `libfuse2` (or extract with `--appimage-extract`) |
| Origin/CI Rust edition errors | `rustup update stable` (needs 1.90+) |
