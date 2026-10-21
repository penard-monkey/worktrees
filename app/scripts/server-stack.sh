#!/usr/bin/env bash
# A dev stack for the Server Edition: THIS BRANCH's `worktrees serve`, on its
# own config dir, its own tmux prefix and its own port, so it cannot collide
# with the app you have open or with another branch's stack.
#
#   app/scripts/server-stack.sh                 # build, start, print serve.json's path
#   app/scripts/server-stack.sh --port 7311     # a fixed port (default: ephemeral)
#   app/scripts/server-stack.sh --allow-origin http://localhost:5173
#                                               # a browser-hosted frontend (repeatable;
#                                               #   http://localhost:1420 is always allowed)
#   app/scripts/server-stack.sh --stop          # stop this branch's stack
#   app/scripts/server-stack.sh --clean         # …and remove its config dir
#
# WHAT THIS ISOLATES (the same list as sandbox.sh, one process further out)
#
#   config dir     `serve.json`, `ui-state.json`, the rings, `shell-cwds.json`
#                  all live in the config dir, so the stack gets its own under
#                  ~/.cache/worktrees/sandbox/<slug>/serve-config. The TOKEN is
#                  in that serve.json, mode 0600; this script never prints it.
#   tmux prefix    WORKTREES_PREFIX=sbx-<slug>, so places the stack opens are
#                  `sbx-<slug>-<place>` in `tmux ls` and never attach to yours.
#   XDG dirs       The registry, profiles and the skill store are the stack's
#                  own — it starts with NO projects. Add one through the client.
#   the log        ~/.cache/worktrees/serve.log is NOT isolated: it is the CLI's
#                  one log, and every line carries the pid, so `grep <pid>` is
#                  how you read one stack's lines. (HOME is deliberately not
#                  faked — a faked HOME loses your real PATH and your ~/.claude,
#                  which is exactly what the nav-dot poll needs to read.)
#
# The frontend lane points its transport at the printed serve.json; a browser
# frontend on another port passes `--allow-origin` for it, which is a FLAG and
# never a config key, so a dev entry cannot ship.
set -euo pipefail

APP_DIR="$(cd "$(dirname "$0")/.." && pwd)"
ROOT_DIR="$(cd "$APP_DIR/.." && pwd)"
BRANCH="$(git -C "$ROOT_DIR" rev-parse --abbrev-ref HEAD 2>/dev/null || echo detached)"
SLUG="$(printf '%s' "$BRANCH" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | cut -c1-12 | sed 's/-*$//')"
PREFIX="sbx-${SLUG:-branch}"
SBX="$HOME/.cache/worktrees/sandbox/$SLUG"
CFG="$SBX/serve-config"
PIDFILE="$SBX/serve.pid"
BIN="$ROOT_DIR/target/release/worktrees"

MODE=start PORT=0 ORIGINS=("http://localhost:1420")
while [ $# -gt 0 ]; do
  case "$1" in
    --port)         PORT="$2"; shift 2 ;;
    --allow-origin) ORIGINS+=("$2"); shift 2 ;;
    --stop)         MODE=stop; shift ;;
    --clean)        MODE=clean; shift ;;
    -h|--help)      sed -n '2,32p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

stop_stack() {
  if [ -f "$PIDFILE" ]; then
    pid="$(cat "$PIDFILE")"
    if kill -0 "$pid" 2>/dev/null; then
      kill -TERM "$pid"
      # serve flushes the rings and removes serve.json on the way out.
      for _ in $(seq 1 100); do kill -0 "$pid" 2>/dev/null || break; sleep 0.1; done
      kill -0 "$pid" 2>/dev/null && { echo "serve $pid did not stop; sending KILL" >&2; kill -KILL "$pid" || true; }
      echo "stopped serve (pid $pid)" >&2
    fi
    rm -f "$PIDFILE"
  fi
}

case "$MODE" in
  stop)
    stop_stack
    exit 0 ;;
  clean)
    stop_stack
    for s in $(tmux ls -F '#{session_name}' 2>/dev/null | grep "^$PREFIX-" || true); do
      tmux kill-session -t "$s" 2>/dev/null || true
    done
    rm -rf "$CFG" "$SBX/config" "$SBX/data"
    echo "server stack for '$BRANCH' removed ($CFG)" >&2
    exit 0 ;;
esac

# A stack already up for this branch is the one to talk to, not a second one:
# `serve` would refuse it anyway (one engine per config dir) — say so first.
if [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; then
  echo "a stack for '$BRANCH' is already running (pid $(cat "$PIDFILE")); --stop it first" >&2
  echo "$CFG/serve.json"
  exit 0
fi

echo "→ building $BRANCH" >&2
( cd "$ROOT_DIR" && cargo build --release -p worktrees-cli >&2 )

mkdir -p "$CFG" "$SBX/config" "$SBX/data"
export XDG_CONFIG_HOME="$SBX/config" XDG_DATA_HOME="$SBX/data" WORKTREES_PREFIX="$PREFIX"

args=(serve --config-dir "$CFG" --port "$PORT")
for o in "${ORIGINS[@]}"; do args+=(--allow-origin "$o"); done

# Detached into its own session: a stack must outlive the terminal that
# started it, and a tool call's cleanup must not take it down (AGENTS.md: a
# harness started with `nohup … &` inside a tool call was SIGTERM'd by the next
# call's group cleanup). Not `setsid(1)`: macOS does not ship it, and where it
# exists it forks, so `$!` would be the wrapper's pid rather than serve's and
# the pid in serve.json would never match it. python3's `os.setsid` + `execvp`
# keeps ONE pid from here to the server (python3 is already what `make
# test-mcp` requires).
rm -f "$CFG/serve.json"
nohup python3 -c 'import os, sys; os.setsid(); os.execvp(sys.argv[1], sys.argv[1:])' \
  "$BIN" "${args[@]}" > "$SBX/serve.out" 2>&1 < /dev/null &
echo $! > "$PIDFILE"

# Wait for serve.json to name THIS pid: the login-shell PATH probe in the
# preamble can take up to five seconds.
pid="$(cat "$PIDFILE")"
for _ in $(seq 1 300); do
  if [ -f "$CFG/serve.json" ] && grep -q "\"pid\": $pid" "$CFG/serve.json" 2>/dev/null; then
    break
  fi
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "serve exited:" >&2; cat "$SBX/serve.out" >&2; rm -f "$PIDFILE"; exit 1
  fi
  sleep 0.1
done
[ -f "$CFG/serve.json" ] || { echo "serve.json never appeared; see $SBX/serve.out" >&2; exit 1; }

port="$(sed -n 's/.*"port": *\([0-9]*\).*/\1/p' "$CFG/serve.json" | head -1)"
{
  echo
  echo "server stack ready — branch '$BRANCH'"
  echo "  serve      127.0.0.1:$port  (pid $pid, $("$BIN" --version 2>/dev/null || echo '?'))"
  echo "  serve.json $CFG/serve.json   ← the token is in here, mode 0600"
  echo "  origins    ${ORIGINS[*]}  (+ tauri://localhost, http://tauri.localhost)"
  echo "  sessions   $PREFIX-<slug>     ← yours keep their own prefix"
  echo "  log        ~/.cache/worktrees/serve.log   (grep $pid)"
  echo "  stop       app/scripts/server-stack.sh --stop"
  echo
} >&2
# stdout is the ONE thing a caller needs: the file a client reads to connect.
echo "$CFG/serve.json"
