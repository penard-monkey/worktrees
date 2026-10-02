#!/usr/bin/env bats
# Agent guidance, phase 2 (docs/proposals/agent-guidance.md §4.2, §5.2):
# per-launch delivery, the settings file, `guide`, and the PreToolUse guard.

load helpers/common

setup() {
  common_setup
  unset WORKTREES_AGENT_GUIDANCE
}

agent_dir() { ls -d "$HOME"/.local/share/worktrees/agent/*/ | head -n1; }

@test "guidance: a claude lane gets the worktrees plugin; off in settings, it does not" {
  WORKTREES_AI_CMD=claude run_wt new feat-g
  [ "$status" -eq 0 ]
  cmd="$(tmux_pane0_cmd repo-feat-g)"
  # The pane line is `sh -ic '…'`, so the plugin path's own quotes arrive as '\''.
  [[ "$cmd" == *"claude --plugin-dir "*"$HOME/.local/share/worktrees/agent/"*"/claude"*" --name "* ]]
  [[ "$cmd" != *"claude-guard"* ]]
  dir="$(agent_dir)"
  grep -q '^name: worktrees$' "$dir/claude/skills/worktrees/SKILL.md"
  [ ! -e "$dir/claude/hooks" ]

  mkdir -p "$HOME/.config/worktrees"
  echo '{"enabled": false}' > "$HOME/.config/worktrees/agent-guidance.json"
  WORKTREES_AI_CMD=claude run_wt new feat-h
  [[ "$(tmux_pane0_cmd repo-feat-h)" != *"--plugin-dir"* ]]
  WORKTREES_AGENT_GUIDANCE=on WORKTREES_AI_CMD=claude run_wt new feat-i
  [[ "$(tmux_pane0_cmd repo-feat-i)" == *"--plugin-dir"* ]]
}

@test "guidance: the guard is off by default and, when on, hooks worktrees itself" {
  mkdir -p "$HOME/.config/worktrees"
  echo '{"guard": true}' > "$HOME/.config/worktrees/agent-guidance.json"
  WORKTREES_AI_CMD=claude run_wt new feat-guard
  [[ "$(tmux_pane0_cmd repo-feat-guard)" == *"/claude-guard"*" --name "* ]]
  hooks="$(agent_dir)/claude-guard/hooks/hooks.json"
  grep -q '"matcher": "Bash"' "$hooks"
  grep -q "guard pretooluse" "$hooks"
}

@test "guidance: codex gets developer_instructions only when the user has none" {
  cat > "$SHIMS/codex" <<SH
#!/usr/bin/env bash
if [ "\$1 \$2" = "debug prompt-input" ]; then
  echo "\$@" >> "$BATS_TEST_TMPDIR/probe.log"
  cat "$BATS_TEST_TMPDIR/prompt-input.json"; exit 0
fi
echo "\$@" >> "$BATS_TEST_TMPDIR/codex.log"
SH
  chmod +x "$SHIMS/codex"
  echo '[{"type":"message","role":"developer","content":[{"type":"input_text","text":"<skills_instructions> x"}]}]' > "$BATS_TEST_TMPDIR/prompt-input.json"
  # The probe STARTS Codex's MCP servers, so it switches off each one it can see.
  mkdir -p "$HOME/.codex"
  printf '[mcp_servers.worktrees]\ncommand = "worktrees"\n\n[mcp_servers."odd.name"]\ncommand = "x"\n' > "$HOME/.codex/config.toml"
  WORKTREES_AI_CMD=codex run_wt new feat-cx
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd 'repo-feat-cx~agent~codex')" == *"-c "*"developer_instructions="*"Managed by worktrees:"* ]]

  echo '[{"type":"message","role":"developer","content":[{"type":"input_text","text":"Always answer in French."}]}]' > "$BATS_TEST_TMPDIR/prompt-input.json"
  WORKTREES_AI_CMD=codex run_wt new feat-cy
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd 'repo-feat-cy~agent~codex')" != *"developer_instructions"* ]]
  grep -q 'mcp_servers.worktrees.enabled=false' "$BATS_TEST_TMPDIR/probe.log"
  grep -q 'mcp_servers."odd.name".enabled=false' "$BATS_TEST_TMPDIR/probe.log"
  probes="$(wc -l < "$BATS_TEST_TMPDIR/probe.log")"
  [ "$probes" -eq 2 ]
  # Status and doctor report the last launch's answer; they never probe.
  run_wt guide --status
  [[ "$output" == *"Codex"*"skipped: you set your own developer_instructions"* ]]
  run_wt doctor --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"severity":"info","code":"guidance-skipped"'* ]]
  # Turned off on purpose: nothing to report.
  WORKTREES_AGENT_GUIDANCE=off run_wt doctor --json
  [[ "$output" != *"guidance-skipped"* ]]
  [ "$(wc -l < "$BATS_TEST_TMPDIR/probe.log")" -eq "$probes" ]
  # Reopening the same place reuses the cached answer while nothing changed.
  run_wt close feat-cy
  ! tmux_session_exists 'repo-feat-cy~agent~codex'
  WORKTREES_AI_CMD=codex run_wt open feat-cy
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-feat-cy~agent~codex'
  [ "$(wc -l < "$BATS_TEST_TMPDIR/probe.log")" -eq "$probes" ]
}

@test "guidance: the guard refuses a new branch in (main) and a raw worktree add, nothing else" {
  run_wt new feat-x            # one place: the repo is managed
  mkdir -p "$HOME/.config/worktrees"
  echo '{"guard": true}' > "$HOME/.config/worktrees/agent-guidance.json"
  hook() { printf '%s' "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"$1\"},\"cwd\":\"$2\"}" | "$WT_BIN" guard pretooluse; }
  run hook "git checkout -b fix" "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"permissionDecision":"deny"'*"create_worktree"* ]]
  run hook "git checkout -b fix" "$REPO/.worktrees/feat-x"
  [ "$status" -eq 0 ] && [ -z "$output" ]
  run hook "git worktree add ../scratch -b y" "$REPO/.worktrees/feat-x"
  [[ "$output" == *'"permissionDecision":"deny"'* ]]
  run hook "git status" "$REPO"
  [ -z "$output" ]
  # The toggle is LIVE: the hook reads the settings on every call, so turning
  # the guard off allows the very next command, with no relaunch.
  echo '{"guard": false}' > "$HOME/.config/worktrees/agent-guidance.json"
  run hook "git checkout -b fix" "$REPO"
  [ "$status" -eq 0 ] && [ -z "$output" ]
  echo '{"guard": true}' > "$HOME/.config/worktrees/agent-guidance.json"
  # Not a repository, or garbage on stdin: allow, silently, exit 0.
  run hook "git checkout -b fix" "$BATS_TEST_TMPDIR"
  [ "$status" -eq 0 ] && [ -z "$output" ]
  run bash -c 'echo "not json" | "$1" guard pretooluse' _ "$WT_BIN"
  [ "$status" -eq 0 ] && [ -z "$output" ]
}

@test "guidance: an unmanaged repo's guard allows everything" {
  run bash -c 'printf "%s" "{\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"git checkout -b fix\"},\"cwd\":\"$1\"}" | "$2" guard pretooluse' _ "$REPO" "$WT_BIN"
  [ "$status" -eq 0 ] && [ -z "$output" ]
}

@test "guidance: guide prints the skill anywhere; --rules the one line" {
  run_wt -C "$BATS_TEST_TMPDIR" guide
  [ "$status" -eq 0 ]
  [[ "${lines[1]}" == "name: worktrees" ]]
  run_wt -C "$BATS_TEST_TMPDIR" guide --rules
  [[ "$output" == "Managed by worktrees:"*"worktrees guide"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" guide --status --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"settings":{"enabled":true,"guard":false}'* ]]
  run_wt guide --bogus
  [ "$status" -eq 1 ]
}

@test "guidance: an edited skill is what guide prints and lanes get; --default is the shipped one" {
  mkdir -p "$HOME/.config/worktrees/guidance"
  run_wt -C "$BATS_TEST_TMPDIR" guide --default
  shipped="$output"
  printf -- '---\nname: worktrees\ndescription: My own words.\n---\n\nEDITED BODY\n' > "$HOME/.config/worktrees/guidance/SKILL.md"
  run_wt -C "$BATS_TEST_TMPDIR" guide
  [ "$status" -eq 0 ]
  [[ "$output" == *"EDITED BODY"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" guide --default
  [ "$output" = "$shipped" ]
  # No base record: the edit is used, and the status says nobody knows what it forked.
  run_wt -C "$BATS_TEST_TMPDIR" guide --status
  [[ "$output" == *"skill:  your edit ("*"based on is unknown"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" guide --status --json
  [[ "$output" == *'"skill_edit":{"text":"---\nname: worktrees'*'"stale":true'* ]]
  WORKTREES_AI_CMD=claude run_wt new feat-edit
  [ "$status" -eq 0 ]
  # The lane's --plugin-dir is the directory holding the edit.
  plugin="$(tmux_pane0_cmd repo-feat-edit | grep -o "$HOME/.local/share/worktrees/agent/[0-9a-f]*/claude")"
  grep -q 'EDITED BODY' "$plugin/skills/worktrees/SKILL.md"

  # A broken edit is never handed to an agent: guide falls back and says why.
  printf 'no frontmatter\n' > "$HOME/.config/worktrees/guidance/SKILL.md"
  run_wt -C "$BATS_TEST_TMPDIR" guide
  [ "$output" = "$shipped" ]
  run_wt -C "$BATS_TEST_TMPDIR" guide --status
  [[ "$output" == *"skill:  the default — your edit at "*"is not used: it must start with"* ]]
}
