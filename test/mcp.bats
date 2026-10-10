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

# `show_doc` writes nothing in the repo but it drives the user's SCREEN, so it
# sits in the --mutations tier while carrying readOnlyHint: true. HOME is
# per-test (helpers/common), so the inbox these write to is a throwaway.
@test "show_doc is in the --mutations tier, not the read tier" {
  mcp "" '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  [[ "$output" != *show_doc* ]]
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
  [[ "$output" == *show_doc* ]]
}

@test "show_doc queues a request for a repo file and refuses one outside" {
  printf '# notes\n' > "$REPO/CLAUDE.md"
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"show_doc","arguments":{"path":"CLAUDE.md"}}}'
  [[ "$output" == *'"isError":false'* ]]
  # The ask has to actually reach the inbox — a tool that reports success and
  # queues nothing is the failure this test exists for.
  run bash -c "cat '$HOME'/.cache/worktrees/inbox/*.json"
  [ "$status" -eq 0 ]
  [[ "$output" == *CLAUDE.md* ]]

  # A path that RESOLVES outside the pinned repo is refused, and queues nothing.
  rm -f "$HOME"/.cache/worktrees/inbox/*.json
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"show_doc","arguments":{"path":"/etc/hosts"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"outside this repository"* ]]
  run bash -c "ls '$HOME'/.cache/worktrees/inbox/*.json 2>/dev/null | wc -l"
  [[ "$output" == *0* ]]
}

# The CLI twin, through the real binary: `file:LINE` is the form every
# compiler and agent prints, and it must queue the LINE, not a missing file.
@test "worktrees show splits path:line and queues the position" {
  printf 'a\nb\nc\n' > "$REPO/notes.txt"
  run bash -c "cd '$REPO' && '$WT_BIN' show notes.txt:2"
  [ "$status" -eq 0 ]
  [[ "$output" == *"at line 2"* ]]
  run bash -c "cat '$HOME'/.cache/worktrees/inbox/*.json"
  [[ "$output" == *'"line":2'* ]]
  [[ "$output" == *notes.txt\"* ]]

  rm -f "$HOME"/.cache/worktrees/inbox/*.json
  run bash -c "cd '$REPO' && '$WT_BIN' show notes.txt --line 0"
  [ "$status" -ne 0 ]
  [[ "$output" == *"1-based"* ]]
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

@test "create_worktree opens a single pane by default; spare:true splits" {
  # An agent's place has nobody at the keyboard for a spare shell.
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-a"}}}'
  [[ "$output" == *'"isError":false'* ]]
  tmux_session_exists repo-agent-a
  [ -z "$(tmux_pane1_cmd repo-agent-a)" ]
  mcp --mutations '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-e","spare":false}}}'
  [[ "$output" == *'"isError":false'* ]]
  [ -z "$(tmux_pane1_cmd repo-agent-e)" ]
  ! grep -q 'split-window' "$TMUX_LOG"
  # The CLI no longer splits by default, so spare:true has to ASK for it.
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-b","spare":true}}}'
  [[ "$output" == *'"isError":false'* ]]
  [ -n "$(tmux_pane1_cmd repo-agent-b)" ]
  grep -q 'split-window' "$TMUX_LOG"
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

@test "create_worktree can choose Codex without changing the project's Claude default" {
  install_fake_cmd codex
  export WORKTREES_AI_CMD=claude
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-codex","provider":"codex","brief":"Review this branch"}}}'
  [[ "$output" == *'"isError":false'* ]]
  tmux_session_exists 'repo-agent-codex~agent~codex'
  [[ "$(tmux_pane0_cmd 'repo-agent-codex~agent~codex')" == *"codex -c forced_login_method=chatgpt"*"Read .planning/brief.md and begin."* ]]
  [ "$(cat "$REPO/.worktrees/agent-codex/.planning/brief.md")" = "Review this branch" ]
  mcp --mutations '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-default"}}}'
  tmux_session_exists repo-agent-default
  [[ "$(tmux_pane0_cmd repo-agent-default)" == *"claude --name"* ]]
  mcp --mutations '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-default","provider":"codex"}}}'
  [[ "$output" == *'"isError":false'* ]]
  ! tmux_session_exists repo-agent-default
  tmux_session_exists 'repo-agent-default~agent~codex'
  mcp --mutations '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-invalid","provider":"other"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [ ! -d "$REPO/.worktrees/agent-invalid" ]
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

# The server used to EXIT here, which was fine while it was added per-repo and
# became wrong once the app started installing it at user scope: claude launches
# a user-scope server for every session, including the ones started in a home or
# scratch directory, and each of those showed "✘ Failed to connect:
# CONNECTION_CLOSED" for a perfectly correct install.
#
# The contract is now: handshake normally, advertise NOTHING, exit 0, and say on
# STDERR (never stdout, which carries protocol) why there are no tools.
@test "outside a git repository the server serves, with no tools" {
  printf '%s\n' \
    '{"jsonrpc":"2.0","id":1,"method":"initialize"}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$BATS_TEST_TMPDIR' && '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [ "$status" -eq 0 ]
  [[ "$output" == *'"serverInfo"'* ]]
  [[ "$output" == *'"tools":[]'* ]]
  [[ "$output" != *'"list_places"'* ]]

  # ...and the explanation goes to stderr, with stdout left pure protocol.
  run bash -c "cd '$BATS_TEST_TMPDIR' && '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>&1 >/dev/null"
  [[ "$output" == *'no tools'* ]]
}

# The setup verbs run OUTSIDE a repository too — they are about this machine's
# claude, not about a checkout. A git guard here would answer "not a git
# repository", which reads as a verdict on the MCP setup rather than on the cwd.
@test "mcp --status answers outside a git repository" {
  run bash -c "cd '$BATS_TEST_TMPDIR' && HOME='$BATS_TEST_TMPDIR' '$WT_BIN' mcp --status --json"
  [[ "$output" == *'"state"'* ]]
  [[ "$output" != *'Not inside a git repository'* ]]
}

# Claude MCP is independent of the configured default: both providers can run
# in one place, and the setup state remains inspectable for each of them.
@test "mcp --status reports Claude independently of the default AI command" {
  run bash -c "cd '$BATS_TEST_TMPDIR' && HOME='$BATS_TEST_TMPDIR' '$WT_BIN' mcp --status --json"
  [[ "$output" == *'"state":"absent"'* ]]
  [[ "$output" == *'"ai_cmd":"fake-ai"'* ]]
}

# ...and with claude as the AI command and no ~/.claude.json at all (a brand-new
# machine — exactly who the app's nudge is for) the answer is "absent": not a
# crash, and not "installed".
@test "mcp --status reads absent from an empty HOME" {
  run bash -c "cd '$BATS_TEST_TMPDIR' && HOME='$BATS_TEST_TMPDIR' WORKTREES_AI_CMD=claude '$WT_BIN' mcp --status --json"
  [[ "$output" == *'"state":"absent"'* ]]
  [[ "$output" == *'"user":null'* ]]
  [[ "$output" == *'"found_in":[]'* ]]
  # The command is always spelled out, so the UI (and a human) can run it by hand.
  [[ "$output" == *'claude mcp add -s user worktrees --'* ]]
}

# ── automations ──────────────────────────────────────────────────────────────

# The recursion guard (proposal §4.5), end to end over the real transport. A run
# HOLDS this server, so `run_automation` reaching it would let a brief spawn
# runs, and `upsert_automation` would let one rewrite the job it is executing.
# `remove_worktree` is on the list for the other reason: an unattended caller
# never reaches the one path that can destroy commits.
#
# Both halves are asserted. The tool list is advice — a model with the name from
# a document walks straight past a missing entry — so the call has to refuse too.
@test "inside a run the server hides the automation mutations and remove_worktree" {
  export WORKTREES_RUN_ID=2026-09-22T08-02-11Z-sweep
  mcp "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
  [ "$status" -eq 0 ]
  for gone in upsert_automation delete_automation run_automation apply_proposal remove_worktree; do
    [[ "$output" != *"\"name\":\"$gone\""* ]] || { echo "$gone was advertised inside a run"; false; }
  done
  # Reads survive: a brief must still be able to say "compare with yesterday".
  [[ "$output" == *'"name":"list_automations"'* ]]
  [[ "$output" == *'"name":"list_runs"'* ]]
  [[ "$output" == *'"name":"get_run"'* ]]
  [[ "$output" == *'"name":"list_places"'* ]]

  mcp "--mutations" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"run_automation","arguments":{"slug":"sweep"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *'inside an automation run'* ]]
}

# Outside a run the same server offers them — otherwise the test above would
# pass on a server that simply never had the tools.
@test "outside a run the mutating automation tools are advertised" {
  mcp "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
  for want in upsert_automation delete_automation run_automation apply_proposal; do
    [[ "$output" == *"\"name\":\"$want\""* ]] || { echo "$want missing"; false; }
  done
}

# `run_automation` is async BY CONSTRUCTION: it spawns `current_exe()` and does
# not wait. A unit test cannot exercise that — under `cargo test`, `current_exe`
# is the test harness — so this is the one place the real binary re-enters
# itself, with the child inheriting the fake claude, the fake tmux and the
# isolated state dir from this server's environment.
@test "run_automation returns an id at once and the child finishes the run" {
  export XDG_STATE_HOME="$BATS_TEST_TMPDIR/state"
  install_fake_claude
  export WORKTREES_AI_CMD=claude
  export FAKE_CLAUDE_FINDINGS="$BATS_TEST_TMPDIR/findings.json"
  echo '{"findings":[{"slug":"(main)","text":"Nothing pushed."}]}' > "$FAKE_CLAUDE_FINDINGS"
  run_wt automations add --name Sweep --brief "look at everything"
  [ "$status" -eq 0 ]

  mcp "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run_automation","arguments":{"slug":"sweep"}}}'
  [ "$status" -eq 0 ]
  [[ "$output" == *'\"status\":\"running\"'* ]]
  [[ "$output" == *'-sweep'* ]]

  # The child is detached, so the ledger is polled rather than waited on.
  for _ in $(seq 1 60); do
    entry="$(find "$XDG_STATE_HOME/worktrees/runs" -name '*-sweep.json' 2>/dev/null | head -n1)"
    [ -n "$entry" ] && grep -q '"status": "findings"' "$entry" && break
    sleep 0.5
  done
  [ -n "$entry" ]
  grep -q '"status": "findings"' "$entry"
  grep -q '"trigger": "mcp"' "$entry"
}

@test "mcp --status --ai codex reads Codex configuration independently" {
  local codex_home="$BATS_TEST_TMPDIR/codex-home"
  mkdir -p "$codex_home"
  printf '[mcp_servers.worktrees]\ncommand = "%s"\nargs = ["mcp", "--mutations"]\n' "$WT_BIN" > "$codex_home/config.toml"
  run env CODEX_HOME="$codex_home" "$WT_BIN" mcp --status --ai codex --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"state":"installed"'* ]]
  [[ "$output" == *'"mutations":true'* ]]
}

@test "mcp --status --ai pi reads pi's mcp.json, never runs pi mcp list" {
  local agent="$BATS_TEST_TMPDIR/pi-agent" bin="$BATS_TEST_TMPDIR/pibin"
  mkdir -p "$agent" "$bin"
  # A pi that records every call: status must not make one.
  printf '#!/bin/sh\necho "$*" >> "%s/calls"\n' "$bin" > "$bin/pi"; chmod +x "$bin/pi"
  run env PI_CODING_AGENT_DIR="$agent" WORKTREES_PI_BIN="$bin/pi" "$WT_BIN" mcp --status --ai pi --json
  [ "$status" -eq 1 ]
  [[ "$output" == *'"state":"absent"'* ]]
  printf '{"mcpServers":{"worktrees":{"command":"%s","args":["mcp","--mutations"],"env":{"WORKTREES_MCP_PROVIDER":"pi"},"exposure":"direct"}}}' "$WT_BIN" > "$agent/mcp.json"
  run env PI_CODING_AGENT_DIR="$agent" WORKTREES_PI_BIN="$bin/pi" "$WT_BIN" mcp --status --ai pi --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"state":"installed"'* ]]
  [[ "$output" == *'"exposure":"direct"'* ]]
  [ ! -e "$bin/calls" ]
}

@test "mcp --install --ai pi shells out to pi mcp add with the decided line, in pi's agent dir" {
  local agent="$BATS_TEST_TMPDIR/pi-agent" bin="$BATS_TEST_TMPDIR/pibin"
  mkdir -p "$agent" "$bin"
  # The fake writes what the real `pi mcp add` writes, into ITS agent dir —
  # so a status that disagreed with the install about the dir would read absent.
  cat > "$bin/pi" <<'SH'
#!/bin/sh
echo "$PI_CODING_AGENT_DIR :: $*" >> "$(dirname "$0")/calls"
[ "$1 $2" = "mcp add" ] || exit 0
wt="$9"
printf '{"mcpServers":{"worktrees":{"command":"%s","args":["mcp","--mutations"],"env":{"WORKTREES_MCP_PROVIDER":"pi"},"exposure":"direct"}}}' "$wt" > "$PI_CODING_AGENT_DIR/mcp.json"
SH
  chmod +x "$bin/pi"
  run env PI_CODING_AGENT_DIR="$agent" WORKTREES_PI_BIN="$bin/pi" WORKTREES_CLI_BIN="$WT_BIN" "$WT_BIN" mcp --install --ai pi --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"state":"installed"'* ]]
  grep -qF "$agent :: mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi --exposure direct -- $WT_BIN mcp --mutations" "$bin/calls"
  ! grep -qE -- ' (-l|--local)( |$)' "$bin/calls"
}

@test "a pi client's server signs from its cwd even with CLAUDE_PROJECT_DIR set" {
  run_wt new feat-p --no-tmux
  local wt="$REPO/.worktrees/feat-p"
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report","arguments":{"text":"hi"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$wt' && CLAUDE_PROJECT_DIR='$REPO' WORKTREES_MCP_PROVIDER=pi '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [[ "$output" == *'\"from\": \"feat-p\"'* ]]
}

# ── place↔place messaging ───────────────────────────────────────────────────
# Every server below is started with its cwd PINNED to a directory inside
# $REPO (the fixture), never the suite's own cwd: a `report` writes into the
# git common dir of whatever repo the server discovers, and the suite's cwd is
# this repository.

# mcp_in <dir> "<extra server args>" '<msg>'...
mcp_in() {
  local dir="$1" extra="$2"; shift 2
  case "$dir" in "$REPO"|"$REPO"/*) ;; *) echo "mcp_in: $dir is outside the fixture" >&2; return 1 ;; esac
  printf '%s\n' "$@" > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$dir' && '$WT_BIN' mcp $extra < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
}

# The first frame's tool-result body, parsed.
result_body='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
r = frames[-1]["result"]
print(json.dumps({"isError": r["isError"], "body": r["content"][0]["text"]}))'

@test "report from a worktree reaches (main) through messages, signed by the sender's place" {
  run_wt new feat-m --no-tmux
  local wt="$REPO/.worktrees/feat-m"
  # `from` in the arguments is ignored: the server signs with its own place.
  mcp_in "$wt" "" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"report","arguments":{"text":"done: tests green","from":"(main)"}}}'
  [[ "$output" == *'"isError":false'* ]]
  # Stored in the git COMMON dir, untracked — not in either checkout.
  [ "$(find "$REPO/.git/worktrees-messages" -name '*.json' | wc -l | tr -d ' ')" = 1 ]
  [ -z "$(git -C "$REPO" status --porcelain)" ]
  local pick='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
b = json.loads(frames[-1]["result"]["content"][0]["text"])
print(b["place"], len(b["messages"]), *(m["from"] + ":" + m["text"] for m in b["messages"]))'
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"messages","arguments":{}}}'
  [ "$(jq_out "$pick")" = "(main) 1 feat-m:done: tests green" ]
  # Acked on read: the next look is empty.
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"messages","arguments":{}}}'
  [ "$(jq_out "$pick")" = "(main) 0" ]
  # wait until: message from the worktree now sees nothing new, at once.
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"wait","arguments":{"until":"message","timeout_s":0}}}'
  [[ "$output" == *'\"event\": \"timeout\"'* ]]
}

@test "wait until idle answers at once for a place with no agent" {
  run_wt new feat-w --no-tmux
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"wait","arguments":{"until":"idle","slug":"feat-w","timeout_s":5}}}'
  [[ "$output" == *'"isError":false'* ]]
  [[ "$output" == *'\"event\": \"none\"'* ]]
}

@test "place_status reports a Codex place's activity from its rollout" {
  run_wt new feat-cx --no-tmux
  local wt="$REPO/.worktrees/feat-cx"
  # The managed codex session, running codex (not a shell).
  printf 'cwd=%s\n' "$wt" > "$TMUX_STATE/repo-feat-cx~agent~codex"
  printf 'codex' > "$TMUX_STATE/repo-feat-cx~agent~codex.cmd"
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"
  local day="$CODEX_HOME/sessions/2026/09/26"; mkdir -p "$day"
  printf '{"type":"session_meta","payload":{"cwd":"%s","source":"cli"}}\n{"type":"event_msg","payload":{"type":"task_started"}}\n{"type":"event_msg","payload":{"type":"task_complete","completed_at":1790000000}}\n' "$wt" > "$day/rollout-a.jsonl"
  local q='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"feat-cx"}}}'
  local pick='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
p = json.loads(frames[-1]["result"]["content"][0]["text"])
a = p["activity"]
print(p["agent_state"], a["provider"], a["state"], a["last_done"], a["session"])'
  mcp_in "$REPO" "" "$q"
  [ "$(jq_out "$pick")" = "idle codex idle 1790000000 repo-feat-cx~agent~codex" ]
  # Mid-turn: busy (the fake tmux answers no capture, which reads as busy).
  printf '{"type":"event_msg","payload":{"type":"task_started"}}\n' >> "$day/rollout-a.jsonl"
  mcp_in "$REPO" "" "$q"
  [ "$(jq_out "$pick")" = "busy codex busy None repo-feat-cx~agent~codex" ]
  # …and `wait until: idle` on it times out rather than lying.
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"wait","arguments":{"until":"idle","slug":"feat-cx","timeout_s":0}}}'
  [[ "$output" == *'\"event\": \"timeout\"'* ]]
}

# A program in a place's OWN session that no harness reports on — here vim,
# typed into the pane. Before, place_status said `none` and wait returned at
# once: an orchestrator was told nobody was working there.
@test "an unclaimed program in a place's own session is unknown to place_status and wait alike" {
  run_wt new feat-u --no-tmux
  printf 'cwd=%s\n' "$REPO/.worktrees/feat-u" > "$TMUX_STATE/repo-feat-u"
  printf 'vim' > "$TMUX_STATE/repo-feat-u.cmd"
  local pick='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
p = json.loads(frames[-1]["result"]["content"][0]["text"])
a = p.get("activity") or {}
print(p.get("agent_state", p.get("event")), a["state"], a["provider"], a.get("session"), "did not launch" in a.get("reason", ""))'
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"feat-u"}}}'
  [ "$(jq_out "$pick")" = "unknown unknown None repo-feat-u True" ]
  # The same derivation answers wait: it keeps waiting rather than calling it done.
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"wait","arguments":{"until":"idle","slug":"feat-u","timeout_s":1}}}'
  [ "$(jq_out "$pick")" = "timeout unknown None repo-feat-u True" ]
  # vim is not `node`: naming it cost no ps.
  [ ! -e "$BATS_TEST_TMPDIR/ps.log" ]
  # Back at its shell, nobody is there — and wait says so at once.
  printf 'zsh' > "$TMUX_STATE/repo-feat-u.cmd"
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"wait","arguments":{"until":"idle","slug":"feat-u","timeout_s":5}}}'
  [ "$(jq_out "$pick")" = "none none None None False" ]
}

# pi typed by hand into a place's own session: tmux reports `node`, and only
# the tty's foreground process-group leader says `pi`. The shim's tty is
# /dev/tty-<session>; ps spells it without the /dev/.
@test "pi typed into a place's own session is pi to place_status, named by its tty's foreground" {
  run_wt new feat-h --no-tmux
  printf 'cwd=%s\n' "$REPO/.worktrees/feat-h" > "$TMUX_STATE/repo-feat-h"
  printf 'node' > "$TMUX_STATE/repo-feat-h.cmd"
  local q='{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"feat-h"}}}'
  local pick='
import sys, json
frames = [json.loads(l) for l in sys.stdin.read().splitlines() if l.strip()]
p = json.loads(frames[-1]["result"]["content"][0]["text"])
a = p["activity"]
print(a["provider"], a.get("session"), *[g["provider"] + "@" + str(g["tmux"]) for g in p["agents"]])'
  # ps names nothing here: a bare `node` nobody reports on.
  mcp_in "$REPO" "" "$q"
  [ "$(jq_out "$pick")" = "None repo-feat-h" ]
  printf '??           1     1 /sbin/launchd\ntty-repo-feat-h  501   777 /bin/zsh\ntty-repo-feat-h  777   777 pi\n' > "$BATS_TEST_TMPDIR/ps.out"
  mcp_in "$REPO" "" "$q"
  [ "$(jq_out "$pick")" = "pi repo-feat-h pi@repo-feat-h" ]
  # One ps per populated project snapshot, never per question or empty legacy probe.
  [ "$(grep -c '^ps ' "$BATS_TEST_TMPDIR/ps.log")" -eq "$(grep -c -- "-L $(cat "$TMUX_STATE.primary") .*|list-panes" "$TMUX_LOG.globals")" ]
  # …and only over the ttys that want naming, as ps spells them.
  [ "$(sort -u "$BATS_TEST_TMPDIR/ps.log")" = "ps -t tty-repo-feat-h -o tty=,pid=,tpgid=,comm=" ]
}

@test "send is a --mutations tool and refuses what it must not type" {
  run_wt new feat-s --no-tmux
  local wt="$REPO/.worktrees/feat-s"
  mcp_in "$REPO" "" '{"jsonrpc":"2.0","id":1,"method":"tools/list"}'
  [[ "$output" != *'"name":"send"'* ]]
  [[ "$output" == *'"name":"report"'* ]]
  # A control character: refused before anything is looked up.
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"a\nb"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"control character"* ]]
  # Nobody there.
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *"no agent is running in feat-s"* ]]
  # A codex in a session this project did NOT create (another prefix): refused.
  printf 'cwd=%s\n' "$wt" > "$TMUX_STATE/other-feat-s"
  printf 'codex' > "$TMUX_STATE/other-feat-s.cmd"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *"did not create"* ]]
  rm -f "$TMUX_STATE/other-feat-s" "$TMUX_STATE/other-feat-s.cmd"
  # A Claude place is not typed into: the answer names its session for SendMessage.
  mkdir -p "$HOME/.claude/sessions"
  printf '{"pid":%s,"cwd":"%s","status":"idle","name":"repo-feat-s","updatedAt":5,"statusUpdatedAt":5}' "$$" "$wt" > "$HOME/.claude/sessions/$$.json"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *'SendMessage to \"repo-feat-s\"'* ]]
  # Nothing was typed anywhere.
  ! grep -q 'send-keys' "$TMUX_LOG"
}

@test "send types a labelled line into this project's own Codex pane and files the copy read" {
  run_wt new feat-s --no-tmux
  local wt="$REPO/.worktrees/feat-s"
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"; mkdir -p "$CODEX_HOME"
  printf 'cwd=%s\n' "$wt" > "$TMUX_STATE/repo-feat-s~agent~codex"
  printf 'codex' > "$TMUX_STATE/repo-feat-s~agent~codex.cmd"
  printf '%s' 'repo-feat-s~agent~codex' > "$TMUX_STATE/.pane-%0"
  local fixtures="$BATS_TEST_DIRNAME/../crates/worktrees-core/tests/fixtures/codex-send"
  cp "$fixtures/typed.txt" "$TMUX_STATE/repo-feat-s~agent~codex.screen"
  cp "$fixtures/empty.txt" "$TMUX_STATE/.after-enter"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *'"isError":false'* ]]
  [[ "$output" == *'\"delivered\": true'* ]]
  # Typed literally, labelled, then Enter as its own keystroke — in that order.
  local typed enter
  typed="$(grep -n -F 'tmux send-keys -t %0 -l -- [worktrees: message from place "(main)", not from the user] hi' "$TMUX_LOG" | cut -d: -f1)"
  enter="$(grep -n -F 'tmux send-keys -t %0 Enter' "$TMUX_LOG" | cut -d: -f1)"
  # Separate assertions: in an `a && b && c` line, set -e ignores a false
  # `a`, and the test passes having checked nothing.
  [ -n "$typed" ]
  [ -n "$enter" ]
  [ "$typed" -lt "$enter" ]
  # Each write leaves copy-mode first, or a pane the user scrolled back in the
  # app eats the keys: the leave is logged on the line right before each one.
  local leave='tmux copy-mode -q -t %0'
  [ "$(sed -n "$((typed - 1))p" "$TMUX_LOG")" = "$leave" ]
  [ "$(sed -n "$((enter - 1))p" "$TMUX_LOG")" = "$leave" ]
  # The log copy is RAW, and filed already-read for the recipient.
  local store="$REPO/.git/worktrees-messages"
  grep -q '"text":"hi"' "$store"/*.json
  [ "$(find "$store/.read" -type f | wc -l | tr -d ' ')" = 1 ]
}

@test "send refuses a Codex that is waiting on an approval, and never presses Enter into one" {
  run_wt new feat-s --no-tmux
  local wt="$REPO/.worktrees/feat-s"
  local s='repo-feat-s~agent~codex'
  printf 'cwd=%s\n' "$wt" > "$TMUX_STATE/$s"
  printf 'codex' > "$TMUX_STATE/$s.cmd"
  printf '  1. Yes, proceed\n  Press enter to confirm or esc to cancel\n' > "$TMUX_STATE/$s.screen"
  printf '%s' "$s" > "$TMUX_STATE/.pane-%0"
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"
  local day="$CODEX_HOME/sessions/2026/09/26"; mkdir -p "$day"
  # Mid-turn per the rollout, and the screen shows the approval modal.
  printf '{"type":"session_meta","payload":{"cwd":"%s","source":"cli"}}\n{"type":"event_msg","payload":{"type":"task_started"}}\n' "$wt" > "$day/rollout-a.jsonl"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *'"isError":true'* ]]
  [[ "$output" == *"waiting on you"* ]]
  ! grep -q 'send-keys' "$TMUX_LOG"
  # The modal opens only AFTER the look (the rollout says idle): the text is
  # typed, but the fresh look before Enter sees the modal and holds Enter back.
  printf '{"type":"event_msg","payload":{"type":"task_complete","completed_at":1790000000}}\n' >> "$day/rollout-a.jsonl"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *'\"delivered\": false'* ]]
  grep -q -- 'send-keys -t %0 -l --' "$TMUX_LOG"
  ! grep -q 'send-keys -t %0 Enter' "$TMUX_LOG"
}

@test "a running MCP server reports a replaced binary without executing it" {
  local root
  root="$(cd "$(dirname "$WT_BIN")/.." && pwd)"
  local binary="$root/target/release/worktrees"
  [ -x "$binary" ] || binary="$root/target/debug/worktrees"
  run bash -c 'cd "$1" && python3 "$2/scripts/mcp-stale-check.py" "$3"' _ "$REPO" "$root" "$binary"
  [ "$status" -eq 0 ]
}

@test "legacy tool requests receive provider capabilities even with a current server" {
  mcp "--mutations" \
    '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"--invalid"}}}' \
    '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"(main)"}}}'
  [ "$status" -eq 0 ]
  jq_out '
import json, sys
results = [json.loads(line)["result"] for line in sys.stdin]
assert [r["isError"] for r in results] == [True, False]
for r in results:
    assert len(r["content"]) == 2, r
    notice = r["content"][1]["text"]
    assert "create_worktree.provider accepts claude or codex" in notice, notice
    assert "full session restart is unverified" in notice, notice
    assert "installed binary" not in notice, notice
'
}

@test "send leaves a stuck Codex paste unconfirmed and its message unread" {
  run_wt new feat-s --no-tmux
  local wt="$REPO/.worktrees/feat-s" s='repo-feat-s~agent~codex'
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"; mkdir -p "$CODEX_HOME"
  printf 'cwd=%s\n' "$wt" > "$TMUX_STATE/$s"
  printf 'codex' > "$TMUX_STATE/$s.cmd"
  printf '%s' "$s" > "$TMUX_STATE/.pane-%0"
  cp "$BATS_TEST_DIRNAME/../crates/worktrees-core/tests/fixtures/codex-send/pasted.txt" "$TMUX_STATE/$s.screen"
  grep -q '\[Pasted Content 1284 chars\]' "$TMUX_STATE/$s.screen"
  mcp_in "$REPO" "--mutations" '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send","arguments":{"slug":"feat-s","text":"hi"}}}'
  [[ "$output" == *'\"delivered\": false'* ]]
  local store="$REPO/.git/worktrees-messages"
  grep -q '"text":"hi"' "$store"/*.json
  [ ! -d "$store/.read" ] || [ "$(find "$store/.read" -type f | wc -l | tr -d ' ')" = 0 ]
  [ "$(grep -c 'send-keys -t %0 Enter' "$TMUX_LOG")" = 3 ]
}

@test "create_worktree keeps stdout pure JSON-RPC: git's own output never reaches the protocol stream" {
  # Core passes git's success output through ("branch 'x' set up to track …",
  # "HEAD is now at …"). On the CLI that is the terminal; in `worktrees mcp`
  # stdout IS the protocol, and one non-JSON line is a parse error for the
  # client. Every line must parse.
  mcp --mutations '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"agent-pure"}}}'
  [ -d "$REPO/.worktrees/agent-pure" ]
  printf '%s\n' "$output" > "$BATS_TEST_TMPDIR/out.jsonl"
  while IFS= read -r line; do
    [ -z "$line" ] && continue
    printf '%s' "$line" | python3 -c 'import sys, json; json.loads(sys.stdin.read())' || { echo "not JSON: $line"; return 1; }
  done < "$BATS_TEST_TMPDIR/out.jsonl"
}
