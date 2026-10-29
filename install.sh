#!/usr/bin/env bash
# Install or update Pointcast on macOS 12 or later (Apple silicon and Intel).
#
#   curl -fsSL https://raw.githubusercontent.com/naidx0/pointcast/main/install.sh | bash
#
# To install a disk image you already have: POINTCAST_DMG=/path/to/Pointcast.dmg bash install.sh
set -euo pipefail

REPO=naidx0/pointcast
say() { printf '  %s\n' "$*"; }
fail() {
  printf '\n  Pointcast was not installed: %s\n\n' "$*" >&2
  exit 1
}

printf '\n  Pointcast\n'
[ "$(uname -s)" = Darwin ] || fail "this script is for macOS. On Windows, in PowerShell: irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex"
ver=$(sw_vers -productVersion)
[ "${ver%%.*}" -ge 12 ] || fail "Pointcast needs macOS 12 or later; this Mac has $ver."

tmp=$(mktemp -d)
cleanup() {
  hdiutil detach -quiet "$tmp/mnt" >/dev/null 2>&1 || true
  rm -rf "$tmp"
}
trap cleanup EXIT

if [ -n "${POINTCAST_DMG:-}" ]; then
  dmg=$POINTCAST_DMG
  [ -f "$dmg" ] || fail "no file at $dmg."
else
  say "Finding the latest version..."
  json=$(curl -fsSL -H 'User-Agent: pointcast-installer' "https://api.github.com/repos/$REPO/releases/latest") ||
    fail "can't reach GitHub. Check your internet connection and run this again."
  url=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*\.dmg"' | head -1 | sed -E 's/.*"(https[^"]*)"$/\1/')
  [ -n "$url" ] || fail "the latest release has no macOS download yet."
  say "Downloading $(basename "$url")..."
  curl -fL --progress-bar -o "$tmp/Pointcast.dmg" "$url" ||
    fail "the download did not finish. Check your connection and run this again."
  dmg=$tmp/Pointcast.dmg
fi

mkdir -p "$tmp/mnt"
hdiutil attach -nobrowse -noautoopen -quiet -mountpoint "$tmp/mnt" "$dmg" || fail "the disk image would not open."
app=$(find "$tmp/mnt" -maxdepth 1 -name '*.app' -print -quit)
[ -n "$app" ] || fail "there is no app inside the disk image."

dest=/Applications
[ -w "$dest" ] || dest="$HOME/Applications"
mkdir -p "$dest"

# An update replaces the running app; Pointcast keeps what you were recording for its next start.
osascript -e 'quit app "Pointcast"' >/dev/null 2>&1 || true
pkill -x pointcast >/dev/null 2>&1 || true
sleep 1

say "Installing..."
rm -rf "$dest/Pointcast.app"
ditto "$app" "$dest/Pointcast.app"
# Downloaded with curl, so macOS has not quarantined it; clear the flag anyway for a copied image.
xattr -dr com.apple.quarantine "$dest/Pointcast.app" 2>/dev/null || true
# Release builds are ad-hoc signed; re-sign only if that signature did not survive the copy.
codesign --verify "$dest/Pointcast.app" >/dev/null 2>&1 ||
  codesign --force --deep --sign - "$dest/Pointcast.app" >/dev/null 2>&1 || true

open "$dest/Pointcast.app"
say "Installed to $dest/Pointcast.app."
say "Pointcast is in the menu bar (top right). The setup assistant opens now."
printf '\n'
