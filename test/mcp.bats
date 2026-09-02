#!/usr/bin/env bats
# `worktrees mcp` — the stdio MCP server, spoken to as a client would.
#
# The server is hand-rolled JSON-RPC (see crates/worktrees-cli/src/mcp.rs for
# why), so these tests are the contract: framing, handshake, and the gates that
# keep a model from doing more than the user allowed.

load 'helpers/common'

setup() { common_setup; }

# Feed newline-delimited JSON-RPC to the server and capture its replies.
#   mcp "<extra server args>" '<msg>' ['<msg>'...]
# stdin comes from a file rather than a pipe so `run` sees the server's status.
mcp() {
  local extra="$1"; shift
  printf '%s\n' "$@" > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp $extra < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
}

# Pull one field out of the reply stream with a real JSON parser, so assertions
# bind to structure instead of hoping a substring appears somewhere.
jq_out() { printf '%s' "$output" | python3 -c "$1"; }

@test "initialize echoes a protocol version the client asked for" {
  mcp "" '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"protocolVersion":"2025-06-18"'* ]]
  [[ "$output" == *'"name":"worktrees"'* ]]
}

@test "an unknown protocol version falls back to ours rather than failing" {
  mcp "" '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}'
  [[ "$output" == *'"protocolVersion":"2025-11-25"'* ]]
}

@test "a notification gets no reply at all" {
  # Answering one corrupts the stream — the client is not expecting a frame.
  # This is also the only assertion that would catch a stray write to stdout.
  mcp "" '{"jsonrpc":"2.0","method":"notifications/initialized"}'
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "malformed JSON produces a well-formed error frame, not a crash" {
  mcp "" 'not json'
  [ "$status" -eq 0 ]
  [[ "$output" == *'"code":-32700'* ]]
  [[ "$output" == *'"jsonrpc":"2.0"'* ]]
}

@test "an unknown method is -32601 and a missing method is -32600" {
  mcp "" '{"jsonrpc":"2.0","id":9,"method":"nope/nope"}' '{"jsonrpc":"2.0","id":10}'
  [[ "$output" == *'"code":-32601'* ]]
  [[ "$output" == *'"code":-32600'* ]]
}

@test "a malformed tools/call is a protocol error, not a tool result" {
  # The spec distinguishes a broken request from a tool that ran and failed.
  mcp "" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"arguments":{}}}' \
         '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_places","arguments":"oops"}}'
  [[ "$output" == *'"code":-32602'* ]]
  [ "$(printf '%s' "$output" | grep -c -- '-32602')" -eq 2 ]
}

@test "without --mutations only read and metadata tools are advertised" {
  mcp "" '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  [[ "$output" == *list_places* ]]
  [[ "$output" == *set_note* ]]
  [[ "$output" != *remove_worktree* ]]
  [[ "$output" != *create_worktree* ]]
}

@test "a tool the server did not advertise cannot be called" {
  # --mutations must be a gate, not a hint: calling an unadvertised tool has to
  # fail even though the dispatch arm for it exists.
  mcp "" '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remove_worktree","arguments":{"slug":"x","confirm":true}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"without --mutations"* ]]
}

@test "with --mutations the destructive tool appears but still needs confirm" {
  mcp --mutations '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remove_worktree","arguments":{"slug":"x"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"confirm: true"* ]]
}

@test "confirm cannot be satisfied by a truthy non-boolean" {
  mcp --mutations '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"remove_worktree","arguments":{"slug":"x","confirm":"true"}}}'
  [[ "$output" == *"confirm: true"* ]]
}

@test "annotations are bound to the right tools, not merely present somewhere" {
  # The previous version of this test grepped the whole blob for
  # "destructiveHint":true and "readOnlyHint":true — which stays green even if
  # every annotation is swapped onto the wrong tool.
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  local checked
  checked="$(jq_out '
import sys, json
tools = {t["name"]: t["annotations"] for t in json.load(sys.stdin)["result"]["tools"]}
assert tools["list_places"]["readOnlyHint"] is True, "list_places must be read-only"
assert tools["list_places"]["destructiveHint"] is False
assert tools["remove_worktree"]["destructiveHint"] is True, "remove_worktree must be destructive"
assert tools["remove_worktree"]["readOnlyHint"] is False
assert tools["set_note"]["readOnlyHint"] is False
print("ok")
')"
  [ "$checked" = "ok" ]
}

@test "a model-supplied value cannot become a command-line flag" {
  # base=--ai=<cmd> reached resolve_ai_cmd, which ops::launch interpolates into
  # `sh -ic '<cmd>; …'` — a tool advertised as "create a worktree" was arbitrary
  # code execution.
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"pwned","base":"--ai=touch '"$BATS_TEST_TMPDIR"'/PWNED"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"may not begin with"* ]]
  [ ! -e "$BATS_TEST_TMPDIR/PWNED" ]
  [ ! -d "$REPO/.worktrees/pwned" ]
}

@test "create_worktree opens a single pane by default; spare:true keeps the split" {
  # An agent's place has nobody at the keyboard for a spare shell.
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-a"}}}'
  [[ "$output" == *'"isError":false'* ]]
  tmux_session_exists repo-agent-a
  [ -z "$(tmux_pane1_cmd repo-agent-a)" ]
  ! grep -q 'split-window' "$TMUX_LOG"
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-b","spare":true}}}'
  [[ "$output" == *'"isError":false'* ]]
  [ -n "$(tmux_pane1_cmd repo-agent-b)" ]
  mcp --mutations '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-c","spare":"yes"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [ ! -d "$REPO/.worktrees/agent-c" ]
}

@test "create_worktree with a brief writes .planning/brief.md and launches claude on it" {
  export WORKTREES_AI_CMD=claude
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-d","brief":"# Task\n- do x\n- then y"}}}'
  [[ "$output" == *'"isError":false'* ]]
  [ "$(cat "$REPO/.worktrees/agent-d/.planning/brief.md")" = $'# Task\n- do x\n- then y' ]
  [[ "$(tmux_pane0_cmd repo-agent-d)" == *"--name"*"repo-agent-d"*"Read .planning/brief.md and begin."* ]]
  [[ "$(tmux_pane0_cmd repo-agent-d)" != *"do x"* ]]
  # a flag-shaped brief is a file, never a command line (the `base` guard's twin)
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-e","brief":"--ai=touch '"$BATS_TEST_TMPDIR"'/PWNED"}}}'
  [[ "$output" == *'"isError":false'* ]]
  [ ! -e "$BATS_TEST_TMPDIR/PWNED" ]
  [[ "$(tmux_pane0_cmd repo-agent-e)" == *"claude --name"* ]]
  # blank and non-string briefs are refused before anything is created
  mcp --mutations '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-f","brief":"  "}}}'
  [[ "$output" == *'"isError":true'* ]]
  [ ! -d "$REPO/.worktrees/agent-f" ]
  mcp --mutations '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-g","brief":7}}}'
  [[ "$output" == *'"isError":true'* ]]
  [ ! -d "$REPO/.worktrees/agent-g" ]
}

@test "place_status reports the claude session(s) working in the place" {
  run_wt new feat-ag --no-tmux
  local q='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"feat-ag"}}}'
  local pick='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
p = json.loads(frames[-1]["result"]["content"][0]["text"])
a = p["agents"][0] if p["agents"] else {}
print(p["agent_state"], len(p["agents"]), a.get("name", "-"), a.get("tmux", "-"))'
  mcp "" "$q"
  [ "$(jq_out "$pick")" = "none 0 - -" ]
  # A live probe (our own pid) whose cwd is this place: the app's dot and the
  # orchestrator now read the same file.
  mkdir -p "$HOME/.claude/sessions"
  local wt="$REPO/.worktrees/feat-ag"
  printf '{"pid":%s,"cwd":"%s","status":"idle","name":"feat-ag-1a","tmux":"repo-feat-ag:@1.%%1","updatedAt":5,"statusUpdatedAt":5}' "$$" "$wt" > "$HOME/.claude/sessions/$$.json"
  # …plus a busy one for the same place, a dead pid, and a probe elsewhere
  printf '{"pid":%s,"cwd":"%s","status":"busy","name":"feat-ag-2b","updatedAt":5,"statusUpdatedAt":5}' "$PPID" "$wt" > "$HOME/.claude/sessions/$PPID.json"
  printf '{"pid":2147483000,"cwd":"%s","status":"busy","name":"ghost"}' "$wt" > "$HOME/.claude/sessions/2147483000.json"
  printf '{"pid":%s,"cwd":"%s","status":"busy","name":"main-1"}' "$$" "$REPO" > "$HOME/.claude/sessions/other.json"
  mcp "" "$q"
  # most active first; the dead pid and the other place are not counted
  [ "$(jq_out "$pick")" = "busy 2 feat-ag-2b -" ]
}

@test "metadata tools refuse a slug that names no place" {
  # store::edit creates the entry it is given, so an unchecked slug left a ghost
  # record for a place that never existed.
  mcp "" '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_note","arguments":{"slug":"ghost","note":"x"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"no such place"* ]]
  [ ! -f "$REPO/.worktrees.places.json" ]
}

@test "set_pin refuses a non-boolean rather than silently unpinning" {
  run_wt new feat-x --no-tmux
  mcp "" '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_pin","arguments":{"slug":"feat-x","pinned":"yes"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"true or false"* ]]
}

@test "list_places reports this repository" {
  mcp "" '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_places","arguments":{}}}'
  [[ "$output" == *'"isError":false'* ]]
  [[ "$output" == *'(main)'* ]]
}

@test "set_note writes declared state the CLI can read back" {
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  mcp "" '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"set_note","arguments":{"slug":"feat-x","note":"from mcp"}}}'
  [[ "$output" == *'"isError":false'* ]]
  grep -q 'from mcp' "$REPO/.worktrees.places.json"
}

@test "outside a git repository the server refuses to start" {
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize"}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$BATS_TEST_TMPDIR' && '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>&1"
  [ "$status" -ne 0 ]
}
