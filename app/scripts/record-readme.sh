#!/usr/bin/env bash
# Regenerate the README desktop-app media (docs/media/desktop-*.{gif,png}).
#
# Starts the browser mock harness (VITE_MOCK=1), drives the core flow with
# Playwright (record-readme.py), encodes the webm -> gif with ffmpeg, and copies
# the results into docs/media. Deterministic: same fixtures every run.
#
# Deps: node_modules installed (pnpm i), python `playwright` + chromium
# (`pip install playwright && playwright install chromium`), and ffmpeg.
#
# Usage:  app/scripts/record-readme.sh        # PORT=1425 by default
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"      # app/
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"            # repo root
MEDIA_DIR="$ROOT_DIR/docs/media"
PORT="${PORT:-1425}"
OUT="$(mktemp -d)"
trap 'kill "$(cat "$OUT/vite.pid" 2>/dev/null)" 2>/dev/null || true; rm -rf "$OUT"' EXIT

echo "→ starting mock harness on :$PORT"
( cd "$APP_DIR" && VITE_MOCK=1 ./node_modules/.bin/vite --port "$PORT" --strictPort \
    >"$OUT/vite.log" 2>&1 & echo $! >"$OUT/vite.pid" )
for _ in $(seq 1 60); do
  curl -sf "http://localhost:$PORT/" >/dev/null 2>&1 && break
  sleep 0.25
done

echo "→ recording flow + stills"
python "$APP_DIR/scripts/record-readme.py" --port "$PORT" --out "$OUT"

echo "→ encoding gif"
WEBM="$(find "$OUT" -name '*.webm' | head -1)"
ffmpeg -v error -y -i "$WEBM" \
  -vf "fps=15,scale=920:-1:flags=lanczos,palettegen=stats_mode=diff" "$OUT/palette.png"
ffmpeg -v error -y -i "$WEBM" -i "$OUT/palette.png" \
  -lavfi "fps=15,scale=920:-1:flags=lanczos[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=3:diff_mode=rectangle" \
  "$OUT/desktop-flow.gif"

mkdir -p "$MEDIA_DIR"
cp "$OUT/desktop-flow.gif"  "$MEDIA_DIR/desktop-flow.gif"
cp "$OUT/shot_home.png"     "$MEDIA_DIR/desktop-overview.png"
cp "$OUT/shot_session.png"  "$MEDIA_DIR/desktop-session.png"
echo "✓ wrote $MEDIA_DIR/desktop-{flow.gif,overview.png,session.png}"
