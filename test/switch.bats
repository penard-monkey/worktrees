#!/usr/bin/env bats
# Tests for cmd_switch (worktrees switch / sw).

load 'helpers/common'

setup() {
  common_setup
  # ($REPO physicalized centrally in make_repo; symlinked-path handling is
  #  fixed in bin/worktrees via pwd -P — regression test in misc.bats.)
}

# Create a registered worktree on branch $1 (skips tmux to keep setup minimal).
make_wt() {
  run_wt new "$1" --no-tmux
  [ "$status" -eq 0 ]
}

wt_head() { git -C "$REPO/.worktrees/$1" rev-parse --abbrev-ref HEAD; }

@test "switch <wt> <branch> from repo root: existing local branch → plain switch" {
  make_wt feat-x
  make_local_branch feat-y

  run_wt switch feat-x feat-y
  [ "$status" -eq 0 ]
  [[ "$output" == *"Switching 'feat-x' → 'feat-y'"* ]]
  [[ "$output" == *"Branch exists locally — switching."* ]]
  [[ "$output" == *"was 'feat-x' → now 'feat-y'"* ]]
  [ "$(wt_head feat-x)" = "feat-y" ]
}

@test "switch <branch> from INSIDE a worktree targets that worktree" {
  make_wt feat-x
  make_local_branch feat-y

  run_wt -C "$REPO/.worktrees/feat-x" switch feat-y
  [ "$status" -eq 0 ]
  [[ "$output" == *"Switching 'feat-x' → 'feat-y'"* ]]
  [ "$(wt_head feat-x)" = "feat-y" ]
}

@test "switch <wt> <branch> <base>: new branch created off base" {
  make_wt feat-x
  # local-only base branch with a commit main doesn't have
  git -C "$REPO" checkout -qb mybase
  echo base > "$REPO/base.txt"
  git -C "$REPO" add -A && git -C "$REPO" commit -qm base
  git -C "$REPO" checkout -q main

  run_wt switch feat-x newb mybase
  [ "$status" -eq 0 ]
  [[ "$output" == *"Creating new branch 'newb' off 'mybase'."* ]]
  [ "$(wt_head feat-x)" = "newb" ]
  [ "$(git -C "$REPO" rev-parse newb)" = "$(git -C "$REPO" rev-parse mybase)" ]
}

@test "switch to the branch already checked out → no-op exit 0" {
  make_wt feat-x

  run_wt switch feat-x feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"Already on 'feat-x' — nothing to do."* ]]
}

@test "switch to remote-only branch → fetch + track (upstream set)" {
  make_wt feat-x
  make_remote_branch remote-b

  run_wt switch feat-x remote-b
  [ "$status" -eq 0 ]
  [[ "$output" == *"Tracking remote branch origin/remote-b."* ]]
  [ "$(wt_head feat-x)" = "remote-b" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse --abbrev-ref 'remote-b@{upstream}')" = "origin/remote-b" ]
}

@test "switch to branch that exists nowhere → new branch off default base, tracking origin/main" {
  make_wt feat-x

  run_wt switch feat-x brand-new
  [ "$status" -eq 0 ]
  [[ "$output" == *"Creating new branch 'brand-new' off 'origin/main'."* ]]
  [ "$(wt_head feat-x)" = "brand-new" ]
  [ "$(git -C "$REPO/.worktrees/feat-x" rev-parse --abbrev-ref 'brand-new@{upstream}')" = "origin/main" ]
}

@test "origin/ prefix is stripped from the branch argument" {
  make_wt feat-x
  make_local_branch feat-y

  run_wt switch feat-x origin/feat-y
  [ "$status" -eq 0 ]
  [[ "$output" == *"Switching 'feat-x' → 'feat-y'"* ]]
  [ "$(wt_head feat-x)" = "feat-y" ]
}

@test "dirty worktree → refuses with exit 1 and lists the files" {
  make_wt feat-x
  make_local_branch feat-y
  make_dirty "$REPO/.worktrees/feat-x"

  run_wt switch feat-x feat-y
  [ "$status" -eq 1 ]
  [[ "$output" == *"has uncommitted changes"* ]]
  [[ "$output" == *"README.md"* ]]
  [[ "$output" == *"Refusing to switch."* ]]
  [ "$(wt_head feat-x)" = "feat-x" ]
}

@test "dirty worktree + --force → switches anyway" {
  make_wt feat-x
  make_local_branch feat-y
  make_dirty "$REPO/.worktrees/feat-x"

  run_wt switch --force feat-x feat-y
  [ "$status" -eq 0 ]
  [ "$(wt_head feat-x)" = "feat-y" ]
}

@test "typo guard: inside wt, first arg not a worktree → warns and treats args as <branch> <base>" {
  make_wt feat-x

  # NOTE: by design (warned), args[0] becomes the BRANCH — a worktree-name typo
  # mints a branch named after the typo instead of erroring out.
  run_wt -C "$REPO/.worktrees/feat-x" switch nonexistent-wt main
  [ "$status" -eq 0 ]
  [[ "$output" == *"No worktree 'nonexistent-wt' — treating args as <branch> <base> for 'feat-x' (from cwd)."* ]]
  [ "$(wt_head feat-x)" = "nonexistent-wt" ]
}

@test "ambiguity confirm: inside A targeting B, answer n → Aborted, exit 0, B untouched" {
  make_wt feat-a
  make_wt feat-b
  make_local_branch target-b

  wt_answer n -C "$REPO/.worktrees/feat-a" switch feat-b target-b
  [ "$status" -eq 0 ]
  [[ "$output" == *"You're inside 'feat-a' but this targets worktree 'feat-b' (branch 'target-b')."* ]]
  [[ "$output" == *"Aborted."* ]]
  [ "$(wt_head feat-b)" = "feat-b" ]
  [ "$(wt_head feat-a)" = "feat-a" ]
}

@test "ambiguity confirm: answer y → proceeds and switches B" {
  make_wt feat-a
  make_wt feat-b
  make_local_branch target-b

  wt_answer y -C "$REPO/.worktrees/feat-a" switch feat-b target-b
  [ "$status" -eq 0 ]
  [[ "$output" == *"You're inside 'feat-a' but this targets worktree 'feat-b'"* ]]
  [ "$(wt_head feat-b)" = "target-b" ]
  [ "$(wt_head feat-a)" = "feat-a" ]
}

@test "-y skips the ambiguity prompt entirely" {
  make_wt feat-a
  make_wt feat-b
  make_local_branch target-b

  # run_wt has stdin </dev/null — a prompt would kill the script, so success
  # proves no prompt was issued.
  run_wt -C "$REPO/.worktrees/feat-a" switch -y feat-b target-b
  [ "$status" -eq 0 ]
  [[ "$output" != *"You're inside"* ]]
  [ "$(wt_head feat-b)" = "target-b" ]
}

@test "not inside a worktree + 1 arg → error asks to name one" {
  make_wt feat-x

  run_wt switch some-branch
  [ "$status" -eq 1 ]
  [[ "$output" == *"Not inside a worktree — name one: worktrees switch <worktree> <branch>"* ]]
}

@test "not inside a worktree + 2 args, first not a worktree → error exit 1" {
  make_wt feat-x

  run_wt switch nope some-branch
  [ "$status" -eq 1 ]
  [[ "$output" == *"No worktree 'nope' under .worktrees/."* ]]
}

@test "registration gate: unregistered dir under .worktrees/ → refuses, main repo untouched" {
  mkdir -p "$REPO/.worktrees/fake"

  run_wt switch fake somebranch
  [ "$status" -eq 1 ]
  [[ "$output" == *"not a registered worktree"* ]]
  # main checkout must be untouched — no branch switch, no branch minted
  [ "$(git -C "$REPO" rev-parse --abbrev-ref HEAD)" = "main" ]
  ! git -C "$REPO" show-ref --verify --quiet refs/heads/somebranch
}

@test "branch checked out in another worktree → fails and lists worktrees" {
  make_wt feat-a
  make_wt feat-b

  run_wt switch feat-a feat-b
  [ "$status" -eq 1 ]
  [[ "$output" == *"git switch failed. If the branch is checked out in another worktree:"* ]]
  [[ "$output" == *".worktrees/feat-b"* ]]
  [ "$(wt_head feat-a)" = "feat-a" ]
}

@test "sw alias dispatches to switch" {
  make_wt feat-x
  make_local_branch feat-y

  run_wt sw feat-x feat-y
  [ "$status" -eq 0 ]
  [ "$(wt_head feat-x)" = "feat-y" ]
}
