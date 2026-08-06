#!/usr/bin/env bash
# Regenerate the AI-profiles feature-page screenshots from the mock harness.
# Deterministic: same fixtures every run.
#
# Deps: pnpm i in app/, python `playwright` + chromium, and node >= 22.13.
# Usage: app/scripts/shoot-profiles.sh   # PORT=1426 by default
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"
OUT_DIR="$ROOT_DIR/docs/media/profiles"
PORT="${PORT:-1426}"
TMP="$(mktemp -d)"
trap 'kill "$(cat "$TMP/vite.pid" 2>/dev/null)" 2>/dev/null || true; rm -rf "$TMP"' EXIT

mkdir -p "$OUT_DIR"
echo "→ mock harness on :$PORT"
( cd "$APP_DIR" && VITE_MOCK=1 ./node_modules/.bin/vite --port "$PORT" --strictPort \
    >"$TMP/vite.log" 2>&1 & echo $! >"$TMP/vite.pid" )
for _ in $(seq 1 80); do
  curl -sf "http://localhost:$PORT/" >/dev/null 2>&1 && break
  sleep 0.25
done

echo "→ shooting"
python3 "$APP_DIR/scripts/shoot-profiles.py" --port "$PORT" --out "$OUT_DIR"
echo "→ wrote $OUT_DIR"
ls -1 "$OUT_DIR"
