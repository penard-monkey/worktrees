#!/usr/bin/env bats
# Tests for `worktrees close` (cmd_close / close_one) — kill the tmux session,
# keep the worktree. The inverse of `open`.

load 'helpers/common'

setup() {
  common_setup
}

# Create a registered worktree (with a tmux session) and assert it worked.
mk_wt() {
  run_wt new "$@"
  [ "$status" -eq 0 ]
}

@test "close: kills the session exact-match; worktree, dir, and branch all stay" {
  mk_wt feat-x
  tmux_session_exists repo-feat-x

  run_wt close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-feat-x"* ]]
  [[ "$output" == *"worktree kept"* ]]
  grep -q "kill-session -t =repo-feat-x" "$TMUX_LOG"   # exact-match =target (prefix-match guard)
  ! tmux_session_exists repo-feat-x
  [ -d "$REPO/.worktrees/feat-x" ]
  run git -C "$REPO" worktree list
  [[ "$output" == *"feat-x"* ]]
  git -C "$REPO" show-ref --verify --quiet refs/heads/feat-x
}

@test "close: no live session → friendly no-op, exit 0" {
  mk_wt feat-x
  run_wt close feat-x
  [ "$status" -eq 0 ]

  run_wt close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"nothing to close"* ]]
}

@test "close: resolves a branch name to its holder worktree" {
  mk_wt feat-x --name box
  tmux_session_exists repo-box

  run_wt close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"lives in worktree 'box'"* ]]
  ! tmux_session_exists repo-box
  [ -d "$REPO/.worktrees/box" ]
}

@test "close: 'main' closes the main checkout's session, checkout untouched" {
  # Simulate a live main session (the app opens these; session name = prefix-(main)).
  printf 'cwd=%s\ncmd0=bash\n' "$REPO" > "$TMUX_STATE/repo-(main)"

  run_wt close main
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-(main)"* ]]
  [[ "$output" == *"checkout untouched"* ]]
  ! tmux_session_exists "repo-(main)"
  [ -f "$REPO/README.md" ]
  run git -C "$REPO" status --porcelain
  [ -z "$output" ]
}

@test "close: multiple names — both sessions closed, both worktrees stay" {
  mk_wt feat-a
  mk_wt feat-b

  run_wt close feat-a feat-b
  [ "$status" -eq 0 ]
  ! tmux_session_exists repo-feat-a
  ! tmux_session_exists repo-feat-b
  [ -d "$REPO/.worktrees/feat-a" ]
  [ -d "$REPO/.worktrees/feat-b" ]
}

@test "close: one bad name among good — good closed, exit 1" {
  mk_wt feat-a

  run_wt close feat-a nope
  [ "$status" -eq 1 ]
  [[ "$output" == *"closed tmux repo-feat-a"* ]]
  [[ "$output" == *"No worktree 'nope'"* ]]
}

@test "close: no args errors with exit 1" {
  run_wt close
  [ "$status" -eq 1 ]
  [[ "$output" == *"close needs"* ]]
}

@test "close: unknown flag errors with exit 1" {
  run_wt close --nope feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"Unknown flag: --nope"* ]]
}

@test "close: a worktree literally named 'main' wins over the main-checkout alias" {
  # regression: the alias used to short-circuit BEFORE the dir check and killed
  # the main checkout's session instead of the worktree's.
  mk_wt feat-y --name main
  tmux_session_exists repo-main
  printf 'cwd=%s\ncmd0=bash\n' "$REPO" > "$TMUX_STATE/repo-(main)"   # live main-checkout session

  run_wt close main
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-main"* ]]
  ! tmux_session_exists repo-main
  tmux_session_exists "repo-(main)"          # checkout session untouched
  [ -d "$REPO/.worktrees/main" ]
}

@test "close: '(main)' always targets the main checkout even when .worktrees/main exists" {
  mk_wt feat-y --name main
  printf 'cwd=%s\ncmd0=bash\n' "$REPO" > "$TMUX_STATE/repo-(main)"

  run_wt close '(main)'
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-(main)"* ]]
  ! tmux_session_exists "repo-(main)"
  tmux_session_exists repo-main              # the worktree's session untouched
}

@test "close: empty / '.' / '..' names are rejected like rm" {
  run_wt close ""
  [ "$status" -eq 1 ]
  [[ "$output" == *"Invalid worktree name"* ]]
  run_wt close .
  [ "$status" -eq 1 ]
  run_wt close ..
  [ "$status" -eq 1 ]
}

@test "close: no tmux → error exit 1" {
  mk_wt feat-x
  install_no_tmux_path

  run_wt close feat-x
  [ "$status" -eq 1 ]
  [[ "$output" == *"tmux not found"* ]]
}

@test "close: dotted slug maps '.'→'-' in the session name, dir keeps the dot" {
  mk_wt v1.2
  tmux_session_exists repo-v1-2

  run_wt close v1.2
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-v1-2"* ]]
  grep -q "kill-session -t =repo-v1-2" "$TMUX_LOG"
  ! tmux_session_exists repo-v1-2
  [ -d "$REPO/.worktrees/v1.2" ]
}

# An ADOPTED session is found by pane cwd alone, so it may be someone's personal
# session that merely has ONE pane here (or in a nested unrelated repo under this
# dir) — and tmux kills whole sessions, every window in them. So close names it
# and asks, exactly like `rm` does. A canonical-name close still asks nothing.
adopt_session() {   # $1 = slug, $2 = session name to adopt
  local phys; phys="$(cd "$REPO/.worktrees/$1" && pwd -P)"
  rm -f "$TMUX_STATE/repo-$1"
  printf 'cwd=%s\ncmd0=bash\n' "$phys" > "$TMUX_STATE/$2"
}

@test "close: an ADOPTED session is named, and answering 'n' spares it" {
  mk_wt feat-x
  adopt_session feat-x scratch-session

  wt_answer n close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"scratch-session"* ]]
  [[ "$output" == *"was not opened under this repo's name"* ]]
  [[ "$output" == *"kills the WHOLE session"* ]]
  [[ "$output" == *"Skipped feat-x"* ]]
  tmux_session_exists scratch-session               # still running
  ! grep -q "kill-session" "$TMUX_LOG"
  [ -d "$REPO/.worktrees/feat-x" ]
}

@test "close: an ADOPTED session dies on 'y' — and on -y without a prompt" {
  mk_wt feat-x
  adopt_session feat-x scratch-session

  wt_answer y close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed adopted tmux scratch-session"* ]]
  ! tmux_session_exists scratch-session
  [ -d "$REPO/.worktrees/feat-x" ]

  # -y is the app's/script's path: CaptureUi::confirm always answers no, so a
  # programmatic caller that means it must say so (same convention as `rm`).
  adopt_session feat-x other-session
  run_wt close -y feat-x                            # stdin is /dev/null here
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed adopted tmux other-session"* ]]
  [[ "$output" != *"Kill other-session?"* ]]
  ! tmux_session_exists other-session
}

@test "close: EOF on the adopted prompt declines — nothing is killed unasked" {
  # The app and any redirected caller land here. Declining must be the default.
  mk_wt feat-x
  adopt_session feat-x scratch-session

  run_wt close feat-x                               # stdin </dev/null
  [ "$status" -eq 0 ]
  [[ "$output" == *"Skipped feat-x"* ]]
  tmux_session_exists scratch-session
}

@test "close: a CANONICAL-name close is not a question — no prompt, no -y" {
  # The canonical name is one only this tool writes, so there is no doubt whose
  # session it is. stdin is /dev/null: a prompt here would decline and fail.
  mk_wt feat-x
  run_wt close feat-x
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-feat-x"* ]]
  [[ "$output" != *"Kill"* ]]
  ! tmux_session_exists repo-feat-x

  # …and the same for the main checkout's own session
  printf 'cwd=%s\ncmd0=bash\n' "$REPO" > "$TMUX_STATE/repo-(main)"
  run_wt close main
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed tmux repo-(main)"* ]]
  ! tmux_session_exists "repo-(main)"
}

@test "close: main does NOT kill a personal session adopted at the repo root without asking" {
  # The reported hazard: one pane cwd'd at the repo root (a personal session,
  # five windows) was killed outright by `worktrees close main`.
  printf 'cwd=%s\ncmd0=bash\n' "$REPO" > "$TMUX_STATE/my-personal-session"

  run_wt close main
  [ "$status" -eq 0 ]
  [[ "$output" == *"my-personal-session"* ]]
  [[ "$output" == *"Skipped (main)"* ]]
  tmux_session_exists my-personal-session
  ! grep -q "kill-session" "$TMUX_LOG"

  wt_answer y close main
  [ "$status" -eq 0 ]
  [[ "$output" == *"closed adopted tmux my-personal-session"* ]]
  ! tmux_session_exists my-personal-session
}

@test "close: does not touch the declared store" {
  mk_wt feat-x
  # a populated declared store must survive a close untouched
  cat > "$REPO/.worktrees.places.json" <<'EOF'
{"version":1,"updated_epoch":1,"places":{"feat-x":{"lifecycle":"saved","pinned":true,"note":"keep","last_opened_epoch":1}}}
EOF
  cp "$REPO/.worktrees.places.json" "$BATS_TEST_TMPDIR/store-before.json"

  run_wt close feat-x
  [ "$status" -eq 0 ]
  diff "$BATS_TEST_TMPDIR/store-before.json" "$REPO/.worktrees.places.json"
}
