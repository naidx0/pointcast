# Speech helper: whisper-server

Pointcast can transcribe with Whisper Turbo on the user's GPU through a helper program: `whisper-server`, the HTTP
server example of [whisper.cpp](https://github.com/ggml-org/whisper.cpp), built by this repository's CI from an
unmodified, pinned whisper.cpp release. The app downloads the archive for its platform, checks it against the pinned
`manifest.json`, starts the server on `127.0.0.1` and posts audio to it.

This folder holds everything that build needs; the workflow is
[`.github/workflows/whisper-helper.yml`](../.github/workflows/whisper-helper.yml).

| File | What it does |
|---|---|
| `versions.env` | Every pinned input: the whisper.cpp tag, its tarball's SHA-256, the Vulkan SDK, the test model. |
| `build.sh` | Fetches and checks the source, configures and builds `whisper-server` for one target, stages it with exactly the libraries it loads, checks them, packs the archive. |
| `check_deps.py` | Prints what every program and library loads and fails on the VC++ runtime, OpenMP runtimes or anything not in the archive or the OS. |
| `package.py` | Packs the staged folder into a `.tar.bz2` that comes out byte for byte the same each time (sorted, one mtime, no owners, no symlinks). |
| `smoke.py` | Unpacks an archive, starts the server the way Pointcast does, transcribes speech and a tone, checks the replies and prints which backend was picked. |
| `manifest.py` | Writes `manifest.json`: every archive and every file inside it with size and SHA-256. |
| `fetch.py` | Downloads a pinned file and keeps it only if its SHA-256 matches. |
| `NOTICE.txt` | Shipped in every archive: what the folder is and the third-party licences. |

## What is pinned

| Input | Pin |
|---|---|
| whisper.cpp | `v1.9.5` (2026-10-06, commit `d1be6fde11ac`), GitHub tag tarball `https://github.com/ggml-org/whisper.cpp/archive/refs/tags/v1.9.5.tar.gz`, SHA-256 `ff1a9053feb509ff9d7729703355541ae9690073a6b1c40eb692c962e0dc1720` (9,626,464 bytes). ggml inside it is 0.26.0. |
| Vulkan SDK (build only) | LunarG 1.4.357.0: Windows installer SHA-256 `81f474711e9042f4cd22b31b2f7a8870db2e428b21586fb43dd80150be97310d`, Linux tarball SHA-256 `0f09bf6a0625e346bf004be70b92907e934a4c76606b323441b2baf3a5a0e66d`. Nothing from it is shipped: the Vulkan loader comes with the GPU driver. |
| Test model (smoke test only) | `ggml-tiny.en.bin` from `ggerganov/whisper.cpp` on Hugging Face at revision `5359861c739e`, 77,704,715 bytes, SHA-256 `921e4cf8686fdd993dcd081a5da5b6c365bfde1162e72b08d75ac75289920b1f`. The speech sample is `samples/jfk.wav` from the pinned source. |
| Runners | `windows-2025`, `ubuntu-24.04` with an `ubuntu:22.04` container, `macos-15` (arm64), `macos-15-intel` (x64). |

## The archives

Each archive is `whisper-helper-<tag>-<os>-<arch>.tar.bz2` (the format Pointcast's installer unpacks, see
`unpack_tar_bz2` in `src-tauri/src/voice/models.rs`), holding one folder of the same name with plain files only.

| Archive | Backends | Runs on | The user's machine needs |
|---|---|---|---|
| `windows-x64` | Vulkan (`ggml-vulkan.dll`), CPU variants loaded at run time (`ggml-cpu-*.dll`: x64, SSE4.2, Sandy Bridge, Haswell, Skylake-X, Cannon Lake, Cascade Lake, Ice Lake, Alder Lake; MSVC cannot build the Ivy Bridge, Piledriver, Cooper Lake, Zen 4 and Sapphire Rapids ones, so those CPUs take the next variant down) | Windows 10/11 x64 | Nothing installed: the C and C++ runtime is linked statically (`/MT`), no VC++ Redistributable. For the GPU, a driver that provides Vulkan (`vulkan-1.dll` plus the driver's ICD; every current NVIDIA, AMD and Intel driver does). Without one it runs on the CPU. |
| `linux-x64` | Vulkan (`libggml-vulkan.so`), CPU variants at run time (`libggml-cpu-*.so`: the 14 from x64 to Sapphire Rapids and Zen 4) | Built on Ubuntu 22.04 (GCC 11.4, glibc 2.35). The newest symbol versions it needs are GLIBC 2.34, GLIBCXX 3.4.30 and CXXABI 1.3.13, so: Ubuntu 22.04, Debian 12, Fedora 36 and later; not RHEL 9 with its stock libstdc++ (3.4.29). | `libstdc++.so.6` from GCC 12 or newer. For the GPU, `libvulkan.so.1` and a Vulkan driver (Mesa or NVIDIA). |
| `macos-arm64` | Metal, shaders embedded in the program (no `.metallib`), CPU otherwise | macOS 12 or newer on Apple Silicon | Nothing. |
| `macos-x64` | Metal (embedded) where the GPU supports it, CPU otherwise | macOS 12 or newer on an Intel Mac with AVX2 (every Intel Mac that runs macOS 12 except the 2013 Mac Pro) | Nothing. |

Common to all: Release build of the pinned source with `WHISPER_BUILD_IS_DEV=OFF`, only the `whisper-server` target,
`GGML_NATIVE=OFF`, OpenMP off (it would add `vcomp140.dll`, `libgomp` or `libomp` to ship), symbols stripped.
Windows and Linux use `BUILD_SHARED_LIBS=ON GGML_BACKEND_DL=ON GGML_CPU_ALL_VARIANTS=ON GGML_VULKAN=ON`; the Linux
libraries find each other through `RUNPATH=$ORIGIN` and sit in the archive under their soname as real files. macOS
is one static program (`BUILD_SHARED_LIBS=OFF GGML_METAL=ON GGML_METAL_EMBED_LIBRARY=ON`), deployment target 12.0,
signed ad hoc (`codesign -s -`): Pointcast's own signing may sign it again when it bundles it. Its BLAS backend is
off (`GGML_BLAS=OFF`): ggml calls Accelerate's new LAPACK interface, which macOS only has from 13.3, so on macOS 12
the first BLAS call would jump to a missing symbol. The build fails if the compiler reports any call that is newer
than macOS 12 and unguarded. ggml's own CPU code still uses Accelerate's vDSP, which macOS 12 has.

The build fails rather than ship something that loads a library it should not: `check_deps.py` lists every import
(`dumpbin /dependents` on Windows, `readelf -d` on Linux, `otool -L` on macOS) and rejects `VCRUNTIME*`, `MSVCP*`,
`api-ms-win-crt-*`, `vcomp*`, `libomp`, `libgomp`, and anything that is neither in the archive, part of the OS, nor
the GPU driver's Vulkan loader. Each build's `report-<os>-<arch>` artifact has the full list.

## manifest.json

```json
{
  "schema": 1,
  "whisper_cpp": { "tag": "v1.9.5", "source_url": "...", "source_sha256": "..." },
  "vulkan_sdk": "1.4.357.0",
  "built_from": { "repo": "naidx0/pointcast", "commit": "<sha>", "run": "<run url>" },
  "archives": [
    {
      "name": "whisper-helper-v1.9.5-windows-x64.tar.bz2", "os": "windows", "arch": "x64",
      "target": "x86_64-pc-windows-msvc", "size": 0, "sha256": "...",
      "exe": "whisper-helper-v1.9.5-windows-x64/whisper-server.exe",
      "files": [ { "path": "whisper-helper-v1.9.5-windows-x64/ggml-base.dll", "size": 0, "sha256": "...", "mode": "755" } ]
    }
  ]
}
```

Paths are relative to the folder the archive is unpacked into, with `/`, like the `files` of a runtime in
`src-tauri/models.json`. Archives and files are sorted and keys are sorted, so the manifest of the same archives is
always the same bytes. Two builds of the same pin are not guaranteed to produce the same binaries (compilers embed
build paths and timestamps), so the app pins one published build's manifest, not the source.

## How Pointcast runs it

```
whisper-server -m MODEL --host 127.0.0.1 --port P --request-path /<random> --inference-path /i-<random> -t 4
```

with the working directory set to the helper's own folder. Per request, a `multipart/form-data` POST to
`/<random>/i-<random>` with the audio as `file` (16 kHz mono WAV) and the fields `beam_size=5`, `temperature=0`,
`response_format=json` and `language` (an ISO code, or `auto`); the reply is `{"text": "..."}`. `GET
/<random>/health` answers `{"status":"ok"}` once the model is loaded: the port only opens after loading, so until
then connections are refused. `-ng` forces the CPU, `-dev N` picks another GPU.

Things the integration has to do, because of how upstream whisper-server and ggml behave (all checked in this
build's smoke test or source):

- **Pass `--request-path` with a random prefix, not only `--inference-path`.** Every other route stays at the root
  otherwise, including `POST /load`, which makes the server load any file path it is sent (the smoke test shows it
  answering 200 without the prefix and 404 with it), and every reply carries `Access-Control-Allow-Origin: *`, so a
  web page that finds the port can call it. With a 128-bit random prefix nothing is reachable without the secret.
- **Start it with the helper's folder as the working directory and without `GGML_BACKEND_PATH` in the
  environment.** ggml loads backends named `ggml-<name>-*.dll` / `libggml-<name>-*.so` from the program's folder
  and from the current directory, and one more from `GGML_BACKEND_PATH`. Hashing the pinned files is not enough on
  its own: the app should also refuse to start the helper if its folder holds a `ggml-*` file the manifest does not
  list.
- The server reads `examples/server/public` relative to the working directory to serve static files; in the
  helper's folder there is none, so nothing is served.
- Which backend it picked is in its stderr: `load_backend: loaded Vulkan backend from ...` and
  `whisper_backend_init_gpu: using Vulkan0 backend` (or `MTL0` on macOS), or `whisper_backend_init_gpu: no GPU found`
  when it runs on the CPU. `load_backend: loaded CPU backend from .../ggml-cpu-haswell.dll` names the CPU variant,
  and `system_info:` its features. A Release build says nothing about a Vulkan library it could not use: on a
  machine with `vulkan-1.dll` but no driver behind it (the Windows CI runner is one) the only line is `no GPU found`.
- On macOS the first start compiles the embedded Metal shaders before the port opens: 27 s (arm64) and 35 s (x64)
  on the CI runners' virtual GPUs, then 0.5 s and 1.3 s on the next start, from the system's shader cache. Allow a
  long wait for the first `/health`.

## Licences

whisper.cpp and ggml are MIT (Copyright (c) 2023-2026 The ggml authors); the archive carries that text as
`LICENSE.txt`. Also compiled in: cpp-httplib 0.20.0 (MIT), JSON for Modern C++ 3.11.2 (MIT), miniaudio 0.11.24
(public domain or MIT-0), stb_vorbis 1.22 (public domain or MIT), and on Windows and Linux the Khronos Vulkan
headers (Apache-2.0 or MIT; MIT chosen). The Windows build links Microsoft's C and C++ runtime statically, which the
Visual Studio licence allows for distributable code. `NOTICE.txt` in each archive lists all of this with the
copyright lines (the Vulkan lines are copied from the headers each build used). Nothing GPL, no CUDA, no OpenMP
runtime, no Vulkan loader is shipped.

## Rebuild

Actions > **Whisper helper** > Run workflow (choose all targets or one), or from a shell:

```
gh workflow run whisper-helper.yml --ref main -f targets=all
gh run watch
gh run download <run-id> -D whisper-helper     # archives, manifest, reports, smoke logs
```

GitHub only dispatches a workflow whose file is on the default branch; until this one is merged, a push to
`feat/whisper-helper` that touches `helper/` or the workflow runs it instead.

Jobs: one build per target (about 5 minutes for Windows and Linux, 1 to 3 for macOS), then a smoke test of each
archive on a fresh runner of the same OS, then `manifest`; the whole run takes about 10 minutes. The artifacts are
`whisper-helper-<tag>-<os>-<arch>` (the archive), `whisper-helper-manifest` (`manifest.json` and `SHA256SUMS`),
`report-<os>-<arch>` (configuration, the full dependency list, file hashes) and `smoke-<os>-<arch>` (server logs).
CI runners have no GPU: the smoke tests prove the archives start, load the model and transcribe on the CPU, and on
Linux a second pass runs the Vulkan backend on Mesa's software GPU (lavapipe). The GPU path on real hardware is
checked by hand with `gh run download`.

To build one target on a machine of that OS: `helper/build.sh <windows|linux|macos> <x64|arm64>` from the repository
root, with cmake, a compiler and Python; for Windows and Linux also `VULKAN_SDK` pointing at the SDK (and on Windows
`DUMPBIN` at `dumpbin.exe`, `PYTHON=python`, run from Git Bash). The result lands in `dist/`.

## Bump the pin

1. Pick the release at <https://github.com/ggml-org/whisper.cpp/releases> (a `vX.Y.Z` tag, not a `bNNNN` build).
2. In `versions.env` set `WHISPER_TAG` and `WHISPER_SRC_URL`, and the hash and time of that tarball:
   ```
   curl -fsSL https://github.com/ggml-org/whisper.cpp/archive/refs/tags/vX.Y.Z.tar.gz | sha256sum
   gh api repos/ggml-org/whisper.cpp/commits/vX.Y.Z --jq .commit.committer.date   # to epoch seconds: ARCHIVE_MTIME
   ```
3. If upstream ggml needs a newer Vulkan SDK (configure fails or warns about glslc), take the version llama.cpp's
   release workflow uses and hash both downloads the same way:
   `https://sdk.lunarg.com/sdk/download/<v>/windows/vulkansdk-windows-X64-<v>.exe` and
   `https://sdk.lunarg.com/sdk/download/<v>/linux/vulkansdk-linux-x86_64-<v>.tar.xz`.
4. Check that the server's flags and routes used above still exist (`examples/server/server.cpp`) and update the
   third-party versions in `NOTICE.txt` and this file (`CPPHTTPLIB_VERSION` in `examples/server/httplib.h`,
   `NLOHMANN_JSON_VERSION_*` in `examples/json.hpp`, the `miniaudio` and `stb_vorbis` headers in `examples/`).
5. Run the workflow, read the smoke logs and the reports, try the Windows archive on a real GPU, then pin the new
   `manifest.json` in the app.
