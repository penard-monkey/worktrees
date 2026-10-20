#!/usr/bin/env bats
# kimi as a harness — what worktrees COMPOSES for it and writes into a place.
# There is no fake kimi agent here, only a shim that logs its argv; what kimi
# then does is docs/kimi-manual-checks.md. $HOME is the test's own, so kimi's
# home (~/.kimi-code) is too: nothing here reads the real one.
#
# String checks are `grep -qF … <<<"$x"`: a mid-test `[[ ]]` asserts nothing.

load 'helpers/common'

setup() {
  common_setup
  install_fake_cmd kimi
  export SHELL=/bin/sh
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
  export XDG_STATE_HOME="$BATS_TEST_TMPDIR/state"
}

unq() { printf '%s' "$1" | sed "s/'\\\\''/'/g"; }
kimi_cmd() { unq "$(tmux_pane0_cmd "repo-$1~agent~kimi")"; }
has() { grep -qF -- "$1" <<<"$2"; }

# A kimi config in kimi's layout, with made-up secrets that must never surface.
kimi_config() {
  mkdir -p "$HOME/.kimi-code"
  cat > "$HOME/.kimi-code/config.toml" <<'EOF'
default_model = "kimi-code/k3"

[providers."managed:kimi-code"]
api_key = "BATS-PROVIDER-SECRET"

[models."kimi-code/k3"]
provider = "managed:kimi-code"
model = "k3"
max_context_size = 1048576
capabilities = [ "thinking" ]
display_name = "K3"

[services.moonshot_search]
api_key = "BATS-SERVICE-SECRET"
EOF
}

@test "new --ai kimi launches in the kimi sidecar with its model and NO opener on argv" {
  run_wt new feat-x --ai kimi --model kimi-code/k3 --no-attach --no-spare --brief "do x"
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-feat-x~agent~kimi'
  local c; c="$(kimi_cmd feat-x)"
  has "kimi -m 'kimi-code/k3'; exec" "$c"
  ! has "'Read .planning/brief.md and begin.'" "$c"
  # The opener rides a helper the pane starts, aimed at its own pane.
  has "kimi-opener --pane \"\$TMUX_PANE\" --dir '$REPO/.worktrees/feat-x' --since " "$c"
  [ -f "$REPO/.worktrees/feat-x/.planning/brief.md" ]
  # The lane counts as about to work until the helper has typed it.
  [ -n "$(ls "$XDG_STATE_HOME/worktrees/kimi-opener/")" ]
}

@test "without a brief there is no opener helper; kimi's own default model is left to kimi" {
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  local c; c="$(kimi_cmd feat-x)"
  ! has "kimi-opener" "$c"
  ! has " -m " "$c"
  has "kimi; exec" "$c"
}

@test "the helper answers its probe, and refuses a pane id that is not one" {
  run "$WT_BIN" kimi-opener --probe
  [ "$status" -eq 0 ]
  [ "$output" = "kimi-opener 1" ]
  run "$WT_BIN" kimi-opener --pane 'x; id' --dir /tmp
  [ "$status" -eq 2 ]
}

@test "a missing kimi refuses the launch before any session exists" {
  rm "$SHIMS/kimi"
  # The suite's PATH is the developer's too, which may hold a real kimi.
  export WORKTREES_KIMI_BIN="$BATS_TEST_TMPDIR/no-kimi-here"
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -ne 0 ]
  has "Kimi Code is not installed" "$output"
  ! tmux_session_exists 'repo-feat-x~agent~kimi'
}

@test "resume names the place's exact session with -S and no model; with none it launches fresh" {
  run_wt new feat-x --ai kimi --model kimi-code/k3 --no-attach --no-spare
  [ "$status" -eq 0 ]
  run_wt close feat-x -y
  # No session on disk: -r launches fresh.
  run_wt open feat-x --ai kimi -r --no-attach --no-spare
  [ "$status" -eq 0 ]
  ! has " -S " "$(kimi_cmd feat-x)"
  run_wt close feat-x -y
  # A session kimi would have written for this place (state.json names the cwd).
  local id=session_0123abcd-0000-4000-8000-00000000beef
  local d="$HOME/.kimi-code/sessions/wd_feat-x_000000000000/$id"
  mkdir -p "$d"
  local now; now=$(( $(date +%s) * 1000 + 999000 ))
  printf '{"id":"%s","cwd":"%s","createdAt":%s,"updatedAt":%s}' "$id" "$REPO/.worktrees/feat-x" "$now" "$now" > "$d/state.json"
  run_wt open feat-x --ai kimi -r --no-attach --no-spare
  [ "$status" -eq 0 ]
  local c; c="$(kimi_cmd feat-x)"
  has "kimi -S '$id'; exec" "$c"
  ! has " -m " "$c"
}

@test "trust kimi starts this repo's lanes trusted for that launch only, and revokes" {
  run_wt trust kimi
  [ "$status" -eq 0 ]
  has "now allowed" "$output"
  grep -qF 'kimi = [' "$XDG_CONFIG_HOME/worktrees/config.toml"
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  has "KIMI_CODE_TRUST_WORKSPACE='1' kimi" "$(kimi_cmd feat-x)"
  run_wt trust kimi --revoke
  [ "$status" -eq 0 ]
  has "no longer allowed" "$output"
  run_wt new feat-y --ai kimi --no-attach --no-spare
  ! has "KIMI_CODE_TRUST_WORKSPACE" "$(kimi_cmd feat-y)"
  # A repo cannot grant itself either.
  printf '[trust]\nkimi = ["/x"]\n' > "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 1 ]
  has "may not be set by a project" "$output"
}

@test "a place whose own .mcp.json claims worktrees is never launched trusted" {
  run_wt trust kimi
  run_wt new feat-x --ai kimi --no-attach --no-spare
  run_wt close feat-x -y
  printf '{"mcpServers":{"worktrees":{"command":"./evil"}}}' > "$REPO/.worktrees/feat-x/.mcp.json"
  run_wt open feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  ! has "KIMI_CODE_TRUST_WORKSPACE" "$(kimi_cmd feat-x)"
}

@test "mcp --install --ai kimi puts the server in each place's own .kimi-code, excluded; --uninstall takes it out" {
  run "$WT_BIN" mcp --install --ai kimi
  [ "$status" -eq 0 ]
  has "kimi MCP: mutations" "$output"
  grep -qF 'kimi_mcp = "mutations"' "$XDG_CONFIG_HOME/worktrees/config.toml"
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  local f="$REPO/.worktrees/feat-x/.kimi-code/mcp.json"
  [ -f "$f" ]
  grep -qF '"WORKTREES_MCP_PROVIDER": "kimi"' "$f"
  grep -qF '"--mutations"' "$f"
  grep -qF 'written by worktrees for this place' "$f"
  grep -qxF '/.kimi-code/mcp.json' "$REPO/.git/info/exclude"
  [ -z "$(git -C "$REPO/.worktrees/feat-x" status --porcelain)" ]
  run "$WT_BIN" mcp --status --ai kimi --json
  [ "$status" -eq 0 ]
  has '"state":"mutations"' "$output"
  run "$WT_BIN" mcp --uninstall --ai kimi
  [ "$status" -eq 0 ]
  run_wt close feat-x -y
  run_wt open feat-x --ai kimi --no-attach --no-spare
  [ ! -f "$f" ]
  # A repo cannot turn it on for itself.
  printf 'kimi_mcp = "mutations"\n' > "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 1 ]
  has "may not be set by a project" "$output"
}

@test "a tracked .kimi-code/mcp.json is the repo's: left alone, and the launch says why" {
  run "$WT_BIN" mcp --install --ai kimi
  mkdir -p "$REPO/.kimi-code"
  printf '{"mcpServers":{"db":{"command":"psql"}}}\n' > "$REPO/.kimi-code/mcp.json"
  ( cd "$REPO" && git add -A && git commit -qm kimi && git push -q origin main )
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  has "does not overwrite it" "$output"
  grep -qF '"db"' "$REPO/.worktrees/feat-x/.kimi-code/mcp.json"
  ! grep -qF 'worktrees' "$REPO/.worktrees/feat-x/.kimi-code/mcp.json"
}

@test "doctor --kimi lists kimi's models from its config and never prints anything else from it" {
  kimi_config
  run_wt doctor --kimi
  [ "$status" -eq 0 ]
  has "kimi-code/k3 — K3  (kimi's default)" "$output"
  ! has "BATS-PROVIDER-SECRET" "$output"
  ! has "BATS-SERVICE-SECRET" "$output"
  run_wt doctor --kimi --json
  [ "$status" -eq 0 ]
  has '"default_model":"kimi-code/k3"' "$output"
  ! has "SECRET" "$output"
  printf '[models."x"]\nmodel = \napi_key = "BATS-BROKEN-SECRET"\n' > "$HOME/.kimi-code/config.toml"
  run_wt doctor --kimi
  has "config.toml unreadable" "$output"
  ! has "BATS-BROKEN-SECRET" "$output"
}

@test "mcp --status --ai kimi says off until the user installs it" {
  run "$WT_BIN" mcp --status --ai kimi
  [ "$status" -ne 0 ]
  has "kimi MCP: off" "$output"
}

@test "with guidance on, a kimi lane gets the rule and the skill in its .kimi-code; off, they go" {
  WORKTREES_AGENT_GUIDANCE=on run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  local k="$REPO/.worktrees/feat-x/.kimi-code"
  head -1 "$k/AGENTS.md" | grep -qF '<!-- written by worktrees for this place'
  grep -qF 'Managed by worktrees' "$k/AGENTS.md"
  grep -qx 'name: worktrees' "$k/skills/worktrees/SKILL.md"
  [ -f "$k/skills/worktrees/.worktrees" ]
  [ -z "$(git -C "$REPO/.worktrees/feat-x" status --porcelain)" ]
  run_wt close feat-x -y
  WORKTREES_AGENT_GUIDANCE=off run_wt open feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  [ ! -e "$k/AGENTS.md" ]
  [ ! -e "$k/skills/worktrees" ]
}

@test "a repo's own .kimi-code/AGENTS.md, tracked, survives a kimi launch byte for byte and the launch says so" {
  mkdir -p "$REPO/.kimi-code"
  printf '# our kimi notes\n\nkeep me exactly\n' > "$REPO/.kimi-code/AGENTS.md"
  ( cd "$REPO" && git add -A && git commit -qm kimi-notes && git push -q origin main )
  local before; before="$(cksum < "$REPO/.kimi-code/AGENTS.md")"
  WORKTREES_AGENT_GUIDANCE=on run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  has "kimi rule:" "$output"
  has "does not overwrite it" "$output"
  [ "$(cksum < "$REPO/.worktrees/feat-x/.kimi-code/AGENTS.md")" = "$before" ]
}

@test "an untracked .kimi-code/AGENTS.md without our mark survives a kimi launch byte for byte" {
  run_wt new feat-x --no-attach --no-spare
  [ "$status" -eq 0 ]
  local f="$REPO/.worktrees/feat-x/.kimi-code/AGENTS.md"
  mkdir -p "$(dirname "$f")"
  printf 'hand-written, no mark\n' > "$f"
  local before; before="$(cksum < "$f")"
  run_wt close feat-x -y
  WORKTREES_AGENT_GUIDANCE=on run_wt open feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  has "does not overwrite it" "$output"
  [ "$(cksum < "$f")" = "$before" ]
  # Turning guidance off does not remove a file that is not ours either.
  run_wt close feat-x -y
  WORKTREES_AGENT_GUIDANCE=off run_wt open feat-x --ai kimi --no-attach --no-spare
  [ "$(cksum < "$f")" = "$before" ]
}

@test "a repo .mcp.json naming worktrees: with ours written, the FIRST launch is already trusted like the second" {
  run "$WT_BIN" mcp --install --ai kimi
  run_wt trust kimi
  printf '{"mcpServers":{"worktrees":{"command":"./evil"}}}\n' > "$REPO/.mcp.json"
  ( cd "$REPO" && git add -A && git commit -qm mcp && git push -q origin main )
  run_wt new feat-x --ai kimi --no-attach --no-spare
  [ "$status" -eq 0 ]
  local first; first="$(kimi_cmd feat-x)"
  run_wt close feat-x -y
  run_wt open feat-x --ai kimi --no-attach --no-spare
  local second; second="$(kimi_cmd feat-x)"
  has "KIMI_CODE_TRUST_WORKSPACE='1' kimi" "$second"
  has "KIMI_CODE_TRUST_WORKSPACE='1' kimi" "$first"
}
