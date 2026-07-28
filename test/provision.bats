#!/usr/bin/env bats
# `worktrees provision` — port slots + `.worktree.env` (proposal §6), and the
# `[compose]` teardown `rm` performs. git and the filesystem are REAL here (only
# tmux and docker are faked), so the env file is read, sourced and diffed
# directly.
#
# §1.1 is why this file exists: a worktree that exists with no `.worktree.env` is
# not "portless", it is DESTRUCTIVE — the consumer's dev script reads the file's
# absence as "not a worktree" and `pkill -9`s the main checkout's whole stack.

load 'helpers/common'

setup() {
  common_setup
  make_secret_repo '.env'
}

wt()   { echo "$REPO/.worktrees/$1"; }
env_f() { echo "$REPO/.worktrees/$1/.worktree.env"; }
slot() { sed -n 's/^WORKTREE_SLOT=//p' "$(env_f "$1")" | tail -n1; }

# Ports chosen high and odd on purpose: these tests really do bind, so they must
# not squat on anything a developer plausibly runs. 39001/39251 differ by 250,
# which is not a multiple of the stride — so the parse-time collision lint passes.
write_ports_config() {   # $1 = max_slots (default 5)
  write_project_config '[ports]' "max_slots = ${1:-5}" 'stride = 100' \
                       'base = { API = 39001, PG = 39251 }'
}

# ── the wire format (§3) ─────────────────────────────────────────────────────

@test "new: writes .worktree.env, and every line of it is valid POSIX shell" {
  write_ports_config
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -f "$(env_f feat-x)" ]
  grep -qFx 'WORKTREE_SLOT=1' "$(env_f feat-x)"
  grep -qFx 'API_PORT=39101' "$(env_f feat-x)"
  grep -qFx 'PG_PORT=39351' "$(env_f feat-x)"
  # no [compose] section ⇒ no COMPOSE_PROJECT_NAME line
  ! grep -q 'COMPOSE_PROJECT_NAME' "$(env_f feat-x)"

  # THE contract: six consumers do `set -a; . .worktree.env`. If any emitted
  # line were not a plain assignment this fails (or executes something).
  run bash -c 'set -a; . "$1"; set +a; printf "%s %s %s" "$WORKTREE_SLOT" "$API_PORT" "$PG_PORT"' _ "$(env_f feat-x)"
  [ "$status" -eq 0 ]
  [ "$output" = "1 39101 39351" ]
}

@test "the port file never makes a worktree dirty — switch and rm work without --force" {
  # §8's trap: .worktree.env is untracked, so without ensure_excluded wt_dirty is
  # true forever, switch AND rm refuse, and the GUI (which never passes --force
  # to switch) is stuck with no remedy.
  write_ports_config
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -f "$(env_f feat-x)" ]
  grep -qFx '.worktree.env' "$REPO/.git/info/exclude"
  [ -z "$(git -C "$(wt feat-x)" status --porcelain)" ]

  run_wt switch feat-x other-branch
  [ "$status" -eq 0 ]
  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  [ ! -d "$(wt feat-x)" ]
}

# ── allocation (§6) ──────────────────────────────────────────────────────────

@test "slots count up, and rm frees the hole for the next worktree" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  [ "$(slot feat-a)" = 1 ]
  [ "$(slot feat-b)" = 2 ]

  # There is no release step: the directory goes, the env file goes with it, and
  # the next scan sees the hole.
  run_wt rm feat-a --branch -y
  [ "$status" -eq 0 ]
  run_wt new feat-c --no-tmux
  [ "$(slot feat-c)" = 1 ]
}

@test "provision: the sibling scan decides — a hand-placed slot is respected and holes are filled" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  printf 'WORKTREE_SLOT=3\n' > "$(env_f feat-b)"     # leaves 2 as the hole

  run_wt new feat-c --no-tmux
  [ "$status" -eq 0 ]
  [ "$(slot feat-c)" = 2 ]
}

@test "provision: a hand-edited WORKTREE_SLOT is adopted, never rewritten" {
  write_ports_config
  run_wt new feat-x --no-tmux
  printf 'export WORKTREE_SLOT=4\n' > "$(env_f feat-x)"   # what the old bash script wrote

  run_wt provision feat-x
  [ "$status" -eq 0 ]
  [ "$(slot feat-x)" = 4 ]
  grep -qFx 'API_PORT=39401' "$(env_f feat-x)"        # ports follow the adopted slot

  # …and re-running changes nothing at all
  cp "$(env_f feat-x)" "$BATS_TEST_TMPDIR/before"
  run_wt provision feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"already current"* ]]
  diff "$BATS_TEST_TMPDIR/before" "$(env_f feat-x)"
}

@test "provision: two concurrent runs get distinct slots" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  rm -f "$(env_f feat-a)" "$(env_f feat-b)"

  ( cd "$REPO" && "${RUN_BASH:-bash}" "$WT_BIN" provision feat-a >/dev/null 2>&1 ) &
  local a=$!
  ( cd "$REPO" && "${RUN_BASH:-bash}" "$WT_BIN" provision feat-b >/dev/null 2>&1 ) &
  local b=$!
  wait "$a"; wait "$b"

  [ -n "$(slot feat-a)" ]
  [ -n "$(slot feat-b)" ]
  [ "$(slot feat-a)" != "$(slot feat-b)" ]
  [ ! -e "$REPO/.worktrees.slots.lock" ]     # the lock never outlives the run
}

@test "provision: exhaustion is an error that says what ran out" {
  write_ports_config 1
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  [ "$(slot feat-a)" = 1 ]

  run_wt new feat-b --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *"no free slot in 1..=1"* ]]
  [[ "$output" == *"declared by other places"* ]]
  [ ! -e "$(env_f feat-b)" ]
  [ -d "$(wt feat-b)" ]   # creation is non-transactional: the worktree survives
}

# ── conflicts: refuse, never silently re-allocate (§6) ───────────────────────

@test "provision: a slot conflict is REFUSED and only --reallocate resolves it" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  printf 'WORKTREE_SLOT=1\n' > "$(env_f feat-b)"    # a restored backup, say

  run_wt provision feat-b
  [ "$status" -eq 1 ]
  [[ "$output" == *"claimed by BOTH"* ]]
  [[ "$output" == *"feat-a"* ]]
  [[ "$output" == *"feat-b"* ]]
  [[ "$output" == *"--reallocate"* ]]
  [ "$(cat "$(env_f feat-b)")" = "WORKTREE_SLOT=1" ]   # untouched by the refusal

  run_wt provision feat-b --reallocate
  [ "$status" -eq 0 ]
  [ "$(slot feat-b)" = 2 ]
  [ "$(slot feat-a)" = 1 ]
}

# ── doctor (§6 findings) ─────────────────────────────────────────────────────

@test "doctor: a place with no .worktree.env in a [ports] project is an ERROR" {
  run_wt new feat-x --no-tmux      # created BEFORE the config existed
  write_ports_config

  run_wt doctor feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"worse than none"* ]]
  [[ "$output" == *"worktrees provision feat-x"* ]]
  run_wt doctor --json
  [[ "$output" == *'"code":"no-slot"'* ]]

  run_wt provision feat-x
  [ "$status" -eq 0 ]
  run_wt doctor feat-x
  [ "$status" -eq 0 ]
}

@test "doctor: two places claiming one slot is a slot-conflict error" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  printf 'WORKTREE_SLOT=1\n' > "$(env_f feat-b)"

  run_wt doctor
  [ "$status" -eq 2 ]
  [[ "$output" == *"slot 1 is also claimed by"* ]]
  run_wt doctor --json
  [[ "$output" == *'"code":"slot-conflict"'* ]]

  run_wt provision feat-b --reallocate
  [ "$status" -eq 0 ]
  run_wt doctor
  [ "$status" -eq 0 ]
}

@test "doctor --config-only stays filesystem-free in a [ports] project" {
  write_ports_config
  run_wt new feat-x --no-tmux
  rm -f "$(env_f feat-x)"          # would be an Error for a normal doctor run
  run_wt doctor --config-only
  [ "$status" -eq 0 ]
  run_wt doctor
  [ "$status" -eq 2 ]
}

# ── [compose] (§5, §7) ───────────────────────────────────────────────────────

@test "rm: the compose stack comes down BEFORE the worktree does" {
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[ports]' 'base = { API = 39001 }' '' \
                       '[compose]' 'file = "docker-compose.worktree.yml"' \
                       'project = "{prefix}-wt-{slug}"'

  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  grep -qFx 'COMPOSE_PROJECT_NAME=repo-wt-feat-x' "$(env_f feat-x)"

  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  [ ! -d "$(wt feat-x)" ]
  # the tool assembled the argv itself — never a command string from the repo
  grep -q -- '-p repo-wt-feat-x down -v --remove-orphans' "$BATS_TEST_TMPDIR/docker.log"
  grep -q -- "-f $REPO/.worktrees/feat-x/docker-compose.worktree.yml" "$BATS_TEST_TMPDIR/docker.log"
}

@test "rm: EVERY declared compose file is passed, in order — volumes live in the BASE file" {
  # THE leak: `down -v` removes only the volumes declared in the files docker is
  # handed. A real project declares `postgres_data` in docker-compose.yml and
  # nothing in the worktree override, so passing the override alone left
  # <project>_postgres_data behind on every rm.
  printf 'services: {}\nvolumes: {postgres_data: {}}\n' > "$REPO/docker-compose.yml"
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[compose]' \
                       'files = ["docker-compose.yml", "docker-compose.worktree.yml"]' \
                       'project = "p-{slug}"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]

  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  # order is semantic to docker (a later -f overrides an earlier one)
  grep -q -- "-f $(wt feat-x)/docker-compose.yml -f $(wt feat-x)/docker-compose.worktree.yml -p p-feat-x down -v --remove-orphans" \
    "$BATS_TEST_TMPDIR/docker.log"
}

@test "rm: one missing file in the list skips the teardown rather than tearing down a subset" {
  # All-or-nothing: docker fails on a missing -f anyway, and a silently dropped
  # file is exactly the partial `down -v` this list exists to prevent.
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[compose]' \
                       'files = ["docker-compose.yml", "docker-compose.worktree.yml"]' \
                       'project = "p-{slug}"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]

  run_wt rm feat-x -y
  [ "$status" -eq 0 ]                       # removal still proceeds
  [[ "$output" == *"docker-compose.yml\` is missing here"* ]]
  [ ! -e "$BATS_TEST_TMPDIR/docker.log" ]   # nothing ran at all
  [ ! -d "$(wt feat-x)" ]
}

@test "doctor: a .worktree.env that predates a declared port is a finding, not clean" {
  # 11 of 15 worktrees in the audited consumer repo look exactly like this: a
  # file the old script wrote before WEBSITE joined the map. The slot is present
  # and unique, so every other check passes — and the site binds MAIN's 3002.
  write_project_config '[ports]' 'max_slots = 5' 'stride = 100' \
                       'base = { API = 39001, WEBSITE = 39251 }'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  grep -v '^WEBSITE_PORT=' "$(env_f feat-x)" > "$BATS_TEST_TMPDIR/env"
  cp "$BATS_TEST_TMPDIR/env" "$(env_f feat-x)"

  run_wt doctor feat-x
  [ "$status" -eq 0 ]                       # a Warn: the file is real, provision fixes it
  [[ "$output" == *"WEBSITE_PORT"* ]]
  [[ "$output" == *"MAIN checkout's port"* ]]
  [[ "$output" != *"clean"* ]]
  run_wt doctor --json
  [[ "$output" == *'"code":"missing-port"'* ]]

  # provision repairs it in place, keeping the slot the stack is running on
  run_wt provision feat-x
  [ "$status" -eq 0 ]
  [ "$(slot feat-x)" = 1 ]
  grep -qFx 'WEBSITE_PORT=39351' "$(env_f feat-x)"
  run_wt doctor feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"clean"* ]]
}

@test "rm: a missing docker is a warning, and the removal still happens" {
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[compose]' 'file = "docker-compose.worktree.yml"' 'project = "p-{slug}"'
  run_wt new feat-x --no-tmux

  # `rm -f "$SHIMS/docker"` would NOT do it: $SHIMS is prepended to the real
  # PATH, so removing the shim just uncovers the developer's own docker and this
  # test would quietly run `compose down -v` against their daemon while still
  # passing (removal proceeds either way). Same technique as install_no_tmux_path.
  install_no_docker_path
  run command -v docker
  [ "$status" -ne 0 ]                       # genuinely unfindable, not just unshimmed

  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  [ ! -d "$(wt feat-x)" ]
  # the intended branch really was taken: nothing was executed, and it was said
  [ ! -e "$BATS_TEST_TMPDIR/docker.log" ]
  [[ "$output" == *"compose teardown skipped"* ]]
  [[ "$output" == *"docker not available"* ]]
}

@test "rm: a hostile COMPOSE_PROJECT_NAME is sanitized before it reaches docker -p" {
  # §5: `.worktree.env` can be COMMITTED by a clone — info/exclude does nothing
  # for a file git already tracks — so the recorded name is repo-supplied text on
  # its way to an argv, and it goes through sanitize_prefix like the template.
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[ports]' 'base = { API = 39001 }' '' \
                       '[compose]' 'file = "docker-compose.worktree.yml"' \
                       'project = "{prefix}-wt-{slug}"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  printf 'WORKTREE_SLOT=1\nCOMPOSE_PROJECT_NAME="Evil Name; rm -rf /" # committed\n' \
    > "$(env_f feat-x)"

  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  grep -q -- '-p evil-name--rm--rf-- down -v --remove-orphans' "$BATS_TEST_TMPDIR/docker.log"
  ! grep -q 'Evil Name' "$BATS_TEST_TMPDIR/docker.log"
  ! grep -q ';' "$BATS_TEST_TMPDIR/docker.log"
}

@test "provision: the recorded COMPOSE_PROJECT_NAME survives a template change, and doctor says so" {
  printf 'services: {}\n' > "$REPO/docker-compose.worktree.yml"
  ( cd "$REPO" && git add -A && git commit -qm compose && git push -q origin main )
  write_project_config '[ports]' 'base = { API = 39001 }' '' \
                       '[compose]' 'file = "docker-compose.worktree.yml"' \
                       'project = "{prefix}-wt-{slug}"'
  run_wt new feat-x --no-tmux
  grep -qFx 'COMPOSE_PROJECT_NAME=repo-wt-feat-x' "$(env_f feat-x)"

  # The template changes under a place whose stack is already named the old way.
  write_project_config '[ports]' 'base = { API = 39001 }' '' \
                       '[compose]' 'file = "docker-compose.worktree.yml"' \
                       'project = "{prefix}-new-{slug}"'
  run_wt provision feat-x
  [ "$status" -eq 0 ]
  grep -qFx 'COMPOSE_PROJECT_NAME=repo-wt-feat-x' "$(env_f feat-x)"   # kept, not re-rendered

  # …and the drift is REPORTED, or `rm` would down a project nobody started. A
  # Warn, so exit stays 0 — nothing is broken, the user just has not reallocated.
  run_wt doctor feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"repo-wt-feat-x"* ]]
  [[ "$output" == *"repo-new-feat-x"* ]]
  run_wt doctor --json
  [[ "$output" == *'"code":"compose-drift"'* ]]

  # only --reallocate moves it, and then doctor has nothing to say
  run_wt provision feat-x --reallocate
  [ "$status" -eq 0 ]
  grep -qFx 'COMPOSE_PROJECT_NAME=repo-new-feat-x' "$(env_f feat-x)"
  run_wt doctor feat-x
  [ "$status" -eq 0 ]
  [[ "$output" != *"compose-drift"* ]]
  [[ "$output" != *"repo-wt-feat-x"* ]]

  # teardown uses the name the file records, whatever the template now says
  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  grep -q -- '-p repo-new-feat-x down -v' "$BATS_TEST_TMPDIR/docker.log"
}

# ── usage + absent config (§2.4) ─────────────────────────────────────────────

@test "provision: usage guards, no config, and a config with no [ports]" {
  run_wt provision
  [ "$status" -eq 1 ]
  [[ "$output" == *"needs a worktree"* ]]
  run_wt provision --bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --bogus"* ]]
  run_wt provision feat-x --all
  [ "$status" -eq 1 ]
  run_wt provision main
  [ "$status" -eq 1 ]

  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ ! -e "$(env_f feat-x)" ]          # no config ⇒ nothing written, nothing said
  run_wt provision feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"No .worktrees.toml"* ]]

  write_project_config '[[file]]' 'path = ".env"'
  run_wt provision feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"no [ports]"* ]]
  [ ! -e "$(env_f feat-x)" ]
}

@test "provision --all: covers every registered worktree, skipping stale dirs" {
  write_ports_config
  run_wt new feat-a --no-tmux
  run_wt new feat-b --no-tmux
  rm -f "$(env_f feat-a)" "$(env_f feat-b)"
  mkdir -p "$REPO/.worktrees/stale"

  run_wt provision --all
  [ "$status" -eq 0 ]
  [ -f "$(env_f feat-a)" ]
  [ -f "$(env_f feat-b)" ]
  [ "$(slot feat-a)" != "$(slot feat-b)" ]
  [ ! -e "$REPO/.worktrees/stale/.worktree.env" ]
}
