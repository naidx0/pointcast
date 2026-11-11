#!/usr/bin/env bash
# The macOS signing steps, the same for the release and for the update test (docs/SIGNING.md).
#
#   mac-signing.sh setup          With MAC_SIGN_P12 (a .p12 in base64) and MAC_SIGN_PASSWORD set: put the
#                                 certificate in a keychain of its own, prove codesign can sign with it and name
#                                 it to the build (APPLE_SIGNING_IDENTITY). Without them: nothing, and the build
#                                 is signed ad hoc as tauri.conf.json says.
#   mac-signing.sh check <path>   Print the designated requirement of each .app (or of the app inside each .dmg).
#                                 After a setup with a certificate, fail unless it names that certificate.
#   mac-signing.sh cleanup        Delete the keychain.
set -euo pipefail

KEYCHAIN="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/pointcast-signing.keychain-db"
BUNDLE_ID=dev.pointcast.app
stop() {
  echo "mac-signing: $*" >&2
  exit 1
}
requirement() { { codesign -d -r- "$1" 2>/dev/null || true; } | sed -n 's/^[# ]*designated => //p'; }
lower() { printf '%s' "$1" | tr 'A-F' 'a-f'; }

setup() {
  if [ -z "${MAC_SIGN_P12:-}" ] || [ -z "${MAC_SIGN_PASSWORD:-}" ]; then
    echo "No signing certificate here (MAC_SIGN_P12 and MAC_SIGN_PASSWORD): this build is signed ad hoc."
    return 0
  fi
  local lock identity
  # Not local: the trap runs when the script ends, after this function has returned.
  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  printf '%s' "$MAC_SIGN_P12" | openssl base64 -d -A >"$work/cert.p12" || stop "MAC_SIGN_P12 is not base64."
  [ -s "$work/cert.p12" ] || stop "MAC_SIGN_P12 is empty once decoded."

  lock=$(openssl rand -hex 24)
  security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
  security create-keychain -p "$lock" "$KEYCHAIN"
  security set-keychain-settings "$KEYCHAIN"
  security unlock-keychain -p "$lock" "$KEYCHAIN"
  security import "$work/cert.p12" -f pkcs12 -k "$KEYCHAIN" -P "$MAC_SIGN_PASSWORD" -T /usr/bin/codesign >/dev/null ||
    stop "the certificate could not be imported (is MAC_SIGN_PASSWORD the password of that .p12?)."
  # Let codesign use the key without a dialog nobody is there to answer.
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$lock" "$KEYCHAIN" >/dev/null
  # codesign looks for an identity in the keychains of the search list: add this one, keep the others.
  security list-keychains -d user | tr -d '"' | sed 's/^ *//' >"$work/keychains"
  {
    echo "$KEYCHAIN"
    cat "$work/keychains"
  } | tr '\n' '\0' | xargs -0 security list-keychains -d user -s

  identity=$(security find-certificate -a -Z "$KEYCHAIN" | awk '/^SHA-1 hash:/ { print $3; exit }')
  [ -n "$identity" ] || stop "the .p12 holds no certificate."
  echo "Signing certificate: SHA-1 $identity"
  security find-identity -p codesigning "$KEYCHAIN" || true

  # Before a build of a quarter of an hour: can codesign sign with it, and does what it signs name the certificate?
  echo probe >"$work/probe"
  codesign --force --sign "$identity" --identifier "$BUNDLE_ID" "$work/probe" ||
    stop "codesign cannot sign with the certificate."
  POINTCAST_SIGNED=$identity check "$work/probe"

  if [ -n "${GITHUB_ENV:-}" ]; then
    {
      echo "APPLE_SIGNING_IDENTITY=$identity"
      echo "POINTCAST_SIGNED=$identity"
    } >>"$GITHUB_ENV"
  fi
  echo "Builds from here on are signed with it."
}

# One app (or signed file): print what macOS will know it by, and hold it to the certificate if there is one.
check_one() {
  local req
  codesign --verify --deep --strict "$1" || stop "$1 does not pass codesign --verify."
  req=$(requirement "$1")
  echo "$1"
  echo "  designated requirement: $req"
  if [ -n "${POINTCAST_SIGNED:-}" ]; then
    case $req in
    *"identifier \"$BUNDLE_ID\""*"certificate"*"H\"$(lower "$POINTCAST_SIGNED")\""*) ;;
    *) stop "a certificate was set up, but this is not signed with it: macOS would not keep its permissions across updates." ;;
    esac
    echo "  names the certificate: yes"
  else
    echo "  signed ad hoc: macOS asks for its permissions again after every update."
  fi
}

check() {
  [ $# -gt 0 ] || stop "check needs a .app or a .dmg."
  local path mnt inside
  for path in "$@"; do
    [ -e "$path" ] || stop "nothing at $path."
    case $path in
    *.dmg)
      mnt=$(mktemp -d)
      hdiutil attach -nobrowse -noautoopen -readonly -quiet -mountpoint "$mnt" "$path" || stop "$path would not open."
      inside=$(find "$mnt" -maxdepth 1 -name '*.app' -print -quit)
      if [ -z "$inside" ]; then
        hdiutil detach -quiet "$mnt" || true
        stop "there is no app inside $path."
      fi
      echo "In $path:"
      (check_one "$inside") || {
        hdiutil detach -quiet "$mnt" || true
        exit 1
      }
      hdiutil detach -quiet "$mnt" || true
      ;;
    *) check_one "$path" ;;
    esac
  done
}

cleanup() {
  security delete-keychain "$KEYCHAIN" >/dev/null 2>&1 || true
}

case ${1:-} in
setup) setup ;;
check)
  shift
  check "$@"
  ;;
cleanup) cleanup ;;
*) stop "usage: mac-signing.sh setup | check <app or dmg>... | cleanup" ;;
esac
