#!/usr/bin/env bats
# AI profiles, exercised end to end through the compiled binary.
#
# These are the ONLY tests that see a profiled launch: the adapter is
# claude-only, and the rest of the suite runs with WORKTREES_AI_CMD=fake-ai, so
# every other pane0 assertion deliberately describes the unprofiled path.

load 'helpers/common'

setup() { common_setup; }

# Point the binary's profile lookup at this test's sandbox and declare one
# profile as the global default. common_setup unsets both XDG vars, so setting
# them here cannot leak into the developer's real dirs.
use_profile() {   # $1 = extra JSON for the profile body (may be empty)
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/cfg"
  export XDG_DATA_HOME="$BATS_TEST_TMPDIR/data"
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  cat > "$XDG_CONFIG_HOME/worktrees/profiles.json" <<JSON
{
  "version": 1,
  "default_id": "work",
  "profiles": {
    "work": { "id": "work", "name": "Work", "rules": "be terse" ${1:+, $1} }
  },
  "assignments": {}
}
JSON
  # the adapter only engages for claude
  export WORKTREES_AI_CMD=claude
  install_fake_cmd claude
}

@test "a profiled launch swaps the config dir and carries the profile's flags" {
  use_profile
  run_wt new feat-x
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"

  # the swap, and the flags that carry what a swapped dir cannot
  [[ "$p0" == *"CLAUDE_CONFIG_DIR="*"/worktrees/profiles/work"* ]]
  [[ "$p0" == *"--append-system-prompt-file"*"rules.md"* ]]
  [[ "$p0" == *"--mcp-config"*"mcp.json"* ]]
  [[ "$p0" == *"--strict-mcp-config"* ]]
}

@test "the env assignment precedes the command so tmux adoption still sees claude" {
  use_profile
  run_wt new feat-x
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  # ordered glob: CLAUDE_CONFIG_DIR must come BEFORE the program. Reversing the
  # two would leave pane_current_command as an assignment, silently degrading
  # session adoption and disabling auto-resume.
  [[ "$p0" == *"CLAUDE_CONFIG_DIR="*" claude"* ]]
  [[ "$p0" != *"claude"*"CLAUDE_CONFIG_DIR="* ]]
}

@test "the resume arg stays adjacent to the ai word, profile flags follow it" {
  use_profile
  # --no-tmux so `open` is what creates the session; otherwise it just attaches
  # to the one `new` made and pane0 still describes that launch.
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  run_wt open feat-x -r
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *"claude -r"* ]]
  [[ "$p0" == *"claude -r"*"--append-system-prompt-file"* ]]
}

@test "materialization actually writes the profile dir" {
  use_profile
  run_wt new feat-x
  [ "$status" -eq 0 ]
  local d="$XDG_DATA_HOME/worktrees/profiles/work"
  [ -f "$d/rules.md" ]
  [ -f "$d/mcp.json" ]
  [ -f "$d/.claude.json" ]
  grep -q 'be terse' "$d/rules.md"
}

@test "a profile pinning a model passes it quoted" {
  use_profile '"model": "opus"'
  run_wt new feat-x
  [ "$status" -eq 0 ]
  # pane0 carries tmux's own quoting layer on top of the shell one, so the
  # single quotes appear escaped here. That the value is QUOTED at all is pinned
  # by the unit test a_hostile_model_string_cannot_break_out_of_the_shell; this
  # only proves the flag reaches the launch line.
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *--model*opus* ]]
}

@test "WORKTREES_PROFILE=none opts out of an otherwise-default profile" {
  use_profile
  WORKTREES_PROFILE=none run_wt new feat-x
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" != *"CLAUDE_CONFIG_DIR"* ]]
  [[ "$p0" == *claude* ]]
}

@test "a non-claude ai_cmd is launched unmodified even with a profile set" {
  use_profile
  export WORKTREES_AI_CMD=fake-ai
  run_wt new feat-x
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" == *fake-ai* ]]
  [[ "$p0" != *"CLAUDE_CONFIG_DIR"* ]]
  [[ "$p0" != *"--mcp-config"* ]]
}

@test "no profiles.json at all leaves the launch exactly as it was" {
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/cfg"
  export XDG_DATA_HOME="$BATS_TEST_TMPDIR/data"
  export WORKTREES_AI_CMD=claude
  install_fake_cmd claude
  run_wt new feat-x
  [ "$status" -eq 0 ]
  local p0; p0="$(tmux_pane0_cmd repo-feat-x)"
  [[ "$p0" != *"CLAUDE_CONFIG_DIR"* ]]
  [[ "$p0" == *claude* ]]
}
