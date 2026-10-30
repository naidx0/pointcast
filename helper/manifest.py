#!/usr/bin/env python3
"""Write manifest.json for a set of helper archives.

For each archive: its name, target, size and SHA-256, the path of whisper-server inside it, and every file inside with
its size, SHA-256 and mode, all read back out of the archive itself. Archives and files are sorted and the JSON is
written with sorted keys, so two manifests of the same archives are byte for byte the same.

Usage: manifest.py ARCHIVE... --out manifest.json [--commit SHA] [--run-url URL]
"""
import argparse
import hashlib
import json
import os
import re
import sys
import tarfile

TARGETS = {
    ("windows", "x64"): "x86_64-pc-windows-msvc",
    ("linux", "x64"): "x86_64-unknown-linux-gnu",
    ("macos", "arm64"): "aarch64-apple-darwin",
    ("macos", "x64"): "x86_64-apple-darwin",
}
BACKENDS = {
    "windows": "Vulkan (needs the GPU driver's vulkan-1.dll) with CPU fallback; CPU variants picked at run time",
    "linux": "Vulkan (needs libvulkan.so.1 and a GPU driver) with CPU fallback; CPU variants picked at run time",
    "macos": "Metal (shaders embedded) with CPU fallback; one static program",
}


def sha256_file(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def read_env(path):
    env = {}
    with open(path) as f:
        for line in f:
            m = re.match(r"^([A-Z0-9_]+)=(\S*)\s*$", line)
            if m:
                env[m.group(1)] = m.group(2)
    return env


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("archives", nargs="+")
    ap.add_argument("--out", required=True)
    ap.add_argument("--commit", default="")
    ap.add_argument("--run-url", default="")
    a = ap.parse_args()
    pins = read_env(os.path.join(os.path.dirname(os.path.abspath(__file__)), "versions.env"))
    tag = pins["WHISPER_TAG"]

    entries = []
    for path in sorted(a.archives, key=os.path.basename):
        name = os.path.basename(path)
        m = re.fullmatch(r"whisper-helper-(v[\d.]+)-(windows|linux|macos)-(x64|arm64)\.tar\.bz2", name)
        if not m or m.group(1) != tag:
            sys.exit(f"{name}: not a whisper-helper-{tag}-<os>-<arch>.tar.bz2 archive")
        os_name, arch = m.group(2), m.group(3)
        top = name[: -len(".tar.bz2")]
        files = []
        with tarfile.open(path, "r:bz2") as tf:
            for mem in tf.getmembers():
                if mem.isdir():
                    continue
                if not mem.isfile():
                    sys.exit(f"{name}: {mem.name} is not a regular file")
                if not mem.name.startswith(top + "/"):
                    sys.exit(f"{name}: {mem.name} is outside the top folder {top}/")
                data = tf.extractfile(mem).read()
                files.append({"path": mem.name, "size": len(data), "sha256": hashlib.sha256(data).hexdigest(),
                              "mode": format(mem.mode & 0o777, "o")})
        files.sort(key=lambda f: f["path"])
        exe = f"{top}/whisper-server" + (".exe" if os_name == "windows" else "")
        if exe not in {f["path"] for f in files}:
            sys.exit(f"{name}: {exe} is missing")
        entries.append({
            "name": name, "os": os_name, "arch": arch, "target": TARGETS[(os_name, arch)],
            "backends": BACKENDS[os_name], "size": os.path.getsize(path), "sha256": sha256_file(path),
            "exe": exe, "files": files,
        })

    manifest = {
        "schema": 1,
        "whisper_cpp": {"tag": tag, "source_url": pins["WHISPER_SRC_URL"], "source_sha256": pins["WHISPER_SRC_SHA256"]},
        "vulkan_sdk": pins["VULKAN_SDK_VERSION"],
        "built_from": {"repo": "naidx0/pointcast", "commit": a.commit, "run": a.run_url},
        "archives": entries,
    }
    with open(a.out, "w", newline="\n") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")
    for e in entries:
        print(f"{e['sha256']}  {e['size']:>10}  {e['name']}  ({len(e['files'])} files)")
    print(f"wrote {a.out}")


if __name__ == "__main__":
    main()
