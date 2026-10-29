#!/usr/bin/env bash
# Build the universal macOS release and upload the dmg to the matching GitHub release.
# Run on a Mac, from anywhere in the repo:  scripts/release-mac.sh [--no-upload]
set -euo pipefail

UPLOAD=1
for arg in "$@"; do
  case "$arg" in
    --no-upload) UPLOAD=0 ;;
    -h|--help)
      echo "Usage: scripts/release-mac.sh [--no-upload]"
      echo "Builds the universal dmg and uploads it to the GitHub release for the version in tauri.conf.json."
      exit 0 ;;
    *) echo "Unknown option: $arg (only --no-upload is supported)." >&2; exit 2 ;;
  esac
done

fail() { echo "FAILED: $*" >&2; exit 1; }
step() { echo; echo "==> $*"; }

MOUNT=""
cleanup() {
  if [ -n "$MOUNT" ] && [ -d "$MOUNT" ]; then
    hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || true
    rmdir "$MOUNT" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

[ "$(uname -s)" = "Darwin" ] || fail "this script builds the macOS app and must run on a Mac."

cd "$(dirname "$0")/.."
[ -f src-tauri/tauri.conf.json ] || fail "src-tauri/tauri.conf.json not found; run this from the Pointcast repo."

step "Checking prerequisites"
xcode-select -p >/dev/null 2>&1 || fail "Xcode command line tools are missing. Run: xcode-select --install"
command -v node >/dev/null 2>&1 || fail "node is missing. Install Node.js 20 or newer from https://nodejs.org"
command -v npm >/dev/null 2>&1 || fail "npm is missing. Install Node.js 20 or newer from https://nodejs.org"
command -v rustup >/dev/null 2>&1 || fail "rustup is missing. Install Rust from https://rustup.rs"
command -v cargo >/dev/null 2>&1 || fail "cargo is missing. Install Rust from https://rustup.rs"

INSTALLED_TARGETS="$(rustup target list --installed)"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  if ! printf '%s\n' "$INSTALLED_TARGETS" | grep -qx "$target"; then
    echo "Adding Rust target $target"
    rustup target add "$target" || fail "could not add the Rust target $target."
  fi
done

if [ "$UPLOAD" = "1" ]; then
  command -v gh >/dev/null 2>&1 || fail "the GitHub CLI is missing. Install it with: brew install gh"
  gh auth status >/dev/null 2>&1 || fail "gh is not logged in. Run: gh auth login"
fi

VERSION="$(node -p "require('./src-tauri/tauri.conf.json').version")"
[ -n "$VERSION" ] || fail "could not read the version from src-tauri/tauri.conf.json."
TAG="v$VERSION"
echo "Version $VERSION (release tag $TAG)"

step "Installing dependencies (npm ci)"
npm ci || fail "npm ci did not complete."

step "Building the universal app and dmg"
npm run tauri build -- --target universal-apple-darwin --bundles app,dmg || fail "the Tauri build did not complete."

BUNDLE="src-tauri/target/universal-apple-darwin/release/bundle"
APP="$BUNDLE/macos/Pointcast.app"
DMG="$BUNDLE/dmg/Pointcast_${VERSION}_universal.dmg"
[ -d "$APP" ] || fail "the build finished but $APP was not produced."
[ -f "$DMG" ] || fail "the build finished but $DMG was not produced."

step "Verifying the app signature"
codesign --verify --deep --strict "$APP" || fail "codesign could not verify the signature of $APP."
echo "Signature is valid."

step "Checking the dmg"
MOUNT="$(mktemp -d "${TMPDIR:-/tmp}/pointcast-dmg.XXXXXX")"
hdiutil attach "$DMG" -nobrowse -readonly -noverify -mountpoint "$MOUNT" -quiet || fail "the dmg did not mount."
[ -d "$MOUNT/Pointcast.app" ] || fail "the dmg mounted but does not contain Pointcast.app."
echo "The dmg mounts and contains Pointcast.app."
hdiutil detach "$MOUNT" -quiet || fail "could not unmount the dmg."
rmdir "$MOUNT" 2>/dev/null || true
MOUNT=""

SHA="$(shasum -a 256 "$DMG" | awk '{print $1}')"
SIZE="$(stat -f %z "$DMG")"
echo
echo "File:    $DMG"
echo "Size:    $SIZE bytes"
echo "SHA-256: $SHA"

if [ "$UPLOAD" = "0" ]; then
  echo
  echo "Done (--no-upload): build verified, nothing uploaded."
  exit 0
fi

step "Uploading to GitHub release $TAG"
gh release upload "$TAG" "$DMG" --clobber || fail "the upload to release $TAG failed. Create the release (a draft is fine) with that tag first."

step "Confirming the uploaded file"
REPO="$(gh repo view --json nameWithOwner -q .nameWithOwner)" || fail "could not read the repository name from gh."
NAME="$(basename "$DMG")"
REMOTE="$(gh api "repos/$REPO/releases" --paginate \
  --jq ".[] | select(.tag_name == \"$TAG\") | .assets[] | select(.name == \"$NAME\") | .digest")" \
  || fail "could not read the release assets from GitHub."
REMOTE="${REMOTE#sha256:}"
[ -n "$REMOTE" ] || fail "GitHub reports no digest for $NAME on $TAG."
[ "$REMOTE" = "$SHA" ] || fail "GitHub's digest ($REMOTE) does not match the local SHA-256 ($SHA)."

echo "GitHub digest matches: $REMOTE"
echo
echo "Done. $NAME is on release $TAG. Publish the draft on GitHub when every platform is uploaded."
