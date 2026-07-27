#!/usr/bin/env bash
#
# Record docs/demo.gif.
#
# Drives a real mirador under tmux, captures the screen at a fixed rate, and
# writes an asciicast that `agg` turns into a GIF. The tasks and the note are
# the ones mirador seeds on a first run, and the weather, prices and graphs are
# whatever was true while it recorded. The one staged thing is the calendar —
# see the note beside it below, and the README caption, which says so too.
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

# Wide enough for ten panels to be themselves rather than to survive. mirador
# degrades gracefully — the clock drops its block numerals in a short row, the
# forecast sheds columns, the watchlist drops its sparkline — and a recording
# made in a cramped terminal shows every one of those fallbacks instead of the
# thing being demonstrated. High-resolution screens are ordinary now; this is
# what the dashboard is for.
COLS=200
ROWS=50
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

# The one thing here that *is* staged, and it is called out in the README
# caption as well: a sample calendar. mirador seeds tasks and notes itself, but
# it deliberately never invents a calendar — an empty one would be a lie about
# your day — so a recording with nothing configured would show the agenda panel
# explaining how to point it at a file. True, and a poor demonstration of what
# the panel is for.
#
# Dates are relative to the day of the recording, so the GIF does not age into
# showing a week in the past.
CONFIG="$WORK/config.toml"
CAL="$WORK/sample.ics"
TODAY=$(date +%Y%m%d)
IN_THREE=$(date -v+3d +%Y%m%d 2>/dev/null || date -d '+3 days' +%Y%m%d)
{
  echo "BEGIN:VCALENDAR"
  echo "VERSION:2.0"
  printf 'BEGIN:VEVENT\nDTSTART:%sT131500\nDTEND:%sT133000\nSUMMARY:Standup\nLOCATION:Zoom\nRRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR\nEND:VEVENT\n' "$TODAY" "$TODAY"
  printf 'BEGIN:VEVENT\nDTSTART:%sT160000\nDURATION:PT1H\nSUMMARY:Quarterly planning\nLOCATION:Room 12\nEND:VEVENT\n' "$TODAY"
  printf 'BEGIN:VEVENT\nDTSTART;VALUE=DATE:%s\nSUMMARY:Public holiday\nEND:VEVENT\n' "$IN_THREE"
  echo "END:VCALENDAR"
} > "$CAL"

"$BIN" --print-config > "$CONFIG"
python3 - "$CONFIG" "$CAL" <<'CONFIGURE'
import pathlib, sys
config, calendar = pathlib.Path(sys.argv[1]), sys.argv[2]
text = config.read_text().replace('# file        = "~/calendar.ics"', f'file        = "{calendar}"')
config.write_text(text)
CONFIGURE

tmux new-session -d -s "$SESSION" -x "$COLS" -y "$ROWS" "$BIN --config $CONFIG"

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
# Long enough for the graphs to have a past. `[cpu]` and `[network]` sample
# every two seconds and pack two samples per cell, so a chart roughly 25 cells
# wide needs about a hundred seconds before it is a shape rather than a stub —
# and a dashboard whose graphs have just started looks like a dashboard that
# has just crashed.
#
# The load is real and comes in bursts, so the trace has peaks and troughs
# instead of a flat ceiling.
SETTLE=105
(
  end=$((SECONDS + SETTLE))
  while [ $SECONDS -lt $end ]; do
    for _ in 1 2 3 4 5 6; do
      (spin=$((SECONDS + 4)); while [ $SECONDS -lt $spin ]; do :; done) &
    done
    wait
    # Something for the network graph to draw, and a gap so the trace dips.
    curl -s -o /dev/null https://github.com 2>/dev/null || true
    sleep 6
  done
) &
LOAD_PID=$!

# The weather and price fetches are on their own threads, and an empty forecast
# is not what the dashboard looks like either.
sleep "$SETTLE"
wait "$LOAD_PID" 2>/dev/null || true

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

# `End` selects the last widget, `network`. A re-added widget goes to the
# emptiest row, so removing the *last* one is the case where it comes back
# exactly where it was; any other choice reshuffles the row, which reads as a
# glitch rather than as a feature. The panels that stay put are carried across
# rather than rebuilt, so nothing else on screen so much as blinks.
tmux send-keys -t "$SESSION" 'w'; hold 1.0      # the panel picker
key End 0.5
key space 1.3                                    # a panel goes, the grid reflows
key space 1.3                                    # and comes back
key Escape 0.8

# The help overlay before the timer, so the GIF ends on a running clock rather
# than on a dialog. (This used to be a workaround: toggling a panel restarted
# the weather and price fetches, and the overlay hid the few seconds of
# "loading". Panels are carried across now, so it is just the better order.)
tmux send-keys -t "$SESSION" '?'; hold 1.8      # every binding, scrollable
key Down 0.4
key Down 0.4
key Down 0.8
tmux send-keys -t "$SESSION" 'x'; hold 1.0      # any other key closes it

# Started last so the recording ends on something moving.
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
