#!/usr/bin/env bats
# `worktrees mcp` — the stdio MCP server, spoken to as a client would.
#
# The server is hand-rolled JSON-RPC (see crates/worktrees-cli/src/mcp.rs for
# why), so these tests are the contract: framing, handshake, and the gates that
# keep a model from doing more than the user allowed.

load 'helpers/common'

setup() { common_setup; }

# Pipe newline-delimited JSON-RPC in, get responses out. Runs in $REPO so the
# server pins this test's repository.
mcp() {   # each arg = one JSON message
  printf '%s\n' "$@" | (cd "$REPO" && "${WT_BIN}" mcp "${MCP_ARGS[@]:-}" 2>/dev/null)
}

@test "initialize echoes a protocol version the client asked for" {
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-06-18\"}}'"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"protocolVersion":"2025-06-18"'* ]]
  [[ "$output" == *'"name":"worktrees"'* ]]
}

@test "an unknown protocol version falls back to ours rather than failing" {
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"1999-01-01\"}}'"
  [[ "$output" == *'"protocolVersion":"2025-'* ]]
}

@test "a notification gets no reply at all" {
  # Answering one corrupts the stream — the client is not expecting a frame.
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}'"
  [ "$status" -eq 0 ]
  [ -z "$output" ]
}

@test "malformed JSON produces a well-formed error frame, not a crash" {
  run bash -c "printf 'not json\n' | (cd '$REPO' && '${WT_BIN}' mcp 2>/dev/null)"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"code":-32700'* ]]
  [[ "$output" == *'"jsonrpc":"2.0"'* ]]
}

@test "an unknown method is a JSON-RPC error, not silence" {
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"nope/nope\"}'"
  [[ "$output" == *'"code":-32601'* ]]
}

@test "without --mutations only read and metadata tools are advertised" {
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}'"
  [[ "$output" == *list_places* ]]
  [[ "$output" == *set_note* ]]
  [[ "$output" != *remove_worktree* ]]
  [[ "$output" != *create_worktree* ]]
}

@test "a tool the server did not advertise cannot be called" {
  # --mutations must be a gate, not a hint: calling an unadvertised tool has to
  # fail even though the dispatch arm for it exists.
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"remove_worktree\",\"arguments\":{\"slug\":\"x\",\"confirm\":true}}}'"
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"without --mutations"* ]]
}

@test "with --mutations the destructive tool appears but still needs confirm" {
  run bash -c "printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"remove_worktree\",\"arguments\":{\"slug\":\"x\"}}}' | (cd '$REPO' && '${WT_BIN}' mcp --mutations 2>/dev/null)"
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"confirm: true"* ]]
}

@test "destructive tools are annotated so a client can warn" {
  run bash -c "printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}' | (cd '$REPO' && '${WT_BIN}' mcp --mutations 2>/dev/null)"
  [[ "$output" == *'"destructiveHint":true'* ]]
  [[ "$output" == *'"readOnlyHint":true'* ]]
}

@test "list_places reports this repository" {
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"list_places\",\"arguments\":{}}}'"
  [[ "$output" == *'"isError":false'* ]]
  [[ "$output" == *'(main)'* ]]
}

@test "set_note writes declared state the CLI can read back" {
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  run bash -c "$(declare -f mcp); MCP_ARGS=(); WT_BIN='${WT_BIN}' REPO='$REPO' mcp '{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"set_note\",\"arguments\":{\"slug\":\"feat-x\",\"note\":\"from mcp\"}}}'"
  [[ "$output" == *'"isError":false'* ]]
  grep -q 'from mcp' "$REPO/.worktrees.places.json"
}

@test "outside a git repository the server refuses to start" {
  run bash -c "printf '%s\n' '{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}' | (cd '$BATS_TEST_TMPDIR' && '${WT_BIN}' mcp 2>&1)"
  [ "$status" -ne 0 ]
}
