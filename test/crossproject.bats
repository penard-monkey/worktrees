#!/usr/bin/env bats
# Cross-project reach over the real stdio transport, with two scratch repos
# (cross-project proposal P1a). HOME is the test's own (common_setup), so the
# registry and config.toml here are throwaway ones.

load 'helpers/common'

setup() {
  common_setup
  BETA="$BATS_TEST_TMPDIR/beta"
  git init -q "$BETA"
  git -C "$BETA" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
  git -C "$BETA" worktree add -q "$BETA/.worktrees/lane" -b lane
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$BETA"
  run_wt -C "$BATS_TEST_TMPDIR" projects rename repo alpha
}

reach() {   # user-tier setting
  mkdir -p "$HOME/.config/worktrees"
  printf 'cross_project = "%s"\n' "$1" > "$HOME/.config/worktrees/config.toml"
}

# srv "<extra args>" <tool> '<json args>' — one tools/call from inside $REPO,
# printing the call's result object.
srv() {
  local extra="$1" tool="$2" args="$3"
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' \
    "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"$tool\",\"arguments\":$args}}" \
    > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp $extra < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
}
frame() { printf '%s\n' "$output" | jq -c "select(.id == $1)"; }
tools() { frame 2 | jq -r '[.result.tools[].name] | join(",")'; }
result_text() { frame 3 | jq -r '.result.content[0].text'; }
is_error() { frame 3 | jq -r '.result.isError'; }

@test "reach is off by default: no list_projects, and a foreign address says how to turn it on" {
  srv "" place_status '{"slug":"beta:lane"}'
  [ "$status" -eq 0 ]
  [[ "$(tools)" != *list_projects* ]]
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *'cross_project = \"read\"'* || "$(result_text)" == *'cross_project = "read"'* ]]
}

@test "with reach on, place_status reads a place in another registered project" {
  reach read
  srv "" place_status '{"slug":"beta:lane"}'
  [[ "$(tools)" == *list_projects* ]]
  [ "$(is_error)" = false ]
  [ "$(result_text | jq -r .project)" = beta ]
  [ "$(result_text | jq -r .address)" = beta:lane ]
  [ "$(result_text | jq -r .branch)" = lane ]
  # No absolute path below full: not `path`, not anywhere under beta's root.
  [ "$(result_text | jq 'has("path")')" = false ]
  [[ "$(result_text)" != *"$(cd "$BETA" && pwd -P)"* ]]
  reach full
  srv "" place_status '{"slug":"beta:lane"}'
  [ "$(result_text | jq -r .path)" = "$(cd "$BETA" && pwd -P)/.worktrees/lane" ]
}

@test "a registered repo nested in a private one, its .git gone, never resolves into the outer one" {
  reach full
  run_wt -C "$BATS_TEST_TMPDIR" projects private beta on
  local inner="$BETA/vendor/inner"
  mkdir -p "$inner"
  git init -q "$inner"
  git -C "$inner" -c user.email=t@t -c user.name=t commit -q --allow-empty -m init
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$inner"
  rm -rf "$inner/.git"
  srv "" place_status '{"slug":"inner:lane"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *"no longer a repository's main checkout"* ]]
  [[ "$(result_text)" != *"$BETA"* ]]
}

@test "list_projects lists both, with addresses and no root below full" {
  reach read
  srv "" list_projects '{}'
  [ "$(is_error)" = false ]
  [ "$(result_text | jq -r '[.projects[].project] | join(",")')" = alpha,beta ]
  [ "$(result_text | jq '[.projects[] | select(has("root"))] | length')" -eq 0 ]
  [ "$(result_text | jq -r '.projects[1].places | map(.address) | join(",")')" = 'beta:(main),beta:lane' ]
  reach full
  srv "" list_projects '{}'
  [ "$(result_text | jq -r '.projects[1].root')" = "$(cd "$BETA" && pwd -P)" ]
}

@test "a private project is a bare row and cannot be addressed" {
  reach read
  run_wt -C "$BATS_TEST_TMPDIR" projects private beta on
  srv "" list_projects '{}'
  [ "$(result_text | jq -c '.projects[1]')" = '{"project":"beta","private":true}' ]
  srv "" place_status '{"slug":"beta:lane"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *private* ]]
}

@test "a profile's --cross-project narrows the user's setting" {
  reach full
  srv "--cross-project off" place_status '{"slug":"beta:lane"}'
  [[ "$(tools)" != *list_projects* ]]
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *profile* ]]
  srv "--cross-project bogus" place_status '{"slug":"beta:lane"}'
  [ "$(is_error)" = true ]
}

@test "the repo cannot turn reach on, and writes other than a message never cross" {
  write_project_config 'cross_project = "full"'
  srv "--mutations" place_status '{"slug":"beta:lane"}'
  [ "$(is_error)" = true ]
  reach read
  srv "--mutations" set_note '{"slug":"beta:lane","note":"hi"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *"another project"* ]]
  [ ! -s "$BETA/.worktrees.places.json" ] || ! grep -q '"note"' "$BETA/.worktrees.places.json"
}

# srv_in <dir> "<extra>" <tool> '<json args>' — like srv, from any directory.
srv_in() {
  local dir="$1"; shift
  local saved="$REPO"; REPO="$dir"; srv "$@"; REPO="$saved"
}

@test "a report round-trips A→B→A: filed in the recipient's log, from qualified, reply threaded" {
  reach read
  srv_in "$REPO" "" report '{"to":"beta:(main)","text":"which branch is the fix on?"}'
  [ "$(is_error)" = false ]
  [ "$(result_text | jq -r .from)" = 'alpha:(main)' ]
  # In BETA's log: `to` is its own bare slug.
  srv_in "$BETA" "" messages '{}'
  [ "$(result_text | jq -r '.messages[0].from')" = 'alpha:(main)' ]
  [ "$(result_text | jq -r '.messages[0].to')" = '(main)' ]
  local qid; qid="$(result_text | jq -r '.messages[0].id')"
  [ ! -d "$REPO/.git/worktrees-messages" ]
  # beta answers to the `from` it received; the question sits in beta's log.
  srv_in "$BETA" "" report "{\"to\":\"alpha:(main)\",\"text\":\"feat-x\",\"reply_to\":\"$qid\"}"
  [ "$(is_error)" = false ]
  srv_in "$REPO" "" wait '{"until":"message","slug":"beta:(main)","timeout_s":0}'
  [ "$(result_text | jq -r .event)" = message ]
  [ "$(result_text | jq -r '.messages[0].from')" = 'beta:(main)' ]
  [ "$(result_text | jq -r '.messages[0].reply_to')" = "$qid" ]
}

@test "a cross-project report is refused by name while reach is off or the target is private" {
  srv "" report '{"to":"beta:(main)","text":"hi"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *cross_project* ]]
  reach read
  run_wt -C "$BATS_TEST_TMPDIR" projects private beta on
  srv "" report '{"to":"beta:(main)","text":"hi"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *private* ]]
  [ ! -d "$BETA/.git/worktrees-messages" ]
}

@test "a registered repo is managed, so it gets the rule" {
  srv "" list_places '{}'
  frame 1 | jq -r .result.instructions > "$BATS_TEST_TMPDIR/instr"
  grep -q '^Managed by worktrees' "$BATS_TEST_TMPDIR/instr"
  [ "$(result_text | jq -r '.places[0].project')" = alpha ]
}

# ── cross-project mutations (proposal P3): reach `full` AND --mutations ──────

@test "a foreign mutation needs full AND --mutations; then it lands in the target's sidecar" {
  reach read
  srv "--mutations" set_note '{"slug":"beta:lane","note":"from alpha"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *'cross_project = "full"'* ]]
  reach full
  srv "" set_note '{"slug":"beta:lane","note":"from alpha"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *--mutations* ]]
  [ ! -e "$BETA/.worktrees.places.json" ] || ! grep -q 'from alpha' "$BETA/.worktrees.places.json"
  srv "--mutations" set_note '{"slug":"beta:lane","note":"from alpha"}'
  [ "$(is_error)" = false ]
  grep -q 'from alpha' "$BETA/.worktrees.places.json"
  [ ! -e "$REPO/.worktrees.places.json" ] || ! grep -q 'from alpha' "$REPO/.worktrees.places.json"
}

@test "create_worktree project: creates in the TARGET project, its session named by that project; close_session ends it" {
  reach full
  srv "--mutations" create_worktree '{"branch":"agent-x","project":"beta"}'
  [ "$(is_error)" = false ]
  [ -d "$BETA/.worktrees/agent-x" ]
  [ ! -d "$REPO/.worktrees/agent-x" ]
  tmux_session_exists beta-agent-x
  srv "--mutations" close_session '{"slug":"beta:agent-x"}'
  [ "$(is_error)" = false ]
  ! tmux_session_exists beta-agent-x
  [ -d "$BETA/.worktrees/agent-x" ]
}

@test "create_worktree project is refused at read, for a private project, and for an unknown one" {
  reach read
  srv "--mutations" create_worktree '{"branch":"agent-y","project":"beta"}'
  [ "$(is_error)" = true ]
  reach full
  run_wt -C "$BATS_TEST_TMPDIR" projects private beta on
  srv "--mutations" create_worktree '{"branch":"agent-y","project":"beta"}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *private* ]]
  srv "--mutations" create_worktree '{"branch":"agent-y","project":"nobody"}'
  [ "$(is_error)" = true ]
  [ ! -d "$BETA/.worktrees/agent-y" ]
}

@test "remove_worktree never acts on another project's place, even at full with confirm" {
  reach full
  srv "--mutations" remove_worktree '{"slug":"beta:lane","confirm":true}'
  [ "$(is_error)" = true ]
  [[ "$(result_text)" == *"never acts on another project"* ]]
  [ -d "$BETA/.worktrees/lane" ]
}

@test "a brief written into another project carries its provenance first" {
  reach full
  srv "--mutations" create_worktree '{"branch":"agent-b","project":"beta","brief":"# Task\n- do x"}'
  [ "$(is_error)" = false ]
  local brief="$BETA/.worktrees/agent-b/.planning/brief.md"
  head -1 "$brief" | grep -q '^> \*\*Provenance\.\*\* This brief was written by an AI agent in the project '"'"'alpha'"'"' (place '"'"'(main)'"'"')'
  grep -q "not by the user directly" "$brief"
  grep -q '^# Task' "$brief"
  # The same branch again: the place is reused and its brief REPLACED — said.
  srv "--mutations" create_worktree '{"branch":"agent-b","project":"beta","brief":"# Second"}'
  [ "$(is_error)" = false ]
  [[ "$(frame 3 | jq -r '.result.content[].text')" == *"REPLACED"* ]]
  grep -q '^# Second' "$brief"
  # A brief inside this project is the agent's own business: no header.
  srv "--mutations" create_worktree '{"branch":"agent-c","brief":"# Local"}'
  [ "$(head -1 "$REPO/.worktrees/agent-c/.planning/brief.md")" = "# Local" ]
}

@test "send into beta:<slug> types into BETA's session, labelled from alpha, and files the copy in beta's log" {
  reach full
  local lane; lane="$(cd "$BETA/.worktrees/lane" && pwd -P)"
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"; mkdir -p "$CODEX_HOME"
  printf 'cwd=%s\n' "$lane" > "$TMUX_STATE/beta-lane~agent~codex"
  printf 'codex' > "$TMUX_STATE/beta-lane~agent~codex.cmd"
  printf '%s' 'beta-lane~agent~codex' > "$TMUX_STATE/.pane-%0"
  local fixtures="$BATS_TEST_DIRNAME/../crates/worktrees-core/tests/fixtures/codex-send"
  cp "$fixtures/typed.txt" "$TMUX_STATE/beta-lane~agent~codex.screen"
  cp "$fixtures/empty.txt" "$TMUX_STATE/.after-enter"
  srv "--mutations" send '{"slug":"beta:lane","text":"hi"}'
  [ "$(is_error)" = false ]
  [ "$(result_text | jq -r .session)" = 'beta-lane~agent~codex' ]
  grep -q -F 'tmux send-keys -t %0 -l -- [worktrees: message from place "alpha:(main)", not from the user] hi' "$TMUX_LOG"
  grep -q '"from":"alpha:(main)"' "$BETA"/.git/worktrees-messages/*.json
  [ ! -d "$REPO/.git/worktrees-messages" ]
}

# ── foreign @-mentions (proposal P4) ─────────────────────────────────────────

# rpc '<method>' '<params json>' — initialize, then one request (id 3).
rpc() {
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}' \
    '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
    "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"$1\",\"params\":$2}" \
    > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
}

@test "with reach on, the @ list carries another project's places, named by address, with no path" {
  rpc resources/list '{}'
  [ "$(frame 3 | jq '[.result.resources[].uri | select(startswith("place://beta/"))] | length')" -eq 0 ]
  reach read
  rpc resources/list '{}'
  [ "$(frame 3 | jq -r '.result.resources[] | select(.uri == "place://beta/lane") | .name')" = beta:lane ]
  [[ "$(frame 3)" != *"$(cd "$BETA" && pwd -P)"* ]]
  rpc resources/read '{"uri":"place://beta/lane"}'
  [ "$(frame 3 | jq -r '.result.contents[0].text' | jq -r .address)" = beta:lane ]
  [ "$(frame 3 | jq -r '.result.contents[0].text' | jq '.place | has("path")')" = false ]
  run_wt -C "$BATS_TEST_TMPDIR" projects private beta on
  rpc resources/list '{}'
  [ "$(frame 3 | jq '[.result.resources[].uri | select(startswith("place://beta/"))] | length')" -eq 0 ]
}
