#!/usr/bin/env python3
"""Print what every program and library in a staged helper folder loads, and fail on anything it may not load.

Allowed: files in the folder itself, the operating system's own libraries, and the GPU driver's loader (vulkan-1.dll,
libvulkan.so.1). Never allowed: the VC++ runtime and UCRT redistributables, OpenMP runtimes, and anything else. A
control binary that is known to load the C runtime (by default dumpbin.exe itself, or /bin/ls) is read first: if the
reader does not find it there, the reader is broken and its verdict on the folder would mean nothing.

Usage: check_deps.py windows FOLDER --dumpbin PATH [--control PATH]
       check_deps.py linux FOLDER [--control PATH]
       check_deps.py macos FOLDER [--control PATH] [--min-macos 12.0]
"""
import argparse
import os
import re
import subprocess
import sys

WIN_RUNTIME = re.compile(r"^(vcruntime|msvcp|msvcr|vcomp|concrt|ucrtbase|api-ms-win-crt-|libomp|libgomp|libiomp"
                         r"|libwinpthread|libstdc\+\+|libgcc)", re.I)
WIN_SYSTEM = {
    "kernel32.dll", "advapi32.dll", "user32.dll", "gdi32.dll", "ws2_32.dll", "shell32.dll", "shlwapi.dll",
    "ole32.dll", "oleaut32.dll", "bcrypt.dll", "crypt32.dll", "ntdll.dll", "dbghelp.dll", "version.dll",
    "winmm.dll", "psapi.dll", "iphlpapi.dll", "secur32.dll", "userenv.dll", "setupapi.dll", "cfgmgr32.dll",
    "powrprof.dll", "dxgi.dll", "d3d12.dll",
}
LINUX_SYSTEM = {
    "libc.so.6", "libm.so.6", "libstdc++.so.6", "libgcc_s.so.1", "libpthread.so.0", "libdl.so.2", "librt.so.1",
    "ld-linux-x86-64.so.2",
}
DRIVER = {"vulkan-1.dll", "libvulkan.so.1"}
LINUX_RUNTIME = re.compile(r"^(libgomp|libomp|libiomp|libtbb)", re.I)


def run(cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def pe_deps(dumpbin, path):
    out = run([dumpbin, "/nologo", "/dependents", path])
    return [m.group(1) for m in re.finditer(r"(?im)^\s+([\w.+\-]+\.dll)\s*$", out)]


def elf_deps(path):
    return re.findall(r"\(NEEDED\)\s+Shared library: \[([^\]]+)\]", run(["readelf", "-d", path]))


def macho_deps(path):
    lines = run(["otool", "-L", path]).splitlines()[1:]
    return [l.strip().split(" (")[0] for l in lines if l.strip()]


def classify(os_name, dep, local):
    base = os.path.basename(dep)
    if os_name == "windows":
        low = dep.lower()
        if WIN_RUNTIME.match(low):
            return "C/C++ or OpenMP runtime: NOT ALLOWED"
        if low in local:
            return "in the archive"
        if low in DRIVER:
            return "GPU driver"
        if low in WIN_SYSTEM:
            return "Windows"
        return "unknown: NOT ALLOWED"
    if os_name == "linux":
        if LINUX_RUNTIME.match(dep):
            return "OpenMP/threading runtime: NOT ALLOWED"
        if dep in local:
            return "in the archive"
        if dep in DRIVER:
            return "GPU driver (optional: without it the Vulkan backend is skipped)"
        if dep in LINUX_SYSTEM:
            return "system"
        return "unknown: NOT ALLOWED"
    if dep.startswith(("/usr/lib/", "/System/Library/")):
        return "macOS"
    if base in local and dep.startswith(("@executable_path/", "@loader_path/")):
        return "in the archive"
    return "unknown: NOT ALLOWED"


def deps_of(a, path):
    if a.os == "windows":
        return pe_deps(a.dumpbin, path)
    if a.os == "linux":
        return elf_deps(path)
    return macho_deps(path)


def is_binary(os_name, path):
    if os_name == "windows":
        return path.lower().endswith((".exe", ".dll"))
    with open(path, "rb") as f:
        magic = f.read(4)
    if os_name == "linux":
        return magic == b"\x7fELF"
    return magic in (b"\xcf\xfa\xed\xfe", b"\xce\xfa\xed\xfe", b"\xca\xfe\xba\xbe")


def linux_symbol_floor(path):
    out = run(["objdump", "-T", path])
    floor = {}
    for lib in ("GLIBC", "GLIBCXX", "CXXABI"):
        vs = {tuple(int(x) for x in v.split(".")) for v in re.findall(rf"\b{lib}_([\d.]+)\b", out)}
        if vs:
            floor[lib] = ".".join(map(str, max(vs)))
    return floor


def macos_min(path):
    out = run(["otool", "-l", path])
    m = re.search(r"cmd LC_BUILD_VERSION.*?minos ([\d.]+)", out, re.S) or \
        re.search(r"cmd LC_VERSION_MIN_MACOSX.*?version ([\d.]+)", out, re.S)
    return m.group(1) if m else None


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("os", choices=["windows", "linux", "macos"])
    ap.add_argument("folder")
    ap.add_argument("--dumpbin")
    ap.add_argument("--control")
    ap.add_argument("--min-macos")
    a = ap.parse_args()
    if a.os == "windows" and not a.dumpbin:
        sys.exit("--dumpbin is required on Windows")

    # dumpbin.exe itself links VCRUNTIME140.dll; /bin/ls links the C library.
    control = a.control or {"windows": a.dumpbin, "linux": "/bin/ls", "macos": "/bin/ls"}[a.os]
    seen = deps_of(a, control)
    want = {"windows": WIN_RUNTIME, "linux": re.compile(r"^libc\.so\.6$"),
            "macos": re.compile(r"^/usr/lib/libSystem\.B\.dylib$")}[a.os]
    hits = [d for d in seen if want.match(d.lower() if a.os == "windows" else d)]
    if not hits:
        sys.exit(f"control {control}: read {seen}, expected a C runtime among them; the reader is broken")
    print(f"control {control}: the reader finds {', '.join(hits)} as it should")

    names = sorted(os.listdir(a.folder))
    local = {n.lower() if a.os == "windows" else n for n in names}
    bad, floors = [], {}
    print(f"\nDependencies of every binary in {a.folder}:")
    for n in names:
        p = os.path.join(a.folder, n)
        if not os.path.isfile(p) or not is_binary(a.os, p):
            continue
        print(f"{n}")
        for d in deps_of(a, p):
            kind = classify(a.os, d, local)
            print(f"    {d:<48} {kind}")
            if "NOT ALLOWED" in kind:
                bad.append(f"{n} -> {d}")
        if a.os == "linux":
            rp = re.findall(r"\((?:RUNPATH|RPATH)\)\s+Library (?:runpath|rpath): \[([^\]]+)\]", run(["readelf", "-d", p]))
            print(f"    runpath: {rp[0] if rp else '(none)'}")
            for lib, v in linux_symbol_floor(p).items():
                if tuple(map(int, v.split("."))) > tuple(map(int, floors.get(lib, "0").split("."))):
                    floors[lib] = v
        if a.os == "macos":
            minos = macos_min(p)
            print(f"    minimum macOS: {minos}")
            if a.min_macos and minos != a.min_macos:
                bad.append(f"{n}: minimum macOS {minos}, wanted {a.min_macos}")
    if floors:
        print("\nNewest symbol versions required (the oldest system this runs on has at least these): "
              + ", ".join(f"{k} {v}" for k, v in sorted(floors.items())))
    if bad:
        print("\nNOT ALLOWED:\n  " + "\n  ".join(bad))
        sys.exit(1)
    print("\nEvery dependency is in the archive, part of the OS, or the GPU driver's loader.")


if __name__ == "__main__":
    main()
