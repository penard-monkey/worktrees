#!/usr/bin/env bash
# Regenerate the AI-profiles walkthrough clip (docs/media/profiles/walkthrough.mp4).
#
# Starts the mock harness, records the flow with Playwright, encodes webm -> mp4
# with ffmpeg. Deterministic: same fixtures every run.
#
# Deps: pnpm i in app/, python `playwright` + chromium, ffmpeg, node >= 22.13.
# Usage: app/scripts/record-profiles.sh   # PORT=1441 by default
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"
MEDIA_DIR="$ROOT_DIR/docs/media/profiles"
PORT="${PORT:-1441}"
OUT="$(mktemp -d)"
trap 'kill "$(cat "$OUT/vite.pid" 2>/dev/null)" 2>/dev/null || true; rm -rf "$OUT"' EXIT

echo "→ mock harness on :$PORT"
( cd "$APP_DIR" && VITE_MOCK=1 ./node_modules/.bin/vite --port "$PORT" --strictPort \
    >"$OUT/vite.log" 2>&1 & echo $! >"$OUT/vite.pid" )
for _ in $(seq 1 80); do
  curl -sf "http://localhost:$PORT/" >/dev/null 2>&1 && break
  sleep 0.25
done

echo "→ recording"
python3 "$APP_DIR/scripts/record-profiles.py" --port "$PORT" --out "$OUT"
WEBM="$(find "$OUT" -name '*.webm' | head -n1)"
[ -n "$WEBM" ] || { echo "no video produced" >&2; exit 1; }

echo "→ encoding"
# MP4, not GIF. This lands on a web page as a muted looping <video>: the same
# clip is ~10x smaller than the gif (3.2MB -> a few hundred KB) and sharper,
# because a gif has to quantise this UI's flat dark fills to a 128-colour
# palette. A gif would only be worth it for a README, which cannot autoplay one.
ffmpeg -v error -y -i "$WEBM" \
  -vf "fps=24,scale=1000:-2:flags=lanczos" \
  -c:v libx264 -profile:v high -pix_fmt yuv420p -crf 30 -preset slow \
  -movflags +faststart -an "$OUT/walkthrough.mp4"

mkdir -p "$MEDIA_DIR"
cp "$OUT/walkthrough.mp4" "$MEDIA_DIR/walkthrough.mp4"
echo "→ wrote $MEDIA_DIR/walkthrough.mp4 ($(du -h "$MEDIA_DIR/walkthrough.mp4" | cut -f1))"
