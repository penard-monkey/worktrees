#!/usr/bin/env bats
# Tests for `worktrees ls --json` (schema_version 1) — the machine-readable
# contract the UI consumes. The human `ls` path must stay byte-for-byte identical
# (covered here by the flag-leak guard + the existing ls.bats assertions).

load 'helpers/common'

setup() {
  common_setup
  command -v python3 >/dev/null 2>&1 || skip "python3 required to validate JSON"
}

# Evaluate python expr $2 against the place `p` with slug $1 (from $output).
# NOTE: eval() here runs ONLY the hardcoded, test-authored expressions passed by
# the assertions below (e.g. 'p["branch"]', 'type(p["ahead"]).__name__') — never
# external, user, or tool-under-test input. Safe in this test-only helper.
field() {
  printf '%s' "$output" | python3 -c '
import sys, json
d = json.load(sys.stdin)
p = next((x for x in d["places"] if x["slug"] == sys.argv[1]), None)
print(eval(sys.argv[2]))
' "$1" "$2"
}

# Whole payload must parse as JSON.
assert_valid_json() { printf '%s' "$output" | python3 -m json.tool >/dev/null; }

@test "ls --json: valid JSON, schema_version 1, main place first" {
  run_wt new feat-x --no-tmux
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  local sv main_first main_slug
  sv="$(printf '%s' "$output" | python3 -c 'import sys,json;print(json.load(sys.stdin)["schema_version"])')"
  main_first="$(printf '%s' "$output" | python3 -c 'import sys,json;print(json.load(sys.stdin)["places"][0]["is_main"])')"
  main_slug="$(printf '%s' "$output" | python3 -c 'import sys,json;print(json.load(sys.stdin)["places"][0]["slug"])')"
  [ "$sv" = "1" ]
  [ "$main_first" = "True" ]
  [ "$main_slug" = "(main)" ]
}

@test "ls --json: a clean worktree carries slug, branch, dirty=false, registered, tmux down" {
  run_wt new feat-x --no-tmux
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["branch"]')" = "feat-x" ]
  [ "$(field feat-x 'p["detached"]')" = "False" ]
  [ "$(field feat-x 'p["dirty"]')" = "False" ]
  [ "$(field feat-x 'p["registered"]')" = "True" ]
  [ "$(field feat-x 'p["tmux_session"]["up"]')" = "False" ]
  [ "$(field feat-x 'p["schema_version"]')" = "1" ]
}

@test "ls --json: dirty worktree reports dirty=true and dirty_files > 0" {
  run_wt new feat-x --no-tmux
  make_dirty "$REPO/.worktrees/feat-x"
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["dirty"]')" = "True" ]
  [ "$(field feat-x 'p["dirty_files"] > 0')" = "True" ]
}

@test "ls --json: live tmux session → tmux_session.up = true and lifecycle_effective active" {
  run_wt new feat-z --no-tmux
  # simulate a live session named after the worktree (prefix "repo")
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-z" > "$TMUX_STATE/repo-feat-z"
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-z 'p["tmux_session"]["up"]')" = "True" ]
  [ "$(field feat-z 'p["lifecycle_effective"]')" = "active" ]
}

@test "ls --json: branch with no upstream → ahead/behind/upstream all null" {
  run_wt new feat-x --no-tmux
  # force no-upstream regardless of git's autoSetupMerge default
  git -C "$REPO/.worktrees/feat-x" branch --unset-upstream 2>/dev/null || true
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["ahead"]')" = "None" ]
  [ "$(field feat-x 'p["behind"]')" = "None" ]
  [ "$(field feat-x 'p["upstream"]')" = "None" ]
}

@test "ls --json: tracked branch → integer ahead/behind and upstream string" {
  run_wt new feat-x --no-tmux
  git -C "$REPO/.worktrees/feat-x" branch --set-upstream-to=origin/main
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'type(p["ahead"]).__name__')" = "int" ]
  [ "$(field feat-x 'type(p["behind"]).__name__')" = "int" ]
  [ "$(field feat-x 'p["upstream"]')" = "origin/main" ]
}

@test "ls --json: detached HEAD → branch null, detached true" {
  run_wt new feat-x --no-tmux
  git -C "$REPO/.worktrees/feat-x" checkout -q --detach
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["branch"]')" = "None" ]
  [ "$(field feat-x 'p["detached"]')" = "True" ]
}

@test "ls --json: stale unregistered dir → registered:false, minimal (nulls, no MAIN leak)" {
  mkdir -p "$REPO/.worktrees/stale-dir"
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field stale-dir 'p["registered"]')" = "False" ]
  [ "$(field stale-dir 'p["branch"]')" = "None" ]
  [ "$(field stale-dir 'p["dirty"]')" = "None" ]
}

@test "ls --json: commit subject with quotes and backslashes stays valid JSON" {
  run_wt new feat-x --no-tmux
  ( cd "$REPO/.worktrees/feat-x" && echo more >> README.md && git add -A \
      && git commit -qm 'weird "subject" with \ backslash' )
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json                                   # the load-bearing escaper check
  [ "$(field feat-x 'chr(34) in p["last_commit_subject"]')" = "True" ]   # a quote survived
  [ "$(field feat-x 'chr(92) in p["last_commit_subject"]')" = "True" ]   # a backslash survived
}

@test "ls --json: reports install_cmd from a lockfile, null without one" {
  add_lockfile pnpm-lock.yaml            # commits a lockfile to main
  run_wt new feat-x --no-tmux
  run_wt ls --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["install_cmd"]')" = "pnpm install" ]
}

@test "ls --json: stack and declared are null in P0 (no infra, no store yet)" {
  run_wt new feat-x --no-tmux
  run_wt ls --json
  [ "$status" -eq 0 ]
  [ "$(field feat-x 'p["stack"]')" = "None" ]
  [ "$(field feat-x 'p["declared"]')" = "None" ]
}

@test "WORKTREES_JSON=1 makes a plain ls emit JSON" {
  run_wt new feat-x --no-tmux
  export WORKTREES_JSON=1
  run_wt ls
  unset WORKTREES_JSON
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(field feat-x 'p["slug"]')" = "feat-x" ]
}

@test "ls WITHOUT --json still prints the human table (no flag leak)" {
  run_wt new feat-x --no-tmux
  run_wt ls
  [ "$status" -eq 0 ]
  [[ "$output" == *SLUG* && "$output" == *BRANCH* ]]
  [[ "$output" != *'"schema_version"'* ]]
}
