# Contributing to Pointcast

Thanks for helping. Pointcast is MIT-licensed. By opening a pull request you agree to license your work under the MIT License.

## Setup

You need Rust 1.90+, Node 20+, and the [Tauri 2 Linux/macOS/Windows prerequisites](https://v2.tauri.app/start/prerequisites/). Full OS notes: [docs/LOCAL_SETUP.md](docs/LOCAL_SETUP.md).

```bash
npm install
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri dev
```

Speech models download on first use into the OS local-data directory (`Pointcast/speech`). They are not committed.

## What to change

- Prompt assembly (markers, HTML clipboard, folder export) lives in `src-tauri/src/prompt.rs` and **must stay covered by unit tests**.
- Capture / privacy hide-list: `capture.rs`, `privacy.rs`, `image_ops.rs`.
- Overlay layer stack (Esc) is `overlay.rs` plus `src/windows/Overlay.tsx`.
- Voice: `src-tauri/src/voice/` (cpal, sherpa-onnx Moonshine v2). Do not link onnxruntime; keep loading it at runtime.

## Tests

```bash
cargo test --manifest-path src-tauri/Cargo.toml
npx tsc --noEmit
```

CI builds macOS (universal) and Windows installers and runs Linux tests.

## Style

- Rust: `cargo fmt` (rustfmt defaults) and small, documented modules.
- TypeScript: strict, no unused-ignore comments as policy.
- Do not add a second component library. shadcn-style primitives live under `src/components/ui`.

## Pull requests

1. One concern per PR.
2. Include tests for prompt/export changes.
3. Mention which OS you actually ran on. Linux-only verification is fine if you say so.
