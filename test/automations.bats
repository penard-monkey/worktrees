#!/usr/bin/env bats
# `worktrees automations` — definitions, the runner, and the ledger.
#
# The runner is the first thing in this tool that launches an AI headlessly from
# the CLI, so the cases below are mostly about what it must NOT do: run in a
# place directory (that would make every place read `active` the next morning —
# proposal §4.3), put the brief on a command line (ADR 0001), act on a proposal
# outside the closed set, or call a run "clean" because claude wrote nothing.

load 'helpers/common'

setup() {
  common_setup
  # The ledger is per-machine state keyed on XDG_STATE_HOME. common_setup UNSETS
  # it (so a developer's exported one cannot leak in); these tests need a real
  # one, because every run writes an entry and a lock under it.
  export XDG_STATE_HOME="$BATS_TEST_TMPDIR/state"
  install_fake_claude
  export WORKTREES_AI_CMD=claude
  # A fixed clock, so a run id is assertable. Same seam and same reason as
  # WORKTREES_STATUS_NOW.
  export WORKTREES_RUN_NOW=1790064131   # 2026-09-22T08:02:11Z
}

# The ledger directory for $REPO — named after a hash of the main root, so it is
# found rather than computed.
ledger_dir() {
  find "$XDG_STATE_HOME/worktrees/runs" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | head -n1
}

# A brief carrying a phrase that must never appear in claude's argv.
add_sweep() {
  run_wt automations add --name "Close-out sweep" \
    --brief "ZEBRA-QUOKKA-7 look at every worktree and say which are done."
  [ "$status" -eq 0 ]
}

write_findings() {
  FAKE_CLAUDE_FINDINGS="$BATS_TEST_TMPDIR/findings.json"
  export FAKE_CLAUDE_FINDINGS
  cat > "$FAKE_CLAUDE_FINDINGS"
}

# ── definitions ──────────────────────────────────────────────────────────────

@test "add writes the sidecar, ls lists it, the repo is taught to ignore it, rm removes it" {
  add_sweep
  [ -f "$REPO/.worktrees.automations.json" ]
  grep -q '/.worktrees.automations.json' "$REPO/.git/info/exclude"
  # ...and exactly one exclude header, however many sidecars appear.
  [ "$(grep -c '^# worktrees — per-machine app state' "$REPO/.git/info/exclude")" -eq 1 ]

  run_wt automations ls --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"slug":"close-out-sweep"'* ]]
  [[ "$output" == *'"kind":"manual"'* ]]
  [[ "$output" == *'"tier":"report"'* ]]
  [[ "$output" == *'"last_run":null'* ]]
  # The sidecar travels between machines, so the BRIEF is in it, not in a
  # per-machine file.
  grep -q 'ZEBRA-QUOKKA-7' "$REPO/.worktrees.automations.json"

  run_wt automations rm close-out-sweep
  [ "$status" -eq 0 ]
  run_wt automations ls --json
  [[ "$output" == *'"automations":[]'* ]]
}

@test "a duplicate slug, an empty brief and an impossible time are all refused" {
  add_sweep
  # A different NAME with the same derived slug is the collision people hit.
  run_wt automations add --name "close out   sweep" --brief "x"
  [ "$status" -eq 1 ]
  [[ "$output" == *"close-out-sweep"* ]]

  run_wt automations add --name "Empty" --brief "   "
  [ "$status" -eq 1 ]
  [[ "$output" == *"brief"* ]]

  run_wt automations add --name "Late" --brief "x" --daily 25:00
  [ "$status" -eq 1 ]
  [[ "$output" == *"24-hour"* ]]
  run_wt automations ls --json
  [[ "$output" != *'"slug":"late"'* ]]

  run_wt automations edit close-out-sweep --weekly funday 09:00
  [ "$status" -eq 1 ]
  run_wt automations edit close-out-sweep --weekly mon 09:00
  [ "$status" -eq 0 ]
  run_wt automations ls --json
  [[ "$output" == *'"kind":"weekly"'* ]]
}

# ── the runner ───────────────────────────────────────────────────────────────

@test "a run with findings exits 2 and lands in the ledger, with facts for every place" {
  run_wt new feat-x
  add_sweep
  write_findings <<'JSON'
{"findings":[
  {"slug":"feat-x","text":"Merged into main 19 days ago.",
   "proposals":[{"tool":"set_lifecycle","args":{"slug":"feat-x","lifecycle":"abandoned"}}]},
  {"slug":"(main)","text":"Three commits not pushed."}
]}
JSON

  run_wt automations run close-out-sweep --json
  [ "$status" -eq 2 ]
  entry="$(ledger_dir)/2026-09-22T08-02-11Z-close-out-sweep.json"
  [ -f "$entry" ]
  python3 - "$entry" <<'PY'
import json,sys
r = json.load(open(sys.argv[1]))
assert r["status"] == "findings", r["status"]
assert r["trigger"] == "manual"
assert sorted(r["places"]) == ["(main)", "feat-x"], r["places"]
assert len(r["findings"]) == 2, r["findings"]
assert r["findings"][0]["proposals"][0]["tool"] == "set_lifecycle"
assert r["dropped"] == [], r["dropped"]
assert r["report_md"].startswith("# fake report"), r["report_md"]
assert r["seconds"] is not None
assert r["turns"] is None, "claude -p does not report turns; a guess is not a measurement"
for slug in ("(main)", "feat-x"):
    f = r["facts"][slug]
    assert "verdict" in f, f
    assert f["live_agent"] is False, f
PY

  run_wt automations runs --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'2026-09-22T08-02-11Z-close-out-sweep'* ]]
  [[ "$output" == *'"status":"findings"'* ]]

  run_wt automations show 2026-09-22T08-02-11Z-close-out-sweep --json
  [ "$status" -eq 0 ]
  [[ "$output" == *'"automation":"close-out-sweep"'* ]]
}

@test "the run happens in the WORKTREE ROOT, not a place, and the brief never reaches argv" {
  run_wt new feat-x
  add_sweep
  write_findings <<'JSON'
{"findings":[]}
JSON

  run_wt automations run close-out-sweep
  [ "$status" -eq 0 ]   # no findings = clean

  # §4.3: a headless claude writes a transcript under its cwd, and a place's
  # activity max reads that directory — so cwd may be neither a worktree NOR
  # the main root (`(main)` is a place too). `.worktrees/` is owned by no place.
  grep -q "^PWD=$REPO/.worktrees\$" "$BATS_TEST_TMPDIR/claude.log"
  ! grep -q "^PWD=$REPO\$" "$BATS_TEST_TMPDIR/claude.log"
  ! grep -q "^PWD=$REPO/.worktrees/feat-x" "$BATS_TEST_TMPDIR/claude.log"

  # ADR 0001 / BRIEF_OPENER: the prose travels as a FILE, never as an argument.
  ! grep -q 'ZEBRA-QUOKKA-7' "$BATS_TEST_TMPDIR/claude.log"
  grep -q 'brief.md' "$BATS_TEST_TMPDIR/claude.log"
  grep -q '\-\-max-turns 12' "$BATS_TEST_TMPDIR/claude.log"
  # ...and it IS on disk, where the opener points.
  grep -q 'ZEBRA-QUOKKA-7' "$(ledger_dir)"/2026-09-22T08-02-11Z-close-out-sweep/brief.md
}

@test "a finding for an unknown place and a remove_worktree proposal are dropped and recorded" {
  run_wt new feat-x
  add_sweep
  write_findings <<'JSON'
{"findings":[
  {"slug":"ghost-town","text":"This place does not exist."},
  {"slug":"feat-x","text":"Done with.",
   "proposals":[
     {"tool":"remove_worktree","args":{"slug":"feat-x","confirm":true}},
     {"tool":"set_pin","args":{"slug":"feat-x","pinned":false}}
   ]}
]}
JSON

  run_wt automations run close-out-sweep --json
  [ "$status" -eq 2 ]
  python3 - "$(ledger_dir)/2026-09-22T08-02-11Z-close-out-sweep.json" <<'PY'
import json,sys
r = json.load(open(sys.argv[1]))
assert [f["slug"] for f in r["findings"]] == ["feat-x"], r["findings"]
assert [p["tool"] for p in r["findings"][0]["proposals"]] == ["set_pin"]
whats = " ".join(d["what"] for d in r["dropped"])
assert "ghost-town" in whats, r["dropped"]
assert "remove_worktree" in whats, r["dropped"]
# NEVER silent: the reason travels with the refusal.
assert any("removes a worktree" in d["why"] for d in r["dropped"]), r["dropped"]
PY
}

@test "a failing claude is a failed run, and the reason is the stderr tail" {
  add_sweep
  write_findings <<'JSON'
{"findings":[]}
JSON
  export FAKE_CLAUDE_RC=1

  run_wt automations run close-out-sweep
  [ "$status" -eq 1 ]
  python3 - "$(ledger_dir)/2026-09-22T08-02-11Z-close-out-sweep.json" <<'PY'
import json,sys
r = json.load(open(sys.argv[1]))
assert r["status"] == "failed", r["status"]
assert "DISTINCTIVE-STDERR-TAIL" in r["error"], r["error"]
PY
}

@test "no findings.json is a FAILED run naming the file, never a clean one" {
  add_sweep
  # No $FAKE_CLAUDE_FINDINGS: the shim writes only the report, which is exactly
  # the shape that would look like "clean" to a runner that trusted exit 0.
  run_wt automations run close-out-sweep
  [ "$status" -eq 1 ]
  [[ "$output" == *"findings.json"* ]]
  python3 - "$(ledger_dir)/2026-09-22T08-02-11Z-close-out-sweep.json" <<'PY'
import json,sys
r = json.load(open(sys.argv[1]))
assert r["status"] == "failed", r["status"]
assert "findings.json" in r["error"], r["error"]
assert r["report_md"] is not None, "the report it DID write is still kept"
PY
}

@test "apply makes the call a run proposed, and a bad index is an error" {
  run_wt new feat-x
  add_sweep
  write_findings <<'JSON'
{"findings":[
  {"slug":"feat-x","text":"Done with.",
   "proposals":[{"tool":"set_lifecycle","args":{"slug":"feat-x","lifecycle":"abandoned"}}]}
]}
JSON
  run_wt automations run close-out-sweep
  [ "$status" -eq 2 ]

  run_wt automations apply 2026-09-22T08-02-11Z-close-out-sweep 0 0
  [ "$status" -eq 0 ]
  grep -q '"lifecycle": "abandoned"' "$REPO/.worktrees.places.json"
  python3 - "$(ledger_dir)/2026-09-22T08-02-11Z-close-out-sweep.json" <<'PY'
import json,sys
r = json.load(open(sys.argv[1]))
assert len(r["actions"]) == 1, r["actions"]
assert r["actions"][0]["ok"] is True, r["actions"][0]
assert r["actions"][0]["tool"] == "set_lifecycle"
PY

  run_wt automations apply 2026-09-22T08-02-11Z-close-out-sweep 0 9
  [ "$status" -eq 1 ]
  run_wt automations apply no-such-run 0 0
  [ "$status" -eq 1 ]
}

@test "an ai_cmd that is not claude refuses the run and writes no ledger entry" {
  add_sweep
  export WORKTREES_AI_CMD=fake-ai

  run_wt automations run close-out-sweep
  [ "$status" -eq 1 ]
  [[ "$output" == *"claude"* ]]
  # Not one `failed` entry for a machine that was never going to work.
  [ -z "$(find "$XDG_STATE_HOME/worktrees/runs" -name '*.json' 2>/dev/null)" ]
}

@test "a stale lock is cleared; a live one makes the run say so and exit 0" {
  add_sweep
  write_findings <<'JSON'
{"findings":[]}
JSON
  run_wt automations run close-out-sweep
  [ "$status" -eq 0 ]
  ld="$(ledger_dir)"

  # A crashed run's lock — a pid nothing is using — must not wedge the
  # automation forever.
  echo 4000000 > "$ld/close-out-sweep.lock"
  run_wt automations run close-out-sweep
  [ "$status" -eq 0 ]
  [[ "$output" != *"already running"* ]]
  [ ! -f "$ld/close-out-sweep.lock" ]   # removed on every exit path

  # A LIVE holder: not an error, an answer.
  echo $$ > "$ld/close-out-sweep.lock"
  run_wt automations run close-out-sweep
  [ "$status" -eq 0 ]
  [[ "$output" == *"already running"* ]]
  [ -f "$ld/close-out-sweep.lock" ]     # somebody else's — not ours to remove
}

@test "a run does not move any place's activity stamps" {
  run_wt new feat-x
  add_sweep
  write_findings <<'JSON'
{"findings":[{"slug":"feat-x","text":"Something to say."}]}
JSON
  # There is no CLI verb for these stamps (they are the app's), so the fixture
  # is written directly — which is also the only way to prove a run leaves an
  # EXISTING value alone rather than merely failing to create one.
  cat > "$REPO/.worktrees.places.json" <<'JSON'
{"version":1,"places":{"feat-x":{"last_opened_epoch":1700000000,"last_worked_epoch":1700000001,"last_seen_epoch":1700000002}}}
JSON
  before="$(cat "$REPO/.worktrees.places.json")"

  run_wt automations run close-out-sweep
  [ "$status" -eq 2 ]

  # §4.3 / the `ai_status_report` rule: a report ABOUT a worktree is not work IN
  # it. If this ever changes, every place a sweep touched lights its afterglow
  # dot and reads as active, and the sweep blinds the signal it reports on.
  [ "$before" = "$(cat "$REPO/.worktrees.places.json")" ]
}
