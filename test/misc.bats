#!/usr/bin/env bats
# Plumbing: version/help/dispatch, git guards, ensure_excluded, default_base,
# prefix resolution + sanitization, AI command precedence chain, quote-safety.

load 'helpers/common'

setup() { common_setup; }

# key = value lines into the XDG user config the CLI reads.
write_config() {
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  printf '%s\n' "$@" > "$XDG_CONFIG_HOME/worktrees/config"
}

# ── version / help / dispatch ────────────────────────────────────────────────

@test "--version prints 'worktrees 0.1.0' and exits 0 outside any git repo" {
  run_wt -C "$BATS_TEST_TMPDIR" --version
  [ "$status" -eq 0 ]
  [ "$output" = "worktrees 0.1.0" ]
}

@test "help / -h / --help print usage (contains 'worktrees new') and exit 0 outside any git repo" {
  local v
  for v in help -h --help; do
    run_wt -C "$BATS_TEST_TMPDIR" "$v"
    [ "$status" -eq 0 ]
    [[ "$output" == *"worktrees new"* ]]
  done
}

@test "unknown subcommand prints usage and exits 1" {
  run_wt bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown command: bogus"* ]]
  [[ "$output" == *"worktrees new"* ]]
}

@test "repo-requiring command outside a git repo fails with a clear error" {
  run_wt -C "$BATS_TEST_TMPDIR" ls
  [ "$status" -eq 1 ]
  [[ "$output" == *"Not inside a git repository"* ]]
}

# ── ensure_excluded ──────────────────────────────────────────────────────────

@test "new adds .worktrees/ to .git/info/exclude exactly once" {
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  grep -qFx '.worktrees/' "$REPO/.git/info/exclude"
  run_wt new feat-b --no-tmux
  [ "$status" -eq 0 ]
  [ "$(grep -cFx '.worktrees/' "$REPO/.git/info/exclude")" -eq 1 ]
}

# ── default_base ─────────────────────────────────────────────────────────────

@test "default base falls back to master when main does not exist" {
  local r="$BATS_TEST_TMPDIR/mrepo"
  git init -q -b master "$r"
  ( cd "$r" && echo x > f.txt && git add -A && git commit -qm init )
  run_wt -C "$r" new feat-m --no-tmux --no-fetch
  [ "$status" -eq 0 ]
  [[ "$output" == *"off 'master'"* ]]
  [ "$(git -C "$r/.worktrees/feat-m" rev-parse --abbrev-ref HEAD)" = "feat-m" ]
}

# ── prefix resolution ────────────────────────────────────────────────────────

@test ".worktree-prefix file names the tmux session" {
  echo "myproj" > "$REPO/.worktree-prefix"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists myproj-feat-x
}

@test "WORKTREES_PREFIX env beats the .worktree-prefix file" {
  echo "myproj" > "$REPO/.worktree-prefix"
  export WORKTREES_PREFIX=zzz
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists zzz-feat-x
  [ ! -f "$TMUX_STATE/myproj-feat-x" ]
}

@test "prefix is sanitized: lowercased, '.' and '!' become '-'" {
  echo 'My.Repo!' > "$REPO/.worktree-prefix"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists my-repo--feat-x
}

@test "user-config prefix is used when no env and no file" {
  write_config 'prefix = confpfx'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists confpfx-feat-x
}

# ── AI command precedence chain ──────────────────────────────────────────────

@test "config ai_cmd is used when env vars are unset" {
  unset WORKTREES_AI_CMD
  write_config 'ai_cmd = fake-ai'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"-ic"* ]]
  [[ "$p0" == *"fake-ai"* ]]
}

@test "WORKTREES_CLAUDE_CMD is honored as deprecated fallback when WORKTREES_AI_CMD unset" {
  unset WORKTREES_AI_CMD
  export WORKTREES_CLAUDE_CMD=fake-ai
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai"* ]]
  [[ "$p0" != *"claude"* ]]
}

@test "WORKTREES_AI_CMD beats WORKTREES_CLAUDE_CMD" {
  install_fake_cmd other
  export WORKTREES_AI_CMD=fake-ai WORKTREES_CLAUDE_CMD=other
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai"* ]]
  [[ "$p0" != *"other"* ]]
}

@test "--ai flag beats env WORKTREES_AI_CMD" {
  install_fake_cmd other
  # WORKTREES_AI_CMD=fake-ai is set by common_setup
  run_wt new feat-x --no-install --no-attach --ai other
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"other"* ]]
  [[ "$p0" != *"fake-ai"* ]]
}

@test "config ai_cmd = none gives pane 0 a plain shell (no -ic)" {
  unset WORKTREES_AI_CMD
  write_config 'ai_cmd = none'
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *'exec "${SHELL'* ]]
  [[ "$p0" != *"-ic"* ]]
}

@test "--ai none gives pane 0 a plain shell even with WORKTREES_AI_CMD set" {
  # WORKTREES_AI_CMD=fake-ai is set by common_setup
  run_wt new feat-x --no-install --no-attach --ai none
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *'exec "${SHELL'* ]]
  [[ "$p0" != *"-ic"* ]]
  [[ "$p0" != *"fake-ai"* ]]
}

@test "ai_resume_arg config key changes what -r appends" {
  write_config 'ai_resume_arg = --continue'
  # WORKTREES_AI_RESUME_ARG unset by common_setup; ai cmd stays fake-ai (env)
  run_wt new feat-x --no-install --no-attach -r
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"fake-ai --continue"* ]]
}

@test "AI command containing a single quote is escaped safely into pane 0" {
  export WORKTREES_AI_CMD="fake-ai --note it's"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  # sq() turns it's into it'\''s inside the single-quoted -ic string
  local esc="it'\\''s"
  [[ "$p0" == *"$esc"* ]]
}

@test "symlinked repo path: worktrees stay registered (pwd -P regression)" {
  # macOS $TMPDIR traverses /var → /private/var, so $REPO_LOGICAL reaches the
  # repo THROUGH a symlink. Before the pwd -P fix, wt_registered() compared the
  # logical WT_ROOT against git's physical paths: ls showed every worktree as
  # stale, switch refused, rm plain-deleted instead of `git worktree remove`.
  [ "$REPO_LOGICAL" != "$REPO" ] || skip "tmpdir not behind a symlink on this OS"
  run_wt -C "$REPO_LOGICAL" new feat/sym --no-tmux
  [ "$status" -eq 0 ]
  run_wt -C "$REPO_LOGICAL" ls
  [ "$status" -eq 0 ]
  [[ "$output" != *"stale"* ]]
  [[ "$output" == *"feat/sym"* ]]
  run_wt -C "$REPO_LOGICAL" switch feat-sym feat/sym2
  [ "$status" -eq 0 ]
  run_wt -C "$REPO_LOGICAL" rm -y feat-sym
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-sym"* ]]   # git-removed, NOT plain-deleted
}
