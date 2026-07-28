#!/usr/bin/env bats
# `worktrees relink` — re-apply .worktrees.toml's [[file]] plan to worktrees that
# already exist. git and the filesystem are REAL here (only tmux is faked), so
# `[ -L ]`, readlink and file contents are asserted directly.

load 'helpers/common'

setup() {
  common_setup
  make_secret_repo '.env' 'apps/mobile/google-services.json'
  printf 'SECRET=1\n' > "$REPO/.env"
}

wt() { echo "$REPO/.worktrees/$1"; }

# ── the case the design cares most about ─────────────────────────────────────

@test "relink: a worktree file that shadows a declared link is reported, not overwritten" {
  # The bash tool's `ln -sfn` deletes this file: exit 0, no output, no backup.
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  printf 'LOCAL — the only copy\n' > "$(wt feat-x)/.env"
  cp "$(wt feat-x)/.env" "$BATS_TEST_TMPDIR/shadow-before"

  write_project_config '[[file]]' 'path = ".env"'
  run_wt relink feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"shadows declared link"* ]]
  [ ! -L "$(wt feat-x)/.env" ]
  diff "$BATS_TEST_TMPDIR/shadow-before" "$(wt feat-x)/.env"

  # --force moves it aside — renamed, never deleted
  run_wt relink feat-x --force
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]
  diff "$BATS_TEST_TMPDIR/shadow-before" "$(wt feat-x)/.env.bak"
  [ "$(cat "$(wt feat-x)/.env")" = "SECRET=1" ]
}

# ── idempotence + incremental config changes ─────────────────────────────────

@test "relink: links the declared file with an ABSOLUTE target, and a second run is a no-op" {
  run_wt new feat-x --no-tmux
  write_project_config '[[file]]' 'path = ".env"'

  run_wt relink feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"linked .env"* ]]
  [ -L "$(wt feat-x)/.env" ]
  # absolute, and pointing at MAIN (sync-macs.sh depends on identical absolute paths)
  [ "$(readlink "$(wt feat-x)/.env")" = "$REPO/.env" ]
  [ "$(cat "$(wt feat-x)/.env")" = "SECRET=1" ]

  run_wt relink feat-x
  [ "$status" -eq 0 ]
  [[ "$output" != *"linked"* ]]
  [ "$(readlink "$(wt feat-x)/.env")" = "$REPO/.env" ]
}

@test "relink: after adding a config entry, only the NEW file is linked" {
  mkdir -p "$REPO/apps/mobile"; printf '{}\n' > "$REPO/apps/mobile/google-services.json"
  run_wt new feat-x --no-tmux
  write_project_config '[[file]]' 'path = ".env"'
  run_wt relink feat-x
  [ "$status" -eq 0 ]

  write_project_config '[[file]]' 'path = ".env"' '' '[[file]]' 'path = "apps/mobile/google-services.json"'
  run_wt relink feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"linked apps/mobile/google-services.json"* ]]
  [[ "$output" != *"linked .env"* ]]
  # the intermediate dirs were created one component at a time
  [ -d "$(wt feat-x)/apps/mobile" ]
  [ -L "$(wt feat-x)/apps/mobile/google-services.json" ]
}

@test "relink --all: covers every worktree, and creation links on its own" {
  write_project_config '[[file]]' 'path = ".env"'
  # `new` materializes at creation time — before the install pane could race it
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]

  run_wt new feat-y --no-tmux
  rm "$(wt feat-y)/.env"
  run_wt relink --all
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]
  [ -L "$(wt feat-y)/.env" ]
}

@test "relink: a stale (unregistered) dir is skipped by --all and refused by name" {
  write_project_config '[[file]]' 'path = ".env"'
  mkdir -p "$REPO/.worktrees/stale"
  run_wt relink --all
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/.worktrees/stale/.env" ]
  run_wt relink stale
  [ "$status" -eq 1 ]
  [[ "$output" == *"not a registered worktree"* ]]
}

# ── the loud failures (§2.5: the bug being fixed is silence) ─────────────────

@test "relink: a declared file absent from main WARNS instead of skipping silently" {
  run_wt new feat-x --no-tmux
  rm "$REPO/.env"
  write_project_config '[[file]]' 'path = ".env"'

  run_wt relink feat-x
  [ "$status" -eq 0 ]                     # a warning never fails the run
  [[ "$output" == *"absent from the main checkout"* ]]
  [ ! -e "$(wt feat-x)/.env" ]
}

@test "relink: a declared path that is not gitignored is an error and nothing is written" {
  printf 'PLAIN=1\n' > "$REPO/not-ignored.env"
  run_wt new feat-x --no-tmux
  write_project_config '[[file]]' 'path = "not-ignored.env"'

  run_wt relink feat-x
  [ "$status" -eq 2 ]
  [[ "$output" == *"not gitignored"* ]]
  [ ! -e "$(wt feat-x)/not-ignored.env" ]
}

@test "relink: an invalid .worktrees.toml is a guard failure with file:line" {
  run_wt new feat-x --no-tmux
  write_project_config '[[file]]' 'path = "/etc/passwd"'
  run_wt relink feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *".worktrees.toml:2: absolute path not allowed"* ]]
}

# ── `new`'s exit-code wiring (§8: report, keep the worktree, never roll back) ─

@test "new: a declared file absent from main warns and still exits 0" {
  rm "$REPO/.env"
  write_project_config '[[file]]' 'path = ".env"'

  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]                     # a warning never fails the run
  [[ "$output" == *"absent from the main checkout"* ]]
  [ -d "$(wt feat-x)" ]
  [ ! -e "$(wt feat-x)/.env" ]
}

@test "new: a declared path that is not gitignored exits 2 and KEEPS the worktree" {
  printf 'PLAIN=1\n' > "$REPO/not-ignored.env"
  write_project_config '[[file]]' 'path = "not-ignored.env"'

  run_wt new feat-x --no-tmux
  [ "$status" -eq 2 ]                     # Error findings, not a hard failure
  [[ "$output" == *"not gitignored"* ]]
  # loud guard: the worktree exists and stays registered, and nothing was linked
  [ -d "$(wt feat-x)" ]
  run git -C "$REPO" worktree list
  [[ "$output" == *"feat-x"* ]]
  [ ! -e "$(wt feat-x)/not-ignored.env" ]
}

@test "new: an invalid .worktrees.toml refuses BEFORE the worktree is created" {
  # §1.1 — a parse failure is knowable ahead of every side effect, so `new`
  # refuses rather than leaving a created-but-unprovisioned worktree behind.
  write_project_config '[[file]]' 'path = "/etc/passwd"'

  run_wt new feat-x --no-tmux
  [ "$status" -eq 1 ]
  [[ "$output" == *".worktrees.toml:2: absolute path not allowed"* ]]
  [ ! -e "$(wt feat-x)" ]
  run git -C "$REPO" worktree list
  [[ "$output" != *"feat-x"* ]]
}

@test "relink: usage guards" {
  run_wt relink
  [ "$status" -eq 1 ]
  [[ "$output" == *"needs a worktree"* ]]
  run_wt relink feat-x --all
  [ "$status" -eq 1 ]
  run_wt relink --bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --bogus"* ]]
  run_wt relink main
  [ "$status" -eq 1 ]
  [[ "$output" == *"SOURCE of the declared files"* ]]
}

# ── copies are seeds, not mirrors (§7) ───────────────────────────────────────

@test "relink: mode=copy seeds a real file and never clobbers a local rewrite" {
  write_project_config '[[file]]' 'path = ".env"' 'mode = "copy"'
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -f "$(wt feat-x)/.env" ] && [ ! -L "$(wt feat-x)/.env" ]
  [ "$(cat "$(wt feat-x)/.env")" = "SECRET=1" ]

  printf 'REWRITTEN BY deploy-local.sh\n' > "$(wt feat-x)/.env"
  run_wt relink feat-x
  [ "$status" -eq 0 ]
  [ "$(cat "$(wt feat-x)/.env")" = "REWRITTEN BY deploy-local.sh" ]

  run_wt relink feat-x --force
  [ "$status" -eq 0 ]
  [ "$(cat "$(wt feat-x)/.env")" = "SECRET=1" ]
  [ "$(cat "$(wt feat-x)/.env.bak")" = "REWRITTEN BY deploy-local.sh" ]
}

# ── absent config ⇒ byte-identical behavior (§2.4) ───────────────────────────

@test "no .worktrees.toml: 'new' prints nothing extra, and an empty config is byte-identical" {
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" != *"linked"* ]]
  printf '%s\n' "$output" > "$BATS_TEST_TMPDIR/before.txt"

  run_wt rm feat-a --branch -y
  [ "$status" -eq 0 ]
  : > "$REPO/.worktrees.toml"          # present, declaring nothing
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  printf '%s\n' "$output" > "$BATS_TEST_TMPDIR/after.txt"
  diff "$BATS_TEST_TMPDIR/before.txt" "$BATS_TEST_TMPDIR/after.txt"
}

@test "relink: with no .worktrees.toml it says so and exits 0" {
  run_wt new feat-x --no-tmux
  run_wt relink feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"No .worktrees.toml"* ]]
}
