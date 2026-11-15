#!/bin/bash
# Runs INSIDE a test VM (ssh … bash -s -- <beta-keys…>), never on the host.
# Installs the bundle the driver unpacked at ~/vmlab-build/worktrees.app,
# turns on the named beta switches in the app's ui-state.json while the app is
# quit (the frontend writes that file whole, so it is edited only then), and
# starts the app. Prints one JSON line the driver reads.
set -u
export PATH="$HOME/.local/bin:/opt/homebrew/bin:$PATH"
D="$HOME/Library/Application Support/net.casadelvalle.worktrees"

osascript -e 'tell application "worktrees" to quit' >/dev/null 2>&1
for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -f 'worktrees.app/Contents/MacOS/app' >/dev/null || break; sleep 1; done
# An app-owned serve stops itself once its app is gone; one that lingers past
# the wait is the old build's and must not answer the new app.
for _ in 1 2 3 4 5 6 7 8 9 10; do pgrep -f 'worktrees serve' >/dev/null || break; sleep 1; done
pkill -f 'worktrees.app/Contents/MacOS/worktrees serve' 2>/dev/null; sleep 1

rm -rf /Applications/worktrees.app
cp -R "$HOME/vmlab-build/worktrees.app" /Applications/worktrees.app
mkdir -p "$HOME/.local/bin"
cp /Applications/worktrees.app/Contents/MacOS/worktrees "$HOME/.local/bin/worktrees"

mkdir -p "$D"
python3 - "$D/ui-state.json" "$@" <<'PY'
import json, os, sys
path, keys = sys.argv[1], sys.argv[2:]
try:
    with open(path) as f:
        state = json.load(f)
except (OSError, ValueError):
    state = {}
for k in keys:
    state[k] = True
tmp = path + ".vmlab"
with open(tmp, "w") as f:
    json.dump(state, f)
os.replace(tmp, path)
PY

open -a /Applications/worktrees.app
for _ in $(seq 1 30); do
  [ -s "$D/serve.json" ] && worktrees serve call shares_all >/dev/null 2>&1 && break
  sleep 1
done
v=$(worktrees --version 2>/dev/null | awk '{print $2}')
up=false; worktrees serve call shares_all >/dev/null 2>&1 && up=true
printf '{"version":"%s","serve_up":%s}\n' "$v" "$up"
