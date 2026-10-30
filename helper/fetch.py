#!/usr/bin/env python3
"""Download a pinned file and keep it only if its SHA-256 matches.

Usage: fetch.py URL SHA256 OUT
"""
import hashlib
import os
import sys
import time
import urllib.request


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def main():
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    url, want, out = sys.argv[1], sys.argv[2].lower(), sys.argv[3]
    if os.path.isfile(out) and sha256_of(out) == want:
        print(f"{out}: already here, sha256 {want}")
        return
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    part = out + ".part"
    for attempt in range(1, 4):
        try:
            req = urllib.request.Request(url, headers={"User-Agent": "pointcast-whisper-helper-ci"})
            h, size = hashlib.sha256(), 0
            with urllib.request.urlopen(req, timeout=120) as r, open(part, "wb") as f:
                for block in iter(lambda: r.read(1 << 20), b""):
                    f.write(block)
                    h.update(block)
                    size += len(block)
            break
        except OSError as e:
            print(f"{url}: attempt {attempt} failed: {e}", file=sys.stderr)
            if attempt == 3:
                raise
            time.sleep(10 * attempt)
    got = h.hexdigest()
    if got != want:
        os.remove(part)
        sys.exit(f"{url}: sha256 {got} ({size} bytes), pinned {want}: refusing it")
    os.replace(part, out)
    print(f"{out}: {size} bytes, sha256 {got} matches the pin")


if __name__ == "__main__":
    main()
