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
  # Closed for the update and the update failed: open the copy that is still there.
  [ "${quit:-0}" = 1 ] && [ -d "${dest:-}/Pointcast.app" ] && open "$dest/Pointcast.app" >/dev/null 2>&1
  exit 1
}
# What macOS knows an app by when it keeps Screen Recording and Accessibility for it: the designated requirement of
# its signature. A build signed with Pointcast's certificate names the certificate, the same in every version; a
# build signed ad hoc names only itself.
requirement() { { codesign -d -r- "$1" 2>/dev/null || true; } | sed -n 's/^[# ]*designated => //p'; }
# POINTCAST_TIMING=1: say how long each step took, in seconds since the start (the update test reads these lines).
since() { perl -MTime::HiRes=time -e 'printf "%.2f", time - $ARGV[0]' "${1:-0}"; }
t0=$([ -z "${POINTCAST_TIMING:-}" ] || since)
mark() { [ -z "${POINTCAST_TIMING:-}" ] || printf '  time %-9s %s\n' "$1" "$(since "$t0")"; }

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
  # Each release lists its own installers in latest.json. Reading it is a download, which GitHub does not count
  # against the requests an hour it allows a network, so it also works in an office, a school or behind a VPN.
  url=""
  want=""
  list=$(curl -fsSL -H 'User-Agent: pointcast-installer' "${POINTCAST_MANIFEST:-$RELEASES/download/latest.json}" 2>/dev/null) || list=""
  mac=$(printf '%s' "$list" | tr -d ' \t\r\n' | sed -n 's/.*"macos":{\([^}]*\)}.*/\1/p')
  listed=$(printf '%s' "$mac" | sed -n 's/.*"url":"\([^"]*\)".*/\1/p')
  sum=$(printf '%s' "$mac" | sed -n 's/.*"sha256":"\([0-9a-f]*\)".*/\1/p')
  case $listed in
  "https://github.com/$REPO/releases/download/"*)
    if [ ${#sum} = 64 ]; then
      url=$listed
      want=$sum
    fi
    ;;
  esac
  # A release from before the list: ask GitHub's API for it.
  if [ -z "$url" ]; then
  resp=$(curl -sSL -H 'User-Agent: pointcast-installer' -w '\n%{http_code}' \
    "${POINTCAST_API:-https://api.github.com/repos/$REPO/releases/latest}" 2>/dev/null) ||
    fail "can't reach GitHub. Check your internet connection and run this again."
  code=${resp##*$'\n'}
  json=${resp%$'\n'*}
  case $code in
  200) ;;
  404) fail "there is no published release yet. See https://github.com/$REPO/releases." ;;
  403 | 429) limited=1 ;;
  *) fail "GitHub answered $code. Try again in a minute." ;;
  esac
  if [ "${limited:-0}" = 1 ]; then
    # Shared networks (offices, schools, VPNs) can use up GitHub's request allowance; the release's fixed download
    # link is not counted against it. GitHub's listed SHA-256 needs that allowance, so this path checks over HTTPS only.
    say "GitHub is busy on this network, so Pointcast downloads the latest version directly."
    url="https://github.com/$REPO/releases/latest/download/Pointcast-macOS.dmg"
    want=""
  else
  # The first .dmg asset: its address, then the SHA-256 GitHub lists for it.
  pick=$(printf '%s' "$json" | tr ',{}' '\n\n\n' | awk '
    function flush() { if (u ~ /[.]dmg$/ && !done) { print u; print d; done = 1 } }
    /^ *"name":/ { flush(); u = ""; d = "" }
    /^ *"digest":/ { d = $0; sub(/^[^:]*: */, "", d); gsub(/"/, "", d) }
    /^ *"browser_download_url":/ { u = $0; sub(/^[^:]*: */, "", u); gsub(/"/, "", u) }
    END { flush() }') || true
  url=$(printf '%s\n' "$pick" | sed -n 1p)
  want=$(printf '%s\n' "$pick" | sed -n 2p | sed 's/^sha256://' | tr 'A-F' 'a-f')
  [ -n "$url" ] || fail "the latest release has no macOS download yet. See $RELEASES."
  printf '%s' "$want" | grep -Eq '^[0-9a-f]{64}$' ||
    fail "GitHub lists no SHA-256 for this download, so it cannot be checked. See $RELEASES."
  fi
  fi
  say "Downloading $(basename "$url")..."
  curl -fL --progress-bar -o "$tmp/Pointcast.dmg" "$url" ||
    fail "the download did not finish. Check your connection and run this again."
  if [ -n "$want" ]; then
    got=$(shasum -a 256 "$tmp/Pointcast.dmg" | awk '{print $1}')
    [ "$got" = "$want" ] || fail "the download does not match the SHA-256 GitHub lists for it. Run this again, or download the .dmg from $RELEASES."
  fi
  dmg=$tmp/Pointcast.dmg
  mark fetched
fi

mkdir -p "$tmp/mnt"
# -noverify: hdiutil would otherwise read and check the whole image before mounting it, seconds on every new
# download. The download was already checked against its SHA-256, and the app's signature is checked below.
hdiutil attach -nobrowse -noautoopen -noverify -quiet -mountpoint "$tmp/mnt" "$dmg" || fail "the disk image would not open."
mark attached
app=$(find "$tmp/mnt" -maxdepth 1 -name '*.app' -print -quit)
[ -n "$app" ] || fail "there is no app inside the disk image."

say "$([ $had = 1 ] && echo Updating || echo Installing)..."
# Copy first, then swap, so a failed copy never leaves the Mac without Pointcast.
new="$dest/.Pointcast.app.new"
ditto "$app" "$new" || fail "could not copy Pointcast into $dest (is the disk full?)."
mark copied
# Downloaded with curl, so macOS has not quarantined it; clear the flag anyway for a copied image.
xattr -dr com.apple.quarantine "$new" 2>/dev/null || true

# macOS keeps what it was allowed when the new version meets the requirement of the one it replaces: so it is from
# one certificate-signed version to the next, and not when the signer changed (an ad hoc build on either side).
# Checking that also checks the whole signature, so an update between signed versions reads the app only once.
kept=0
was=""
[ $had = 1 ] && was=$(requirement "$dest/Pointcast.app")
if [ -n "$was" ] && codesign --verify -R="$was" "$new" >/dev/null 2>&1; then
  kept=1
elif ! codesign --verify "$new" >/dev/null 2>&1; then
  # A build signed with the certificate has to arrive whole: signing it again here would replace the certificate's
  # signature with an ad hoc one, and macOS would no longer know the app. Only an ad hoc build is signed again.
  case $(requirement "$app") in
  *certificate* | *anchor*) fail "the new version's signature did not survive the copy. The Pointcast you had is still there." ;;
  esac
  codesign --force --deep --sign - "$new" >/dev/null 2>&1 || true
fi
mark checked

# An update replaces the running app; Pointcast keeps what you were recording for its next start.
# -a: pgrep and pkill leave out their own ancestors unless told otherwise, and when Pointcast updates itself it is
# this script's ancestor. Without it the running Pointcast was never found or closed, and never became the new one.
if pgrep -a -x pointcast >/dev/null 2>&1; then
  quit=1
  # Asked to quit, Pointcast saves what it was doing and ends by itself. The timeout bounds a Pointcast too busy to
  # answer; it is then closed below.
  osascript -e 'with timeout of 5 seconds' -e 'quit app "Pointcast"' -e 'end timeout' >/dev/null 2>&1 || true
  mark asked
  # Wait until it has gone, and no longer than it takes: opened while the old copy is still closing, the new one
  # would hand over to it and close too, and nothing would be left running. Still there after 3 s, it is closed;
  # after 6 s, ended.
  # Counted in seconds, not turns of the loop: on a busy Mac one pgrep alone can take a tenth of a second.
  asked=$SECONDS
  termed=0
  while pgrep -a -x pointcast >/dev/null 2>&1; do
    waited=$((SECONDS - asked))
    if [ $waited -ge 9 ]; then
      break
    elif [ $waited -ge 6 ]; then
      pkill -9 -a -x pointcast >/dev/null 2>&1 || true
    elif [ $waited -ge 3 ] && [ $termed = 0 ]; then
      termed=1
      pkill -a -x pointcast >/dev/null 2>&1 || true
    fi
    sleep 0.1
  done
  mark closed
fi
rm -rf "$dest/Pointcast.app" || fail "could not replace the Pointcast in $dest (it may belong to another user)."
mv "$new" "$dest/Pointcast.app" || fail "could not move Pointcast into $dest."
mark swapped

if [ $had = 1 ] && [ $kept = 0 ]; then
  # The signer changed, so macOS no longer matches what it allowed to this app: clear the old grants so it asks
  # cleanly instead of showing them on while they no longer apply.
  tccutil reset ScreenCapture dev.pointcast.app >/dev/null 2>&1 || true
  tccutil reset Accessibility dev.pointcast.app >/dev/null 2>&1 || true
  # macOS will show its Screen Recording prompt again, so the note that Pointcast already asked goes too.
  rm -f "${POINTCAST_DATA_DIR:-$HOME/Library/Application Support/dev.pointcast.app}/screen-asked"
fi

open "$dest/Pointcast.app"
mark opened
if [ "${quit:-0}" = 1 ]; then
  # It was running, so it has to be running again: ask once more if macOS did not start it.
  opened=$SECONDS
  until pgrep -a -x pointcast >/dev/null 2>&1; do
    if [ $((SECONDS - opened)) -ge 10 ]; then
      open -n "$dest/Pointcast.app" >/dev/null 2>&1 || true
      break
    fi
    sleep 0.1
  done
  mark running
fi
now=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$dest/Pointcast.app/Contents/Info.plist" 2>/dev/null) || now=""
if [ $had = 1 ] && [ $kept = 1 ]; then
  say "Updated${now:+ to $now}."
elif [ $had = 1 ]; then
  say "Updated${now:+ to $now}. macOS may ask again for Screen Recording and Accessibility: this version is signed differently from the one it replaced."
else
  say "Installed to $dest/Pointcast.app."
  say "Pointcast is in the menu bar (top right). The setup assistant opens now."
fi
printf '\n'
