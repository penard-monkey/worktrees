#!/usr/bin/env bash
# An isolated worktrees sandbox for running docs/ai-profiles-manual-checks.md
# WITHOUT disturbing a worktrees app you already have open.
#
# What it isolates, and why each one matters:
#   WORKTREES_PREFIX  tmux session names are <prefix>-<slug>, computed from the
#                     repo — so a second build would otherwise ATTACH TO THE SAME
#                     SESSION your open app is using, and closing it there would
#                     kill it here. A distinct prefix is what keeps them apart,
#                     and is also how you tell which is which in `tmux ls`.
#   XDG_CONFIG_HOME   profiles.json + the skill manifest
#   XDG_DATA_HOME     materialized profile dirs + skill store content
#   a scratch repo    nothing touches your real projects
#
# Your real ~/.claude is deliberately NOT isolated: the checklist needs to verify
# that your global CLAUDE.md still loads under a profile, which is the one thing
# a profile cannot suppress.
#
# Usage:  eval "$(app/scripts/profiles-sandbox.sh)"     # sets up + prints exports
#         worktrees ls                                   # now sandboxed
#         app/scripts/profiles-sandbox.sh --clean        # tear it all down
set -euo pipefail

ROOT="${WT_SANDBOX:-$HOME/.cache/worktrees/profiles-sandbox}"
PREFIX="wtsbx"
BIN="$(cd "$(dirname "$0")/../.." && pwd)/target/release/worktrees"

if [ "${1:-}" = "--clean" ]; then
  for s in $(tmux ls -F '#{session_name}' 2>/dev/null | grep "^$PREFIX-" || true); do
    tmux kill-session -t "$s" 2>/dev/null || true
  done
  # Each sandbox profile has its own keychain item, keyed to its config dir.
  echo "note: profile sign-ins remain in your login keychain as" >&2
  echo "      'Claude Code-credentials-<hash>' — remove via Keychain Access." >&2
  rm -rf "$ROOT"
  echo "sandbox removed: $ROOT" >&2
  exit 0
fi

mkdir -p "$ROOT/config" "$ROOT/data" "$ROOT/repo"
if [ ! -d "$ROOT/repo/.git" ]; then
  git -C "$ROOT/repo" init -q
  printf '# sandbox\n' > "$ROOT/repo/README.md"
  git -C "$ROOT/repo" add -A
  git -C "$ROOT/repo" -c user.email=s@s -c user.name=sandbox commit -qm "init"
fi

cat <<EOF
export XDG_CONFIG_HOME="$ROOT/config"
export XDG_DATA_HOME="$ROOT/data"
export WORKTREES_PREFIX="$PREFIX"
export PATH="$(dirname "$BIN"):\$PATH"
cd "$ROOT/repo"
EOF

{
  echo
  echo "sandbox ready:"
  echo "  repo     $ROOT/repo"
  echo "  profiles $ROOT/config/worktrees/profiles.json"
  echo "  binary   $BIN"
  echo "  sessions tmux names start with '$PREFIX-' (yours keep their own prefix)"
  echo
  echo "tear down with: $0 --clean"
} >&2
