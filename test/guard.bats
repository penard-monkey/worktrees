#!/usr/bin/env bats
# The launch guard (`worktrees_core::guard`): a live-agent cap and a memory
# floor, on the same Soft-refusal path as the quota gate.
#
# common_setup pins WORKTREES_GUARD_PROBE=off (no memory reading, so no
# RAM-derived cap); these tests set `[limits]` in the user's config or point
# the probe at a fixture. "Live agents" are planted fake-tmux sessions whose
# pane command is an agent's — nothing real is started.

load 'helpers/common'

setup() {
  common_setup
  mkdir -p "$HOME/.config/worktrees"
}

limits() { printf '%s\n' "[limits]" "$@" > "$HOME/.config/worktrees/config.toml"; }

# A live session named $1 running $2 (default claude), in a directory that is
# not this repo's — the cap is per machine.
plant() {
  printf 'cwd=/elsewhere/%s\ncmd0=x\n' "$1" > "$TMUX_STATE/$1"
  printf '%s' "${2:-claude}" > "$TMUX_STATE/$1.cmd"
}

memory_fixture() {
  printf '%s' "$1" > "$BATS_TEST_TMPDIR/mem.json"
  export WORKTREES_GUARD_PROBE="$BATS_TEST_TMPDIR/mem.json"
}

@test "guard: the cap refuses the launch, keeps the place, and --force launches" {
  limits 'max_live_agents = 2'
  plant other-a; plant other-b codex
  run_wt new feat-g
  [ "$status" -eq 5 ]
  [[ "$output" == *"2 agents are already live on this machine (limit 2"* ]]
  [[ "$output" == *"other-a, other-b"* ]]
  [[ "$output" == *"worktrees open feat-g --force"* ]]
  [ -d "$REPO/.worktrees/feat-g" ]
  ! tmux_session_exists repo-feat-g
  run_wt open feat-g --force --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-g
}

@test "guard: below the cap launches, and shells and editors are not agents" {
  limits 'max_live_agents = 2'
  plant other-a; plant notes vim; plant build bash
  run_wt new feat-ok
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-ok
}

@test "guard: with no [limits] the cap is half the RAM" {
  memory_fixture '{"ram_bytes": 4294967296}'
  plant other-a; plant other-b
  run_wt new feat-ram
  [ "$status" -eq 5 ]
  [[ "$output" == *"(limit 2, \`[limits] max_live_agents\`)"* ]]
}

@test "guard: a per-provider cap counts that provider only" {
  limits 'max_live_agents = 0' 'max_live = { claude = 1 }'
  export WORKTREES_AI_CMD=claude FAKE_TMUX_NEW_CMD=claude
  plant other-x codex; plant other-y codex
  run_wt new feat-cl
  [ "$status" -eq 0 ]
  run_wt new feat-cl2
  [ "$status" -eq 5 ]
  [[ "$output" == *"1 claude agents are already live"* ]]
}

@test "guard: low memory refuses, enough memory launches" {
  memory_fixture '{"free_pct": 10}'
  run_wt new feat-low
  [ "$status" -eq 5 ]
  [[ "$output" == *"has 10% of its memory free (limit 20%"* ]]
  memory_fixture '{"free_pct": 55}'
  run_wt open feat-low --no-attach
  [ "$status" -eq 0 ]
}

@test "guard: zero turns each limit off" {
  memory_fixture '{"free_pct": 3, "ram_bytes": 1073741824}'
  limits 'max_live_agents = 0' 'min_free_memory_pct = 0'
  plant other-a
  run_wt new feat-off
  [ "$status" -eq 0 ]
}

@test "guard: parallel launches count each other (the machine launch lock)" {
  limits 'max_live_agents = 1'
  local s
  for s in p1 p2 p3; do run_wt new "$s" --no-tmux; [ "$status" -eq 0 ]; done
  export FAKE_TMUX_NEW_CMD=claude FAKE_TMUX_NEW_DELAY=0.5
  for s in p1 p2 p3; do
    ( cd "$REPO" && "$WT_BIN" open "$s" --no-attach ) > "$BATS_TEST_TMPDIR/open-$s.log" 2>&1 < /dev/null 3>&- &
  done
  wait
  local up=0
  for s in p1 p2 p3; do tmux_session_exists "repo-$s" && up=$((up + 1)); done
  [ "$up" -eq 1 ]
  [ "$(cat "$BATS_TEST_TMPDIR"/open-*.log | grep -c 'agents are already live')" -eq 2 ]
}

@test "a repo cannot set [limits] (ADR 0001)" {
  write_project_config '[limits]' 'max_live_agents = 500'
  run_wt doctor
  [ "$status" -ne 0 ]
  [[ "$output" == *"limits may not be set by a project"* ]]
}
