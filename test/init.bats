#!/usr/bin/env bats
# `worktrees init` — the suggestion flow (proposal §9). git and the filesystem
# are REAL here (only tmux/docker are faked), so gitignore rules, `git ls-files`
# and the emitted file are all asserted directly.
#
# The contract this file gates: init WRITES NOTHING without confirmation, what it
# writes parses, and the passive nudge on `new` appears once — not every time.

load 'helpers/common'

setup() {
  common_setup
  make_secret_repo '.env' 'apps/mobile/google-services.json'
  printf 'SECRET=1\n' > "$REPO/.env"
}

wt() { echo "$REPO/.worktrees/$1"; }

# ── the confirmation gate ────────────────────────────────────────────────────

@test "init: prints the config it would write and, answering n, writes nothing" {
  wt_answer n init
  [ "$status" -eq 0 ]
  [[ "$output" == *"[[file]]"* ]]
  [[ "$output" == *'path = ".env"'* ]]
  [[ "$output" == *"Nothing written"* ]]
  [ ! -e "$REPO/.worktrees.toml" ]
}

@test "init: EOF on the prompt declines — a non-interactive caller never gets a config" {
  run_wt init                       # run_wt feeds /dev/null
  [ "$status" -eq 0 ]
  [ ! -e "$REPO/.worktrees.toml" ]
}

@test "init: answering y writes a config that doctor can read" {
  wt_answer y init
  [ "$status" -eq 0 ]
  [[ "$output" == *"wrote $REPO/.worktrees.toml"* ]]
  [ -f "$REPO/.worktrees.toml" ]
  grep -q '^path = "\.env"$' "$REPO/.worktrees.toml"
  # the file's whole value is its comments (§8) — assert they survived
  grep -q '^# ── files' "$REPO/.worktrees.toml"

  # and the parser agrees with the emitter: doctor neither chokes nor warns
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"unknown key"* ]]
  [[ "$output" != *"not yet honored"* ]]

  # end to end: a worktree created now gets the declared file linked
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]
}

@test "init -y: skips the prompt" {
  run_wt init -y
  [ "$status" -eq 0 ]
  [ -f "$REPO/.worktrees.toml" ]
}

@test "init --print: prints ONLY the file (pipeable), and never writes" {
  run_wt init --print --force
  [ "$status" -eq 0 ]
  [[ "$output" == *"[[file]]"* ]]
  [ ! -e "$REPO/.worktrees.toml" ]
  # no banner, no commentary — `init --print > .worktrees.toml` must be valid
  [[ "$output" != *"═══"* ]]
  [[ "$output" != *"▸"* ]]
  printf '%s\n' "$output" > "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 0 ]
}

# ── guards ───────────────────────────────────────────────────────────────────

@test "init: refuses when .worktrees.toml exists, unless --force" {
  write_project_config '[[file]]' 'path = ".env"' '# hand written'
  run_wt init
  [ "$status" -eq 1 ]
  [[ "$output" == *"already exists"* ]]
  grep -q 'hand written' "$REPO/.worktrees.toml"

  # --force replaces it (and skips the prompt: it is the "I know" flag)
  run_wt init --force
  [ "$status" -eq 0 ]
  ! grep -q 'hand written' "$REPO/.worktrees.toml"
  grep -q '^path = "\.env"$' "$REPO/.worktrees.toml"
}

@test "init: --print still works when a config already exists" {
  write_project_config '[[file]]' 'path = ".env"'
  run_wt init --print
  [ "$status" -eq 0 ]
}

@test "init: unknown flag is a usage guard" {
  run_wt init --bogus
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --bogus"* ]]
  [ ! -e "$REPO/.worktrees.toml" ]
}

# ── a repo that qualifies for nothing ────────────────────────────────────────

@test "init: a repo with nothing to declare says so plainly and exits 0" {
  rm "$REPO/.env"                   # the only gitignored file on disk
  run_wt init
  [ "$status" -eq 0 ]
  [[ "$output" == *"Nothing to configure"* ]]
  [ ! -e "$REPO/.worktrees.toml" ]
}

@test "init: a TRACKED .env is not a candidate — three questions, not one" {
  # present + named like a credential, but committed: linking it would be
  # pointless, and it is not the silent-failure class this file is for.
  printf 'PUBLIC=1\n' > "$REPO/.env.example"
  git -C "$REPO" add -f .env.example
  git -C "$REPO" commit -qm example
  rm "$REPO/.env"
  run_wt init
  [ "$status" -eq 0 ]
  [[ "$output" == *"Nothing to configure"* ]]
}

# ── detection over a real repo ───────────────────────────────────────────────

@test "init: a credential file is flagged louder than a plain .env" {
  mkdir -p "$REPO/apps/mobile"; printf '{}\n' > "$REPO/apps/mobile/google-services.json"
  wt_answer n init
  [ "$status" -eq 0 ]
  [[ "$output" == *'path = "apps/mobile/google-services.json"'* ]]
  [[ "$output" == *"⚠ Credentials"* ]]
  [[ "$output" == *"is a credential"* ]]
}

@test "init: a compose file publishing host ports suggests [ports] with real numbers" {
  cat > "$REPO/docker-compose.yml" <<'YAML'
services:
  api:
    image: node
    ports:
      - "3000:3000"
  pg:
    image: postgres
    ports:
      - "5432:5432"
YAML
  run_wt init --print
  [ "$status" -eq 0 ]
  [[ "$output" == *"[ports]"* ]]
  [[ "$output" == *"API = 3000"* ]]
  [[ "$output" == *"PG = 5432"* ]]
  # no worktree override ⇒ no [compose] section
  [[ "$output" != *"[compose]"* ]]
}

@test "init: a worktree compose override suggests [compose] too, and it parses" {
  cat > "$REPO/docker-compose.worktree.yml" <<'YAML'
services:
  api:
    ports:
      - "3000:3000"
YAML
  run_wt init -y
  [ "$status" -eq 0 ]
  grep -q 'file = "docker-compose.worktree.yml"' "$REPO/.worktrees.toml"
  grep -q 'project = "{prefix}-wt-{slug}"' "$REPO/.worktrees.toml"

  # provision reads the [ports] it just wrote, with no complaints about the file
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  grep -q '^WORKTREE_SLOT=1$' "$(wt feat-x)/.worktree.env"
  run_wt doctor
  [ "$status" -eq 0 ]
}

@test "init: ports it cannot read confidently are commented out, never invented" {
  cat > "$REPO/docker-compose.worktree.yml" <<'YAML'
services:
  api:
    ports:
      - "${API_PORT}:3000"
YAML
  run_wt init -y
  [ "$status" -eq 0 ]
  [[ "$output" == *"commented out"* ]]
  grep -q '^# \[ports\]' "$REPO/.worktrees.toml"
  ! grep -q '^\[ports\]' "$REPO/.worktrees.toml"
  # nothing downstream trips over it
  run_wt doctor
  [ "$status" -eq 0 ]
}

@test "init: .worktree-prefix is folded in COMMENTED OUT (v3 defers honoring it)" {
  printf 'teamx\n' > "$REPO/.worktree-prefix"
  run_wt init -y
  [ "$status" -eq 0 ]
  grep -q '^# prefix = "teamx"$' "$REPO/.worktrees.toml"
  # an UNcommented [project] prefix would warn on every command from here on
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"not yet honored"* ]]
}

@test "init: existing worktrees that predate the config are named for relink" {
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ ! -e "$(wt feat-x)/.env" ]

  run_wt init -y
  [ "$status" -eq 0 ]
  [[ "$output" == *"feat-x"* ]]
  [[ "$output" == *"relink --all"* ]]

  run_wt relink --all
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]
}

@test "init: the walk skips .git, .worktrees and node_modules" {
  # a credential inside a skipped dir must NOT be suggested: linking a file out
  # of node_modules (or out of another worktree) is never what anyone wants.
  mkdir -p "$REPO/node_modules/pkg"
  printf '{}\n' > "$REPO/node_modules/pkg/google-services.json"
  printf 'node_modules/\n.env\napps/mobile/google-services.json\n' > "$REPO/.gitignore"
  ( cd "$REPO" && git add -A && git commit -qm ignore-node-modules )
  run_wt new feat-x --no-tmux
  printf 'LOCAL\n' > "$(wt feat-x)/.env"

  run_wt init --print
  [ "$status" -eq 0 ]
  [[ "$output" != *"node_modules"* ]]
  [[ "$output" != *".worktrees/"* ]]
}

# ── the passive nudge on `new` (§9) ──────────────────────────────────────────

@test "new: the credential hint appears once, and not the second time" {
  mkdir -p "$REPO/apps/mobile"; printf '{}\n' > "$REPO/apps/mobile/google-services.json"

  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" == *"untracked credential file"* ]]
  [[ "$output" == *"run 'worktrees init'"* ]]

  run_wt new feat-b --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" != *"untracked credential"* ]]
}

@test "new: the hint returns when the repo gains ANOTHER credential" {
  mkdir -p "$REPO/apps/mobile"; printf '{}\n' > "$REPO/apps/mobile/google-services.json"
  run_wt new feat-a --no-tmux
  [[ "$output" == *"1 untracked credential file"* ]]

  # keyed by WHAT was detected, not a boolean: the file that shows up after you
  # dismissed the hint is exactly the one nobody links.
  printf 'release\n' > "$REPO/app.keystore"
  printf '.env\napps/mobile/google-services.json\napp.keystore\n' > "$REPO/.gitignore"
  ( cd "$REPO" && git add -A && git commit -qm keystore )
  run_wt new feat-b --no-tmux
  [[ "$output" == *"2 untracked credential files"* ]]
}

@test "new: no hint once .worktrees.toml exists" {
  mkdir -p "$REPO/apps/mobile"; printf '{}\n' > "$REPO/apps/mobile/google-services.json"
  write_project_config '[[file]]' 'path = ".env"'
  run_wt new feat-a --no-tmux
  [ "$status" -eq 0 ]
  [[ "$output" != *"untracked credential"* ]]
  [[ "$output" != *"worktrees init"* ]]
}

@test "new: no hint in a repo with no credentials at all" {
  run_wt new feat-a --no-tmux       # only a gitignored .env exists
  [ "$status" -eq 0 ]
  [[ "$output" != *"worktrees init"* ]]
}
