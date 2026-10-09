#!/usr/bin/env bats
load 'helpers/common'
setup() { common_setup; }

legacy_state() {
  local socket="/tmp/tmux-$(id -u)/default"
  [ ! -d /private/tmp ] || socket="/private${socket}"
  local key
  key="$(printf '%s' "$socket" | cksum | cut -d ' ' -f1)"
  printf '%s.endpoints/legacy-%s' "$TMUX_STATE" "$key"
}

@test "routing: same-named lanes in same-basename clones close independently" {
  run_wt new same --no-install --no-attach
  [ "$status" -eq 0 ]
  local other="$BATS_TEST_TMPDIR/other/repo"
  mkdir -p "$(dirname "$other")"
  git clone -q "$ORIGIN" "$other"
  local original="$REPO"
  REPO="$other"
  run_wt new same --no-install --no-attach
  [ "$status" -eq 0 ]
  local key
  key="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["key"])' "$other/.git/worktrees-tmux/project.json")"
  [ -f "$TMUX_STATE.endpoints/$key/repo-same" ]
  REPO="$original"
  run_wt close same -y
  [ "$status" -eq 0 ]
  [ ! -f "$TMUX_STATE/repo-same" ]
  [ -f "$TMUX_STATE.endpoints/$key/repo-same" ]
  REPO="$other"
  run_wt ls --json
  [ "$status" -eq 0 ]
  printf '%s' "$output" | python3 -c 'import json,sys; p=next(p for p in json.load(sys.stdin)["places"] if p["slug"]=="same"); assert p["tmux_session"]["up"]; assert "wt-repo-" in p["tmux_session"]["server"]'
}

@test "routing: legacy lane is reused then moves only after close and reopen" {
  run_wt new same --no-tmux
  [ "$status" -eq 0 ]
  local legacy
  legacy="$(legacy_state)"; mkdir -p "$legacy"
  printf 'cwd=%s\n' "$REPO/.worktrees/same" > "$legacy/repo-same"
  printf 'cwd=%s\n' "$BATS_TEST_TMPDIR/elsewhere" > "$legacy/unmanaged"
  run_wt open same --no-attach
  [ "$status" -eq 0 ]
  [ ! -f "$TMUX_STATE/repo-same" ]
  [ -f "$legacy/repo-same" ]
  run_wt close same -y
  [ "$status" -eq 0 ]
  [ ! -f "$legacy/repo-same" ]
  [ -f "$legacy/unmanaged" ]
  run_wt open same --no-attach
  [ "$status" -eq 0 ]
  [ -f "$TMUX_STATE/repo-same" ]
  [ -f "$legacy/unmanaged" ]
  ! grep -q 'kill-server' "$TMUX_LOG"
}

@test "routing: duplicate endpoints refuse launch and close without mutation" {
  run_wt new same --no-tmux
  [ "$status" -eq 0 ]
  local legacy
  legacy="$(legacy_state)"; mkdir -p "$legacy"
  printf 'cwd=%s\n' "$REPO/.worktrees/same" > "$legacy/repo-same"
  cp "$legacy/repo-same" "$TMUX_STATE/repo-same"
  : > "$TMUX_LOG"
  run_wt open same --no-attach
  [ "$status" -ne 0 ]
  [[ "$output" == *ambig* ]]
  run_wt close same -y
  [ "$status" -ne 0 ]
  [ -f "$legacy/repo-same" ] && [ -f "$TMUX_STATE/repo-same" ]
  ! grep -E 'tmux (kill-session|new-session)' "$TMUX_LOG"
}

@test "routing: unreachable known project is unknown and refuses auto-relaunch" {
  run_wt new same --no-install --no-attach
  [ "$status" -eq 0 ]
  touch "$TMUX_STATE/.unreachable"
  : > "$TMUX_LOG"
  run_wt open same --no-attach
  [ "$status" -ne 0 ]
  [[ "$output" == *unreachable* ]]
  ! grep -q 'tmux new-session' "$TMUX_LOG"
  run_wt ls --json
  [ "$status" -eq 0 ]
  printf '%s' "$output" | python3 -c 'import json,sys; p=next(p for p in json.load(sys.stdin)["places"] if p["slug"]=="same"); assert p["tmux_session"]["error"]; assert p["lifecycle_effective"]=="unknown"'
}

@test "routing: legacy sidecar alone keeps the lane on legacy and close cleans it" {
  run_wt new same --no-tmux
  [ "$status" -eq 0 ]
  local legacy
  legacy="$(legacy_state)"; mkdir -p "$legacy"
  printf 'cwd=%s\n' "$REPO/.worktrees/same" > "$legacy/repo-same~term~2"
  run_wt open same --no-attach
  [ "$status" -eq 0 ]
  [ -f "$legacy/repo-same" ]
  [ ! -f "$TMUX_STATE/repo-same" ]
  run_wt close same -y
  [ "$status" -eq 0 ]
  [ ! -f "$legacy/repo-same~term~2" ]
}

@test "routing: cross-project MCP send addresses the target's repeated percent-zero pane" {
  install_fake_cmd codex
  export WORKTREES_PREFIX=shared
  run_wt new same --no-install --no-attach --ai codex
  [ "$status" -eq 0 ]
  local original="$REPO" other="$BATS_TEST_TMPDIR/other/repo"
  mkdir -p "$(dirname "$other")"
  git clone -q "$ORIGIN" "$other"
  REPO="$other"
  run_wt new same --no-install --no-attach --ai codex
  [ "$status" -eq 0 ]
  local key
  key="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["key"])' "$other/.git/worktrees-tmux/project.json")"
  local target="$TMUX_STATE.endpoints/$key" name='shared-same~agent~codex'
  local fixtures="$BATS_TEST_DIRNAME/../crates/worktrees-core/tests/fixtures/codex-send"
  printf codex > "$TMUX_STATE/$name.cmd"
  printf codex > "$target/$name.cmd"
  printf '%s' "$name" > "$TMUX_STATE/.pane-%0"
  printf '%s' "$name" > "$target/.pane-%0"
  cp "$fixtures/typed.txt" "$TMUX_STATE/$name.screen"
  cp "$fixtures/typed.txt" "$target/$name.screen"
  cp "$fixtures/empty.txt" "$target/.after-enter"
  REPO="$original"
  run_wt projects add "$original"
  [ "$status" -eq 0 ]
  run_wt projects rename repo alpha
  [ "$status" -eq 0 ]
  run_wt projects add "$other"
  [ "$status" -eq 0 ]
  mkdir -p "$HOME/.config/worktrees"
  printf 'cross_project = "full"\n' > "$HOME/.config/worktrees/config.toml"
  printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"send","arguments":{"slug":"repo:same","text":"hi"}}}' > "$BATS_TEST_TMPDIR/in.jsonl"
  run bash -c "cd '$REPO' && '$WT_BIN' mcp --mutations < '$BATS_TEST_TMPDIR/in.jsonl'"
  [[ "$output" == *'\"delivered\": true'* ]]
  cmp "$fixtures/typed.txt" "$TMUX_STATE/$name.screen"
  cmp "$fixtures/empty.txt" "$target/$name.screen"
  grep -F -- "-L $key -N|send-keys" "$TMUX_LOG.globals"
}
