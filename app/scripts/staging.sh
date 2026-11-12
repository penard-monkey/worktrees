#!/usr/bin/env bash
# A sandbox that FOLLOWS a branch (default `staging`): a developer tool.
#
#   app/scripts/staging.sh                  # = --update: pull the branch, build if it moved
#   app/scripts/staging.sh --app            # update, then launch the sandbox app from it
#   app/scripts/staging.sh --watch          # a launchd agent runs --update every 10 min
#   app/scripts/staging.sh --unwatch        # remove that agent
#   app/scripts/staging.sh --clean          # sandbox.sh --clean, in the branch's place
#   app/scripts/staging.sh --branch <b> …   # follow another branch
#
# The branch lives in its own worktrees PLACE (`worktrees new <b> --no-tmux
# --no-install`, reused when it exists), never in the main checkout and never
# in a raw `git worktree add`. An update is `git fetch origin` +
# `git merge --ff-only origin/<b>`, refused when the place has local changes or
# local commits; a commit not built yet is bootstrapped (submodules, the app's
# pnpm install under the pinned node) and built (`cargo build --release -p
# worktrees-cli` in `worktrees run --slot gate`). `--app` then execs
# `sandbox.sh --app` from the place, which isolates by branch on its own.
#
# The launchd agent only PULLS AND BUILDS. It never starts, stops or relaunches
# an app: a sandbox app that is gone was closed by its human, and an automatic
# relaunch is a loop. While a `--app` run holds the place (its lock is held by
# the app's own pid, through the exec, for the app's whole life) the agent
# skips: a merge under a running `tauri dev` would rebuild and relaunch the
# window by itself.
#
# Everything lives in main() so that a merge which rewrites THIS file while it
# runs (it is in the place it updates, if you run that copy) cannot change the
# lines bash has not read yet.
set -euo pipefail

LABEL_BASE="net.casadelvalle.worktrees.staging"
# Lock, log and build marker. WORKTREES_STAGING_CACHE moves them (a test
# must not write into the real one); the agent is handed the same value.
CACHE="${WORKTREES_STAGING_CACHE:-$HOME/.cache/worktrees/staging}"

say() { printf '%s\n' "$*" >&2; }
die() { say "✗ $*"; exit 1; }

# Every git call names a directory, and that directory must be a real one
# under the repository: `git -C ""` means HERE (AGENTS.md).
g() {
  local dir="$1"; shift
  [ -n "$dir" ] || die "internal: git with an empty directory"
  case "$dir/" in "$REPO"/*) ;; *) die "internal: git outside $REPO: $dir" ;; esac
  [ -d "$dir" ] || die "internal: not a directory: $dir"
  git -C "$dir" "$@"
}

slug_of() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-' | sed 's/-*$//'; }

# The worktree that has refs/heads/<branch> checked out, or nothing.
place_of() {
  local wt="" line
  while IFS= read -r line; do
    case "$line" in
      "worktree "*) wt="${line#worktree }" ;;
      "branch refs/heads/$BRANCH") printf '%s\n' "$wt"; return 0 ;;
    esac
  done < <(git -C "$REPO" worktree list --porcelain)
  return 0
}

# --- the lock: one directory per branch, owned by "<pid> <start time>" -------
# A pid alone could be a stranger's after a reboot; its start time cannot.
proc_start() { ps -o lstart= -p "$1" 2>/dev/null | sed 's/^ *//' || true; }

lock_take() {
  local owner pid start
  for _ in 1 2; do
    if mkdir "$LOCK" 2>/dev/null; then
      printf '%s %s\n' "$$" "$(proc_start $$)" > "$LOCK/owner"
      LOCK_MINE=1
      return 0
    fi
    owner="$(cat "$LOCK/owner" 2>/dev/null || true)"
    pid="${owner%% *}"; start="${owner#* }"
    if [ -n "$pid" ] && [ "$pid" != "$owner" ] && kill -0 "$pid" 2>/dev/null \
       && [ "$(proc_start "$pid")" = "$start" ]; then
      LOCK_HOLDER="$pid"
      return 1
    fi
    # Its owner is gone (an app quit, a run killed): reclaim it.
    rm -rf "$LOCK"
  done
  LOCK_HOLDER="?"
  return 1
}

lock_drop() { if [ "${LOCK_MINE:-0}" = 1 ]; then rm -rf "$LOCK"; LOCK_MINE=0; fi; }

# --- the pinned node, without `nvm use` (AGENTS.md: it can take pnpm off PATH)
node_bin_dir() {
  local want d best="" v
  want="$(tr -d ' \nv' < "$PLACE/.nvmrc")"
  [ -n "$want" ] || die "$PLACE/.nvmrc is empty"
  for d in "${NVM_DIR:-$HOME/.nvm}"/versions/node/v"$want" "${NVM_DIR:-$HOME/.nvm}"/versions/node/v"$want".*; do
    [ -x "$d/bin/node" ] && best="$d/bin"
  done
  if [ -z "$best" ] && command -v node >/dev/null 2>&1; then
    v="$(node --version 2>/dev/null || true)"
    case "$v" in "v$want"|"v$want".*) best="$(dirname "$(command -v node)")" ;; esac
  fi
  [ -n "$best" ] || die "no node $want (the place's .nvmrc): install it with \`nvm install $want\`"
  printf '%s\n' "$best"
}

# --- update ------------------------------------------------------------------
update() {
  local fetch_ok=1 branch_now old new built=""
  if [ -z "$PLACE" ]; then
    say "→ no place has '$BRANCH' checked out: creating one"
    ( cd "$REPO" && worktrees new "$BRANCH" --no-tmux --no-install --no-attach ) >&2 \
      || die "worktrees new $BRANCH failed (above)"
    PLACE="$(place_of)"
    [ -n "$PLACE" ] || die "worktrees new succeeded but no worktree has '$BRANCH' checked out"
  fi
  [ "$PLACE" != "$REPO" ] || die "'$BRANCH' is checked out in the main checkout ($REPO); this script only follows a branch in its own place"
  say "→ $BRANCH in $PLACE"

  branch_now="$(g "$PLACE" symbolic-ref --quiet --short HEAD || true)"
  [ "$branch_now" = "$BRANCH" ] || die "the place is on '${branch_now:-a detached HEAD}', not '$BRANCH'"

  if ! g "$PLACE" fetch --quiet origin; then
    [ "$MODE" = app ] || die "git fetch origin failed"
    say "! git fetch origin failed: launching what is already there"
    fetch_ok=0
  fi

  local dirty
  dirty="$(g "$PLACE" status --porcelain --untracked-files=normal)"
  if [ -n "$dirty" ]; then
    say "✗ the place has local changes, so it will not be updated:"
    printf '%s\n' "$dirty" | head -20 | sed 's/^/    /' >&2
    die "commit, move or discard them in $PLACE (it follows origin/$BRANCH and holds no work of its own)"
  fi

  old="$(g "$PLACE" rev-parse HEAD)"
  if [ "$fetch_ok" = 1 ]; then
    g "$PLACE" rev-parse --verify --quiet "refs/remotes/origin/$BRANCH" >/dev/null \
      || die "origin has no branch '$BRANCH'"
    if ! g "$PLACE" -c advice.diverging=false merge --ff-only --quiet "origin/$BRANCH" >&2; then
      say "✗ cannot fast-forward to origin/$BRANCH: the place has commits origin does not:"
      g "$PLACE" log --oneline "origin/$BRANCH..HEAD" | head -20 | sed 's/^/    /' >&2
      die "push or drop them; this place only follows origin/$BRANCH"
    fi
  fi
  new="$(g "$PLACE" rev-parse HEAD)"
  if [ "$old" = "$new" ]; then
    say "  up to date at $(g "$PLACE" log -1 --format='%h %s' "$new")"
  else
    say "  $(g "$PLACE" rev-parse --short "$old") → $(g "$PLACE" rev-parse --short "$new")"
    g "$PLACE" log --oneline "$old..$new" | sed 's/^/    /' >&2
  fi

  # Build what has not been built: a commit, not "HEAD moved", so a build that
  # failed last tick is tried again on the next one.
  if [ -f "$BUILT" ]; then built="$(cat "$BUILT")"; fi
  if [ "$FORCE_BUILD" = 0 ] && [ "$built" = "$PLACE $new" ] && [ -x "$PLACE/target/release/worktrees" ] \
     && [ -d "$PLACE/app/node_modules" ]; then
    say "  built already"
    return 0
  fi
  rm -f "$BUILT"
  say "→ submodules"
  g "$PLACE" submodule update --init --recursive >&2
  local nb
  nb="$(node_bin_dir)"
  say "→ pnpm install (node $("$nb/node" --version), pnpm from app/package.json)"
  # cwd app/, never `--dir`: corepack picks pnpm from the CWD (AGENTS.md).
  ( cd "$PLACE/app" && PATH="$nb:$PATH" COREPACK_ENABLE_DOWNLOAD_PROMPT=0 \
      corepack pnpm install --frozen-lockfile ) >&2 || die "pnpm install failed (above)"
  say "→ cargo build --release -p worktrees-cli (slot gate)"
  ( cd "$PLACE" && worktrees run --slot gate -- cargo build --release -p worktrees-cli ) >&2 \
    || die "the build failed (above); the next update tries again"
  printf '%s %s\n' "$PLACE" "$new" > "$BUILT"
  say "  built $(g "$PLACE" rev-parse --short "$new")"
}

# --- the launchd agent -------------------------------------------------------
xml() { printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'; }

watch() {
  [ "$(uname -s)" = Darwin ] || die "--watch is macOS only (launchd); not on Linux: run \`$0 --update\` from cron or a systemd timer yourself"
  command -v worktrees >/dev/null 2>&1 || die "worktrees is not on PATH, and the agent runs it"
  local plist="$HOME/Library/LaunchAgents/$LABEL.plist" self env_xml="" k v
  self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
  mkdir -p "$CACHE" "$HOME/Library/LaunchAgents"
  # launchd gives an agent a bare PATH (no cargo, no homebrew, no tmux): hand
  # it the one this was installed from, and the config dir if one is set.
  for k in PATH XDG_CONFIG_HOME NVM_DIR CARGO_HOME RUSTUP_HOME WORKTREES_STAGING_CACHE; do
    v="$(eval "printf '%s' \"\${$k:-}\"")"
    if [ -n "$v" ]; then
      env_xml="$env_xml    <key>$k</key><string>$(xml "$v")</string>
"
    fi
  done
  cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$(xml "$LABEL")</string>
  <key>ProgramArguments</key>
  <array>
    <string>/bin/bash</string>
    <string>$(xml "$self")</string>
    <string>--update</string>
    <string>--agent</string>
    <string>--branch</string><string>$(xml "$BRANCH")</string>
    <string>--repo</string><string>$(xml "$REPO")</string>
  </array>
  <key>EnvironmentVariables</key>
  <dict>
$env_xml  </dict>
  <key>StartInterval</key><integer>600</integer>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>$(xml "$LOG")</string>
  <key>StandardErrorPath</key><string>$(xml "$LOG")</string>
</dict>
</plist>
EOF
  plutil -lint "$plist" >/dev/null || die "the generated plist does not parse: $plist"
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  launchctl bootstrap "gui/$(id -u)" "$plist" || die "launchctl bootstrap failed for $plist"
  say "✓ $LABEL runs \`staging.sh --update\` for '$BRANCH' now and every 10 minutes"
  say "  plist  $plist"
  say "  log    $LOG"
  say "  it pulls and builds only; it never starts or relaunches an app"
  local again="$0 --unwatch${BRANCH_FLAG}"
  if [ "$LABEL" != "$LABEL_BASE" ] && [ "$LABEL" != "$LABEL_BASE-$(slug_of "$BRANCH")" ]; then again="$again --label $LABEL"; fi
  say "  remove it: $again"
}

unwatch() {
  [ "$(uname -s)" = Darwin ] || die "--unwatch is macOS only (launchd)"
  local plist="$HOME/Library/LaunchAgents/$LABEL.plist"
  launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null || true
  rm -f "$plist"
  say "✓ $LABEL removed (its log stays: $LOG)"
}

main() {
  MODE=update BRANCH=staging REPO="" FORCE_BUILD=0 AGENT=0 LABEL="" BRANCH_FLAG=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --update) MODE=update; shift ;;
      --app) MODE=app; shift ;;
      --watch) MODE=watch; shift ;;
      --unwatch) MODE=unwatch; shift ;;
      --clean) MODE=clean; shift ;;
      --force-build) FORCE_BUILD=1; shift ;;
      --agent) AGENT=1; shift ;;   # set by the launchd agent: a held lock is a skip
      --branch) [ $# -ge 2 ] || die "--branch needs a value"; BRANCH="$2"; BRANCH_FLAG=" --branch $2"; shift 2 ;;
      --repo) [ $# -ge 2 ] || die "--repo needs a value"; REPO="$2"; shift 2 ;;
      --label) [ $# -ge 2 ] || die "--label needs a value"; LABEL="$2"; shift 2 ;;
      -h|--help) sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//'; return 0 ;;
      *) die "unknown option: $1" ;;
    esac
  done
  case "$BRANCH" in ""|-*|*..*|*" "*) die "not a branch name: '$BRANCH'" ;; esac
  BRANCH="${BRANCH#origin/}"

  # The repository: --repo, else the one this script sits in. Its MAIN
  # checkout, found through the common dir, so a copy run from any place works.
  local start common
  start="${REPO:-$(cd "$(dirname "$0")" && pwd)}"
  [ -d "$start" ] || die "no such directory: $start"
  common="$(git -C "$start" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)" \
    || die "not inside a git repository: $start"
  REPO="$(cd "$common/.." && pwd -P)"
  [ -n "$REPO" ] && [ -d "$REPO/.git" ] || die "cannot find the main checkout from $start"

  local slug
  slug="$(slug_of "$BRANCH")"
  if [ -z "$LABEL" ]; then
    if [ "$BRANCH" = staging ]; then LABEL="$LABEL_BASE"; else LABEL="$LABEL_BASE-$slug"; fi
  fi
  case "$LABEL" in "$LABEL_BASE"|"$LABEL_BASE"[.-]*) ;; *) die "--label must start with $LABEL_BASE" ;; esac
  mkdir -p "$CACHE"
  LOG="$CACHE/$slug.log"
  LOCK="$CACHE/$slug.lock"
  BUILT="$CACHE/$slug.built"

  case "$MODE" in
    watch) watch; return 0 ;;
    unwatch) unwatch; return 0 ;;
  esac

  PLACE="$(place_of)"
  if [ "$MODE" = clean ]; then
    [ -n "$PLACE" ] || die "no place has '$BRANCH' checked out: nothing to clean"
    exec "$PLACE/app/scripts/sandbox.sh" --clean
  fi

  if [ "$AGENT" = 1 ]; then say "== $(date '+%Y-%m-%d %H:%M:%S') staging.sh --update ($BRANCH, agent) =="; fi
  if ! lock_take; then
    if [ "$AGENT" = 1 ]; then
      say "  skipped: pid $LOCK_HOLDER holds $LOCK (a sandbox app or another update)"
      return 0
    fi
    die "pid $LOCK_HOLDER holds $LOCK: a sandbox app from it is open, or an update runs. Quit that app (⌘Q) first."
  fi
  trap lock_drop EXIT

  update

  if [ "$MODE" = app ]; then
    [ -x "$PLACE/app/scripts/sandbox.sh" ] || die "$PLACE has no app/scripts/sandbox.sh"
    say "→ sandbox.sh --app in $PLACE (DISABLE_AUTOUPDATER=1)"
    # The lock stays held by this pid through the exec, for the app's life,
    # so the agent never merges under a running `tauri dev`.
    trap - EXIT
    cd "$PLACE"
    DISABLE_AUTOUPDATER=1 exec "$PLACE/app/scripts/sandbox.sh" --app
  fi
}

main "$@"
