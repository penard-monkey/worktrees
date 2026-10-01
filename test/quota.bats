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

OPENER="Read .planning/brief.md and begin."

@test "quota: open --force relaunches a refused place WITH the brief's opener" {
  usage_fixture 92 warning
  run_wt new feat-retry --brief "do x"
  [ "$status" -eq 5 ]
  [ -f "$REPO/.worktrees/feat-retry/.planning/brief.md" ]
  ! tmux_session_exists repo-feat-retry
  # The exact retry the refusal printed. Without the opener claude starts
  # BLANK and the brief the refusal promised would survive is never read.
  run_wt open feat-retry --force
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-retry
  [[ "$(tmux_pane0_cmd repo-feat-retry)" == *"claude --name"*"repo-feat-retry"*"$OPENER"* ]]
}

@test "quota: a plain open once the window has headroom also sends the opener" {
  usage_fixture 92 warning
  run_wt new feat-later --brief "do x"
  [ "$status" -eq 5 ]
  usage_fixture 40 normal
  run_wt open feat-later
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-later)" == *"$OPENER"* ]]
}

@test "quota: an open where claude has ALREADY run sends no opener" {
  # The other half: the opener is for a brief nobody has read. A place with a
  # claude conversation on disk has had its launch, and re-sending would make
  # every fresh open restart the task.
  usage_fixture 40 normal
  run_wt new feat-ran --brief "do x" --no-attach
  [ "$status" -eq 0 ]
  local d m
  for d in "$REPO/.worktrees/feat-ran" "$(cd "$REPO/.worktrees/feat-ran" && pwd -P)"; do
    m="$(printf '%s' "$d" | sed 's/[^A-Za-z0-9]/-/g')"
    mkdir -p "$HOME/.claude/projects/$m" && : > "$HOME/.claude/projects/$m/s.jsonl"
  done
  run_wt close feat-ran
  run_wt open feat-ran --no-attach
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-ran)" != *"$OPENER"* ]]
}

@test "quota: codex too — open --force after a refusal sends the opener" {
  install_fake_cmd codex
  cat > "$FIXTURE" <<JSON
{"codex":[{"label":"5h","percent":91,"severity":"warning","resets_at":$(( $(date +%s) + 3600 ))}]}
JSON
  export WORKTREES_USAGE_PROBE="$FIXTURE"
  run_wt new feat-cx --ai codex --brief "do x"
  [ "$status" -eq 5 ]
  run_wt open feat-cx --ai codex --force
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd 'repo-feat-cx~agent~codex')" == *"codex -c forced_login_method=chatgpt"*"$OPENER"* ]]
}

@test "quota: MCP create_worktree is refused, and force:true launches with the brief" {
  usage_fixture 92 warning
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-q","brief":"do x"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp --mutations < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [[ "$output" == *"92% of its 5h window"* ]]
  [[ "$output" == *"force"* ]]
  ! tmux_session_exists repo-agent-q
  [ -f "$REPO/.worktrees/agent-q/.planning/brief.md" ]
  printf '%s\n' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-q2","brief":"do y","force":true}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp --mutations < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [[ "$output" == *'"isError":false'* ]]
  tmux_session_exists repo-agent-q2
  [[ "$(tmux_pane0_cmd repo-agent-q2)" == *"$OPENER"* ]]
}
