#!/usr/bin/env bats
load helpers/common
setup() {
  common_setup
  install_fake_cmd codex
  export WORKTREES_AI_CMD=codex
  export CODEX_HOME="$HOME/custom-codex"
  mkdir -p "$CODEX_HOME"
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
}

unq() { printf '%s' "$1" | sed "s/'\\\\''/'/g"; }
codex_cmd() { unq "$(tmux_pane0_cmd "repo-$1~agent~codex")"; }

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

@test "trust codex: allow, launch trusted for that run only, list, revoke — and Codex's config is never written" {
  printf '# untouched\n' > "$CODEX_HOME/config.toml"
  cp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
  # Not allowed: refused, and the words name the way (not --force as the fix).
  run_wt new trust-off --brief 'do the work'
  [ "$status" -eq 5 ]
  grep -qF 'Settings → Policy → Codex folder trust' <<<"$output"
  grep -qF 'worktrees trust codex' <<<"$output"
  grep -qF 'it will wait at that prompt' <<<"$output"
  run_wt trust codex
  [ "$status" -eq 0 ]
  grep -qF 'now allowed' <<<"$output"
  grep -qF 'codex = [' "$XDG_CONFIG_HOME/worktrees/config.toml"
  run_wt trust
  grep -qF "codex allowed: $(cd "$REPO" && pwd -P)" <<<"$output"
  # Allowed: the same lane launches, with ONE per-run -c in its argv.
  run_wt open trust-off --ai codex --no-attach
  [ "$status" -eq 0 ]
  c="$(codex_cmd trust-off)"
  grep -qF -- "-c 'projects={\"$(cd "$REPO" && pwd -P)\"={trust_level=\"trusted\"}}'" <<<"$c"
  cmp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
  # Revoked: refused again at the next launch.
  run_wt trust codex --revoke
  [ "$status" -eq 0 ]
  grep -qF 'no longer allowed' <<<"$output"
  run_wt close trust-off -y
  run_wt open trust-off --ai codex --no-attach
  [ "$status" -eq 5 ]
  run_wt trust
  ! grep -qF 'codex allowed' <<<"$output" || false
  cmp "$CODEX_HOME/config.toml" "$BATS_TEST_TMPDIR/before"
}

@test "trust codex: a repo cannot grant itself, and only pi, kimi and codex take trust" {
  run_wt new trust-self
  printf '[trust]\ncodex = ["/x"]\n' > "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 1 ]
  grep -qF 'may not be set by a project' <<<"$output"
  rm "$REPO/.worktrees.toml"
  run_wt trust opencode
  [ "$status" -eq 1 ]
  grep -qF 'pi, kimi and codex only' <<<"$output"
}

@test "doctor --codex: each allowed repo, and whether a launch would be trusted" {
  run_wt trust codex
  run_wt doctor --codex
  [ "$status" -eq 0 ]
  grep -qF 'this repo is ALLOWED' <<<"$output"
  grep -qF 'launch trusted' <<<"$output"
  run_wt doctor --codex --json
  grep -qF '"source":"allowance"' <<<"$output"
  run_wt trust codex --revoke
  run_wt doctor --codex
  grep -qF 'allowed none' <<<"$output"
}
