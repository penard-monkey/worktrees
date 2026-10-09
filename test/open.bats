#!/usr/bin/env bats
# cmd_open — reopen/attach an existing worktree's tmux session.

load 'helpers/common'

setup() { common_setup; }

# Make a worktree WITHOUT a tmux session, so `open` is what creates/attaches it.
make_worktree() { # args passed through to `new`
  run_wt new "$@" --no-tmux
  [ "$status" -eq 0 ]
}

# Count real session files in the fake-tmux registry (excludes *.cmd and .last).
session_count() {
  local n=0 f
  for f in "$TMUX_STATE"/*; do
    [ -f "$f" ] || continue
    case "$f" in *.cmd|*/.last) continue ;; esac
    n=$((n + 1))
  done
  echo "$n"
}

@test "open by slug creates the session cwd'd in the worktree with pane0 = AI" {
  make_worktree feat-x
  run_wt open feat-x
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-x
  grep -qx "cwd=$REPO/.worktrees/feat-x" "$TMUX_STATE/repo-feat-x"
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *fake-ai* ]]
}

@test "open (default) is single pane: no spare shell" {
  make_worktree feat-sp
  run_wt open feat-sp
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-sp
  [[ "$(tmux_pane0_cmd repo-feat-sp)" == *fake-ai* ]]
  [ -z "$(tmux_pane1_cmd repo-feat-sp)" ]   # no split-window → no pane 1
  ! grep -q 'split-window' "$TMUX_LOG"
}

@test "open --spare splits a spare shell into pane 1" {
  make_worktree feat-ss
  run_wt open feat-ss --spare
  [ "$status" -eq 0 ]
  [ -n "$(tmux_pane1_cmd repo-feat-ss)" ]   # split-window ran → pane 1 exists
}

@test "open --spare on a live session says it applies only at creation, and splits nothing" {
  make_worktree feat-ru
  run_wt open feat-ru --no-attach
  [ "$status" -eq 0 ]
  : > "$TMUX_LOG"
  run_wt open feat-ru --spare --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"reusing it."* ]]
  [[ "$output" == *"--spare applies only when the session is created"* ]]
  ! grep -q 'split-window' "$TMUX_LOG"
  [ -z "$(tmux_pane1_cmd repo-feat-ru)" ]
}

@test "open on a live session without --spare says nothing about it" {
  make_worktree feat-rn
  run_wt open feat-rn --no-attach
  run_wt open feat-rn --no-attach
  [ "$status" -eq 0 ]
  [[ "$output" == *"reusing it."* ]]
  [[ "$output" != *"--spare"* ]]
}

@test "open --no-spare is still accepted (the app's path): single pane" {
  make_worktree feat-ns
  run_wt open feat-ns --no-spare
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-ns
  [ -z "$(tmux_pane1_cmd repo-feat-ns)" ]
}

@test "open by BRANCH resolves the differently-named holder worktree" {
  make_worktree feat/foo --name topic
  local repo_phys; repo_phys="$REPO"   # already physical (make_repo)
  run_wt -C "$repo_phys" open feat/foo
  [ "$status" -eq 0 ]
  [[ "$output" == *"lives in worktree 'topic'"* ]]
  tmux_session_exists repo-topic
  grep -qx "cwd=$repo_phys/.worktrees/topic" "$TMUX_STATE/repo-topic"
}

@test "open reuses an AI session already living in the worktree under another name" {
  make_worktree feat-x
  # Simulate an AI pane already running here under a foreign session name.
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-x" > "$TMUX_STATE/other-sess"
  echo fake-ai > "$TMUX_STATE/other-sess.cmd"
  run_wt_tty open feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"already in this worktree"* ]]
  # No second session minted — registry still holds exactly the pre-made one.
  ! tmux_session_exists repo-feat-x
  [ "$(session_count)" -eq 1 ]
  tmux_session_exists other-sess
  grep -q "attach -t =other-sess" "$TMUX_LOG"
}

@test "open nonexistent name errors with the Create-it hint" {
  run_wt open nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"No worktree 'nope'"* ]]
  [[ "$output" == *"Create it"* ]]
}

@test "open -r appends the resume arg to pane0" {
  make_worktree feat-x
  run_wt open -r feat-x
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd repo-feat-x)" == *"fake-ai -r"* ]]
}

@test "open --ai overrides WORKTREES_AI_CMD from the environment" {
  install_fake_cmd codex
  make_worktree feat-x
  run_wt open --ai codex feat-x
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd 'repo-feat-x~agent~codex')" == *codex* ]]
  [[ "$(tmux_pane0_cmd 'repo-feat-x~agent~codex')" == *"codex -c forced_login_method=chatgpt"* ]]
  [[ "$(tmux_pane0_cmd 'repo-feat-x~agent~codex')" != *fake-ai* ]]
}

@test "open switches providers and leaves only one live agent in the worktree" {
  install_fake_cmd claude
  install_fake_cmd codex
  make_worktree feat-both
  run_wt open --no-attach --ai claude feat-both
  [ "$status" -eq 0 ]
  run_wt open --no-attach --ai codex feat-both
  [ "$status" -eq 0 ]
  ! tmux_session_exists repo-feat-both
  tmux_session_exists 'repo-feat-both~agent~codex'
  [[ "$(tmux_pane0_cmd 'repo-feat-both~agent~codex')" == *codex* ]]
  run_wt open --no-attach --ai claude feat-both
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-both
  ! tmux_session_exists 'repo-feat-both~agent~codex'
  [[ "$(tmux_pane0_cmd repo-feat-both)" == *claude* ]]
}

@test "a legacy canonical Codex session is closed before Claude starts" {
  install_fake_cmd claude
  install_fake_cmd codex
  make_worktree feat-legacy
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-legacy" > "$TMUX_STATE/repo-feat-legacy"
  echo codex > "$TMUX_STATE/repo-feat-legacy.cmd"
  run_wt open --no-attach --ai claude feat-legacy
  [ "$status" -eq 0 ]
  ! tmux_session_exists repo-feat-legacy
  tmux_session_exists 'repo-feat-legacy~agent~claude'
  [[ "$(tmux_pane0_cmd 'repo-feat-legacy~agent~claude')" == *claude* ]]
  run_wt close --ai claude feat-legacy
  [ "$status" -eq 0 ]
  ! tmux_session_exists repo-feat-legacy
  ! tmux_session_exists 'repo-feat-legacy~agent~claude'
}

@test "provider switch refuses to kill an adopted agent session" {
  install_fake_cmd claude
  make_worktree feat-adopted
  printf 'cwd=%s\ncmd0=x\n' "$REPO/.worktrees/feat-adopted" > "$TMUX_STATE/personal"
  echo codex > "$TMUX_STATE/personal.cmd"
  run_wt open --no-attach --ai claude feat-adopted
  [ "$status" -eq 1 ]
  [[ "$output" == *"adopted tmux session 'personal'"* ]]
  tmux_session_exists personal
  ! tmux_session_exists repo-feat-adopted
  ! tmux_session_exists 'repo-feat-adopted~agent~claude'
}

@test "concurrent provider opens leave only one live agent" {
  install_fake_cmd claude
  install_fake_cmd codex
  make_worktree feat-race
  ( cd "$REPO" && "$WT_BIN" open --no-attach --ai claude feat-race > "$BATS_TEST_TMPDIR/claude.log" 2>&1 ) &
  local claude_pid=$!
  ( cd "$REPO" && "$WT_BIN" open --no-attach --ai codex feat-race > "$BATS_TEST_TMPDIR/codex.log" 2>&1 ) &
  local codex_pid=$!
  wait "$claude_pid"
  wait "$codex_pid"
  if tmux_session_exists repo-feat-race; then
    ! tmux_session_exists 'repo-feat-race~agent~codex'
  else
    tmux_session_exists 'repo-feat-race~agent~codex'
  fi
}

@test "open --ai without a value errors" {
  make_worktree feat-x
  run_wt open feat-x --ai
  [ "$status" -eq 1 ]
  [[ "$output" == *"--ai needs a value"* ]]
}

@test "open --no-attach creates the session but never attaches" {
  make_worktree feat-x
  : > "$TMUX_LOG"
  run_wt open --no-attach feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"Session ready (detached)"* ]]
  tmux_session_exists repo-feat-x
  ! grep -E ' (attach|attach-session|switch-client) ' "$TMUX_LOG"
  ! grep -E '(attach|switch-client)' "$TMUX_LOG"
}

@test "open hard-errors when tmux is absent (unlike new)" {
  make_worktree feat-x
  install_no_tmux_path   # portable tmux-less PATH (ubuntu's tmux is /usr/bin/tmux)
  run_wt open feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"tmux not found"* ]]
}

# An agent's Bash tool has no tty but DOES inherit $TMUX from its pane, so a
# bare switch-client moved the APP's embedded client onto the new lane.
@test "open with no tty never attaches or switches, even inside tmux" {
  make_worktree feat-x
  : > "$TMUX_LOG"
  TMUX=/tmp/fake,1,0 run_wt open feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"Session ready (detached)"* ]]
  ! grep -E '(attach|switch-client)' "$TMUX_LOG"
}

@test "new with no tty never attaches or switches, even inside tmux" {
  : > "$TMUX_LOG"
  TMUX=/tmp/fake,1,0 run_wt new feat-nt --no-install
  [ "$status" -eq 0 ]
  [[ "$output" == *"Session ready (detached)"* ]]
  ! grep -E '(attach|switch-client)' "$TMUX_LOG"
}

@test "open on a tty inside tmux still switches the client" {
  make_worktree feat-x
  : > "$TMUX_LOG"
  run_wt ls --json
  local socket
  socket="$(printf '%s' "$output" | python3 -c 'import json,sys; print(json.load(sys.stdin)["places"][0]["tmux_session"]["server"])')"
  TMUX="$socket,1,0" run_wt_tty open feat-x
  [ "$status" -eq 0 ]
  grep -q 'switch-client -t =repo-feat-x' "$TMUX_LOG"
}

@test "open on a tty outside tmux still attaches" {
  make_worktree feat-x
  : > "$TMUX_LOG"
  run_wt_tty open feat-x
  [ "$status" -eq 0 ]
  grep -q 'attach -t =repo-feat-x' "$TMUX_LOG"
}

@test "codex resume names only this place's user thread despite newer sibling and subagent rollouts" {
  install_fake_cmd codex
  make_worktree feat-isolation
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"
  local day="$CODEX_HOME/sessions/2026/10/05"
  mkdir -p "$day"
  printf '{"type":"session_meta","payload":{"id":"11111111-1111-4111-8111-111111111111","cwd":"%s","source":"cli"}}\n' "$REPO/.worktrees/feat-isolation" > "$day/rollout-a.jsonl"
  printf '{"type":"session_meta","payload":{"id":"22222222-2222-4222-8222-222222222222","cwd":"%s","source":"cli"}}\n{"type":"event_msg","payload":{"type":"task_started"}}\n' "$REPO" > "$day/rollout-z.jsonl"
  printf '{"type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"%s","source":{"subagent":"guardian"}}}\n' "$REPO/.worktrees/feat-isolation" > "$day/rollout-y.jsonl"
  WORKTREES_AI_RESUME_ARG='resume --last --all' run_wt open --ai codex -r feat-isolation
  [ "$status" -eq 0 ]
  local cmd="$(tmux_pane0_cmd 'repo-feat-isolation~agent~codex')"
  [[ "$cmd" == *resume*11111111-1111-4111-8111-111111111111* ]]
  [[ "$cmd" != *--last* ]]
  [[ "$cmd" != *22222222* ]]
  [[ "$cmd" != *33333333* ]]
}

@test "codex may_resume refuses a place whose only rollout is a subagent" {
  install_fake_cmd codex
  make_worktree feat-isolation
  export CODEX_HOME="$BATS_TEST_TMPDIR/codex"
  local day="$CODEX_HOME/sessions/2026/10/05"
  mkdir -p "$day"
  printf '{"type":"session_meta","payload":{"id":"33333333-3333-4333-8333-333333333333","cwd":"%s","source":{"subagent":"guardian"}}}\n' "$REPO/.worktrees/feat-isolation" > "$day/rollout-y.jsonl"
  run_wt open --ai codex -r feat-isolation
  [ "$status" -eq 0 ]
  [[ "$(tmux_pane0_cmd 'repo-feat-isolation~agent~codex')" != *resume* ]]
}

# tmux -N ("do not start the server"): a client that may start one, finding the
# socket refused or missing, unlinks it and starts a SECOND server — orphaning
# every session in the first. Only `new-session` may leave it off.
@test "open passes -N to every tmux call except new-session" {
  make_worktree feat-x
  : > "$TMUX_LOG.globals"
  run_wt_tty open feat-x --spare
  [ "$status" -eq 0 ]
  grep -Eq '^-L wt-[^ ]+\|new-session$' "$TMUX_LOG.globals"
  for sub in list-sessions list-panes split-window select-pane set-option attach; do
    grep -Eq -- "^-[LS] [^ ]+ -N\|$sub$" "$TMUX_LOG.globals" || { echo "no -N on $sub"; cat "$TMUX_LOG.globals"; false; }
  done
  # Nothing but new-session runs without it.
  [ "$(grep -v ' -N|' "$TMUX_LOG.globals" | grep -vcE '^-L wt-[^ ]+\|new-session$')" -eq 0 ]
  # And the subcommand log reads exactly as it did before.
  grep -q 'attach -t =repo-feat-x' "$TMUX_LOG"
  grep -q '^tmux new-session -d -s repo-feat-x' "$TMUX_LOG"
}

@test "open on a tmux older than 3.2 never passes -N (it would reject it)" {
  make_worktree feat-x
  : > "$TMUX_LOG.globals"
  FAKE_TMUX_VERSION=3.1c run_wt_tty open feat-x --spare
  [ "$status" -eq 0 ]
  tmux_session_exists repo-feat-x
  grep -Eq '^-L wt-[^ ]+\|attach$' "$TMUX_LOG.globals"
  ! grep -q -- '-N' "$TMUX_LOG.globals"
}
