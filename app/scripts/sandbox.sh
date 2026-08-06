#!/usr/bin/env bash
# Spin up an isolated worktrees for testing THIS BRANCH, without disturbing a
# worktrees you already have open.
#
#   eval "$(app/scripts/sandbox.sh)"        # CLI sandbox: env + a scratch repo
#   app/scripts/sandbox.sh --app            # …and launch the app against it
#   app/scripts/sandbox.sh --repo ~/src/x   # use a real repo instead of scratch
#   app/scripts/sandbox.sh --clean          # tear this branch's sandbox down
#
# WHAT COLLIDES IF YOU DO NOT DO THIS
#
#   tmux sessions   Names are <prefix>-<slug>, derived from the repo — so a
#                   second build computes the SAME name and attaches to the
#                   session your open app is using. Closing it in one kills it in
#                   the other. This is the destructive one.
#   app config      Both builds share the bundle id, so both read and write the
#                   same ui-state.json and projects.json. Settings are one
#                   debounced blob: last writer wins.
#   two instances   Nothing stops them, and nothing on screen tells them apart.
#
# HOW THIS AVOIDS IT
#
#   A per-branch tmux prefix, so sessions can never collide — with your app, or
#   with a sandbox from another branch. That prefix is also how you tell them
#   apart in `tmux ls`.
#   Isolated XDG dirs, so profiles and the skill store are the branch's own.
#   For --app, a distinct bundle identifier and product name, so the sandbox app
#   gets its own config dir and its own name in the window title.
#
# Your real ~/.claude is deliberately NOT isolated: testing an AI profile means
# checking that your global CLAUDE.md still loads under it, which is the one
# thing a profile cannot suppress.
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"
BRANCH="$(git -C "$ROOT_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)"
# tmux prefixes are [a-z0-9_-]; keep it short so session names stay readable.
SLUG="$(printf '%s' "$BRANCH" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | cut -c1-12 | sed 's/-*$//')"
PREFIX="sbx-${SLUG:-branch}"
SBX="$HOME/.cache/worktrees/sandbox/$SLUG"
BIN="$ROOT_DIR/target/release/worktrees"

MODE=cli REPO_OVERRIDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --app)   MODE=app; shift ;;
    --repo)  REPO_OVERRIDE="$2"; shift 2 ;;
    --clean) MODE=clean; shift ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

if [ "$MODE" = clean ]; then
  for s in $(tmux ls -F '#{session_name}' 2>/dev/null | grep "^$PREFIX-" || true); do
    tmux kill-session -t "$s" 2>/dev/null || true
  done
  rm -rf "$SBX"
  echo "sandbox for '$BRANCH' removed ($SBX)" >&2
  echo "note: any profile you signed into left a keychain item" >&2
  echo "      ('Claude Code-credentials-<hash>'); remove via Keychain Access." >&2
  exit 0
fi

# Always build the branch under test — the whole point is testing THIS code.
echo "→ building $BRANCH" >&2
( cd "$ROOT_DIR" && cargo build --release -p worktrees-cli >&2 )

mkdir -p "$SBX/config" "$SBX/data"
if [ -n "$REPO_OVERRIDE" ]; then
  REPO="$(cd "$REPO_OVERRIDE" && pwd)"
else
  REPO="$SBX/repo"
  if [ ! -d "$REPO/.git" ]; then
    mkdir -p "$REPO"
    git -C "$REPO" init -q
    printf '# sandbox for %s\n' "$BRANCH" > "$REPO/README.md"
    git -C "$REPO" add -A
    git -C "$REPO" -c user.email=s@s -c user.name=sandbox commit -qm "init"
  fi
fi

export XDG_CONFIG_HOME="$SBX/config" XDG_DATA_HOME="$SBX/data" WORKTREES_PREFIX="$PREFIX"

if [ "$MODE" = app ]; then
  echo "→ launching the app against the sandbox" >&2
  echo "   window title / dock name: 'worktrees (sbx $SLUG)'" >&2
  echo "   tmux sessions:            $PREFIX-<slug>" >&2
  echo "   add the repo inside the app: $REPO" >&2
  echo >&2
  echo "   VERIFY the config dir really moved: Settings → Data & Logs should" >&2
  echo "   show a path under 'net.casadelvalle.worktrees.sbx', NOT the plain" >&2
  echo "   identifier. If it shows the plain one, the override did not take and" >&2
  echo "   this app shares ui-state.json with your installed one — quit your" >&2
  echo "   installed app before continuing." >&2
  echo >&2
  cd "$APP_DIR"
  exec ./node_modules/.bin/tauri dev --config \
    '{"identifier":"net.casadelvalle.worktrees.sbx","productName":"worktrees (sbx '"$SLUG"')"}'
fi

# CLI mode: emit exports for `eval`, and put the branch's binary first on PATH.
cat <<EOF
export XDG_CONFIG_HOME="$XDG_CONFIG_HOME"
export XDG_DATA_HOME="$XDG_DATA_HOME"
export WORKTREES_PREFIX="$PREFIX"
export PATH="$(dirname "$BIN"):\$PATH"
cd "$REPO"
EOF

{
  echo
  echo "sandbox ready — branch '$BRANCH'"
  echo "  repo      $REPO"
  echo "  profiles  $XDG_CONFIG_HOME/worktrees/profiles.json"
  echo "  binary    $BIN  ($("$BIN" --version 2>/dev/null || echo '?'))"
  echo "  sessions  $PREFIX-<slug>     ← yours keep their own prefix"
  echo
  echo "  tmux ls | grep $PREFIX     # this sandbox"
  echo "  tmux ls | grep -v sbx-     # yours, untouched"
  echo
  echo "tear down:  $0 --clean"
} >&2
