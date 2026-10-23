#!/usr/bin/env bats
# `worktrees run --slot` / `worktrees slots` — machine-wide slots for heavy jobs
# (`worktrees_core::slots`).
#
# Real processes, real flock, real kill -9: the whole point is what the kernel
# does when a holder dies, and a mock would only test our belief about that.
# The heaviest thing any test here runs is `sleep`.
#
# State lands under $HOME/.local/state (common_setup isolates HOME and unsets
# XDG_STATE_HOME), so nothing here can touch the developer's real slots.

load 'helpers/common'

setup() { common_setup; }

teardown() {
  # Anything a test left running (holders, orphaned children) — by pid only.
  local p
  for p in ${KILL_ME:-}; do kill -9 "$p" 2>/dev/null || true; done
}

# Start `worktrees run …` in the background from $REPO; its pid in $BG.
bg_run() {
  ( cd "$REPO" && exec "$WT_BIN" run "$@" ) >/dev/null 2>&1 3>&- &
  BG=$!
  disown "$BG"   # no "Killed" job line when teardown kill -9s it
  KILL_ME="${KILL_ME:-} $BG"
}

cfg() { mkdir -p "$HOME/.config/worktrees"; printf '%s\n' "$@" > "$HOME/.config/worktrees/config.toml"; }
cfg_file() { printf '%s' "$HOME/.config/worktrees/config.toml"; }

# What this machine looks like to slots.rs: $1 cores, $2 GiB.
machine() {
  printf '{"cores":%s,"ram_bytes":%s}' "$1" "$(( $2 * 1073741824 ))" > "$BATS_TEST_TMPDIR/machine.json"
  export WORKTREES_SLOTS_PROBE="$BATS_TEST_TMPDIR/machine.json"
}

# Wait (≤5s) until `worktrees slots --json` shows $1 holders of slot $2.
wait_held() {
  local n="$1" name="$2" i got
  for i in $(seq 1 50); do
    got="$("$WT_BIN" slots --json | python3 -c '
import sys, json
d = json.load(sys.stdin)
print(sum(len(s["holders"]) for s in d["slots"] if s["name"] == sys.argv[1]))' "$name")"
    [ "$got" = "$n" ] && return 0
    sleep 0.1
  done
  echo "slot $name: wanted $n holders, saw $got" >&2
  return 1
}

@test "run: never more than N at once, and every job still runs" {
  local log="$BATS_TEST_TMPDIR/overlap.log" i
  : > "$log"
  for i in 1 2 3 4 5; do
    ( cd "$REPO" && "$WT_BIN" run --slot t --slots 2 -- sh -c 'echo s >> "$1"; sleep 1; echo e >> "$1"' _ "$log" ) >/dev/null 2>&1 3>&- &
  done
  wait
  [ "$(grep -c '^s$' "$log")" = 5 ]
  [ "$(grep -c '^e$' "$log")" = 5 ]
  # The log is the ground truth of what overlapped: +1 per start, -1 per end.
  local most
  most="$(awk '/^s$/{n++; if(n>m)m=n} /^e$/{n--} END{print m}' "$log")"
  [ "$most" = 2 ]
}

@test "run: a SIGKILLed holder frees its slot at once" {
  bg_run --slot k --slots 1 -- sleep 30
  local holder="$BG"
  wait_held 1 k
  local child; child="$(pgrep -P "$holder" || true)"
  KILL_ME="$KILL_ME $child"
  local got="$BATS_TEST_TMPDIR/got"
  bg_run --slot k --slots 1 -- sh -c "echo yes > '$got'"
  sleep 1
  [ ! -e "$got" ]                    # still waiting behind the holder
  kill -9 "$holder"
  local i
  for i in $(seq 1 30); do [ -e "$got" ] && break; sleep 0.1; done
  [ "$(cat "$got")" = yes ]
}

@test "run: the child's exit code comes back and the slot is freed" {
  run_wt run --slot e -- sh -c 'exit 3'
  [ "$status" -eq 3 ]
  run "$WT_BIN" slots
  [ "$status" -eq 0 ]
  grep -qxF "no slots in use" <<<"$output"
  ! grep -qF "  holds  " <<<"$output" || false
}

@test "run: a daemon the job leaves behind does not keep the slot" {
  local pidf="$BATS_TEST_TMPDIR/daemon.pid"
  run_wt run --slot d --slots 1 -- sh -c "sleep 30 >/dev/null 2>&1 & echo \$! > '$pidf'"
  [ "$status" -eq 0 ]
  KILL_ME="${KILL_ME:-} $(cat "$pidf")"
  kill -0 "$(cat "$pidf")"           # the daemon is alive…
  run_wt run --slot d --slots 1 --timeout 2 -- true
  [ "$status" -eq 0 ]                # …and the slot is not its
}

@test "run: a waiter says who holds the slot, and --timeout gives up without running" {
  bg_run --slot w --slots 1 -- sleep 30
  KILL_ME="$KILL_ME $(sleep 0.3; pgrep -P "$BG" || true)"
  wait_held 1 w
  local ran="$BATS_TEST_TMPDIR/ran"
  run_wt run --slot w --slots 1 --timeout 1 -- touch "$ran"
  [ "$status" -eq 75 ]
  [[ "$output" == *"waiting for slot w (1/1 held by repo:(main) "* ]]
  [[ "$output" == *"slot w still busy after 1s; nothing ran"* ]]
  [ ! -e "$ran" ]
}

@test "slots: --json names the holder, its place and its command" {
  bg_run --slot j -- sleep 30
  KILL_ME="$KILL_ME $(sleep 0.3; pgrep -P "$BG" || true)"
  wait_held 1 j
  run "$WT_BIN" slots --json
  [ "$status" -eq 0 ]
  printf '%s' "$output" | python3 -c '
import sys, json
s = [s for s in json.load(sys.stdin)["slots"] if s["name"] == "j"][0]
h = s["holders"][0]
assert s["capacity"] == 2 and s["source"] == "built-in" and not s["explicit"], s
assert h["place"] == "repo:(main)" and h["cmd"] == "sleep 30" and h["slots"] == 2, h
'
}

@test "run: capacity comes from the user's [slots]" {
  mkdir -p "$HOME/.config/worktrees"
  printf '[slots]\nc = 1\n' > "$HOME/.config/worktrees/config.toml"
  bg_run --slot c -- sleep 30
  KILL_ME="$KILL_ME $(sleep 0.3; pgrep -P "$BG" || true)"
  wait_held 1 c
  run_wt run --slot c --timeout 1 -- true
  [ "$status" -eq 75 ]
  [[ "$output" == *"(1/1 held by"* ]]
}

@test "a repo cannot set [slots] (ADR 0001)" {
  write_project_config '[slots]' 'gate = 9'
  run_wt doctor
  [ "$status" -ne 0 ]
  [[ "$output" == *"slots may not be set by a project"* ]]
}

@test "run: bad usage is exit 2 and runs nothing" {
  run_wt run -- true
  [ "$status" -eq 2 ]
  [[ "$output" == *"--slot <name> is required"* ]]
  run_wt run --slot ../x -- true
  [ "$status" -eq 2 ]
}

@test "run: a dead waiter's file does not block a free slot, even if its pid lives" {
  # Review of #485, B1: a waiter killed while waiting leaves wait/<pid>.json,
  # and the pid can be reused. Plant one naming THIS shell (alive, older,
  # holding no lock): a free slot must still be taken at once.
  local d="$HOME/.local/state/worktrees/slots/z/wait"
  mkdir -p "$d"
  printf '{"pid":%s,"place":"ghost","cmd":"old gate","since_ms":1,"slots":1}' "$$" > "$d/$$.json"
  run_wt run --slot z --slots 1 --timeout 2 -- echo RAN
  [ "$status" -eq 0 ]
  [ "$output" = RAN ]
  [ ! -e "$d/$$.json" ]
}

@test "run: a second ^C reaches a child that ignores the first" {
  bg_run --slot i -- sh -c 'trap "" INT; sleep 30'
  local run_pid="$BG"
  wait_held 1 i
  kill -INT "$run_pid"
  sleep 0.5
  kill -0 "$run_pid"                 # the first INT is the group's business
  kill -INT "$run_pid"
  local n
  for n in $(seq 1 30); do kill -0 "$run_pid" 2>/dev/null || break; sleep 0.1; done
  ! kill -0 "$run_pid" 2>/dev/null || false   # the second became a TERM; run is gone
  run "$WT_BIN" slots
  [ "$status" -eq 0 ]
  grep -qxF "no slots in use" <<<"$output"
  ! grep -qF "  holds  " <<<"$output" || false
}

@test "run and slots work outside any repository" {
  local out="$BATS_TEST_TMPDIR/nowhere"; mkdir -p "$out"
  run bash -c "cd '$out' && '$WT_BIN' run --slot o -- echo hi"
  [ "$status" -eq 0 ]
  [ "$output" = hi ]
}

@test "slots: the status view gives each slot's capacity and where it comes from" {
  run "$WT_BIN" slots
  [ "$status" -eq 0 ]
  grep -qE '^gate +0/2 held  \(built-in\)$' <<<"$output"
  grep -qE '^default +2 for any other name  \(built-in\)$' <<<"$output"
  machine 10 36
  cfg '[slots]' 'e2e = 1'
  run "$WT_BIN" slots
  grep -qE '^gate +0/3 held  \(machine \(10 cores, 36 GiB\)\)$' <<<"$output"
  grep -qE '^e2e +0/1 held  \(config \[slots\] e2e\)$' <<<"$output"
  grep -qE '^default +3 for any other name  \(machine \(10 cores, 36 GiB\)\)$' <<<"$output"
  cfg '[slots]' 'default = 5'
  run "$WT_BIN" slots
  grep -qE '^gate +0/5 held  \(config \[slots\] default\)$' <<<"$output"
}

@test "slots set/unset: writes [slots], keeps every other line, and says what changed" {
  cfg '# my settings' 'prefix = "wt"  # short' '' '[quota]' 'gate = false' '' '[slots]' 'gate = 2  # tuned'
  machine 10 36
  run_wt slots set gate 4
  [ "$status" -eq 0 ]
  grep -qF "gate: 4 (config [slots] gate), was 2 (config [slots] gate)" <<<"$output"
  [ "$(cat "$(cfg_file)")" = "$(printf '%s\n' '# my settings' 'prefix = "wt"  # short' '' '[quota]' 'gate = false' '' '[slots]' 'gate = 4  # tuned')" ]
  run_wt slots set gate 4
  grep -qF "gate: already 4" <<<"$output"
  run_wt slots set default 1
  [ "$status" -eq 0 ]
  grep -qxF "default = 1" "$(cfg_file)"
  run_wt slots unset gate
  [ "$status" -eq 0 ]
  grep -qF "gate: 1 (config [slots] default), was 4 (config [slots] gate)" <<<"$output"
  ! grep -qE '^gate = [0-9]' "$(cfg_file)" || false
  grep -qxF "gate = false" "$(cfg_file)"   # [quota]'s gate is not [slots]'
  run_wt slots unset default
  grep -qF "default: 3 (machine (10 cores, 36 GiB)), was 1 (config [slots] default)" <<<"$output"
  run_wt slots unset default
  [ "$status" -eq 0 ]
  grep -qF "[slots] default is not set; it is 3" <<<"$output"
}

@test "slots set refuses a bad name or capacity and writes nothing" {
  cfg '[slots]' 'gate = 2'
  local before; before="$(cat "$(cfg_file)")"
  local bad
  for bad in "gate 0" "gate 65" "gate many" "gate -1" "Gate 2" "../x 2" "gate" "gate 2 3"; do
    # shellcheck disable=SC2086
    run_wt slots set $bad
    [ "$status" -eq 1 ]
    [ "$(cat "$(cfg_file)")" = "$before" ]
  done
  run_wt slots set gate 0
  grep -qF "capacity must be a whole number from 1 to 64 (got 0)" <<<"$output"
  run_wt slots set Gate 2
  grep -qF "slot name \`Gate\` must be lowercase letters, digits and dashes" <<<"$output"
  run_wt slots unset
  [ "$status" -eq 1 ]
  grep -qF "usage: worktrees slots" <<<"$output"
}

@test "slots set refuses a layout it cannot edit safely" {
  cfg 'slots = { gate = 2 }'
  run_wt slots set gate 3
  [ "$status" -eq 1 ]
  grep -qF "edit it by hand" <<<"$output"
  [ "$(cat "$(cfg_file)")" = "slots = { gate = 2 }" ]
}

@test "run: with nothing in [slots], capacity is the machine's — and an explicit entry beats it" {
  machine 1 1          # the smallest machine: one slot
  bg_run --slot g -- sleep 30
  KILL_ME="$KILL_ME $(sleep 0.3; pgrep -P "$BG" || true)"
  wait_held 1 g
  run_wt run --slot g --timeout 1 -- true
  [ "$status" -eq 75 ]
  grep -qF "(1/1 held by" <<<"$output"
  cfg '[slots]' 'g = 2'
  run_wt run --slot g --timeout 1 -- echo RAN
  [ "$status" -eq 0 ]
  grep -qxF RAN <<<"$output"
}
