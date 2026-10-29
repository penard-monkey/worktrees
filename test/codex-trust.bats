#!/usr/bin/env bats
load helpers/common
setup() {
  common_setup
  install_fake_cmd codex
  export WORKTREES_AI_CMD=codex
  export CODEX_HOME="$HOME/custom-codex"
  mkdir -p "$CODEX_HOME"
}

@test "Codex trust: refuses before launch, keeps brief, never writes config" {
  printf '# untouched\n' > "$CODEX_HOME/config.toml"
  cp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
  run_wt new trust-new --brief 'do the work'
  [ "$status" -eq 5 ]
  grep -qF 'folder-trust prompt' <<<"$output"
  grep -qF "$REPO" <<<"$output"
  grep -qF "worktrees open 'trust-new' --ai codex" <<<"$output"
  [ -f "$REPO/.worktrees/trust-new/.planning/brief.md" ]
  ! tmux_session_exists 'repo-trust-new~agent~codex' || false
  cmp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
}

@test "Codex trust: root trust covers a linked worktree; an arbitrary parent does not" {
  printf '[projects."%s"]\ntrust_level = "trusted"\n' "$(dirname "$REPO")" > "$CODEX_HOME/config.toml"
  run_wt new trust-parent
  [ "$status" -eq 5 ]
  printf '[projects."%s"]\ntrust_level = "trusted"\n' "$REPO" > "$CODEX_HOME/config.toml"
  cp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
  run_wt open trust-parent --ai codex
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-trust-parent~agent~codex'
  cmp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
}

@test "Codex trust: the place's own level overrides the root's; an entry with none does not" {
  printf '[projects."%s"]\ntrust_level = "trusted"\n[projects."%s/.worktrees/trust-override"]\ntrust_level = "untrusted"\n' "$REPO" "$REPO" > "$CODEX_HOME/config.toml"
  run_wt new trust-override
  [ "$status" -eq 5 ]
  grep -qF "trust $REPO/.worktrees/trust-override yet" <<<"$output"
  printf '[projects."%s"]\ntrust_level = "trusted"\n[projects."%s/.worktrees/trust-override"]\n' "$REPO" "$REPO" > "$CODEX_HOME/config.toml"
  run_wt open trust-override --ai codex
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-trust-override~agent~codex'
}

@test "Codex trust: force launches without creating or changing config" {
  run_wt new trust-forced --force --brief 'do the work'
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-trust-forced~agent~codex'
  [ ! -e "$CODEX_HOME/config.toml" ]
}

@test "Codex trust: the launch-anyway retry it prints starts Codex, not the default harness" {
  unset WORKTREES_AI_CMD
  run_wt new trust-retry --ai codex --brief 'do the work'
  [ "$status" -eq 5 ]
  local retry="worktrees open trust-retry --ai codex --force --brief $REPO/.worktrees/trust-retry/.planning/brief.md"
  grep -qF -- "$retry" <<<"$output"
  # Run exactly that (the binary under test, from the repo).
  run bash -c 'cd "$1" && shift && "$@" --no-attach' _ "$REPO" "$WT_BIN" ${retry#worktrees }
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-trust-retry~agent~codex'
  ! tmux_session_exists 'repo-trust-retry' || false
}
