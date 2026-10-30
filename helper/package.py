#!/usr/bin/env python3
"""Pack a staged helper folder into a .tar.bz2 that packs the same way every time.

Entries are sorted, carry one fixed mtime, owner 0 and no user names; programs and libraries get mode 0755 and text
0644 whatever the build machine's file system said. Symlinks are refused: Pointcast unpacks on Windows too, and pins
every file by its bytes.

Usage: package.py FOLDER ARCHIVE --mtime EPOCH
"""
import argparse
import hashlib
import os
import sys
import tarfile

TEXT = (".txt", ".md", ".json")


def entry(name, mtime, mode, size=0, kind=tarfile.REGTYPE):
    ti = tarfile.TarInfo(name)
    ti.type, ti.mode, ti.size, ti.mtime = kind, mode, size, mtime
    ti.uid = ti.gid = 0
    ti.uname = ti.gname = ""
    return ti


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("folder")
    ap.add_argument("archive")
    ap.add_argument("--mtime", type=int, required=True)
    a = ap.parse_args()

    top = os.path.basename(os.path.normpath(a.folder))
    files, dirs = [], []
    for dirpath, dirnames, filenames in os.walk(a.folder):
        rel = os.path.relpath(dirpath, a.folder).replace(os.sep, "/")
        prefix = "" if rel == "." else rel + "/"
        for d in dirnames:
            if os.path.islink(os.path.join(dirpath, d)):
                sys.exit(f"{os.path.join(dirpath, d)}: symlinks are not allowed in the archive")
            dirs.append(prefix + d)
        for f in filenames:
            p = os.path.join(dirpath, f)
            if os.path.islink(p) or not os.path.isfile(p):
                sys.exit(f"{p}: only regular files may go in the archive")
            files.append(prefix + f)
    if not files:
        sys.exit(f"{a.folder}: nothing to pack")

    with tarfile.open(a.archive, "w:bz2", compresslevel=9, format=tarfile.USTAR_FORMAT) as tf:
        tf.addfile(entry(top, a.mtime, 0o755, kind=tarfile.DIRTYPE))
        for d in sorted(dirs):
            tf.addfile(entry(f"{top}/{d}", a.mtime, 0o755, kind=tarfile.DIRTYPE))
        for rel in sorted(files):
            p = os.path.join(a.folder, *rel.split("/"))
            mode = 0o644 if rel.lower().endswith(TEXT) else 0o755
            size = os.path.getsize(p)
            h = hashlib.sha256()
            with open(p, "rb") as f:
                for block in iter(lambda: f.read(1 << 20), b""):
                    h.update(block)
            with open(p, "rb") as f:
                tf.addfile(entry(f"{top}/{rel}", a.mtime, mode, size), f)
            print(f"{h.hexdigest()}  {size:>10}  {oct(mode)[2:]}  {top}/{rel}")

    h = hashlib.sha256()
    with open(a.archive, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    print(f"{a.archive}: {os.path.getsize(a.archive)} bytes, sha256 {h.hexdigest()}")


if __name__ == "__main__":
    main()
