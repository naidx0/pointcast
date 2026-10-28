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

1. Install [Xcode Command Line Tools](https://developer.apple.com/download/all/):  
   `xcode-select --install`
2. Install Tauri macOS deps (WebKit is part of the OS). See [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/).
3. You will grant **Microphone**, **Screen Recording**, and (later roadmap) **Accessibility** in System Settings the first time Pointcast asks.

### Windows

1. Install [Visual Studio Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) with the **Desktop development with C++** workload (MSVC + Windows SDK).
2. Install [WebView2 Evergreen](https://developer.microsoft.com/microsoft-edge/webview2/) (most Windows 10/11 machines already have it).
3. Optional: [Microsoft C++ Build Tools](https://visualstudio.microsoft.com/visual-cpp-build-tools/) if `cargo` fails to find a linker.

### Linux (nice-to-have)

On Debian/Ubuntu:

```bash
sudo apt-get update
sudo apt-get install -y \
  libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libssl-dev libasound2-dev libxdo-dev \
  pkg-config libpipewire-0.3-dev clang libclang-dev libx11-dev
```

X11 is the well-supported path; Wayland capture is best-effort via `xcap`.

## Clone and install

```bash
git clone <your-fork-or-repo-url> pointcast
cd pointcast
npm install
```

## Speech models (local STT)

Pointcast uses **Moonshine v2** through **sherpa-onnx** (MIT / Apache-2.0). Nothing is uploaded.

1. Start the app (`npm run tauri dev` below).
2. Open **Settings** from the tray.
3. Click **Download runtime**, then **Get Moonshine Tiny v2** (or Base on a stronger machine).
4. Models land under the OS local-data directory (`Pointcast/speech`). They are gitignored.

You can also trigger downloads from Settings while a session is idle.

## Run in development

```bash
# Rust unit tests (prompt assembly, privacy, overlay layers, …)
cargo test --manifest-path src-tauri/Cargo.toml

# Frontend typecheck
npx tsc --noEmit

# Desktop app (Vite + Tauri)
npm run tauri dev
```

Default hotkeys:

| Action | Windows / Linux | macOS |
| --- | --- | --- |
| Start / stop session | Ctrl+Shift+Space | Cmd+Shift+Space |
| Full screenshot | Ctrl+Shift+1 | Ctrl+Shift+1 |
| Region / annotate | Ctrl+Shift+2 | Ctrl+Shift+2 |
| Zoom crop | Ctrl+Shift+Z | Ctrl+Shift+Z |

The floating indicator shows live transcript, mic/screen beads, and Full / Region / Zoom / Stop.

Browser-only UI preview (no capture/STT):

```bash
npm run dev
# open http://127.0.0.1:1420/
# result preview: http://127.0.0.1:1420/result.html?preview=1
```

## Build installers

```bash
npm run tauri build
```

Artifacts appear under `src-tauri/target/release/bundle/`:

- **macOS:** `.app` / `.dmg` (CI also builds **universal** `aarch64` + `x86_64`)
- **Windows:** NSIS / MSI
- **Linux:** `.deb` when WebKit deps are present

CI workflow: `.github/workflows/ci.yml`.

## macOS permissions

Pointcast is a menu-bar extra (`LSUIElement`). On first use:

1. **Microphone** — System Settings → Privacy & Security → Microphone → enable Pointcast.  
   Copy in the prompt: *Pointcast transcribes your voice locally. Audio never leaves this computer.*
2. **Screen Recording** — Privacy & Security → Screen Recording → enable Pointcast, then **quit and relaunch** the app (macOS requires a restart after granting).
3. **Accessibility** — not required for the MVP; reserved for the roadmap (AX element context).

If capture returns a black frame, Screen Recording is usually the cause. If the indicator shows a mic error, check the Microphone toggle and the selected device in Settings.

## Windows notes

- Tray icon lives in the notification area (you may need to “show hidden icons”).
- Global hotkeys use `tauri-plugin-global-shortcut`.
- Password-manager windows matching the hide-list are painted out before a PNG is kept.

## Recording your own demo video

Tips that worked in our Linux VM pass (`scripts/record-demo.sh`):

1. Prepare a sample buggy page (or use `demo/buggy-shop.html`).
2. Start Pointcast; download Tiny STT if you want live transcript in the take.
3. Use a virtual display or a clean desktop; hide unrelated windows.
4. Record with OBS, QuickTime (macOS), Xbox Game Bar (Windows), or `ffmpeg` `x11grab` / `avfoundation`.
5. Flow: open the buggy UI → start session → talk → Region (box + pin) → optional Zoom → Stop → show the assembled prompt / paste into ChatGPT or Claude.
6. Keep it 30–60 seconds. Drop the file at `docs/demo/pointcast-demo.mp4` (or replace the GIF placeholder in the README).

## Troubleshooting

| Symptom | Likely fix |
| --- | --- |
| `tauri` build fails on Rust edition / 1.90 | `rustup update stable` |
| Linux: pipewire / clang errors building `xcap` | Install `libpipewire-0.3-dev` and `clang` |
| Mic: “device is no longer available” | Pick another device in Settings, or reboot audio (Pulse/PipeWire) |
| Overlay never appears | Ensure a session is **Recording**, then Ctrl+Shift+2; check Screen Recording on macOS |
| Hotkeys do nothing | Another app may own the chord — change it in Settings |

## Architecture pointer

See the root [README](../README.md) for pitch, hotkeys table, module map, privacy hide-list, MCP (`127.0.0.1:47821`), and roadmap.
