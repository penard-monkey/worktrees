#!/usr/bin/env bats
# The project registry (`~/.config/worktrees/projects.json`), through the
# compiled binary — cross-project proposal P0.
#
# `worktrees projects` is USER-GLOBAL and runs ahead of the git guard, like
# `skills`. HOME is the test's own (common_setup), so the registry here is a
# throwaway one.

load 'helpers/common'

setup() { common_setup; }

REG() { echo "$HOME/.config/worktrees/projects.json"; }

# A second repo beside $REPO, with the SAME basename, so the seeded names
# collide the way two clones of one project do.
make_twin() {
  TWIN="$BATS_TEST_TMPDIR/other/repo"
  mkdir -p "$TWIN"
  git init -q "$TWIN"
  ( cd "$TWIN" && echo hi > README.md && git add -A && git commit -qm init )
}

@test "projects runs outside a repository and starts empty" {
  run_wt -C "$BATS_TEST_TMPDIR" projects
  [ "$status" -eq 0 ]
  [[ "$output" == *"No registered projects"* ]]
  [ ! -e "$(REG)" ]
}

@test "projects add registers the main root from a subdirectory, once" {
  mkdir -p "$REPO/sub/dir"
  run_wt -C "$REPO/sub/dir" projects add
  [ "$status" -eq 0 ]
  [[ "$output" == *"registered 'repo'"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"already registered as 'repo'"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects ls --json
  [ "$status" -eq 0 ]
  [ "$(echo "$output" | jq '.projects | length')" -eq 1 ]
  # The canonical main root — macOS's /var → /private/var included.
  [ "$(echo "$output" | jq -r '.projects[0].root')" = "$(cd "$REPO" && pwd -P)" ]
}

@test "two repos with the same basename get distinct names" {
  make_twin
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$TWIN"
  [ "$status" -eq 0 ]
  [[ "$output" == *"registered 'repo-2'"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects ls --json
  [ "$(echo "$output" | jq -r '[.projects[].name] | join(",")')" = "repo,repo-2" ]
}

@test "a repo's prefix file seeds its name; the global WORKTREES_PREFIX does not" {
  echo "shopfront" > "$REPO/.worktree-prefix"
  WORKTREES_PREFIX=everything run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  [ "$status" -eq 0 ]
  [[ "$output" == *"registered 'shopfront'"* ]]
}

@test "rename refuses a taken or malformed name; private toggles" {
  make_twin
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$TWIN"
  run_wt -C "$BATS_TEST_TMPDIR" projects rename repo-2 repo
  [ "$status" -eq 1 ]
  [[ "$output" == *"already taken"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects rename repo-2 "a:b"
  [ "$status" -eq 1 ]
  run_wt -C "$BATS_TEST_TMPDIR" projects rename repo-2 twin
  [ "$status" -eq 0 ]
  run_wt -C "$BATS_TEST_TMPDIR" projects private twin on
  [ "$status" -eq 0 ]
  run_wt -C "$BATS_TEST_TMPDIR" projects ls
  [[ "$output" == *"twin"*"(private)"* ]]
  run_wt -C "$BATS_TEST_TMPDIR" projects private twin off
  run_wt -C "$BATS_TEST_TMPDIR" projects ls --json
  [ "$(echo "$output" | jq '[.projects[] | select(.private)] | length')" -eq 0 ]
}

@test "rm unregisters only; the repo is untouched" {
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing on disk changed"* ]]
  [ -d "$REPO/.git" ]
  run_wt -C "$BATS_TEST_TMPDIR" projects rm repo
  [ "$status" -eq 1 ]
  [[ "$output" == *"no registered project"* ]]
}

@test "add refuses a directory that is not a repository" {
  mkdir -p "$BATS_TEST_TMPDIR/plain"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$BATS_TEST_TMPDIR/plain"
  [ "$status" -eq 1 ]
  [ ! -e "$(REG)" ]
}

@test "a .worktrees.toml cannot set cross_project" {
  write_project_config 'cross_project = "full"'
  run_wt doctor
  [ "$status" -ne 0 ]
  [[ "$output" == *"cross_project may not be set by a project"* ]]
}

@test "doctor flags a registered project nested inside another" {
  local inner="$REPO/vendor/inner"
  mkdir -p "$inner"
  git init -q "$inner"
  ( cd "$inner" && echo x > f && git add -A && git commit -qm init )
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$REPO"
  run_wt -C "$BATS_TEST_TMPDIR" projects add "$inner"
  run_wt doctor --json
  [ "$(echo "$output" | jq '[.findings[] | select(.code == "nested-project")] | length')" -eq 1 ]
}
