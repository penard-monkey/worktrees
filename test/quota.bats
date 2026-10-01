#!/usr/bin/env bats
# The launch-time plan-usage gate (`worktrees_core::quota`).
#
# WHY THIS FILE EXISTS. Before it, nothing could prove the refusal. The suite's
# PATH has no real `codex`, and it cannot shim `/usr/bin/security` (an absolute
# path) or `curl` — so every `new` in every other file took the fail-open path,
# and a green suite said nothing whatever about this gate. `common_setup` pins
# WORKTREES_USAGE_PROBE=off for the rest of the suite; these tests point it at
# a fixture, which is the only way the refusal is ever executed.
#
# The gate runs inside `Adapter::prepare`, so it needs a harness the registry
# knows — the suite's default `fake-ai` matches none, and `prepare` is never
# reached for it. Hence WORKTREES_AI_CMD=claude here.

load 'helpers/common'

setup() {
  common_setup
  export WORKTREES_AI_CMD=claude
  FIXTURE="$BATS_TEST_TMPDIR/usage.json"
}

# A fixture where claude's 5h window is $1 percent at severity $2.
usage_fixture() {
  local pct="$1" sev="$2" resets="${3:-$(( $(date +%s) + 3600 ))}"
  cat > "$FIXTURE" <<JSON
{"claude":[{"label":"5h","percent":$pct,"severity":"$sev","resets_at":$resets}]}
JSON
  export WORKTREES_USAGE_PROBE="$FIXTURE"
}

@test "quota: a nearly spent window refuses the LAUNCH and still makes the place" {
  usage_fixture 92 warning
  run_wt new feat-q
  # Exit 5 is EXIT_LAUNCH_REFUSED — distinct from 1, which is "nothing happened".
  [ "$status" -eq 5 ]
  [[ "$output" == *"92% of its 5h window"* ]]
  [[ "$output" == *"resets in"* ]]
  [[ "$output" == *"Ask the user before overriding"* ]]
  # The place and its branch EXIST: only the agent did not start. That is the
  # whole point of a Soft refusal — the handoff survives the wait.
  [ -d "$REPO/.worktrees/feat-q" ]
  [ "$(git -C "$REPO/.worktrees/feat-q" rev-parse --abbrev-ref HEAD)" = "feat-q" ]
  # …and the retry names the place, not a flag the caller cannot use.
  [[ "$output" == *"worktrees open feat-q --force"* ]]
}

@test "quota: headroom launches exactly as before" {
  usage_fixture 40 normal
  run_wt new feat-ok
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-ok
}

@test "quota: the provider's own grade decides, not the percentage" {
  # 99% graded `normal` must NOT refuse — there is one definition of "nearly
  # out" and it is the provider's.
  usage_fixture 99 normal
  run_wt new feat-norm
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-norm
}

@test "quota: --force launches anyway" {
  usage_fixture 92 warning
  run_wt new feat-forced --force
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-forced
}

@test "quota: an expired window is stale data, not a full one" {
  # resets_at in the PAST. Claude's cached reading can be 30 minutes old, so
  # without this a rolled-over window keeps refusing.
  usage_fixture 92 warning "$(( $(date +%s) - 60 ))"
  run_wt new feat-expired
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-expired
}

@test "quota: no reading at all allows — the gate fails open" {
  export WORKTREES_USAGE_PROBE=off
  run_wt new feat-nodata
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-nodata
}

@test "quota: another provider's spent window never gates this one" {
  cat > "$FIXTURE" <<JSON
{"codex":[{"label":"5h","percent":97,"severity":"warning","resets_at":$(( $(date +%s) + 3600 ))}]}
JSON
  export WORKTREES_USAGE_PROBE="$FIXTURE"
  run_wt new feat-other
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-other
}

@test "quota: --no-tmux starts no agent, so it is never gated" {
  usage_fixture 92 warning
  run_wt new feat-notmux --no-tmux
  [ "$status" -eq 0 ]
  [ -d "$REPO/.worktrees/feat-notmux" ]
  [ ! -s "$TMUX_LOG" ]
}

@test "quota: an unreadable fixture allows rather than inventing a refusal" {
  export WORKTREES_USAGE_PROBE="$BATS_TEST_TMPDIR/does-not-exist.json"
  run_wt new feat-badfix
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-badfix
}

@test "quota: open --force relaunches a place whose launch was refused" {
  usage_fixture 92 warning
  run_wt new feat-retry
  [ "$status" -eq 5 ]
  [ -d "$REPO/.worktrees/feat-retry" ]
  # The exact retry the refusal printed.
  run_wt open feat-retry --force
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-retry
}
