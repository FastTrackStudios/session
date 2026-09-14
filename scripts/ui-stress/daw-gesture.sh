#!/usr/bin/env bash
# Does the studio drop frames while you actually use it?
#
# The window's own rAF meter is the right instrument for this and only
# this: WebKit caps rAF near 60, so a clean run is a FLAT 62.x with a
# 17 ms worst frame. A dropped frame cannot hide — it shows up as ~33 ms
# (one missed vsync) and drags the rate down. What the meter cannot tell
# you is anything above 62; see `apps/session-daw/src/frame_rate.rs`.
#
# WHAT THIS DISPLAY IS NOT: a GPU. Xvfb rasterises in software, so these
# numbers are a FLOOR, not the developer's screen — the real window has
# a compositor and a real GPU and will do better. That makes a clean run
# here strong evidence and a dirty run worth investigating rather than
# believing outright. It is also the only place input can be driven at
# all: the developer's own window is a Wayland surface that xdotool
# cannot see (see webview-display.sh).
#
#   daw-gesture.sh            # scroll + zoom, report worst frames
set -euo pipefail

DISPLAY_NUM="${FTS_DAW_DISPLAY:-:99}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
OUT="${FTS_DAW_OUT:-$ROOT/target/daw-gesture}"
PROJECT="${SESSION_DAW_PROJECT:-/tmp/fts-drum-practice-cache/set-in-stone/set in stone.practice.RPP}"
BIN="$ROOT/target/debug/session-daw"

mkdir -p "$OUT"
[[ -x "$BIN" ]] || { echo "build first: cargo build -p session-daw" >&2; exit 1; }
# This harness owns its own Xvfb, so it does NOT use daw-display.sh —
# that one places windows on the developer's right-hand monitor, which is
# for the on-screen sweeps (`just daw-sweep`).
[[ -e "$PROJECT" ]] || { echo "no project at $PROJECT — run: just daw-prepare" >&2; exit 1; }

cleanup() {
  [[ -n "${APP_PID:-}" ]] && kill "$APP_PID" 2>/dev/null || true
  [[ -n "${WM_PID:-}" ]] && kill "$WM_PID" 2>/dev/null || true
  [[ -n "${X_PID:-}" ]] && kill "$X_PID" 2>/dev/null || true
}
trap cleanup EXIT

Xvfb "$DISPLAY_NUM" -screen 0 1600x900x24 -nolisten tcp >"$OUT/xvfb.log" 2>&1 &
X_PID=$!
for _ in $(seq 50); do
  DISPLAY="$DISPLAY_NUM" xdotool getdisplaygeometry >/dev/null 2>&1 && break
  sleep 0.1
done
# A window manager, or the window is never mapped or focused and the
# keystrokes land nowhere.
DISPLAY="$DISPLAY_NUM" openbox >"$OUT/wm.log" 2>&1 &
WM_PID=$!
sleep 0.5

DISPLAY="$DISPLAY_NUM" GDK_BACKEND=x11 \
  RUST_LOG="warn,daw_ui::studio=info" \
  SESSION_DAW_PROJECT="$PROJECT" \
  "$BIN" >"$OUT/run.log" 2>&1 &
APP_PID=$!

echo "waiting for the project…"
for _ in $(seq 240); do
  grep -q "project mounted" "$OUT/run.log" 2>/dev/null && break
  sleep 0.5
done
grep -q "project mounted" "$OUT/run.log" || { echo "project never mounted" >&2; exit 1; }
sed -E 's/\x1b\[[0-9;]*[a-zA-Z]//g' "$OUT/run.log" | grep -m1 "project mounted"

W=$(DISPLAY="$DISPLAY_NUM" xdotool search --name "Session" | head -1)
DISPLAY="$DISPLAY_NUM" xdotool windowactivate "$W" 2>/dev/null || true
sleep 3   # let the opening settle; its frames are not the gesture's

# The gesture starts HERE. Everything before this line is startup and
# must not be counted — the first paint of a 65-track session is slow and
# has nothing to do with whether scrolling is smooth.
MARK=$(wc -l < "$OUT/run.log")

# The real pointer, via XTEST. `xdotool ... --window` sends XSendEvent,
# which GTK and WebKit both drop: the run completes and the picture never
# moves. Over the lanes, not the track panel, because that is the pane
# with the material in it.
drive() {
  local button=$1 count=$2
  for _ in $(seq "$count"); do
    DISPLAY="$DISPLAY_NUM" xdotool mousemove 900 500 click "$button"
    sleep 0.05
  done
}

echo "scrolling down…"; drive 5 40
echo "scrolling up…";   drive 4 40
echo "zooming in…"
for _ in $(seq 15); do
  DISPLAY="$DISPLAY_NUM" xdotool mousemove 900 500 keydown ctrl click 4 keyup ctrl
  sleep 0.08
done
echo "zooming out…"
for _ in $(seq 15); do
  DISPLAY="$DISPLAY_NUM" xdotool mousemove 900 500 keydown ctrl click 5 keyup ctrl
  sleep 0.08
done
sleep 1

# Only the frames from the gesture window.
tail -n +"$MARK" "$OUT/run.log" \
  | sed -E 's/\x1b\[[0-9;]*[a-zA-Z]//g' \
  | grep -oE 'ui\.fps=[0-9.]+ +ui\.worst_frame_ms=[0-9.]+' \
  | sed -E 's/ui\.fps=([0-9.]+) +ui\.worst_frame_ms=([0-9.]+)/\1 \2/' \
  > "$OUT/samples.txt"

awk '
  { n++; fps[n]=$1; worst[n]=$2
    if (min=="" || $1<min) min=$1
    if ($2>max) max=$2
    if ($2 >= 25) dropped++
  }
  END {
    if (n == 0) { print "NO SAMPLES — the meter never reported"; exit 1 }
    printf "\nsamples        %d (half-second windows)\n", n
    printf "lowest rate    %.1f fps\n", min
    printf "worst frame    %.1f ms\n", max
    printf "windows >25ms  %d   <- a dropped frame is ~33ms\n", dropped+0
    if (max < 25) print "\nVERDICT: no dropped frames."
    else printf "\nVERDICT: %d window(s) dropped frames.\n", dropped
  }
' "$OUT/samples.txt"
