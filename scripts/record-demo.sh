#!/usr/bin/env bash
# Record a ~45s Pointcast flow on the virtual display.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${1:-$ROOT/docs/demo/pointcast-demo.mp4}"
export DISPLAY="${DISPLAY:-:1}"
mkdir -p "$(dirname "$OUT")"

python3 -m http.server 4173 --bind 127.0.0.1 --directory "$ROOT/demo" >/tmp/pc-demo-http.log 2>&1 &
HTTP_PID=$!
cleanup() { kill "$HTTP_PID" "$CH_PID" 2>/dev/null || true; }
trap cleanup EXIT

sleep 0.3
espeak-ng -v en-us -s 140 -w /tmp/pc-voice.wav \
  "The place order button is sitting on top of the total. I am boxing it and dropping a pin." || true

# Start screen capture first so we don't miss the open.
ffmpeg -y -f x11grab -video_size 1920x1200 -framerate 24 -i "${DISPLAY}.0" \
  -t 48 -pix_fmt yuv420p -c:v libx264 -preset veryfast -crf 22 "$OUT" >/tmp/pc-demo-ff.log 2>&1 &
FF_PID=$!
sleep 0.8

google-chrome --no-first-run --disable-gpu --noerrdialogs --disable-infobars \
  --window-size=1400,900 --window-position=80,40 \
  "http://127.0.0.1:4173/buggy-shop.html" >/tmp/pc-chrome.log 2>&1 &
CH_PID=$!
sleep 3

# Click the tray icon (top-left 16×16) to open Settings.
xdotool mousemove 8 8 click 1 || true
sleep 1.5

# Start a recording session (global hotkey).
xdotool key ctrl+shift+space || true
sleep 1.5

# Play TTS into the default sink if one exists (best-effort).
if command -v paplay >/dev/null && [ -f /tmp/pc-voice.wav ]; then
  paplay /tmp/pc-voice.wav >/tmp/pc-paplay.log 2>&1 &
fi

# Region overlay.
xdotool key ctrl+shift+2 || true
sleep 1.2

# Drag a box around the Place order button (right column of the shop).
xdotool mousemove 1020 430 mousedown 1 mousemove 1240 580 mouseup 1 || true
sleep 0.8
# Drop a pin.
xdotool mousemove 1100 500 click 1 || true
sleep 1.2
# Confirm the region.
xdotool key Return || true
sleep 1.2

# Zoom crop.
xdotool mousemove 1100 500 key ctrl+shift+z || true
sleep 1.2

# Stop the session.
xdotool key ctrl+shift+space || true
sleep 3

# Show the last-prompt window if it appeared; otherwise open the walkthrough page.
xdotool search --name "last prompt" windowactivate --sync 2>/dev/null || \
  xdotool search --name "Pointcast" windowactivate --sync 2>/dev/null || true
sleep 4

# End on the assembled-prompt HTML as a fallback view.
google-chrome --no-first-run --disable-gpu --window-size=900,700 --window-position=200,80 \
  "http://127.0.0.1:4173/walkthrough.html" >/tmp/pc-chrome2.log 2>&1 &
sleep 6

kill "$CH_PID" 2>/dev/null || true
wait "$FF_PID" || true
ls -l "$OUT"
echo "Wrote $OUT"
