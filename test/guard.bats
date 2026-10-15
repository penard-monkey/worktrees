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
  plant other-a; plant notes vim; plant build bash; plant dev node
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

# Hold the machine launch lock from another process for $1 seconds.
hold_launch_lock() {
  local lock="$HOME/.local/state/worktrees/launch.lock" ready="$BATS_TEST_TMPDIR/held"
  mkdir -p "$(dirname "$lock")"; rm -f "$ready"
  python3 -c 'import fcntl, sys, time
f = open(sys.argv[1], "a"); fcntl.flock(f, fcntl.LOCK_EX); open(sys.argv[2], "w").close(); time.sleep(float(sys.argv[3]))' \
    "$lock" "$ready" "$1" >/dev/null 2>&1 3>&- &
  HOLDER=$!
  disown "$HOLDER"
  local i; for i in $(seq 1 50); do [ -e "$ready" ] && return 0; sleep 0.1; done
  return 1
}

@test "guard: a harness's own probe runs without the machine lock (review of #486, B2)" {
  # The quota gate (Adapter::prepare) refuses this launch. Its probes can take
  # 15s on a slow network, so they must not run under the machine-wide lock:
  # with the lock held elsewhere, the refusal still comes back at once.
  export WORKTREES_AI_CMD=claude
  printf '{"claude":[{"label":"5h","percent":95,"severity":"warning","resets_at":%s}]}' "$(( $(date +%s) + 3600 ))" > "$BATS_TEST_TMPDIR/usage.json"
  export WORKTREES_USAGE_PROBE="$BATS_TEST_TMPDIR/usage.json"
  hold_launch_lock 4
  local t0=$SECONDS
  run_wt new feat-q
  kill "$HOLDER" 2>/dev/null || true
  [ "$status" -eq 5 ]
  [[ "$output" == *"95% of its 5h window"* ]]
  [ $((SECONDS - t0)) -lt 3 ]
}

@test "guard: a launch that waits for the machine lock says so" {
  hold_launch_lock 1
  run_wt new feat-wait
  [ "$status" -eq 0 ]
  [[ "$output" == *"waiting for another launch to finish"* ]]
  tmux_session_exists repo-feat-wait
}

@test "guard: a launch that only adopts a session already in the place is not refused" {
  limits 'max_live_agents = 1'
  run_wt new feat-ad --no-tmux
  # A hand-made agent session in the place: `open` adopts it, starting nothing.
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-ad" > "$TMUX_STATE/handmade"
  printf 'claude' > "$TMUX_STATE/handmade.cmd"
  run_wt open feat-ad --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"'handmade' already in this worktree"* ]]
}

@test "guard: a [limits] problem is said at launch" {
  limits 'max_live = { cluade = 2 }'
  run_wt new feat-typo
  [ "$status" -eq 0 ]
  [[ "$output" == *"max_live.cluade: no such provider"* ]]
}
