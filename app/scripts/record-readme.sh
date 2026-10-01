#!/usr/bin/env bash
# Regenerate the README desktop-app media (docs/media/desktop-*.{gif,png}).
#
# Starts the browser mock harness (VITE_MOCK=1), drives the two demo clips and
# the two stills with Playwright (record-readme.py), encodes each webm -> gif
# with ffmpeg, and copies the results into docs/media. Deterministic: same
# fixtures every run.
#
# Produces:
#   docs/media/desktop-flow.gif      the core loop: tree -> new worktree -> live
#   docs/media/desktop-dock.gif      the workspace: Files / Docs / Plan
#   docs/media/desktop-overview.png  Home, 2x
#   docs/media/desktop-session.png   a place open, 2x
#
# Deps: node_modules installed (pnpm i), python `playwright` + chromium
# (`pip install playwright && playwright install chromium`), and ffmpeg.
#
# Usage:  app/scripts/record-readme.sh        # PORT=1425 by default
#   PORT=1437  reuse a harness already serving on that port (nothing is killed)
#   FFMPEG=/path/to/ffmpeg   e.g. an imageio-ffmpeg binary when there is no
#                            system ffmpeg on PATH
#   PYTHON=/path/to/python   the interpreter that has playwright installed
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"      # app/
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"            # repo root
MEDIA_DIR="$ROOT_DIR/docs/media"
PORT="${PORT:-1425}"
FFMPEG="${FFMPEG:-ffmpeg}"
PYTHON="${PYTHON:-python3}"
FPS="${FPS:-13}"
GIFW="${GIFW:-920}"
COLORS="${COLORS:-112}"
# The UI is flat, dark and mostly text; dithering buys nothing on it and costs
# ~20% of the file in noise the LZW pass cannot fold away. DITHER=bayer:bayer_scale=5
# if a future theme ever needs it.
DITHER="${DITHER:-none}"
# Intermediates (webm, palettes, vite log). Temporary and removed unless
# RECORD_OUT points somewhere you want to keep — never the repo root.
if [ -n "${RECORD_OUT:-}" ]; then OUT="$RECORD_OUT"; KEEP_OUT=1; mkdir -p "$OUT"
else OUT="$(mktemp -d)"; KEEP_OUT=""; fi
STARTED_VITE=""
trap 'if [ -n "$STARTED_VITE" ]; then kill "$STARTED_VITE" 2>/dev/null || true; fi; if [ -z "$KEEP_OUT" ]; then rm -rf "$OUT"; fi' EXIT

command -v "$FFMPEG" >/dev/null 2>&1 || {
  echo "✗ no ffmpeg at '$FFMPEG' — install one or set FFMPEG=/path/to/ffmpeg" >&2
  exit 1
}
command -v "$PYTHON" >/dev/null 2>&1 || {
  echo "✗ no python at '$PYTHON' — set PYTHON=/path/to/python" >&2
  exit 1
}

if curl -sf "http://localhost:$PORT/" >/dev/null 2>&1; then
  echo "→ reusing the harness already serving on :$PORT"
else
  echo "→ starting mock harness on :$PORT"
  ( cd "$APP_DIR" && VITE_MOCK=1 ./node_modules/.bin/vite --port "$PORT" --strictPort \
      >"$OUT/vite.log" 2>&1 & echo $! >"$OUT/vite.pid" )
  STARTED_VITE="$(cat "$OUT/vite.pid")"
  for _ in $(seq 1 80); do
    curl -sf "http://localhost:$PORT/" >/dev/null 2>&1 && break
    sleep 0.25
  done
  curl -sf "http://localhost:$PORT/" >/dev/null 2>&1 || {
    echo "✗ harness never came up on :$PORT — see below" >&2
    tail -20 "$OUT/vite.log" >&2 || true
    exit 1
  }
fi

echo "→ recording clips + stills"
"$PYTHON" "$APP_DIR/scripts/record-readme.py" --port "$PORT" --out "$OUT"

# Head-of-clip trim: the video starts at context creation, so the first frames
# are a blank page. record-readme.py measures how long that lasted.
trim_for() {
  "$PYTHON" - "$OUT/trims.json" "$1" <<'PY'
import json, sys
try:
    t = json.load(open(sys.argv[1])).get(sys.argv[2], 0.0)
except Exception:
    t = 0.0
print(f"{max(0.0, t - 0.15):.2f}")
PY
}

encode() {           # encode <clip-name> <out.gif>
  local clip="$1" dest="$2" src="$OUT/$1.webm" ss
  [ -f "$src" ] || { echo "✗ missing $src" >&2; exit 1; }
  ss="$(trim_for "$clip")"
  "$FFMPEG" -v error -y -ss "$ss" -i "$src" \
    -vf "fps=$FPS,scale=$GIFW:-1:flags=lanczos,palettegen=max_colors=$COLORS:stats_mode=diff" \
    "$OUT/$clip-palette.png"
  "$FFMPEG" -v error -y -ss "$ss" -i "$src" -i "$OUT/$clip-palette.png" \
    -lavfi "fps=$FPS,scale=$GIFW:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=$DITHER:diff_mode=rectangle:new=0" \
    -loop 0 "$dest"
}

echo "→ encoding gifs (${GIFW}px, ${FPS}fps, ${COLORS} colours, dither=$DITHER)"
encode flow "$OUT/desktop-flow.gif"
encode dock "$OUT/desktop-dock.gif"

mkdir -p "$MEDIA_DIR"
cp "$OUT/desktop-flow.gif" "$MEDIA_DIR/desktop-flow.gif"
cp "$OUT/desktop-dock.gif" "$MEDIA_DIR/desktop-dock.gif"
cp "$OUT/shot_home.png"    "$MEDIA_DIR/desktop-overview.png"
cp "$OUT/shot_session.png" "$MEDIA_DIR/desktop-session.png"

echo "✓ wrote into $MEDIA_DIR:"
for f in desktop-flow.gif desktop-dock.gif desktop-overview.png desktop-session.png; do
  printf '   %-24s %s bytes\n' "$f" "$(wc -c <"$MEDIA_DIR/$f" | tr -d ' ')"
done
