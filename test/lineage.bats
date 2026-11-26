#!/usr/bin/env bats
# Lanes started by a lane (docs/proposals/lane-children.md): who records
# `started_by`, what a removal writes, adoption, and what `ls --json` says.

load 'helpers/common'

setup() { common_setup; }

# One declared field of one place, `-` when absent.
#   decl <slug> <field> [subfield]
decl() {
  python3 - "$REPO/.worktrees.places.json" "$@" <<'PY'
import json, sys
path, slug, field = sys.argv[1:4]
sub = sys.argv[4] if len(sys.argv) > 4 else None
try:
    d = json.load(open(path))["places"].get(slug, {})
except FileNotFoundError:
    d = {}
v = d.get(field)
if v is not None and sub:
    v = v.get(sub)
print("-" if v is None else v)
PY
}

# One place's `lineage.<field>` from `ls --json`, `-` when absent; lists joined by `,`.
lineage() {
  python3 -c '
import json, sys
slug, field = sys.argv[1:3]
rows = {p["slug"]: p for p in json.load(sys.stdin)["places"]}
v = (rows.get(slug, {}).get("lineage") or {}).get(field)
if isinstance(v, list): v = ",".join(v)
if isinstance(v, dict): v = v.get("place")
print("-" if v in (None, "") else v)' "$@" <<<"$LS"
}

ls_json() { LS="$(cd "$REPO" && "$WT_BIN" ls --json)"; }

@test "lineage: new typed in a lane's pane makes a child of that lane" {
  run_wt new starter --no-tmux
  [ "$status" -eq 0 ]
  run_wt -C "$REPO/.worktrees/starter" new kid --no-tmux
  [ "$status" -eq 0 ]
  grep -qF "started by: starter" <<<"$output"
  [ "$(decl kid started_by place)" = starter ]
  [ "$(decl starter started_by)" = - ]
}

@test "lineage: new from (main) records no starter and writes no store" {
  run_wt new plain --no-tmux
  [ "$status" -eq 0 ]
  ! grep -qF "started by" <<<"$output" || false
  [ ! -e "$REPO/.worktrees.places.json" ]
}

@test "lineage: new run in a subdirectory of a lane still names the lane" {
  run_wt new starter --no-tmux
  mkdir -p "$REPO/.worktrees/starter/deep/er"
  run_wt -C "$REPO/.worktrees/starter/deep/er" new kid --no-tmux
  [ "$status" -eq 0 ]
  [ "$(decl kid started_by place)" = starter ]
}

@test "lineage: a reused slug is clean, and reusing a place writes nothing" {
  run_wt new starter --no-tmux
  run_wt -C "$REPO/.worktrees/starter" new kid --no-tmux
  [ "$(decl kid started_by place)" = starter ]
  # Reusing the existing place (same branch) from (main): origin is fixed.
  run_wt new kid --no-tmux
  [ "$status" -eq 0 ]
  [ "$(decl kid started_by place)" = starter ]
  # Remove it and make the slug again from (main): the dead namesake's link goes.
  run_wt rm -y kid
  [ "$status" -eq 0 ]
  run_wt new kid2 --name kid --no-tmux
  [ "$status" -eq 0 ]
  [ "$(decl kid started_by)" = - ]
}

@test "lineage: MCP create_worktree from a lane records it, from (main) does not" {
  run_wt new starter --no-tmux
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"feat/kid"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO/.worktrees/starter' && '$WT_BIN' mcp --mutations < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [ "$status" -eq 0 ]
  [ -d "$REPO/.worktrees/feat-kid" ]
  [ "$(decl feat-kid started_by place)" = starter ]
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_worktree","arguments":{"branch":"feat/top"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp --mutations < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  [ -d "$REPO/.worktrees/feat-top" ]
  [ "$(decl feat-top started_by)" = - ]
}

@test "lineage: ls --json resolves parent, starter, started and the report default" {
  run_wt new a --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  run_wt -C "$REPO/.worktrees/b" new c --no-tmux
  run_wt new old --no-tmux
  ls_json
  [ "$(lineage c parent)" = b ]
  [ "$(lineage b parent)" = a ]
  [ "$(lineage a parent)" = - ]
  [ "$(lineage c starter)" = b ]
  [ "$(lineage a started)" = b ]
  [ "$(lineage a lanes)" = b ]
  # No agent runs anywhere (--no-tmux), so every default report goes to (main).
  [ "$(lineage c reports_to)" = "(main)" ]
  # A place with no record behaves as today: top level, reports to (main).
  [ "$(lineage old parent)" = - ]
  [ "$(lineage old reports_to)" = "(main)" ]
  [ "$(lineage '(main)' reports_to)" = - ]
}

@test "lineage: MCP place_status carries lineage with the place's started lanes" {
  run_wt new a --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"place_status","arguments":{"slug":"a"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp < '$BATS_TEST_TMPDIR/in.jsonl' 2>/dev/null"
  got="$(printf '%s' "$output" | python3 -c 'import json,sys; r=json.loads(sys.stdin.read()); print(json.loads(r["result"]["content"][0]["text"])["lineage"]["started"])')"
  [ "$got" = "['b']" ]
}
