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

# Same as run_wt, but STDOUT goes to a file — so `$output` is STDERR alone. The
# only way to assert what `init --print > .worktrees.toml` actually captures.
wt_stdout_to() {
  local out="$1"; shift
  run bash -c 'cd "$1" && o="$2" && shift 2 && "${RUN_BASH:-bash}" "$@" > "$o"' \
    _ "$REPO" "$out" "$WT_BIN" "$@" < /dev/null
}

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

@test "init: .worktree-prefix is transcribed into a LIVE [project] prefix, renaming nothing" {
  printf 'teamx\n' > "$REPO/.worktree-prefix"
  run_wt init -y
  [ "$status" -eq 0 ]
  grep -q '^\[project\]$' "$REPO/.worktrees.toml"
  grep -q '^prefix = "teamx"$' "$REPO/.worktrees.toml"
  # It is a TRANSCRIPTION: the legacy file still wins (§5), so the two agree,
  # doctor is clean, and the session name is exactly what it was before.
  run_wt doctor
  [ "$status" -eq 0 ]
  [[ "$output" != *"prefix"* ]]
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux_session_exists teamx-feat-x
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

# ── paths the config refuses, in a repo that is perfectly legal ──────────────

@test "init: a file the config cannot name is commented out, not a hard failure" {
  # `$` anywhere and a leading `~` make projcfg reject the WHOLE config, so
  # emitting either live turned a legal repo into exit 1 "please report this".
  mkdir -p "$REPO/apps\$1" "$REPO/~backup"
  printf 'S=1\n' > "$REPO/apps\$1/.env"
  printf 'S=1\n' > "$REPO/~backup/.env"

  run_wt init -y
  [ "$status" -eq 0 ]
  [[ "$output" != *"please report this"* ]]
  [[ "$output" == *"cannot be declared"* ]]
  # each is listed, commented out, with the parser's own reason
  grep -q '^# path = "apps\$1/\.env"$' "$REPO/.worktrees.toml"
  grep -q '^# path = "~backup/\.env"$' "$REPO/.worktrees.toml"
  grep -q 'is not expanded' "$REPO/.worktrees.toml"
  ! grep -q '^path = "apps' "$REPO/.worktrees.toml"
  # …and the ordinary .env beside them is still declared for real
  grep -q '^path = "\.env"$' "$REPO/.worktrees.toml"

  # the whole point: what it wrote is still readable by everything downstream
  run_wt doctor
  [ "$status" -eq 0 ]
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
  [ -L "$(wt feat-x)/.env" ]
}

@test "init: a credential under an accented directory is found (git C-quotes those)" {
  # `git check-ignore` prints "s\303\251crets/.env" under the default
  # core.quotePath, which matches no candidate — so the file used to fall out of
  # the filter with no error and no note. That silence is §1.2 itself.
  git -C "$REPO" config core.quotePath true
  mkdir -p "$REPO/sécrets"
  printf '{}\n' > "$REPO/sécrets/google-services.json"
  printf 'sécrets/\n' >> "$REPO/.gitignore"
  ( cd "$REPO" && git add -A && git commit -qm ignore-secrets )

  run_wt init --print
  [ "$status" -eq 0 ]
  [[ "$output" == *'path = "sécrets/google-services.json"'* ]]
}

# ── --print writes a FILE, and only a file ───────────────────────────────────

@test "init --print > file: a bare repo still gets a config doctor accepts" {
  rm "$REPO/.env"                   # nothing left to declare
  wt_stdout_to "$REPO/.worktrees.toml" init --print
  [ "$status" -eq 0 ]
  # the prose that used to land IN the config: not there, in any stream
  [[ "$output" != *"Nothing to configure"* ]]
  ! grep -q '▸' "$REPO/.worktrees.toml"
  grep -q '^# ── files' "$REPO/.worktrees.toml"

  run_wt doctor
  [ "$status" -eq 0 ]
  run_wt new feat-x --no-tmux
  [ "$status" -eq 0 ]
}

@test "init --print: the truncation warning reaches stderr, never the file" {
  mkdir -p "$REPO/d1/d2/d3/d4/d5/d6/d7/d8/d9"   # one level past MAX_DEPTH
  wt_stdout_to "$BATS_TEST_TMPDIR/out.toml" init --print
  [ "$status" -eq 0 ]
  [[ "$output" == *"search stopped early"* ]]   # reported, never swallowed
  ! grep -q 'stopped early' "$BATS_TEST_TMPDIR/out.toml"

  cp "$BATS_TEST_TMPDIR/out.toml" "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 0 ]
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
