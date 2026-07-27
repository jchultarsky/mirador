#!/usr/bin/env bash
#
# Record docs/demo.gif.
#
# Drives a real mirador under tmux, captures the screen at a fixed rate, and
# writes an asciicast that `agg` turns into a GIF. Nothing here is staged: the
# tasks and the note are the ones mirador seeds on a first run, and the weather
# and prices are whatever the network returned while it was recording.
#
# Requires tmux and agg:
#
#     cargo install --locked --git https://github.com/asciinema/agg
#
# `/docs` is excluded from the published crate, so this script and the GIF it
# makes never ship to crates.io.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="${1:-$ROOT/docs/demo.gif}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/mirador-demo.XXXXXX")"
SESSION="mirador-demo-$$"

# Wide and tall enough for the clock to draw its block numerals: the panel
# degrades to plain text when its row is short, and the numerals are the one
# thing in mirador nobody else's dashboard looks like.
COLS=150
ROWS=42
FPS=5
CAST="$WORK/demo.cast"

cleanup() {
  tmux kill-session -t "$SESSION" 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT

# Built before HOME is redirected: rustup keeps its toolchain config under the
# real one, and a cargo that cannot find it cannot choose a compiler.
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml"
BIN="$ROOT/target/release/mirador"

# A fresh HOME, so the recording shows a genuine first run — the seeded tasks
# and note — rather than whatever is in the author's own files.
export HOME="$WORK/home"
mkdir -p "$HOME"

tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" "$BIN"

# asciicast v2: a header line, then [absolute_time, "o", data] per frame.
python3 - "$CAST" "$COLS" "$ROWS" <<'PY'
import json, sys
path, cols, rows = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])
with open(path, "w") as f:
    json.dump({"version": 2, "width": cols, "height": rows,
               "env": {"TERM": "xterm-256color"}}, f)
    f.write("\n")
PY

# Capture one frame. Each is a full repaint — mirador draws the whole screen
# every time, so a frame is a complete terminal state rather than a diff, which
# is what lets a sampled capture reproduce faithfully.
frame() {
  tmux capture-pane -t "$SESSION" -e -p -S 0 -E "$((ROWS - 1))" \
  | python3 -c '
import json, sys, time
start = float(sys.argv[1])
screen = sys.stdin.read().rstrip("\n").split("\n")
# Home the cursor and clear, so a shorter line never leaves the previous
# frame showing through underneath it.
data = "\x1b[H\x1b[2J" + "\r\n".join(screen)
sys.stdout.write(json.dumps([round(time.time() - start, 3), "o", data]) + "\n")
' "$START" >> "$CAST"
}

# Hold the current screen for N seconds at the capture rate.
hold() {
  local seconds="$1"
  local n
  n=$(python3 -c "print(int($seconds * $FPS))")
  for _ in $(seq 1 "$n"); do
    frame
    python3 -c "import time; time.sleep(1.0 / $FPS)"
  done
}

# Send a key, then hold long enough for the result to be read.
key() {
  tmux send-keys -t "$SESSION" "$1"
  hold "${2:-0.6}"
}

type_text() {
  # One character at a time, so the GIF shows typing rather than a paste.
  local text="$1"
  local i
  for ((i = 0; i < ${#text}; i++)); do
    tmux send-keys -t "$SESSION" -l "${text:$i:1}"
    frame
    python3 -c "import time; time.sleep(0.05)"
  done
}

# Give the CPU and network graphs something to have been watching. An idle
# machine draws a flat line, which is honest and tells the reader nothing about
# what the panel is for. This is real load, not a fake series — it spins up
# during the settle so the history has a hump in it by the time recording
# starts, and has decayed by the end.
LOAD_PIDS=()
for _ in 1 2 3 4 5 6; do
  (end=$((SECONDS + 9)); while [ $SECONDS -lt $end ]; do :; done) &
  LOAD_PIDS+=($!)
done
curl -s -o /dev/null https://github.com 2>/dev/null || true

# Let the panels settle: the weather and price fetches are on their own
# threads, and an empty forecast is not what the dashboard looks like.
sleep 11
wait "${LOAD_PIDS[@]}" 2>/dev/null || true

# The clock starts *after* the settle, so the recording opens on the dashboard
# rather than on six seconds of empty terminal. A GIF is judged on its first
# frame — it is what a reader sees before deciding to watch the rest, and what
# a still preview shows.
START=$(python3 -c 'import time; print(time.time())')

# --- the demo ------------------------------------------------------------

hold 1.6                     # the dashboard, at rest

key Tab 0.7                  # focus moves; everything else recedes
key Tab 0.7
key Tab 0.7

tmux send-keys -t "$SESSION" '4'; hold 0.8      # the task list
key a 0.7                                        # add a task
type_text "Renew the domain"
key Enter 1.8                                    # and it is in the list

# `End` selects the last widget, `network`. Toggling any panel rebuilds the
# whole dashboard, and a re-added widget goes to the emptiest row — so removing
# the *last* one is the case where it comes back exactly where it was. Any other
# choice reshuffles the row, which reads as a glitch rather than as a feature.
tmux send-keys -t "$SESSION" 'w'; hold 1.0      # the panel picker
key End 0.5
key space 1.3                                    # a panel goes, the grid reflows
key space 1.3                                    # and comes back
key Escape 0.8

# The help overlay goes last on purpose. Rebuilding the panels restarts the
# weather and price fetches, so for a few seconds those two read "loading" —
# real behaviour, but it looks like a fault in a silent GIF. Holding the overlay
# over the top gives them time to come back before the last frames.
tmux send-keys -t "$SESSION" '?'; hold 1.8      # every binding, scrollable
key Down 0.4
key Down 0.4
key Down 0.8
tmux send-keys -t "$SESSION" 'x'; hold 1.0      # any other key closes it

# The timer is started last. Toggling a panel rebuilds every panel, which resets
# a running pomodoro — so starting it earlier would show it silently give up.
tmux send-keys -t "$SESSION" '6'; hold 0.6
key space 2.8                                    # started, and counting down

# -------------------------------------------------------------------------

tmux send-keys -t "$SESSION" 'q'
sleep 1

# Shift the timeline so the first frame lands at exactly t=0. Without this the
# first capture is a few milliseconds in, and `agg` faithfully renders those
# milliseconds of empty terminal as a real frame. It is imperceptible in motion,
# but it is the frame a still preview shows, and it flashes black once per loop.
python3 - "$CAST" <<'NORMALISE'
import json, sys
path = sys.argv[1]
with open(path) as f:
    header, *events = [line for line in f.read().splitlines() if line.strip()]
parsed = [json.loads(e) for e in events]
if parsed:
    offset = parsed[0][0]
    for e in parsed:
        e[0] = round(max(0.0, e[0] - offset), 3)
with open(path, "w") as f:
    f.write(header + "\n")
    for e in parsed:
        f.write(json.dumps(e) + "\n")
NORMALISE

agg --quiet --theme asciinema --font-size 13 --line-height 1.35 \
    --idle-time-limit 2 --last-frame-duration 2 \
    "$CAST" "$OUT"

echo "wrote $OUT ($(du -h "$OUT" | cut -f1))"
