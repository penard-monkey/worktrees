#!/usr/bin/env bats
# cmd_open — reopen/attach an existing worktree's tmux session.

load 'helpers/common'

setup() { common_setup; }

# Make a worktree WITHOUT a tmux session, so `open` is what creates/attaches it.
make_worktree() { # args passed through to `new`
  run_wt new "$@" --no-tmux
  [ "$status" -eq 0 ]
}

# Count real session files in the fake-tmux registry (excludes *.cmd and .last).
session_count() {
  local n=0 f
  for f in "$TMUX_STATE"/*; do
    [ -f "$f" ] || continue
    case "$f" in *.cmd|*/.last) continue ;; esac
    n=$((n + 1))
  done
  echo "$n"
}

@test "open by slug creates the session cwd'd in the worktree with pane0 = AI" {
  make_worktree feat-x
  run_wt open feat-x
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-x
  grep -qx "cwd=$REPO/.worktrees/feat-x" "$TMUX_STATE/repo-feat-x"
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *fake-ai* ]]
}

@test "open by BRANCH resolves the differently-named holder worktree" {
  make_worktree feat/foo --name topic
  local repo_phys; repo_phys="$REPO"   # already physical (make_repo)
  run_wt -C "$repo_phys" open feat/foo
  [ "$status" -eq 0 ]
  [[ "$output" == *"lives in worktree 'topic'"* ]]
  tmux_session_exists repo-topic
  grep -qx "cwd=$repo_phys/.worktrees/topic" "$TMUX_STATE/repo-topic"
}

@test "open reuses an AI session already living in the worktree under another name" {
  make_worktree feat-x
  # Simulate an AI pane already running here under a foreign session name.
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-x" > "$TMUX_STATE/other-sess"
  echo fake-ai > "$TMUX_STATE/other-sess.cmd"
  run_wt open feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"already in this worktree"* ]]
  # No second session minted — registry still holds exactly the pre-made one.
  ! tmux_session_exists repo-feat-x
  [ "$(session_count)" -eq 1 ]
  tmux_session_exists other-sess
  grep -q "attach -t other-sess" "$TMUX_LOG"
}

@test "open nonexistent name errors with the Create-it hint" {
  run_wt open nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"No worktree 'nope'"* ]]
  [[ "$output" == *"Create it"* ]]
}

@test "open -r appends the resume arg to pane0" {
  make_worktree feat-x
  run_wt open -r feat-x
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *"fake-ai -r"* ]]
}

@test "open --ai overrides WORKTREES_AI_CMD from the environment" {
  install_fake_cmd codex
  make_worktree feat-x
  run_wt open --ai codex feat-x
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *codex* ]]
  [[ "$(tmux_pane0_cmd repo-feat-x)" != *fake-ai* ]]
}

@test "open --ai without a value errors" {
  make_worktree feat-x
  run_wt open feat-x --ai
  [ "$status" -eq 1 ]
  [[ "$output" == *"--ai needs a value"* ]]
}

@test "open --no-attach creates the session but never attaches" {
  make_worktree feat-x
  : > "$TMUX_LOG"
  run_wt open --no-attach feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"Session ready (detached)"* ]]
  tmux_session_exists repo-feat-x
  ! grep -E ' (attach|attach-session|switch-client) ' "$TMUX_LOG"
  ! grep -E '(attach|switch-client)' "$TMUX_LOG"
}

@test "open hard-errors when tmux is absent (unlike new)" {
  make_worktree feat-x
  remove_fake_tmux
  # Strip homebrew's real tmux from PATH too; keep system dirs for git/bash.
  export PATH="$SHIMS:/usr/bin:/bin"
  run_wt open feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"tmux not found"* ]]
}
