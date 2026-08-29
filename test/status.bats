#!/usr/bin/env bats
# Tests for `worktrees status <name>` — the per-place health verdict.
#
# REAL git throughout (common.bash fakes tmux/docker/rsync/AI, never git), so
# every scenario below is built out of actual commits and actual refs. The fake
# tmux DOES register sessions, which is what makes the parked-vs-cold pair
# testable: a `new` WITHOUT --no-tmux reads `tmux_up = true`.
#
# ⚠ Every staleness case exports WORKTREES_STATUS_NOW (the op's documented test
# seam) at the scenario's commit epoch + 15 days. Without it a just-made commit
# is minutes old and every verdict is "active" — the suite would assert nothing.

load 'helpers/common'

setup() {
  common_setup
  command -v python3 >/dev/null 2>&1 || skip "python3 required to validate JSON"
}

# `status --json` emits ONE object on ONE line (the doctor/ls template), so the
# whole-payload `field`/`assert_valid_json` helpers in json.bats — which index
# into a `places` array — do not apply. This is their single-object twin.
#
# NOTE: eval() runs ONLY the hardcoded, test-authored expressions passed below —
# never external, user, or tool-under-test input. Safe in this test-only helper.
sfield() {
  printf '%s' "$output" | python3 -c '
import sys, json
r = json.loads(sys.stdin.read().strip().splitlines()[-1])
print(eval(sys.argv[1]))
' "$1"
}

assert_valid_json() { printf '%s' "$output" | python3 -m json.tool >/dev/null; }

# Epoch of HEAD in $1, plus 15 days — comfortably past STALE_SECS (14 days).
stale_now() { echo $(( $(git -C "$1" log -1 --format=%ct) + 15 * 86400 )); }

# origin/main gains a commit this worktree does not have (json.bats:106 pattern).
advance_base() {
  ( cd "$REPO" && echo x > newfile && git add -A && git commit -qm advance && git push -q origin main )
}

@test "status: stale worktree with an unpushed commit is at risk" {
  run_wt new feat-x --no-tmux
  local wt="$REPO/.worktrees/feat-x"
  # No upstream regardless of git's autoSetupMerge default — this case is about
  # work that exists NOWHERE but this machine, and a tracking branch would make
  # the assertion below depend on the host's git config.
  git -C "$wt" branch --unset-upstream 2>/dev/null || true
  ( cd "$wt" && echo work > feature.txt && git add -A && git commit -qm "the only copy" )
  export WORKTREES_STATUS_NOW="$(stale_now "$wt")"

  run_wt status feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"at risk"* ]]
  [[ "$output" == *"not on"* ]]

  run_wt status feat-x --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(sfield 'r["verdict"]')" = "at-risk" ]
  [ "$(sfield 'r["facts"]["not_on_base_total"]')" = "1" ]
  [ "$(sfield 'r["facts"]["upstream"]')" = "None" ]
  # The reason that makes this urgent rather than merely unmerged.
  [ "$(sfield 'any("nowhere but this machine" in x for x in r["reasons"])')" = "True" ]
}

@test "status: stale, clean, session up, base moved → parked, and behind is the ONLY thing said" {
  run_wt new feat-y
  local wt="$REPO/.worktrees/feat-y"
  advance_base
  export WORKTREES_STATUS_NOW="$(stale_now "$wt")"

  run_wt status feat-y --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(sfield 'r["facts"]["tmux_up"]')" = "True" ]
  [ "$(sfield 'r["facts"]["behind"] > 0')" = "True" ]
  [ "$(sfield 'r["verdict"]')" = "parked" ]
  # Exactly one reason, asserted by CONTENT — not by grepping for the absence of
  # the word "behind", because the parked sentence itself contains it. That is
  # the whole point: behind is reported as the base moving, never as sickness.
  [ "$(sfield 'len(r["reasons"])')" = "1" ]
  [ "$(sfield 'r["reasons"][0].startswith("clean and nothing ahead of the base")')" = "True" ]
  [ "$(sfield '"is just the base moving" in r["reasons"][0]')" = "True" ]
}

@test "status: same place with no session is cold, not parked" {
  run_wt new feat-y --no-tmux
  local wt="$REPO/.worktrees/feat-y"
  advance_base
  export WORKTREES_STATUS_NOW="$(stale_now "$wt")"

  run_wt status feat-y --json
  [ "$status" -eq 0 ]
  [ "$(sfield 'r["facts"]["tmux_up"]')" = "False" ]
  [ "$(sfield 'r["verdict"]')" = "cold" ]
}

@test "status: a fresh worktree is active" {
  run_wt new feat-x --no-tmux
  run_wt status feat-x --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(sfield 'r["verdict"]')" = "active" ]
  [ "$(sfield 'r["schema_version"]')" = "1" ]
}

@test "status: reports on the main checkout, by the bare name too" {
  # Unquoted parens are a shell syntax error, so `status main` has to work —
  # `(main)` is what the app passes, `main` is what a person types.
  run_wt status main --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(sfield 'r["facts"]["slug"]')" = "(main)" ]
}

@test "status: usage guards — no name, and an unknown name, both exit 1" {
  run_wt status
  [ "$status" -eq 1 ]
  [[ "$output" == *"usage: worktrees status"* ]]

  run_wt status nope
  [ "$status" -eq 1 ]

  run_wt status feat-x --bogus
  [ "$status" -eq 1 ]
}

@test "status --json: the base ref, the note and the commit list ride along" {
  run_wt new feat-x --no-tmux
  local wt="$REPO/.worktrees/feat-x"
  ( cd "$wt" && echo a > a.txt && git add -A && git commit -qm "first" )
  ( cd "$wt" && echo b > b.txt && git add -A && git commit -qm "second" )
  export WORKTREES_STATUS_NOW="$(stale_now "$wt")"

  run_wt status feat-x --json
  [ "$status" -eq 0 ]
  assert_valid_json
  [ "$(sfield 'r["facts"]["base"]')" = "origin/main" ]
  [ "$(sfield 'r["facts"]["not_on_base_total"]')" = "2" ]
  [ "$(sfield 'len(r["facts"]["not_on_base"])')" = "2" ]
  [ "$(sfield 'r["facts"]["not_on_base"][0].endswith("second")')" = "True" ]
}
