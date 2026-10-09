#!/usr/bin/env bats
# Owned planning, phase 1 (docs/proposals/owned-planning.md §9), through the
# compiled binary: `new` writes the pointer only at full, `plan resolve --json`
# answers every row of §3.2's case table and every show-only case, and the
# adoption-time refusal / every-`new` warning for a tracked `.planning/`.
#
# HOME is the test's own (common_setup), so planning.json and projects.json
# here are throwaway ones.

load 'helpers/common'

setup() {
  common_setup
  run_wt projects add
  [ "$status" -eq 0 ]
}

level() { run_wt plan level "$@"; [ "$status" -eq 0 ] || { echo "$output"; return 1; }; }

# `plan resolve --json` in DIR, reduced to "how|rel|topic|reason".
resolved() {
  run bash -c 'cd "$1" && "$2" plan resolve --json' _ "$1" "$WT_BIN"
  [ "$status" -eq 0 ] || { echo "$output"; return 1; }
  printf '%s\n' "$output" | jq -r '[.how_resolved, .plan_rel, .topic, .reason] | map(. // "-") | join("|")'
}

lane() { mkdir -p "$REPO/.worktrees/$1"; echo "$REPO/.worktrees/$1"; }
put() { mkdir -p "$(dirname "$1/$2")"; printf '%s\n' "${3:-# plan}" > "$1/$2"; }

# ── new ──────────────────────────────────────────────────────────────────────

@test "plan: new writes the pointer and an empty topic dir only at full" {
  run_wt new feat-off --no-tmux
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/.worktrees/feat-off/.planning" ]

  level show docs
  run_wt new feat-show --no-tmux
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/.worktrees/feat-show/.planning" ]

  level full
  run_wt new feat-full --no-tmux
  [ "$status" -eq 0 ]
  [ "$(cat "$REPO/.worktrees/feat-full/.planning/.active_plan")" = feat-full ]
  [ -d "$REPO/.worktrees/feat-full/.planning/feat-full" ]
  [ -z "$(ls -A "$REPO/.worktrees/feat-full/.planning/feat-full")" ]
  [[ "$output" == *"plan: .planning/feat-full/"* ]]
  # nothing ignores .planning/ in this repo, so the exclude line is added once
  [ "$(grep -c '^/.planning/$' "$REPO/.git/info/exclude")" -eq 1 ]
  [ "$(resolved "$REPO/.worktrees/feat-full")" = "pending|-|feat-full|-" ]
}

@test "plan: the global default reaches new; a project's off beats it" {
  run_wt plan default full
  [ "$status" -eq 0 ]
  run_wt new feat-g --no-tmux
  [ -f "$REPO/.worktrees/feat-g/.planning/.active_plan" ]
  level off
  run_wt new feat-o --no-tmux
  [ ! -e "$REPO/.worktrees/feat-o/.planning" ]
}

@test "plan: new never overwrites a pointer and leaves an ignored repo's exclude alone" {
  printf '.planning/\n' >> "$REPO/.git/info/exclude"
  level full
  run_wt new feat-x --no-tmux
  [ "$(grep -c 'planning' "$REPO/.git/info/exclude")" -eq 1 ]
  echo chosen > "$REPO/.worktrees/feat-x/.planning/.active_plan"
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ "$(cat "$REPO/.worktrees/feat-x/.planning/.active_plan")" = chosen ]
}

@test "plan: full is refused at adoption where .planning/ holds tracked files, and new warns" {
  ( cd "$REPO" && mkdir -p .planning/orchestrator && echo '# goals' > .planning/orchestrator/task_plan.md \
      && git add -f .planning && git commit -qm tracked && git push -q origin main )
  run_wt plan level full
  [ "$status" -eq 1 ]
  [[ "$output" == *"holds tracked files"*"show only"* ]]
  run_wt plan level
  [[ "$output" == "off"* ]]
  run_wt new feat-t --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *".planning/\` holds tracked files"* ]]
}

# ── owned resolution: §3.2's case table ──────────────────────────────────────

@test "plan: owned resolution, every row of the case table" {
  level full
  local L
  L="$(lane none)";            [ "$(resolved "$L")" = "-|-|-|-" ]
  L="$(lane rootonly)";        put "$L" task_plan.md
                               [ "$(resolved "$L")" = "root|task_plan.md|-|-" ]
  L="$(lane noguess)";         put "$L" .planning/orchestrator/task_plan.md
                               [ "$(resolved "$L")" = "-|-|-|-" ]
  L="$(lane active)";          put "$L" .planning/t/task_plan.md; put "$L" task_plan.md; echo t > "$L/.planning/.active_plan"
                               [ "$(resolved "$L")" = "active_plan|.planning/t/task_plan.md|t|-" ]
  L="$(lane pending)";         mkdir -p "$L/.planning"; echo t > "$L/.planning/.active_plan"
                               [ "$(resolved "$L")" = "pending|-|t|-" ]
  mkdir -p "$L/.planning/t";   [ "$(resolved "$L")" = "pending|-|t|-" ]
  put "$L" task_plan.md;       [ "$(resolved "$L")" = "root|task_plan.md|t|-" ]
  L="$(lane dotdot)";          mkdir -p "$L/.planning"; echo ../x > "$L/.planning/.active_plan"
                               [ "$(resolved "$L")" = "invalid_pointer|-|-|-" ]
  put "$L" task_plan.md;       [ "$(resolved "$L")" = "invalid_pointer|task_plan.md|-|-" ]
  L="$(lane slash)";           mkdir -p "$L/.planning"; echo a/b > "$L/.planning/.active_plan"
                               [ "$(resolved "$L")" = "invalid_pointer|-|-|-" ]
  L="$(lane evil)";            mkdir -p "$L/.planning" "$BATS_TEST_TMPDIR/elsewhere"; echo evil > "$L/.planning/.active_plan"
                               put "$BATS_TEST_TMPDIR/elsewhere" task_plan.md
                               ln -s "$BATS_TEST_TMPDIR/elsewhere" "$L/.planning/evil"
                               [ "$(resolved "$L")" = "invalid_pointer|-|-|-" ]
  L="$(lane linkptr)";         mkdir -p "$L/.planning"; echo t > "$BATS_TEST_TMPDIR/ptr"
                               ln -s "$BATS_TEST_TMPDIR/ptr" "$L/.planning/.active_plan"
                               [ "$(resolved "$L")" = "invalid_pointer|-|-|-" ]
  L="$(lane linkplanning)";    ln -s "$BATS_TEST_TMPDIR/elsewhere" "$L/.planning"
                               [ "$(resolved "$L")" = "invalid_pointer|-|-|-" ]
}

@test "plan: off is today's resolution, newest-dir guess included" {
  local L; L="$(lane legacy)"
  put "$L" .planning/orchestrator/task_plan.md
  [ "$(resolved "$L")" = "newest|.planning/orchestrator/task_plan.md|-|-" ]
  run bash -c 'cd "$1" && "$2" plan resolve --json' _ "$L" "$WT_BIN"
  [ "$(printf '%s\n' "$output" | jq -r .level)" = off ]
}

# ── show only ────────────────────────────────────────────────────────────────

@test "plan: show only — a file, a directory with and without task_plan.md, a trailing slash" {
  mkdir -p "$REPO/docs/plan" "$REPO/docs/empty"
  local L; L="$(lane s)"
  put "$L" docs/goals.md; put "$L" docs/plan/task_plan.md; mkdir -p "$L/docs/empty"
  put "$L" .planning/x/task_plan.md; echo x > "$L/.planning/.active_plan"
  level show docs/goals.md
  [ "$(resolved "$L")" = "show_path|docs/goals.md|-|-" ]
  level show docs/plan/
  run_wt plan level
  [[ "$output" == "show"*"docs/plan" ]]
  [ "$(resolved "$L")" = "show_path|docs/plan/task_plan.md|-|-" ]
  level show docs/empty
  [ "$(resolved "$L")" = "show_path|-|-|no task_plan.md in docs/empty" ]
}

@test "plan: show only — a symlinked dir and an escape through a symlink are refused" {
  mkdir -p "$BATS_TEST_TMPDIR/out"; put "$BATS_TEST_TMPDIR/out" task_plan.md
  local L; L="$(lane s)"
  mkdir -p "$L/docs/real"; put "$L" docs/real/task_plan.md
  ln -s "$L/docs/real" "$L/docs/linked"
  ln -s "$BATS_TEST_TMPDIR/out" "$L/docs/far"
  level show docs/linked
  [ "$(resolved "$L")" = "show_path|-|-|outside the project" ]
  level show docs/far/task_plan.md
  [ "$(resolved "$L")" = "show_path|-|-|outside the project" ]
  # and the setter refuses an escape it can see from main
  ln -s "$BATS_TEST_TMPDIR/out" "$REPO/away"
  run_wt plan level show away --scope main
  [ "$status" -eq 1 ]
  [[ "$output" == *"outside the project"* ]]
}

@test "plan: show only, main scope — a lane reads MAIN's working tree, uncommitted edits included" {
  mkdir -p "$REPO/docs"; echo '# goals, committed' > "$REPO/docs/goals.md"
  ( cd "$REPO" && git add docs && git commit -qm goals && git push -q origin main )
  echo '# goals, edited in main' > "$REPO/docs/goals.md"
  level show docs/goals.md --scope main
  run_wt new feat-m --no-tmux
  local L="$REPO/.worktrees/feat-m"
  [ "$(head -1 "$L/docs/goals.md")" = "# goals, committed" ]
  run bash -c 'cd "$1" && "$2" plan resolve --json' _ "$L" "$WT_BIN"
  [ "$(printf '%s\n' "$output" | jq -r '[.how_resolved, .plan_scope, .title] | join("|")')" = "show_path|main|goals, edited in main" ]
  [ "$(printf '%s\n' "$output" | jq -r .plan_path)" = "$REPO/docs/goals.md" ]
}

# ── hook ─────────────────────────────────────────────────────────────────────

@test "plan: hook prints nothing outside a repo, when off, or at show; always exits 0" {
  run bash -c 'echo "{}" | "$1" plan hook session' _ "$WT_BIN"
  [ "$status" -eq 0 ]; [ -z "$output" ]
  run bash -c 'cd "$1" && echo "{}" | "$2" plan hook session' _ "$REPO" "$WT_BIN"
  [ "$status" -eq 0 ]; [ -z "$output" ]
  mkdir -p "$REPO/docs"; put "$REPO" docs/goals.md
  level show docs/goals.md
  run bash -c 'cd "$1" && echo "{}" | "$2" plan hook session' _ "$REPO" "$WT_BIN"
  [ "$status" -eq 0 ]; [ -z "$output" ]
  level full
  run bash -c 'cd "$1" && echo "{}" | "$2" plan hook session' _ "$REPO" "$WT_BIN"
  [ "$status" -eq 0 ]
  [[ "$output" == "[worktrees planning v"*"not instructions"* ]]
}
