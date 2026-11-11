#!/usr/bin/env bash
# Print latest.json, the list a release carries of its own installers. Pointcast and install.sh read it from
# https://github.com/naidx0/pointcast/releases/latest/download/latest.json to find a newer version: that is a
# download, which GitHub does not count against the requests an hour it allows a network.
#
#   release-manifest.sh <version> <folder with the installers, downloaded back from the release>
#
# A system whose installer is not in the folder is left out; Pointcast then offers that system nothing.
set -euo pipefail

version=${1:?usage: release-manifest.sh <version> <folder with the installers>}
dir=${2:?usage: release-manifest.sh <version> <folder with the installers>}
version=${version#v}
base="https://github.com/${POINTCAST_REPO:-naidx0/pointcast}/releases/download/v$version"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1"
  else
    shasum -a 256 "$1"
  fi | awk '{ print $1 }' | tr -d '\\' | tr 'A-F' 'a-f'
}

# <system> <the installer's fixed name>
entry() {
  [ -f "$dir/$2" ] || return 0
  printf ',\n  "%s": { "url": "%s/%s", "sha256": "%s" }' "$1" "$base" "$2" "$(sha256 "$dir/$2")"
}

listed="$(entry macos Pointcast-macOS.dmg)$(entry windows Pointcast-Windows-setup.exe)"
if [ -z "$listed" ]; then
  echo "release-manifest: no installer in $dir, so there is nothing to list." >&2
  exit 1
fi
printf '{\n  "version": "%s"%s\n}\n' "$version" "$listed"
