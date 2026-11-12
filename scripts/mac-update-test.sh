#!/usr/bin/env bash
# The macOS update test that .github/workflows/update-test.yml runs. It installs into Applications and closes
# every running Pointcast, so it is for a runner, never for a Mac in use.
#
#   mac-update-test.sh <Pointcast.app as the build signed it> <the library's test binary>
#
# Run after `mac-signing.sh setup` with a certificate, while its keychain is still there. From the built app
# (version N) it makes N+1 with the same certificate and an ad hoc copy of each, deletes the keychain (a person's
# Mac has no certificate either) and then checks, with the real install.sh and a stand-in tccutil that only
# writes down what it was asked:
#   a  N and N+1 are different code with one designated requirement;
#   b  N+1 meets N's requirement, and an ad hoc copy of it does not;
#   c  install.sh over a certificate-signed N leaves N+1 running and resets no permission; over an ad hoc N,
#      or from an ad hoc N to a signed N+1, it resets them and says macOS may ask again;
#   d0 the reason install.sh asks pgrep and pkill for ancestors: without -a, a script that Pointcast started does
#      not see Pointcast at all;
#   d  started as the app starts it (update.rs `run`, from a process named pointcast that the installer closes),
#      the installer lives on, swaps the app and opens N+1: one Pointcast, the new one, and no permission reset;
#   e  the one-line install finds its download in a release's latest.json without GitHub's API.
# What it cannot see: macOS itself keeping a permission across the update. That follows from b and is checked
# by hand (docs/SIGNING.md).
set -euo pipefail

# No apostrophe inside ${...}: the bash of macOS reads one there as the start of a quoted string.
app=${1:?usage: mac-update-test.sh <Pointcast.app> <test binary of the library>}
parent=${2:?usage: mac-update-test.sh <Pointcast.app> <test binary of the library>}
identity=${APPLE_SIGNING_IDENTITY:?run mac-signing.sh setup with a certificate first}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/.." && pwd)
work=$(mktemp -d)
logs=${POINTCAST_TEST_LOGS:-$work/logs}
NEXT=99.0.0
LIVE=update::tests::on_a_mac_the_installer_outlives_the_pointcast_that_started_it
dest=/Applications
[ -w "$dest" ] || dest="$HOME/Applications"
mkdir -p "$logs" "$work/bin" "$work/n" "$work/n1" "$work/adhoc-n" "$work/adhoc-n1"
export TCCUTIL_CALLS="$work/tccutil.calls"
# Where install.sh looks for the app's note that it asked for Screen Recording; never the runner's own.
export POINTCAST_DATA_DIR="$work/data"
mkdir -p "$POINTCAST_DATA_DIR"

pass() { printf 'PASS  %s\n' "$*"; }
die() {
  printf 'FAIL  %s\n' "$*" >&2
  exit 1
}
cdhash() { codesign -dvvv "$1" 2>&1 | sed -n 's/^CDHash=//p'; }
requirement() { { codesign -d -r- "$1" 2>/dev/null || true; } | sed -n 's/^[# ]*designated => //p'; }
version_of() { /usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$1/Contents/Info.plist"; }
sign() { codesign --force --options runtime --entitlements "$root/src-tauri/Entitlements.plist" --sign "$1" "$2"; }
count() { pgrep -x pointcast | grep -c . || true; }
close_all() {
  pkill -x pointcast >/dev/null 2>&1 || true
  for _ in $(seq 1 40); do
    [ "$(count)" = 0 ] && return 0
    sleep 0.25
  done
  pkill -9 -x pointcast >/dev/null 2>&1 || true
  sleep 1
}
# Nothing installed, nothing running.
fresh() {
  close_all
  [ ! -d "$dest/Pointcast.app" ] || mv "$dest/Pointcast.app" "$work/removed.$(date +%s).$RANDOM"
}
# Wait until exactly one Pointcast runs, from the installed app; print its process id.
one_running() {
  local pids
  for _ in $(seq 1 60); do
    pids=$(pgrep -x pointcast || true)
    if [ "$(printf '%s\n' "$pids" | grep -c . || true)" = 1 ] &&
      [ "$(ps -o comm= -p "$pids" 2>/dev/null || true)" = "$dest/Pointcast.app/Contents/MacOS/pointcast" ]; then
      echo "$pids"
      return 0
    fi
    sleep 1
  done
  ps -axo pid,comm | grep -i pointcast >&2 || true
  return 1
}
# The real install.sh on a dmg, its output kept as <name>.log; tccutil is the stand-in.
install() {
  : >"$TCCUTIL_CALLS"
  echo 0.4.8 >"$POINTCAST_DATA_DIR/screen-asked"
  if ! PATH="$work/bin:$PATH" POINTCAST_TIMING=1 POINTCAST_DMG="$1" bash "$root/install.sh" >"$logs/$2.log" 2>&1; then
    cat "$logs/$2.log"
    die "install.sh stopped on $1"
  fi
  cat "$logs/$2.log"
}
installed_is() {
  [ "$(version_of "$dest/Pointcast.app")" = "$1" ] || die "installed is $(version_of "$dest/Pointcast.app"), not $1"
  [ "$(cdhash "$dest/Pointcast.app")" = "$2" ] || die "the installed app is not the build that was installed"
  codesign --verify --deep --strict "$dest/Pointcast.app" || die "the installed app does not pass codesign --verify"
}
kept_permissions() {
  [ ! -s "$TCCUTIL_CALLS" ] || die "tccutil was called: $(tr '\n' ';' <"$TCCUTIL_CALLS")"
  ! grep -q "may ask again" "$logs/$1.log" || die "the installer said macOS may ask again"
  [ -f "$POINTCAST_DATA_DIR/screen-asked" ] || die "a quiet update deleted the note that Screen Recording was asked for"
  grep -q "Updated to $NEXT[.]$" "$logs/$1.log" || die "the installer did not end on: Updated to $NEXT."
}
# Asked to quit, the running Pointcast ended by itself, before install.sh would have had to close it (3 s).
closed_by_asking() {
  local took
  took=$(awk '$1 == "time" && $2 == "asked" { a = $3 } $1 == "time" && $2 == "closed" { c = $3 } END { if (a != "" && c != "") printf "%.2f", c - a }' "$logs/$1.log")
  [ -n "$took" ] || die "$1: the installer did not say when it asked Pointcast to quit and when it had gone"
  awk -v t="$took" 'BEGIN { exit !(t < 3) }' || die "$1: Pointcast was still running $took s after it was asked to quit"
  echo "$1: gone $took s after it was asked to quit"
}
reset_permissions() {
  grep -q "^reset ScreenCapture dev.pointcast.app$" "$TCCUTIL_CALLS" || die "Screen Recording was not reset"
  grep -q "^reset Accessibility dev.pointcast.app$" "$TCCUTIL_CALLS" || die "Accessibility was not reset"
  grep -q "may ask again" "$logs/$1.log" || die "the installer did not say macOS may ask again"
  [ ! -e "$POINTCAST_DATA_DIR/screen-asked" ] || die "the grants were reset but the note that Screen Recording was asked for stayed"
}

cat >"$work/bin/tccutil" <<'EOF'
#!/bin/sh
echo "$*" >>"$TCCUTIL_CALLS"
EOF
chmod +x "$work/bin/tccutil"

echo "== The versions"
ditto "$app" "$work/n/Pointcast.app"
ditto "$app" "$work/n1/Pointcast.app"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $NEXT" -c "Set :CFBundleVersion $NEXT" "$work/n1/Pointcast.app/Contents/Info.plist"
sign "$identity" "$work/n1/Pointcast.app"
ditto "$work/n/Pointcast.app" "$work/adhoc-n/Pointcast.app"
sign - "$work/adhoc-n/Pointcast.app"
ditto "$work/n1/Pointcast.app" "$work/adhoc-n1/Pointcast.app"
sign - "$work/adhoc-n1/Pointcast.app"
for v in n n1 adhoc-n adhoc-n1; do
  # A runner's disk arbitration is sometimes busy for a moment ("Resource busy"): try again before calling it a fault.
  for try in 1 2 3; do
    hdiutil create -quiet -ov -volname Pointcast -srcfolder "$work/$v" -format UDZO "$work/$v.dmg" && break
    [ "$try" = 3 ] && die "hdiutil create failed three times for $v"
    sleep 3
  done
  echo "-- $v"
  codesign -dvv "$work/$v/Pointcast.app" 2>&1 | grep -E '^(Identifier|CodeDirectory|Signature|Authority|TeamIdentifier)' || true
  echo "designated => $(requirement "$work/$v/Pointcast.app")"
done
N=$(version_of "$work/n/Pointcast.app")
# N+1 was signed here, not by the build: it has to carry what the build gave N (the hardened runtime, the
# entitlements), or it would stand for a different app.
flags() { codesign -dvv "$1" 2>&1 | sed -n 's/^CodeDirectory .*flags=\([^ ]*\).*/\1/p'; }
entitlements() { codesign -d --entitlements - --xml "$1" 2>/dev/null | plutil -convert json -o - - 2>/dev/null || true; }
[ "$(flags "$work/n/Pointcast.app")" = "$(flags "$work/n1/Pointcast.app")" ] ||
  die "N+1 is signed with other flags ($(flags "$work/n1/Pointcast.app")) than the build ($(flags "$work/n/Pointcast.app"))"
[ -n "$(entitlements "$work/n/Pointcast.app")" ] || die "the build's entitlements could not be read"
[ "$(entitlements "$work/n/Pointcast.app")" = "$(entitlements "$work/n1/Pointcast.app")" ] || die "N+1 has other entitlements than the build"
pass "N+1 ($NEXT) is signed as the build signed N ($N): flags $(flags "$work/n1/Pointcast.app")"

echo "== The certificate leaves: from here on this is a Mac that never had it"
bash "$here/mac-signing.sh" cleanup
if security find-identity -p codesigning 2>/dev/null | grep -qi "$identity"; then
  die "the certificate is still in a keychain"
fi

echo "== a: two versions, one requirement"
h_n=$(cdhash "$work/n/Pointcast.app")
h_n1=$(cdhash "$work/n1/Pointcast.app")
h_adhoc=$(cdhash "$work/adhoc-n1/Pointcast.app")
[ -n "$h_n" ] && [ -n "$h_n1" ] || die "no cdhash"
[ "$h_n" != "$h_n1" ] || die "N and N+1 are the same code ($h_n): the test would update an app to itself"
req=$(requirement "$work/n/Pointcast.app")
case $req in
*'identifier "dev.pointcast.app"'*certificate*) ;;
*) die "N's requirement does not name a certificate: $req" ;;
esac
[ "$(requirement "$work/n1/Pointcast.app")" = "$req" ] || die "N+1 has another requirement than N"
pass "cdhash $h_n (N) and $h_n1 (N+1) differ; both: $req"

echo "== b: N+1 meets N's requirement; an ad hoc copy does not"
codesign --verify --deep --strict "$work/n1/Pointcast.app" || die "N+1 does not verify without the certificate in a keychain"
codesign --verify -R="$req" "$work/n1/Pointcast.app" || die "N+1 does not meet N's requirement"
if codesign --verify -R="$req" "$work/adhoc-n1/Pointcast.app" 2>"$logs/b-adhoc.txt"; then
  die "an ad hoc copy met the certificate's requirement: the check tells nothing apart"
fi
cat "$logs/b-adhoc.txt"
pass "codesign --verify -R: 0 for N+1, not 0 for the ad hoc copy"
# The release's own check, both ways: it stops a release that set a certificate up and came out ad hoc, and it
# lets a build without the secrets (a fork, CI) through as ad hoc.
if POINTCAST_SIGNED="$identity" bash "$here/mac-signing.sh" check "$work/adhoc-n1/Pointcast.app" >"$logs/b-check-adhoc.txt" 2>&1; then
  die "mac-signing.sh check let an ad hoc build pass as signed with the certificate"
fi
cat "$logs/b-check-adhoc.txt"
grep -q "not signed with it" "$logs/b-check-adhoc.txt" || die "the check stopped, but not for the signature"
env -u POINTCAST_SIGNED -u MAC_SIGN_P12 -u MAC_SIGN_PASSWORD bash "$here/mac-signing.sh" setup | tee "$logs/b-setup-none.txt"
grep -q "signed ad hoc" "$logs/b-setup-none.txt" || die "setup without the secrets did not say the build is ad hoc"
env -u POINTCAST_SIGNED bash "$here/mac-signing.sh" check "$work/adhoc-n1/Pointcast.app" | tee "$logs/b-check-none.txt"
grep -q "signed ad hoc" "$logs/b-check-none.txt" || die "the check without a certificate did not say the build is ad hoc"
pass "mac-signing.sh check: stops an ad hoc build when a certificate was set up; passes it as ad hoc when none was"

echo "== c1: install.sh, certificate-signed N to N+1"
fresh
install "$work/n.dmg" c1-install
old=$(one_running) || die "N did not start"
install "$work/n1.dmg" c1-update
installed_is "$NEXT" "$h_n1"
new=$(one_running) || die "N+1 is not the one Pointcast running"
[ "$new" != "$old" ] || die "the Pointcast running is still the old process"
kept_permissions c1-update
closed_by_asking c1-update
pass "N+1 installed and running (pid $new, was $old); tccutil not called; no 'may ask again'"

echo "== q: how a running Pointcast closes, asked as install.sh asks"
since() { perl -MTime::HiRes=time -e 'printf "%.2f", time - $ARGV[0]' "${1:-0}"; }
gone_after() {
  local t
  t=$(since)
  for _ in $(seq 1 100); do
    [ "$(count)" = 0 ] && { since "$t"; return 0; }
    sleep 0.1
  done
  echo "still running after 10 s"
}
pid=$(one_running) || die "N+1 is not running before q"
t=$(since)
osascript -e 'quit app "Pointcast"' >"$logs/q-osascript.txt" 2>&1 && said=0 || said=$?
echo "TIMING q osascript-quit returned $said in $(since "$t") s: $(paste -sd ' ' "$logs/q-osascript.txt")"
echo "TIMING q after the quit event, gone in: $(gone_after)"
close_all
open "$dest/Pointcast.app"
t=$(since)
one_running >/dev/null || die "Pointcast did not open again for q"
echo "TIMING q open to one process: $(since "$t") s"
sleep 2
pkill -x pointcast || true
echo "TIMING q after SIGTERM, gone in: $(gone_after)"
close_all

echo "== c2 (control): install.sh, ad hoc N to ad hoc N+1"
fresh
install "$work/adhoc-n.dmg" c2-install
one_running >/dev/null || die "the ad hoc N did not start"
install "$work/adhoc-n1.dmg" c2-update
installed_is "$NEXT" "$h_adhoc"
one_running >/dev/null || die "the ad hoc N+1 is not running"
reset_permissions c2-update
pass "ad hoc to ad hoc: tccutil reset both, and the installer said so"

echo "== c3 (control): install.sh, ad hoc N to certificate-signed N+1, the one move every Mac makes once"
fresh
install "$work/adhoc-n.dmg" c3-install
one_running >/dev/null || die "the ad hoc N did not start"
install "$work/n1.dmg" c3-update
installed_is "$NEXT" "$h_n1"
one_running >/dev/null || die "N+1 is not running"
reset_permissions c3-update
pass "ad hoc to certificate: tccutil reset both, and the installer said so"

echo "== d0 (control): a script started by Pointcast does not see Pointcast unless it asks for its ancestors"
# A shell under Pointcast's name, asking as install.sh asked before and as it asks now.
mkdir -p "$work/as-pointcast"
cp /bin/bash "$work/as-pointcast/pointcast"
fresh
seen=$("$work/as-pointcast/pointcast" -c 'a=$(pgrep -x pointcast | grep -c .); b=$(pgrep -a -x pointcast | grep -c .); echo "$a $b"' || true)
# With -a it counts that shell, and the copies of it that run the two commands.
case $seen in
"0 "[1-9]*) ;;
*) die "from a child of a process named pointcast, pgrep should count none without -a and some with it; got: $seen" ;;
esac
pass "from a child of a process named pointcast, pgrep -x counts 0 and pgrep -a -x counts ${seen#0 }"

echo "== d: the update as the app runs it"
fresh
install "$work/n.dmg" d-install
old=$(one_running) || die "N did not start"
: >"$TCCUTIL_CALLS"
# update.rs `run`, in a process named as Pointcast is and beside the binary's own files.
starter="$(dirname "$parent")/pointcast"
cp "$parent" "$starter"
PATH="$work/bin:$PATH" POINTCAST_TIMING=1 POINTCAST_TEST_DMG="$work/n1.dmg" POINTCAST_TEST_LOG="$logs/d-update.log" \
  "$starter" --ignored --exact "$LIVE" --nocapture >"$logs/d-starter.log" 2>&1 &
starter_pid=$!
status=0
wait "$starter_pid" || status=$?
cat "$logs/d-starter.log"
grep -q "^LIVE pid=$starter_pid starts " "$logs/d-starter.log" || die "the starter never started the installer (did the test run?)"
[ "$status" = 143 ] || die "the starter ended with $status, not closed by the installer (143)"
for _ in $(seq 1 40); do
  kill -0 "$old" 2>/dev/null || break
  sleep 0.25
done
if kill -0 "$old" 2>/dev/null; then
  die "N (pid $old) is still running"
fi
new=$(one_running) || die "after the update there is not exactly one Pointcast running from $dest"
# The installer says its last words a moment after the new app is up.
for _ in $(seq 1 40); do
  grep -q "Updated" "$logs/d-update.log" 2>/dev/null && break
  sleep 0.5
done
cat "$logs/d-update.log"
[ "$new" != "$old" ] && [ "$new" != "$starter_pid" ] || die "the Pointcast running is not a new process"
installed_is "$NEXT" "$h_n1"
kept_permissions d-update
sleep 8
[ "$(count)" = 1 ] && [ "$(pgrep -x pointcast)" = "$new" ] || die "eight seconds on, it is no longer the one Pointcast running"
pass "the installer outlived the process that started it; N (pid $old) ended; N+1 runs alone (pid $new); tccutil not called"

echo "== e: the one-line install reads the release's list"
if [ -z "${GH_TOKEN:-}" ]; then
  echo "SKIP  no GH_TOKEN to read the published release with"
else
  fresh
  mkdir -p "$work/published"
  tag=$(gh release view --repo naidx0/pointcast --json tagName --jq .tagName)
  if gh release download "$tag" --repo naidx0/pointcast -p Pointcast-macOS.dmg -D "$work/published" 2>/dev/null; then
    bash "$here/release-manifest.sh" "$tag" "$work/published" >"$work/latest.json"
    cat "$work/latest.json"
    : >"$TCCUTIL_CALLS"
    # The API's address answers nothing here: the list alone has to be enough.
    if ! PATH="$work/bin:$PATH" POINTCAST_MANIFEST="file://$work/latest.json" POINTCAST_API="http://127.0.0.1:9/" \
      bash "$root/install.sh" >"$logs/e-install.log" 2>&1; then
      cat "$logs/e-install.log"
      die "the one-line install stopped with the list alone"
    fi
    cat "$logs/e-install.log"
    [ "v$(version_of "$dest/Pointcast.app")" = "$tag" ] || die "installed $(version_of "$dest/Pointcast.app"), the list said $tag"
    ! grep -q "GitHub is busy" "$logs/e-install.log" || die "it took the unchecked download, not the list's"
    one_running >/dev/null || die "the published version did not start"
    # Control: a list whose SHA-256 is not the download's is refused, and what is installed stays.
    sed 's/"sha256": "[0-9a-f]\{8\}/"sha256": "00000000/' "$work/latest.json" >"$work/wrong.json"
    if PATH="$work/bin:$PATH" POINTCAST_MANIFEST="file://$work/wrong.json" POINTCAST_API="http://127.0.0.1:9/" \
      bash "$root/install.sh" >"$logs/e-wrong.log" 2>&1; then
      die "a download that does not match the list's SHA-256 was installed"
    fi
    grep -q "does not match the SHA-256" "$logs/e-wrong.log" || die "refused, but not for the SHA-256: $(cat "$logs/e-wrong.log")"
    # Control: no list and no API is a failure that says so, not a silent success.
    if PATH="$work/bin:$PATH" POINTCAST_MANIFEST="file://$work/none.json" POINTCAST_API="http://127.0.0.1:9/" \
      bash "$root/install.sh" >"$logs/e-none.log" 2>&1; then
      die "with no list and no API the install still claimed to work"
    fi
    grep -q "can't reach GitHub" "$logs/e-none.log" || die "no list, no API, and not the words for it: $(cat "$logs/e-none.log")"
    pass "installed $tag from the list with the API unreachable; a wrong SHA-256 is refused; no list falls back to the API"
    # As the README's one line runs it today, against the real GitHub: whatever the published release carries
    # (a list or, before 0.4.8, none) and whatever the API allows this runner, the published version gets installed.
    fresh
    if ! PATH="$work/bin:$PATH" bash "$root/install.sh" >"$logs/e-real.log" 2>&1; then
      cat "$logs/e-real.log"
      die "the one-line install stopped against the real GitHub"
    fi
    cat "$logs/e-real.log"
    [ "v$(version_of "$dest/Pointcast.app")" = "$tag" ] || die "the real one-line install left $(version_of "$dest/Pointcast.app"), not $tag"
    one_running >/dev/null || die "the published version did not start after the real one-line install"
    pass "the one-line install, with nothing overridden, installed $tag"
  else
    echo "SKIP  the published release $tag has no Pointcast-macOS.dmg"
  fi
fi

close_all
echo "== Timings (seconds since each installer started)"
grep -H "^  time " "$logs"/*.log || true
echo "ALL PASS"
