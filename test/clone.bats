#!/usr/bin/env bats
# `worktrees clone <url>` — clone + register, through the compiled binary.
# The remote is the harness's own bare $ORIGIN over file://, so every clone
# here is a real one with no network. HOME is the test's own, so the registry
# written is a throwaway one.

load 'helpers/common'

setup() { common_setup; }

REG() { echo "$HOME/.config/worktrees/projects.json"; }

@test "clone runs outside a repository, clones, and registers the project" {
  mkdir -p "$BATS_TEST_TMPDIR/dest"
  run_wt -C "$BATS_TEST_TMPDIR/dest" clone "file://$ORIGIN"
  [ "$status" -eq 0 ]
  [ -f "$BATS_TEST_TMPDIR/dest/origin/README.md" ]
  [[ "$output" == *"registered 'origin'"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects ls --json
  [[ "$output" == *"$BATS_TEST_TMPDIR/dest/origin"* ]]
}

@test "clone --into and --name pick the destination; missing parents are made" {
  run_wt -C "$BATS_TEST_TMPDIR" clone "file://$ORIGIN" --into "$BATS_TEST_TMPDIR/a/b" --name mine
  [ "$status" -eq 0 ]
  [ -d "$BATS_TEST_TMPDIR/a/b/mine/.git" ]
}

@test "clone refuses an existing target and leaves it untouched" {
  mkdir -p "$BATS_TEST_TMPDIR/dest/origin"
  echo keep > "$BATS_TEST_TMPDIR/dest/origin/theirs.txt"
  run_wt -C "$BATS_TEST_TMPDIR/dest" clone "file://$ORIGIN"
  [ "$status" -eq 1 ]
  [[ "$output" == *"already exists"* ]]
  [ "$(cat "$BATS_TEST_TMPDIR/dest/origin/theirs.txt")" = keep ]
  [ ! -e "$(REG)" ]
}

@test "a failed clone says why, removes only its own directory, registers nothing" {
  mkdir -p "$BATS_TEST_TMPDIR/dest"
  run_wt -C "$BATS_TEST_TMPDIR/dest" clone "file://$BATS_TEST_TMPDIR/missing.git"
  [ "$status" -eq 1 ]
  [[ "$output" == *"No repository at"* ]]
  [ -d "$BATS_TEST_TMPDIR/dest" ]
  [ ! -e "$BATS_TEST_TMPDIR/dest/missing" ]
  [ ! -e "$(REG)" ]
}

@test "clone refuses option-shaped and remote-helper URLs before git runs" {
  run_wt -C "$BATS_TEST_TMPDIR" clone -- -oProxyCommand=x
  [ "$status" -eq 1 ]
  [[ "$output" == *"cannot start with '-'"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" clone "fd::17"
  [ "$status" -eq 1 ]
  [[ "$output" == *"not supported"* ]]
}

@test "clone with no url prints usage and exits 2" {
  run_wt -C "$BATS_TEST_TMPDIR" clone
  [ "$status" -eq 2 ]
  [[ "$output" == *"usage: worktrees clone"* ]]
}
