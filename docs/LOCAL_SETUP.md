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
git clone <repo-url> pointcast
cd pointcast
npm install
```

## One-click setup (recommended)

```bash
npm run tauri dev
```

1. Open the **tray** menu → **Settings…**
2. Click **Set up Pointcast** — downloads the speech runtime + recommended Moonshine model and requests mic/screen permissions.
3. Optional: if **SuperWhisper** is installed (macOS), use **Use existing SuperWhisper** instead of (or alongside) local STT.
4. Advanced options (hotkeys, model pick, export folder, hide-list, zoom) stay collapsed. Stock hotkeys are fine.

Core loop: **tray / ⌃⇧Space → talk → region annotate (box, pin, arrow, text) → Paste into app**.

Models land under the OS local-data directory (`Pointcast/speech`). Default output is **auto-paste** (also keeps a clipboard copy).

## UI stack notes

- **Fonts:** Geist Sans + Geist Mono via `@fontsource-variable/geist` and `@fontsource-variable/geist-mono` (bundled locally — no CDN), with system-ui fallback.
- **Icons:** single set — `lucide-react` at stroke **1.75**, optically centered in 40×40 hit targets (aligned with Sequence Nova’s Lucide paths).
- **uiarc.dev (Arc UI):** inspected for v3. It is a shadcn-style registry (free core + Pro). We did **not** vendor Arc components — Pro is paid, and Pointcast already ships Nova glassbar / pill / toggle patterns that match the product. Patterns borrowed conceptually: quiet pills, large circular icon buttons, segmented controls. Primitives remain local shadcn-style `Button` / `Input` / `Switch` plus Nova CSS.

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

Default hotkeys: **Ctrl/Cmd+Shift+Space** session · **Ctrl+Shift+1** full · **Ctrl+Shift+2** region · **Ctrl+Shift+Z** zoom (Advanced).

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

1. Open `demo/buggy-shop.html` (or any buggy UI).
2. Start Pointcast; run one-click setup if models are not installed.
3. Tray → Record (or session hotkey) → talk → Region (box + pin / arrow / text) → Stop.
4. Click **Paste into app** with a chat input focused, or tray → **Paste last prompt**.
5. Capture with OBS, QuickTime, Game Bar, or `scripts/record-demo.sh` on Linux X11.
6. Save to `docs/demo/pointcast-demo.mp4`.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| Black captures on macOS | Enable Screen Recording, then quit and relaunch |
| Mic device errors | Pick another mic in Advanced, or restart audio |
| Hotkeys do nothing | Change the chord in Advanced if another app owns it |
| AppImage fails to run | Install `libfuse2` (or extract with `--appimage-extract`) |
| Origin/CI Rust edition errors | `rustup update stable` (needs 1.90+) |
