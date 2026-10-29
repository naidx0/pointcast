#!/usr/bin/env bash
# Install or update Pointcast on macOS 12 or later (Apple silicon and Intel).
#
#   curl -fsSL https://raw.githubusercontent.com/naidx0/pointcast/main/install.sh | bash
#
# To install a disk image you already have: POINTCAST_DMG=/path/to/Pointcast.dmg bash install.sh
set -euo pipefail

REPO=naidx0/pointcast
RELEASES="https://github.com/$REPO/releases/latest"
say() { printf '  %s\n' "$*"; }
fail() {
  printf '\n  Pointcast was not installed: %s\n\n' "$*" >&2
  exit 1
}

printf '\n  Pointcast\n'
[ "$(uname -s)" = Darwin ] || fail "this script is for macOS. On Windows, in PowerShell: irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex"
ver=$(sw_vers -productVersion)
[ "${ver%%.*}" -ge 12 ] || fail "Pointcast needs macOS 12 or later; this Mac has $ver."

dest=/Applications
[ -w "$dest" ] || dest="$HOME/Applications"
mkdir -p "$dest"
had=0
[ -d "$dest/Pointcast.app" ] && had=1

tmp=$(mktemp -d)
cleanup() {
  hdiutil detach -quiet "$tmp/mnt" >/dev/null 2>&1 || true
  rm -rf "$tmp" "$dest/.Pointcast.app.new"
}
trap cleanup EXIT

if [ -n "${POINTCAST_DMG:-}" ]; then
  dmg=$POINTCAST_DMG
  [ -f "$dmg" ] || fail "no file at $dmg."
else
  say "Finding the latest version..."
  resp=$(curl -sSL -H 'User-Agent: pointcast-installer' -w '\n%{http_code}' \
    "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null) ||
    fail "can't reach GitHub. Check your internet connection and run this again."
  code=${resp##*$'\n'}
  json=${resp%$'\n'*}
  case $code in
  200) ;;
  404) fail "there is no published release yet. See https://github.com/$REPO/releases." ;;
  403 | 429) fail "GitHub is limiting requests from your network. Try again in a few minutes, or download the .dmg from $RELEASES." ;;
  *) fail "GitHub answered $code. Try again in a minute." ;;
  esac
  url=$(printf '%s' "$json" | grep -o '"browser_download_url": *"[^"]*\.dmg"' | head -1 | sed -E 's/.*"(https[^"]*)"$/\1/' || true)
  [ -n "$url" ] || fail "the latest release has no macOS download yet. See $RELEASES."
  say "Downloading $(basename "$url")..."
  curl -fL --progress-bar -o "$tmp/Pointcast.dmg" "$url" ||
    fail "the download did not finish. Check your connection and run this again."
  dmg=$tmp/Pointcast.dmg
fi

mkdir -p "$tmp/mnt"
hdiutil attach -nobrowse -noautoopen -quiet -mountpoint "$tmp/mnt" "$dmg" || fail "the disk image would not open."
app=$(find "$tmp/mnt" -maxdepth 1 -name '*.app' -print -quit)
[ -n "$app" ] || fail "there is no app inside the disk image."

say "$([ $had = 1 ] && echo Updating || echo Installing)..."
# Copy first, then swap, so a failed copy never leaves the Mac without Pointcast.
ditto "$app" "$dest/.Pointcast.app.new" || fail "could not copy Pointcast into $dest (is the disk full?)."
# Downloaded with curl, so macOS has not quarantined it; clear the flag anyway for a copied image.
xattr -dr com.apple.quarantine "$dest/.Pointcast.app.new" 2>/dev/null || true
# Release builds are ad-hoc signed; re-sign only if that signature did not survive the copy.
codesign --verify "$dest/.Pointcast.app.new" >/dev/null 2>&1 ||
  codesign --force --deep --sign - "$dest/.Pointcast.app.new" >/dev/null 2>&1 || true

# An update replaces the running app; Pointcast keeps what you were recording for its next start.
if pgrep -x pointcast >/dev/null 2>&1; then
  osascript -e 'quit app "Pointcast"' >/dev/null 2>&1 || true
  sleep 1
  pkill -x pointcast >/dev/null 2>&1 || true
fi
rm -rf "$dest/Pointcast.app" || fail "could not replace the Pointcast in $dest (it may belong to another user)."
mv "$dest/.Pointcast.app.new" "$dest/Pointcast.app" || fail "could not move Pointcast into $dest."

if [ $had = 1 ]; then
  # Each build is signed ad hoc, so macOS treats an update as a new app: clear the old grants so
  # it asks cleanly instead of showing them on while they no longer apply.
  tccutil reset ScreenCapture dev.pointcast.app >/dev/null 2>&1 || true
  tccutil reset Accessibility dev.pointcast.app >/dev/null 2>&1 || true
fi

open "$dest/Pointcast.app"
if [ $had = 1 ]; then
  say "Updated. macOS may ask again for Screen Recording and Accessibility: each update is a new build."
else
  say "Installed to $dest/Pointcast.app."
  say "Pointcast is in the menu bar (top right). The setup assistant opens now."
fi
printf '\n'
