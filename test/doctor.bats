#!/usr/bin/env bats
# `worktrees doctor` — report declared-file drift. Exit codes are the contract:
# 0 clean · 1 usage/guard · 2 findings present (so CI can tell "doctor broke"
# from "doctor found problems").

load 'helpers/common'

setup() {
  common_setup
  make_secret_repo '.env'
  printf 'SECRET=1\n' > "$REPO/.env"
}

wt() { echo "$REPO/.worktrees/$1"; }

@test "doctor: clean tree exits 0, a dangling link exits 2" {
  write_project_config '[[file]]' 'path = ".env"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]

  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean"* ]]

  rm "$REPO/.env"                      # the link now points at nothing
  run_wt doctor
  [ "$status" -eq 2 ]
  [[ "$output" == *"no longer exists"* ]]
}

@test "doctor: a worktree missing a declared file is a finding, not silence" {
  run_wt new feat-x --no-tmux          # created BEFORE the config existed
  write_project_config '[[file]]' 'path = ".env"'
  run_wt doctor feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"declared but not materialized"* ]]
  [[ "$output" == *"feat-x"* ]]
}

@test "doctor: a shadowing file is reported and doctor NEVER writes" {
  run_wt new feat-x --no-tmux
  printf 'LOCAL\n' > "$(wt feat-x)/.env"
  cp "$(wt feat-x)/.env" "$BATS_TEST_TMPDIR/before"
  write_project_config '[[file]]' 'path = ".env"'

  run_wt doctor feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"shadows declared link"* ]]
  diff "$BATS_TEST_TMPDIR/before" "$(wt feat-x)/.env"
  [ ! -L "$(wt feat-x)/.env" ]
}

@test "doctor: a declared path that is not gitignored is an error (B8)" {
  printf 'PLAIN=1\n' > "$REPO/not-ignored.env"
  run_wt new feat-x --no-tmux
  write_project_config '[[file]]' 'path = "not-ignored.env"'
  run_wt doctor feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"not gitignored"* ]]
  [[ "$output" == *"git add -A"* ]]
}

@test "doctor: a source absent from main warns but does not fail" {
  run_wt new feat-x --no-tmux
  rm "$REPO/.env"
  write_project_config '[[file]]' 'path = ".env"'
  run_wt doctor feat-x
  # missing source (warn) + not materialized here (error) — both must be said
  [[ "$output" == *"absent from the main checkout"* ]]
}

@test "doctor --strict: promotes a stale copy to a failure" {
  write_project_config '[[file]]' 'path = ".env"' 'mode = "copy"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  # dest older than a changed source = `stale` (main moved on)
  touch -t 200001010000 "$(wt feat-x)/.env"
  printf 'SECRET=2\n' > "$REPO/.env"

  run_wt doctor feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"main is NEWER"* ]]
  run_wt doctor feat-x --strict
  [ "$status" -eq 2 ]
}

@test "doctor: a locally rewritten copy is Info and never fails a run" {
  write_project_config '[[file]]' 'path = ".env"' 'mode = "copy"'
  run_wt new feat-x --no-tmux
  printf 'REWRITTEN\n' > "$(wt feat-x)/.env"
  run_wt doctor feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"locally modified"* ]]
  run_wt doctor feat-x --strict
  [ "$status" -eq 0 ]
}

@test "doctor --json: one line, schema_version first, explicit nulls" {
  write_project_config '[[file]]' 'path = ".env"'
  run_wt new feat-x --no-tmux
  rm "$(wt feat-x)/.env"

  run_wt doctor --json
  [ "$status" -eq 2 ]
  [ "${#lines[@]}" -eq 1 ]
  [[ "$output" == '{"schema_version":1,"findings":['* ]]
  [[ "$output" == *'"code":"not-linked"'* ]]
  [[ "$output" == *'"place":"feat-x"'* ]]
  [[ "$output" == *'"path":".env"'* ]]

  # WORKTREES_JSON=1 does the same, like `ls`
  run bash -c 'cd "$1" && shift && WORKTREES_JSON=1 exec "${RUN_BASH:-bash}" "$@"' _ "$REPO" "$WT_BIN" doctor
  [ "$status" -eq 2 ]
  [[ "$output" == '{"schema_version":1,'* ]]
}

@test "doctor --config-only: passes without the secrets, flags a COMMITTED one" {
  write_project_config '[[file]]' 'path = ".env"'
  rm "$REPO/.env"                      # a bare clone in CI has no credentials
  run_wt doctor --config-only
  [ "$status" -eq 0 ]

  # someone commits the secret: tracked in git, and check-ignore can't see it
  printf 'SECRET=1\n' > "$REPO/.env"
  git -C "$REPO" add -f .env
  git -C "$REPO" commit -qm oops
  run_wt doctor --config-only
  [ "$status" -eq 2 ]
  [[ "$output" == *"TRACKED in git"* ]]
}

@test "doctor: no .worktrees.toml → nothing to check, exit 0" {
  run_wt new feat-x --no-tmux
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" == *"No .worktrees.toml"* ]]
  run_wt doctor --json
  [ "$status" -eq 0 ]
  [ "$output" = '{"schema_version":1,"findings":[]}' ]
}

@test "doctor: usage guards" {
  run_wt doctor --bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --bogus"* ]]
  write_project_config '[[file]]' 'path = ".env"'
  run_wt doctor nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"No worktree 'nope'"* ]]
}
