#!/usr/bin/env bats
# pi as a harness — what worktrees COMPOSES for it. There is no fake pi agent
# here, only a shim that answers `--version`: the preflight is real (it runs
# `$SHELL -ic` exactly as a launch does), so a fake pi and node on the shim PATH
# are what it measures. What pi then does is docs/pi-manual-checks.md.

load 'helpers/common'

setup() {
  common_setup
  # pi and the node it runs on, as the pane's shell will find them.
  printf '#!/usr/bin/env bash\n[ "$1" = --version ] && { echo 0.99.1; exit 0; }\necho "$@" >> "%s/pi.log"\n' "$BATS_TEST_TMPDIR" > "$SHIMS/pi"
  printf '#!/usr/bin/env bash\necho 22.19.0\n' > "$SHIMS/node"
  chmod +x "$SHIMS/pi" "$SHIMS/node"
  export SHELL=/bin/sh
  # pi's config dir, empty: no declared model host, so nothing is probed.
  export PI_CODING_AGENT_DIR="$BATS_TEST_TMPDIR/pi-agent"; mkdir -p "$PI_CODING_AGENT_DIR"
  export XDG_CONFIG_HOME="$BATS_TEST_TMPDIR/xdg"
}

# Pane 0's command with tmux's outer quoting undone ('\'' → '), so a test can
# assert the inner shell's words as they are written in the -ic string.
# sed, not ${c//…/…}: bash 3.2 (macOS /bin/bash) keeps the quote characters of
# a pattern-substitution REPLACEMENT literally, turning ' into "'".
unq() { printf '%s' "$1" | sed "s/'\\\\''/'/g"; }
pi_cmd() { unq "$(tmux_pane0_cmd "repo-$1~agent~pi")"; }

@test "new --ai pi launches in the pi sidecar with session dir, trust, exact id, model and opener" {
  run_wt new feat-x --ai pi --model lm-studio/qwen3.6-27b --no-attach --no-spare --brief "do x"
  [ "$status" -eq 0 ]
  tmux_session_exists 'repo-feat-x~agent~pi'
  local c; c="$(pi_cmd feat-x)"
  [[ "$c" == *"pi --session-dir '$PI_CODING_AGENT_DIR/sessions/--"*"-feat-x--' --no-approve --session-id 'repo-feat-x-"*"-g1' --model 'lm-studio/qwen3.6-27b' 'Read .planning/brief.md and begin.'"* ]]
  [[ "$c" != *fake-ai* ]]
}

@test "new --ai pi with no model is refused before any session exists (never pi's own default)" {
  run_wt new feat-x --ai pi --no-attach --no-spare
  [ "$status" -eq 1 ]
  [[ "$output" == *"pi needs a model"* ]]
  ! tmux_session_exists 'repo-feat-x~agent~pi'
}

@test "a dead model host refuses with exit 5, and open --force launches WITH the brief's opener" {
  printf '{"providers":{"lm-dead":{"baseUrl":"http://127.0.0.1:1/v1","models":[{"id":"m"}]}}}' > "$PI_CODING_AGENT_DIR/models.json"
  run_wt new feat-x --ai pi --model lm-dead/m --no-attach --no-spare --brief "do x"
  [ "$status" -eq 5 ]
  [[ "$output" == *"pi was not started"* ]]
  [ -f "$REPO/.worktrees/feat-x/.planning/brief.md" ]
  ! tmux_session_exists 'repo-feat-x~agent~pi'
  run_wt open feat-x --ai pi --model lm-dead/m --force --no-attach --no-spare
  [ "$status" -eq 0 ]
  local c; c="$(pi_cmd feat-x)"
  [[ "$c" == *"--model 'lm-dead/m' 'Read .planning/brief.md and begin.'"* ]]
  [[ "$c" == *"-g1'"* ]]
}

@test "--model reaches claude as --model on pane 0; a bad or orphan --model is refused" {
  install_fake_cmd claude
  run_wt new feat-x --ai claude --model opus --no-attach --no-spare
  [ "$status" -eq 0 ]
  [[ "$(unq "$(tmux_pane0_cmd repo-feat-x)")" == *"claude --model 'opus' --name 'repo-feat-x'"* ]]
  run_wt new feat-y --ai claude --model "x; id" --no-attach
  [ "$status" -eq 1 ]
  [[ "$output" == *"may contain only"* ]]
  [ ! -d "$REPO/.worktrees/feat-y" ]
  run_wt new feat-z --model opus --no-attach   # WORKTREES_AI_CMD=fake-ai takes no model
  [ "$status" -eq 1 ]
  [[ "$output" == *"--model needs an agent that takes one"* ]]
  run_wt open feat-x --model
  [ "$status" -eq 1 ]
  [[ "$output" == *"--model needs a value"* ]]
}

@test "trust pi allows and revokes this repo in the user's config.toml, and a repo cannot set it" {
  run_wt trust pi
  [ "$status" -eq 0 ]
  [[ "$output" == *"now allowed"* ]]
  grep -q '^\[trust\]' "$XDG_CONFIG_HOME/worktrees/config.toml"
  run_wt new feat-x --ai pi --model lm-studio/qwen3.6-27b --no-attach --no-spare
  [ "$status" -eq 0 ]
  [[ "$(pi_cmd feat-x)" == *" --approve --session-id "* ]]
  run_wt trust pi --revoke
  [ "$status" -eq 0 ]
  [[ "$output" == *"no longer allowed"* ]]
  printf '[trust]\npi = ["/x"]\n' > "$REPO/.worktrees.toml"
  run_wt doctor
  [ "$status" -eq 1 ]
  [[ "$output" == *"trust may not be set by a project"* ]]
}

@test "a repo that has moved away can still be revoked by the string that is listed" {
  mkdir -p "$XDG_CONFIG_HOME/worktrees"
  printf '[trust]\npi = ["/nowhere/gone-repo"]\n' > "$XDG_CONFIG_HOME/worktrees/config.toml"
  run_wt trust pi /nowhere/gone-repo --revoke
  [ "$status" -eq 0 ]
  [[ "$output" == *"no longer allowed"* ]]
  ! grep -q gone-repo "$XDG_CONFIG_HOME/worktrees/config.toml"
}
