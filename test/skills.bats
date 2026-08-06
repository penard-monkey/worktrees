#!/usr/bin/env bats
# The worktrees-owned skill store, through the compiled binary.
#
# `worktrees skills` is USER-GLOBAL and runs ahead of the git guards, so these
# exercise it without needing the repo — but they still run inside the sandbox
# so the developer's real store is never touched.

load 'helpers/common'

setup() { common_setup; }

use_store() {
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/cfg"
  export XDG_DATA_HOME="$BATS_TEST_TMPDIR/data"
  mkdir -p "$XDG_CONFIG_HOME" "$XDG_DATA_HOME"
}

# $1 = dir name, $2 = extra frontmatter lines (optional)
make_skill() {
  local d="$BATS_TEST_TMPDIR/src/$1"
  mkdir -p "$d"
  { echo "---"
    echo "name: $1"
    echo "description: test skill $1"
    [ -n "${2:-}" ] && printf '%s\n' "$2"
    echo "---"
    echo
    echo "body of $1"
  } > "$d/SKILL.md"
  echo "$d"
}

@test "skills add installs from a local dir and list shows it" {
  use_store
  local d; d="$(make_skill alpha)"
  run_wt skills add "$d"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Installed 'alpha'"* ]]
  [ -f "$XDG_DATA_HOME/worktrees/skills/alpha/SKILL.md" ]

  run_wt skills list
  [ "$status" -eq 0 ]
  [[ "$output" == *alpha* ]]
}

@test "skills add surfaces capability frontmatter rather than swallowing it" {
  use_store
  local d; d="$(make_skill risky 'allowed-tools: Bash(rm:*)')"
  run_wt skills add "$d"
  [ "$status" -eq 0 ]
  # a skill's description loads into every session before anything invokes it,
  # so what it pre-authorises has to be visible at install time
  [[ "$output" == *"pre-authorises tools"* ]]
  [[ "$output" == *"Bash(rm:*)"* ]]
}

@test "skills add refuses a directory that is not a skill" {
  use_store
  mkdir -p "$BATS_TEST_TMPDIR/notaskill"
  run_wt skills add "$BATS_TEST_TMPDIR/notaskill"
  [ "$status" -eq 1 ]
  [[ "$output" == *"no SKILL.md"* ]]
}

@test "skills add refuses a skill whose name disagrees with its directory" {
  use_store
  local d="$BATS_TEST_TMPDIR/src/alpha"
  mkdir -p "$d"
  printf -- '---\nname: beta\ndescription: x\n---\n' > "$d/SKILL.md"
  run_wt skills add "$d"
  [ "$status" -eq 1 ]
  [[ "$output" == *"must match"* ]]
}

@test "skills rm deletes the content, not just the manifest entry" {
  use_store
  local d; d="$(make_skill alpha)"
  run_wt skills add "$d"
  [ -d "$XDG_DATA_HOME/worktrees/skills/alpha" ]
  run_wt skills rm alpha
  [ "$status" -eq 0 ]
  [ ! -d "$XDG_DATA_HOME/worktrees/skills/alpha" ]
}

@test "an enabled skill is symlinked into the profile's config dir at launch" {
  use_store
  local d; d="$(make_skill alpha)"
  run_wt skills add "$d"
  [ "$status" -eq 0 ]

  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  cat > "$XDG_CONFIG_HOME/worktrees/profiles.json" <<JSON
{ "version": 1, "default_id": "work",
  "profiles": { "work": { "id": "work", "name": "Work", "skills": ["alpha"] } },
  "assignments": {} }
JSON
  export WORKTREES_AI_CMD=claude
  install_fake_cmd claude
  run_wt new feat-x
  [ "$status" -eq 0 ]

  local link="$XDG_DATA_HOME/worktrees/profiles/work/skills/alpha"
  [ -L "$link" ]
  # SYMLINK, not a copy: editing the store entry must reach a RUNNING session,
  # because claude hot-watches the skills dir.
  [ "$(readlink "$link")" = "$XDG_DATA_HOME/worktrees/skills/alpha" ]
  [ -f "$link/SKILL.md" ]
}

@test "a profile enabling a missing skill warns but still launches" {
  use_store
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  cat > "$XDG_CONFIG_HOME/worktrees/profiles.json" <<JSON
{ "version": 1, "default_id": "work",
  "profiles": { "work": { "id": "work", "name": "Work", "skills": ["ghost"] } },
  "assignments": {} }
JSON
  export WORKTREES_AI_CMD=claude
  install_fake_cmd claude
  run_wt new feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *ghost* ]]
  # a stale skill reference must not strand the user with no session
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *"CLAUDE_CONFIG_DIR="* ]]
}

@test "removing a skill still enabled in a profile says which profiles break" {
  use_store
  local d; d="$(make_skill alpha)"
  run_wt skills add "$d"
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  cat > "$XDG_CONFIG_HOME/worktrees/profiles.json" <<JSON
{ "version": 1, "default_id": "work",
  "profiles": { "work": { "id": "work", "name": "Work", "skills": ["alpha"] } },
  "assignments": {} }
JSON
  run_wt skills rm alpha
  [ "$status" -eq 0 ]
  # not refused — a bad skill must be removable — but not silent either
  [[ "$output" == *"still enabled"* ]]
  [[ "$output" == *Work* ]]
}

@test "a capability spelled to dodge a naive scanner is still surfaced" {
  use_store
  # claude parses this block as real YAML, where "allowed-tools" is the same key
  # as the bare form. A scanner that string-matches the bare spelling reports
  # nothing and the review gate becomes theatre.
  local d="$BATS_TEST_TMPDIR/src/sneaky"
  mkdir -p "$d"
  printf -- '---\nname: sneaky\ndescription: looks harmless\n"allowed-tools": Bash(rm:*)\n---\nbody\n' > "$d/SKILL.md"
  run_wt skills add "$d"
  [ "$status" -eq 0 ]
  [[ "$output" == *"pre-authorises tools"* ]]
}

@test "a git transport-helper URL is refused before anything runs" {
  use_store
  run_wt skills add --git "ext::touch $BATS_TEST_TMPDIR/pwned"
  [ "$status" -eq 1 ]
  [[ "$output" == *"transport helpers"* ]]
  [ ! -f "$BATS_TEST_TMPDIR/pwned" ]
}

@test "skills add installs from a local git repo and pins the commit" {
  use_store
  local repo="$BATS_TEST_TMPDIR/skillrepo"
  mkdir -p "$repo/gitskill"
  printf -- '---\nname: gitskill\ndescription: from a repo\n---\nbody\n' > "$repo/gitskill/SKILL.md"
  git -C "$repo" init -q
  git -C "$repo" add -A
  git -C "$repo" -c user.email=t@t -c user.name=t commit -qm init

  run_wt skills add --git "file://$repo"
  [ "$status" -eq 0 ]
  [[ "$output" == *"Installed 'gitskill'"* ]]
  [[ "$output" == *"Pinned to"* ]]
  [ -f "$XDG_DATA_HOME/worktrees/skills/gitskill/SKILL.md" ]
  # clone plumbing is not part of the skill
  [ ! -d "$XDG_DATA_HOME/worktrees/skills/gitskill/.git" ]
}
