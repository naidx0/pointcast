#!/usr/bin/env bash
# Build whisper.cpp's whisper-server for one target from the pinned source, stage it with exactly the libraries it
# loads plus the licence files, check what it depends on, and pack it.
#
# Usage: helper/build.sh <windows|linux|macos> <x64|arm64>
# Needs: cmake, a C/C++ toolchain, python3 (PYTHON=python on Windows); VULKAN_SDK set for windows and linux;
#        DUMPBIN (path to dumpbin.exe) on Windows.
# Writes: dist/whisper-helper-<tag>-<os>-<arch>/, dist/whisper-helper-<tag>-<os>-<arch>.tar.bz2,
#         dist/report-<os>-<arch>.txt
set -euo pipefail

os=${1:?usage: build.sh <windows|linux|macos> <x64|arm64>}
arch=${2:?usage: build.sh <windows|linux|macos> <x64|arm64>}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
tmp=${RUNNER_TEMP:-$root/.helper-work}
if [ "$os" = windows ]; then
  # Git Bash: hand native tools (cmake, python, dumpbin) D:/a/... paths, which bash's own tools take too.
  root=$(cd "$here/.." && pwd -W)
  here=$root/helper
  tmp=$(cygpath -m "$tmp")
fi
set -a
. "$here/versions.env"
set +a
py=${PYTHON:-python3}

name=whisper-helper-$WHISPER_TAG-$os-$arch
work=$tmp/whisper-helper-$os-$arch
dist=$root/dist
stage=$dist/$name
report=$dist/report-$os-$arch.txt
rm -rf "$work" "$stage" "$dist/$name.tar.bz2"
mkdir -p "$work" "$stage"

"$py" "$here/fetch.py" "$WHISPER_SRC_URL" "$WHISPER_SRC_SHA256" "$work/src.tar.gz"
# A relative name: GNU tar would read the drive colon of D:/... as a remote host.
(cd "$work" && tar -xzf src.tar.gz)
src=$work/whisper.cpp-${WHISPER_TAG#v}

# Only whisper-server is built. OpenMP stays off everywhere: it would add libgomp, vcomp140.dll or libomp to ship.
flags=(
  -DCMAKE_BUILD_TYPE=Release
  -DWHISPER_BUILD_IS_DEV=OFF
  -DWHISPER_BUILD_TESTS=OFF
  -DWHISPER_BUILD_EXAMPLES=ON
  -DWHISPER_BUILD_SERVER=ON
  -DWHISPER_CURL=OFF
  -DWHISPER_SDL2=OFF
  -DGGML_NATIVE=OFF
  -DGGML_OPENMP=OFF
)
case "$os-$arch" in
  windows-x64)
    # Backends are DLLs picked at run time: Vulkan if the driver offers it, and the best CPU variant for the chip.
    # /MT links the C and C++ runtime into every file, so no VC++ redistributable is needed. whisper.cpp's
    # cmake_minimum_required predates CMP0091, hence the policy default.
    flags+=(-A x64
      -DBUILD_SHARED_LIBS=ON -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_VULKAN=ON
      -DCMAKE_POLICY_DEFAULT_CMP0091=NEW -DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded)
    bin=$work/build/bin/Release
    jobs=${NUMBER_OF_PROCESSORS:-4}
    ;;
  linux-x64)
    # The LunarG tarball keeps the loader's libvulkan.so under lib/VulkanLoader, where FindVulkan does not look.
    flags+=(
      -DBUILD_SHARED_LIBS=ON -DGGML_BACKEND_DL=ON -DGGML_CPU_ALL_VARIANTS=ON -DGGML_VULKAN=ON
      -DVulkan_LIBRARY="$VULKAN_SDK/lib/VulkanLoader/lib/libvulkan.so"
      -DCMAKE_INSTALL_RPATH='$ORIGIN' -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON)
    bin=$work/build/bin
    jobs=$(nproc)
    ;;
  macos-arm64 | macos-x64)
    # One static program: Metal with its shader source embedded (no .metallib beside it). The BLAS backend stays
    # off: ggml calls Accelerate's new LAPACK interface, which exists only from macOS 13.3, so on macOS 12 its first
    # call would jump to a missing symbol. Intel Macs get the Haswell instruction set (AVX2, FMA, F16C), which every
    # Intel Mac that runs macOS 12 has except the 2013 Mac Pro.
    flags+=(-DBUILD_SHARED_LIBS=OFF -DGGML_METAL=ON -DGGML_METAL_EMBED_LIBRARY=ON -DGGML_BLAS=OFF
      -DCMAKE_OSX_DEPLOYMENT_TARGET=12.0)
    if [ "$arch" = arm64 ]; then
      flags+=(-DCMAKE_OSX_ARCHITECTURES=arm64)
    else
      flags+=(-DCMAKE_OSX_ARCHITECTURES=x86_64
        -DGGML_SSE42=ON -DGGML_AVX=ON -DGGML_AVX2=ON -DGGML_FMA=ON -DGGML_F16C=ON -DGGML_BMI2=ON)
    fi
    bin=$work/build/bin
    jobs=$(sysctl -n hw.ncpu)
    ;;
  *)
    echo "unsupported target $os-$arch" >&2
    exit 2
    ;;
esac

cmake -S "$src" -B "$work/build" "${flags[@]}" 2>&1 | tee "$work/configure.log"
cmake --build "$work/build" --config Release --target whisper-server --parallel "$jobs" 2>&1 | tee "$work/build.log"

case "$os" in
  windows)
    cp "$bin/whisper-server.exe" "$bin/whisper.dll" "$bin/ggml.dll" "$bin/ggml-base.dll" "$bin"/ggml-cpu-*.dll \
      "$bin/ggml-vulkan.dll" "$stage/"
    ;;
  linux)
    cp "$bin/whisper-server" "$bin"/libggml-cpu-*.so "$bin/libggml-vulkan.so" "$stage/"
    # Shared libraries go in under the name the loader asks for (their soname), as real files, never symlinks.
    added=1
    while [ "$added" = 1 ]; do
      added=0
      for f in "$stage"/*; do
        for n in $(readelf -d "$f" | sed -n 's/.*(NEEDED).*\[\(.*\)\]/\1/p'); do
          if [ ! -e "$stage/$n" ] && [ -e "$bin/$n" ]; then
            cp -L "$bin/$n" "$stage/$n"
            added=1
          fi
        done
      done
    done
    strip --strip-unneeded "$stage"/*
    ;;
  macos)
    if grep -E "is only available on macOS [0-9.]+ or newer" "$work/build.log"; then
      echo "the build calls an API newer than macOS 12 without a guard: it would crash there" >&2
      exit 1
    fi
    cp "$bin/whisper-server" "$stage/"
    strip "$stage/whisper-server"
    codesign --force --sign - --timestamp=none "$stage/whisper-server"
    codesign --verify --strict --verbose=2 "$stage/whisper-server"
    ;;
esac

{
  echo "# $name"
  echo "whisper.cpp $WHISPER_TAG, source sha256 $WHISPER_SRC_SHA256"
  if [ "$os" != macos ]; then echo "Vulkan SDK $VULKAN_SDK_VERSION ($VULKAN_SDK)"; fi
  echo
  echo "## Configuration (CMakeCache)"
  grep -E '^(CMAKE_BUILD_TYPE|CMAKE_C_COMPILER|CMAKE_CXX_COMPILER|CMAKE_MSVC_RUNTIME_LIBRARY|CMAKE_OSX_[A-Z_]+|BUILD_SHARED_LIBS|GGML_[A-Z0-9_]+|WHISPER_[A-Z0-9_]+|Vulkan_[A-Za-z_]+):' \
    "$work/build/CMakeCache.txt" | sort
  echo
  echo "## Configure log, compiler and backend lines"
  grep -E -i 'compiler identification|whisper.cpp version|ggml version|including|backend|vulkan|metal|blas|openmp|x86 detected|adding cpu' \
    "$work/configure.log" || true
  echo
  echo "## Dependencies"
} > "$report"

if [ "$os" = windows ]; then
  "$py" "$here/check_deps.py" windows "$stage" --dumpbin "${DUMPBIN:?set DUMPBIN to dumpbin.exe}" | tee -a "$report"
elif [ "$os" = macos ]; then
  "$py" "$here/check_deps.py" macos "$stage" --min-macos 12.0 | tee -a "$report"
  echo "architectures: $(lipo -archs "$stage/whisper-server")" | tee -a "$report"
else
  "$py" "$here/check_deps.py" linux "$stage" | tee -a "$report"
  echo "glibc of the build system: $(ldd --version | head -1)" | tee -a "$report"
fi

cp "$src/LICENSE" "$stage/LICENSE.txt"
cp "$here/NOTICE.txt" "$stage/NOTICE.txt"
if [ "$os" != macos ]; then
  {
    echo
    echo "Copyright and licence lines of the Vulkan headers this build used (Vulkan SDK $VULKAN_SDK_VERSION):"
    for h in vulkan/vulkan_core.h vulkan/vulkan.hpp; do
      grep -m 2 -E 'Copyright|SPDX-License-Identifier' "$VULKAN_SDK/include/$h" | sed "s|^|  $h: |"
    done
  } >> "$stage/NOTICE.txt"
fi

{ echo; echo "## Archive"; } >> "$report"
"$py" "$here/package.py" "$stage" "$dist/$name.tar.bz2" --mtime "$ARCHIVE_MTIME" | tee -a "$report"
