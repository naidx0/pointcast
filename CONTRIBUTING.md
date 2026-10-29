# Contributing to Pointcast

Thanks for helping. Pointcast is MIT-licensed; by opening a pull request you agree to license your
work under the MIT License.

## Setup

You need Rust 1.90+, Node 20+, and the [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS. Platform notes: [docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md).

```bash
npm install
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Speech models download on first use into the OS local-data folder (`dev.pointcast.app/speech`);
they are never committed. To try a different model while developing, pick it in Settings → Speech.

## Where things live

- Prompt assembly (markers, HTML clipboard, folder export): `src-tauri/src/prompt.rs`. Keep it
  covered by unit tests.
- Capture, privacy paint-out, pointer trail: `capture.rs`, `privacy.rs`, `pointer.rs`, `image_ops.rs`.
- Speech: `src-tauri/src/voice/`. The runtime (sherpa-onnx) is loaded at run time, never linked.
  New models go in `src-tauri/models.json` with their size and SHA-256 for every archive and file.
- Windows and their placement per OS: `platform.rs`; permissions: `permissions.rs`.
- UI: `src/windows/` (one file per window), shared pieces in `src/components/`, one stylesheet
  with a macOS and a Windows skin in `src/styles/native.css`.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
```

Two tests are opt-in because they download or need local archives: `real_install` and
`every_model` in `src-tauri/src/voice/models.rs`.

## Style

- Rust: `cargo fmt`, small modules, plain-English test names that state the behaviour.
- TypeScript: strict. No new UI library; follow the platform skins in `native.css`.
- Copy: short, plain sentences in the app's voice. Say what happens, not how clever it is.

## Pull requests

1. One concern per pull request.
2. Include tests for behaviour changes.
3. Say which OS you ran it on.
