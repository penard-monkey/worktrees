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

# Set `under` by hand, as an adoption would (the adopt verbs have their own tests).
put_under() {
  python3 - "$REPO/.worktrees.places.json" "$1" "$2" <<'PY'
import json, sys, time
path, slug, to = sys.argv[1:]
d = json.load(open(path))
d["places"].setdefault(slug, {})["under"] = {"place": to, "at": int(time.time()) + 1, "by": "owner"}
json.dump(d, open(path, "w"))
PY
}

@test "lineage: removing a starter moves its lane up to (main), started_by kept" {
  run_wt new a --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  run_wt rm a
  grep -qF "1 lane hangs from it (b). It moves up to (main)" <<<"$output"
  grep -qF "Skipped a." <<<"$output"
  [ "$(decl b handed_to)" = - ]
  run_wt rm -y a
  [ "$status" -eq 0 ]
  grep -qF "moved up to (main): b" <<<"$output"
  [ "$(decl b handed_to place)" = "(main)" ]
  [ "$(decl b handed_to from)" = a ]
  [ "$(decl b started_by place)" = a ]
  ls_json
  [ "$(lineage b parent)" = - ]
  [ "$(lineage b reports_to)" = "(main)" ]
  [ -d "$REPO/.worktrees/b" ]
}

@test "lineage: removing the middle of a chain hands its lanes to its own starter" {
  run_wt new a --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  run_wt -C "$REPO/.worktrees/b" new c --no-tmux
  run_wt -C "$REPO/.worktrees/b" new d --no-tmux
  run_wt rm -y b
  grep -qF "moved up to a: c, d" <<<"$output"
  ls_json
  [ "$(lineage c parent)" = a ]
  [ "$(lineage d parent)" = a ]
  [ "$(lineage a lanes)" = c,d ]
  [ "$(lineage c starter)" = b ]
  # And again: removing a hands c and d (now handed to a) on to (main).
  run_wt rm -y a
  [ "$(decl c handed_to place)" = "(main)" ]
  [ "$(decl c handed_to from)" = a ]
}

@test "lineage: a lane only drawn under the removed place gets no write" {
  run_wt new a --no-tmux
  run_wt new x --no-tmux
  run_wt -C "$REPO/.worktrees/x" new e --no-tmux
  put_under e a
  ls_json
  [ "$(lineage e parent)" = a ]
  run_wt rm -y a
  ! grep -qF "moved up" <<<"$output" || false
  [ "$(decl e handed_to)" = - ]
  ls_json
  # Its `under` no longer holds, so it falls back to its starter.
  [ "$(lineage e parent)" = x ]
}

@test "lineage: a slug made again after removal inherits no lanes" {
  run_wt new a --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  run_wt rm -y a
  sleep 1
  run_wt new a2 --name a --no-tmux
  ls_json
  [ "$(lineage a lanes)" = - ]
  [ "$(lineage a started)" = - ]
  [ "$(lineage b parent)" = - ]
  run_wt rm -y a
  ! grep -qF "moved up" <<<"$output" || false
}

@test "lineage: place under — (main) moves a lane, a lane is refused, started_by survives" {
  run_wt new a --no-tmux
  run_wt new x --no-tmux
  run_wt -C "$REPO/.worktrees/a" new b --no-tmux
  run_wt -C "$REPO/.worktrees/a" place under b x
  [ "$status" -eq 1 ]
  grep -qF "a lane cannot move places in the tree" <<<"$output"
  [ "$(decl b under)" = - ]
  run_wt place under b x
  [ "$status" -eq 0 ]
  grep -qF "b is drawn under x now" <<<"$output"
  [ "$(decl b under place)" = x ]
  [ "$(decl b under by)" = owner ]
  [ "$(decl b started_by place)" = a ]
  ls_json
  [ "$(lineage b parent)" = x ]
  [ "$(lineage b reports_to)" = "(main)" ]
  [ "$(lineage b up)" = a ]
  run_wt place under b --top
  grep -qF "b is drawn at the top level now" <<<"$output"
  ls_json
  [ "$(lineage b parent)" = - ]
  run_wt place under b --reset
  grep -qF "b is drawn under a now" <<<"$output"
  [ "$(decl b under)" = - ]
  # A cycle is refused, and so is (main).
  run_wt place under a b
  [ "$status" -eq 1 ]
  grep -qF "cycle" <<<"$output"
  run_wt place under '(main)' a
  [ "$status" -eq 1 ]
}
