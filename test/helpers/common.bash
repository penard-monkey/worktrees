# Shared test harness. Every test gets: isolated HOME/git config, a throwaway
# repo + bare "origin" (so fetch/track DWIM works offline), a fake tmux PATH
# shim (assertable via $TMUX_LOG + $TMUX_STATE), and fake AI/package-manager
# commands. RUN_BASH picks the bash the CLI runs under (CI sets /bin/bash on
# macOS = real 3.2).

WT_BIN="$BATS_TEST_DIRNAME/../bin/worktrees"

common_setup() {
  export HOME="$BATS_TEST_TMPDIR/home"; mkdir -p "$HOME"
  export GIT_CONFIG_NOSYSTEM=1 GIT_TERMINAL_PROMPT=0
  git config --global user.email t@t
  git config --global user.name t
  git config --global init.defaultBranch main
  git config --global protocol.file.allow always

  export SHIMS="$BATS_TEST_TMPDIR/shims"; mkdir -p "$SHIMS"
  export PATH="$SHIMS:$PATH"
  export TMUX_LOG="$BATS_TEST_TMPDIR/tmux.log" TMUX_STATE="$BATS_TEST_TMPDIR/tmux-state"
  mkdir -p "$TMUX_STATE"; : > "$TMUX_LOG"
  install_fake_tmux
  install_fake_cmd fake-ai
  install_fake_cmd pnpm; install_fake_cmd npm; install_fake_cmd yarn; install_fake_cmd bun
  unset TMUX                      # don't inherit the developer's real tmux
  export WORKTREES_AI_CMD="fake-ai"
  unset WORKTREES_CLAUDE_CMD WORKTREES_AI_RESUME_ARG WORKTREES_PREFIX XDG_CONFIG_HOME || true

  make_repo
}

make_repo() {   # $REPO with one commit pushed to bare $ORIGIN
  ORIGIN="$BATS_TEST_TMPDIR/origin.git"; REPO="$BATS_TEST_TMPDIR/repo"
  git init -q --bare "$ORIGIN"
  git init -q "$REPO"
  ( cd "$REPO" && echo hi > README.md && git add -A && git commit -qm init \
      && git remote add origin "$ORIGIN" && git push -qu origin main )
  # Physical path: the CLI realpath-normalizes everything (pwd -P), so state
  # files / output carry /private/var/... while bats' $TMPDIR says /var/....
  # Keep $REPO physical so assertions compare like with like. The symlinked
  # (logical) form is preserved for the symlink regression test in misc.bats.
  # shellcheck disable=SC2034  # consumed by misc.bats (symlink regression test)
  REPO_LOGICAL="$REPO"
  REPO="$(cd "$REPO" && pwd -P)"
}

make_remote_branch() {   # branch that exists ONLY on origin → exercises fetch+track
  git -C "$REPO" push -q origin "main:refs/heads/$1"
  git -C "$REPO" update-ref -d "refs/remotes/origin/$1" 2>/dev/null || true
}
make_local_branch() { git -C "$REPO" branch "$1"; }
make_dirty()        { echo dirty >> "$1/README.md"; }
add_lockfile()      { ( cd "$REPO" && touch "${1:-pnpm-lock.yaml}" && git add -A && git commit -qm lockfile && git push -q origin main ); }

# Run the CLI from inside $REPO (or -C <dir>) under $RUN_BASH. stdin </dev/null
# so prompts fail fast; tests that answer prompts pipe explicitly.
run_wt() {
  local d="$REPO"
  if [ "${1:-}" = -C ]; then d="$2"; shift 2; fi
  run bash -c 'cd "$1" && shift && exec "${RUN_BASH:-bash}" "$@"' _ "$d" "$WT_BIN" "$@" < /dev/null
}

# Same but with stdin from a heredoc/pipe for confirmation prompts: wt_answer "y" rm foo
wt_answer() {
  local ans="$1"; shift
  local d="$REPO"
  if [ "${1:-}" = -C ]; then d="$2"; shift 2; fi
  run bash -c 'a="$1"; d="$2"; shift 2; cd "$d" && printf "%s\n" "$a" | "${RUN_BASH:-bash}" "$@"' _ "$ans" "$d" "$WT_BIN" "$@"
}

install_fake_cmd() {   # logs argv to $BATS_TEST_TMPDIR/<name>.log, exits 0
  cat > "$SHIMS/$1" <<EOF
#!/usr/bin/env bash
echo "\$@" >> "$BATS_TEST_TMPDIR/$1.log"
exit 0
EOF
  chmod +x "$SHIMS/$1"
}

# Fake tmux: appends argv to $TMUX_LOG; session registry = files in $TMUX_STATE.
# Covers exactly the tmux surface bin/worktrees uses. list-panes emits one row
# per session: name<TAB>cwd<TAB>cmd, where cmd comes from an optional
# $TMUX_STATE/<session>.cmd file (tests write it to simulate a running AI).
install_fake_tmux() {
  cat > "$SHIMS/tmux" <<'EOF'
#!/usr/bin/env bash
echo "tmux $*" >> "$TMUX_LOG"
sub="${1:-}"; shift || true
target=""; session=""; cwd=""; positional=()
while [ $# -gt 0 ]; do
  case "$1" in
    -t) target="$2"; shift 2 ;;
    -s) session="$2"; shift 2 ;;
    -c) cwd="$2"; shift 2 ;;
    -d|-h|-P) shift ;;
    -F) shift 2 ;;
    -L|-f) shift 2 ;;
    *) positional+=("$1"); shift ;;
  esac
done
target="${target#=}"   # exact-match prefix used by kill-session -t "=name"
case "$sub" in
  has-session)   [ -f "$TMUX_STATE/$target" ] ;;
  list-sessions)
    found=1
    for f in "$TMUX_STATE"/*; do
      [ -f "$f" ] || continue
      case "$f" in *.cmd|*/.last) continue ;; esac
      basename "$f"; found=0
    done
    exit $found ;;
  new-session)
    printf 'cwd=%s\ncmd0=%s\n' "$cwd" "${positional[0]:-}" > "$TMUX_STATE/$session"
    printf '%s' "$session" > "$TMUX_STATE/.last"
    echo "%0" ;;
  split-window)
    last="$(cat "$TMUX_STATE/.last" 2>/dev/null || true)"
    [ -n "$last" ] && echo "cmd1=${positional[0]:-}" >> "$TMUX_STATE/$last" ;;
  select-pane|attach|attach-session|switch-client) : ;;
  kill-session)  rm -f "$TMUX_STATE/$target" 2>/dev/null; : ;;
  list-panes)
    for f in "$TMUX_STATE"/*; do
      [ -f "$f" ] || continue
      case "$f" in *.cmd|*/.last) continue ;; esac
      s="$(basename "$f")"
      c="$(sed -n 's/^cwd=//p' "$f" | head -n1)"
      cmd="bash"; [ -f "$TMUX_STATE/$s.cmd" ] && cmd="$(cat "$TMUX_STATE/$s.cmd")"
      printf '%s\t%s\t%s\n' "$s" "$c" "$cmd"
    done ;;
  -V|*) : ;;
esac
EOF
  chmod +x "$SHIMS/tmux"
}

remove_fake_tmux() { rm -f "$SHIMS/tmux"; }

# Assertion sugar over the shim state.
tmux_session_exists() { [ -f "$TMUX_STATE/$1" ]; }
tmux_pane0_cmd()      { sed -n 's/^cmd0=//p' "$TMUX_STATE/$1" | head -n1; }
tmux_pane1_cmd()      { sed -n 's/^cmd1=//p' "$TMUX_STATE/$1" | head -n1; }
