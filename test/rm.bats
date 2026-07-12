#!/usr/bin/env bats
# Tests for `worktrees rm` (cmd_rm / remove_one).

load 'helpers/common'

setup() {
  common_setup
  # ($REPO is physicalized centrally in make_repo — the symlinked-path bug this
  # file originally worked around is fixed in bin/worktrees via pwd -P; the
  # regression test lives in misc.bats.)
}

# Create a registered worktree (with a tmux session) and assert it worked.
mk_wt() {
  run_wt new "$@"
  [ "$status" -eq 0 ]
}

@test "rm: clean worktree with -y is removed (dir gone, unregistered, session killed)" {
  mk_wt feat-x
  tmux_session_exists repo-feat-x

  run_wt rm -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-x"* ]]
  [ ! -e "$REPO/.worktrees/feat-x" ]
  run git -C "$REPO" worktree list
  [[ "$output" != *"feat-x"* ]]
  grep -q "kill-session -t =repo-feat-x" "$TMUX_LOG"   # exact-match =target (prefix-match guard)
  ! tmux_session_exists repo-feat-x
}

@test "rm: dirty worktree refuses with exit 1" {
  mk_wt feat-x
  make_dirty "$REPO/.worktrees/feat-x"

  run_wt rm -y feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"uncommitted changes"* ]]
  [[ "$output" == *"Refusing to remove"* ]]
  [ -d "$REPO/.worktrees/feat-x" ]
}

@test "rm: dirty worktree with --force -y is removed" {
  mk_wt feat-x
  make_dirty "$REPO/.worktrees/feat-x"

  run_wt rm --force -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-x"* ]]
  [ ! -e "$REPO/.worktrees/feat-x" ]
}

@test "rm: default keeps the branch" {
  mk_wt feat-x

  run_wt rm -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"kept branch feat-x"* ]]
  git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
}

@test "rm: --branch -y deletes a merged branch" {
  mk_wt feat-x

  run_wt rm --branch -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"deleted branch feat-x"* ]]
  ! git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
}

@test "rm: --branch -y on an UNMERGED branch warns and keeps it" {
  mk_wt feat-x
  ( cd "$REPO/.worktrees/feat-x" && echo work > work.txt && git add -A && git commit -qm work )

  run_wt rm --branch -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"branch 'feat-x' not deleted"* ]]
  [[ "$output" != *"deleted branch feat-x"* ]]
  git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
  [ ! -e "$REPO/.worktrees/feat-x" ]
}

@test "rm: --branch --force -y force-deletes an unmerged branch (-D)" {
  mk_wt feat-x
  ( cd "$REPO/.worktrees/feat-x" && echo work > work.txt && git add -A && git commit -qm work )

  run_wt rm --branch --force -y feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"deleted branch feat-x"* ]]
  ! git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
}

@test "rm: confirmation prompt answered 'n' skips (exit 0, dir intact)" {
  mk_wt feat-x

  wt_answer n rm feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"Skipped feat-x"* ]]
  [ -d "$REPO/.worktrees/feat-x" ]
  run git -C "$REPO" worktree list
  [[ "$output" == *"feat-x"* ]]
}

@test "rm: confirmation prompt answered 'y' removes" {
  mk_wt feat-x

  wt_answer y rm feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-x"* ]]
  [ ! -e "$REPO/.worktrees/feat-x" ]
}

@test "rm: stale unregistered dir under .worktrees/ is plain-deleted, main repo intact" {
  mkdir -p "$REPO/.worktrees/stale"
  echo junk > "$REPO/.worktrees/stale/file.txt"

  run_wt rm -y stale
  # Regression: a successful branchless removal used to exit 1 (trailing
  # `[[ -n "$branch" ]] && info …` as remove_one's last command). Fixed.
  [ "$status" -eq 0 ]
  [[ "$output" == *"not a registered worktree"* ]]
  [[ "$output" == *"deleted stale dir stale"* ]]
  [ ! -e "$REPO/.worktrees/stale" ]
  # main checkout untouched
  [ -f "$REPO/README.md" ]
  run git -C "$REPO" status --porcelain
  [ -z "$output" ]
}

@test "rm: multiple names with -y removes both" {
  mk_wt feat-a
  mk_wt feat-b

  run_wt rm -y feat-a feat-b
  [ "$status" -eq 0 ]
  [[ "$output" == *"removed worktree feat-a"* ]]
  [[ "$output" == *"removed worktree feat-b"* ]]
  [ ! -e "$REPO/.worktrees/feat-a" ]
  [ ! -e "$REPO/.worktrees/feat-b" ]
}

@test "rm: one bad name among good — good removed, exit 1" {
  mk_wt feat-a

  run_wt rm -y feat-a nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"removed worktree feat-a"* ]]
  [[ "$output" == *"No worktree 'nope'"* ]]
  [ ! -e "$REPO/.worktrees/feat-a" ]
}

@test "rm: refuses when run from inside the target worktree" {
  mk_wt feat-x
  # Resolve the physical path so $PWD matches the CLI's resolved worktree path
  # (macOS $TMPDIR lives under the /var → /private/var symlink).
  local wt_phys
  wt_phys="$(cd "$REPO/.worktrees/feat-x" && pwd -P)"

  run_wt -C "$wt_phys" rm -y feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"cd elsewhere"* ]]
  [ -d "$REPO/.worktrees/feat-x" ]
}

@test "rm: nonexistent name errors with exit 1" {
  run_wt rm -y nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"No worktree 'nope'"* ]]
}

@test "rm: detached-HEAD worktree with --branch -y removes wt, no branch delete attempted" {
  mk_wt feat-x
  git -C "$REPO/.worktrees/feat-x" checkout -q --detach

  run_wt rm --branch -y feat-x
  # Regression: detached HEAD (branch="") used to exit 1 despite successful
  # removal (trailing `[[ -n "$branch" ]] && …`). Fixed; branch ops skipped.
  [ "$status" -eq 0 ]
  [[ "$output" != *"deleted branch"* ]]
  [[ "$output" == *"removed worktree feat-x"* ]]
  [ ! -e "$REPO/.worktrees/feat-x" ]
  # the branch itself is untouched
  git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
}
