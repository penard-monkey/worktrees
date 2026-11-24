#!/bin/bash
# Runs INSIDE a test VM (ssh … bash -s -- <tag>), never on the host.
# Installs a PUBLISHED release the way a user does: quit the app, then
# `curl …/<tag>/install.sh | WORKTREES_INSTALL_APP=1 bash`, which fetches that
# tag's CLI and app from GitHub, checksum-verified. Then starts the app again.
# macOS (osascript quit, /Applications/worktrees.app) and Linux (the AppImage
# in ~/Applications, started inside the GNOME session by ~/launch-app.sh).
# Beta switches are NOT set here: the run turns them on through the UI.
# Prints one JSON line the driver reads.
set -u
tag="${1:?tag}"
export PATH="$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
os=$(uname -s)

if [ "$os" = Darwin ]; then
  D="$HOME/Library/Application Support/net.casadelvalle.worktrees"
  osascript -e 'tell application "worktrees" to quit' >/dev/null 2>&1
  for _ in $(seq 1 10); do pgrep -f 'worktrees.app/Contents/MacOS/app' >/dev/null || break; sleep 1; done
  for _ in $(seq 1 10); do pgrep -f 'worktrees serve' >/dev/null || break; sleep 1; done
  pkill -f 'worktrees.app/Contents/MacOS/worktrees serve' 2>/dev/null; sleep 1
else
  D="$HOME/.config/net.casadelvalle.worktrees"
  pkill -x worktrees-app 2>/dev/null
  for _ in $(seq 1 10); do pgrep -x worktrees-app >/dev/null || break; sleep 1; done
  pkill -f 'worktrees serve' 2>/dev/null; sleep 1
fi

log="$HOME/vmlab-install-$tag.log"
curl -fsSL "https://raw.githubusercontent.com/penard-monkey/worktrees/$tag/install.sh" \
  | WORKTREES_INSTALL_APP=1 bash >"$log" 2>&1
rc=$?

if [ "$os" = Darwin ]; then
  open -a /Applications/worktrees.app
  app_v=$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' /Applications/worktrees.app/Contents/Info.plist 2>/dev/null)
else
  "$HOME/launch-app.sh" >/dev/null 2>&1
  # The AppImage carries the CLI beside the GUI: its version is the bundle's.
  app_v=$(grep -o 'installed: .*AppImage.*' "$log" | head -1)
fi
for _ in $(seq 1 40); do
  [ -s "$D/serve.json" ] && worktrees serve call shares_all >/dev/null 2>&1 && break
  sleep 1
done
v=$(worktrees --version 2>/dev/null | awk '{print $2}')
up=false; worktrees serve call shares_all >/dev/null 2>&1 && up=true
sv=$(grep -o 'startup v[0-9.]*' "$HOME/.cache/worktrees/serve.log" 2>/dev/null | tail -1)
printf '{"install_rc":%s,"cli":"%s","app_bundle":"%s","serve_up":%s,"serve_log":"%s"}\n' \
  "$rc" "$v" "$app_v" "$up" "$sv"
