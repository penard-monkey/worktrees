#!/usr/bin/env bats
# Integration smokes against REAL tmux. The fake tmux shim is removed and the
# server is isolated via TMUX_TMPDIR (socket lives under the test tmpdir), so
# these never touch the developer's own tmux server. Always --no-attach (no tty).

load 'helpers/common'

setup() {
  common_setup
  remove_fake_tmux                        # fall through to the real tmux binary
  export TMUX_TMPDIR="$BATS_TEST_TMPDIR"  # isolated socket dir per test
  export SHELL=/bin/bash                  # deterministic pane shell
  # ($REPO physicalized centrally in make_repo; symlinked-path handling is
  #  fixed in bin/worktrees via pwd -P — regression test in misc.bats.)
}

teardown() {
  tmux kill-server 2>/dev/null || true
}

# bats test_tags=real-tmux
@test "real tmux: new --no-attach creates a session with 2 panes" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux has-session -t repo-feat-x
  run tmux list-panes -t repo-feat-x -F '#{pane_id}'
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 2 ]
}

# With tmux's default `mouse off` the wheel reaches Claude as arrow keys and
# scrolls its prompt history, so our sessions turn it on — session-scoped, the
# global stays whatever the user's tmux config says (here: none, HOME is
# isolated). The opt-out UNSETS rather than forcing off, and it is spelled as a
# TOML boolean on purpose: the string-only config reader would drop it.
# bats test_tags=real-tmux
@test "real tmux: sessions get mouse on; tmux_mouse = false hands it back to tmux" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  [ "$(tmux show-options -t repo-feat-x -v mouse)" = "on" ]
  [ "$(tmux show-options -gv mouse)" = "off" ]
  mkdir -p "$HOME/.config/worktrees"
  printf 'tmux_mouse = false\n' > "$HOME/.config/worktrees/config.toml"
  run_wt new feat-y --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux has-session -t repo-feat-y
  [ -z "$(tmux show-options -t repo-feat-y -v mouse)" ]
}

# bats test_tags=real-tmux
@test "real tmux: open reattaches the existing session (exit 0, still one session)" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  run_wt open feat-x --no-attach
  [ "$status" -eq 0 ]
  tmux has-session -t repo-feat-x
  run tmux list-sessions -F '#{session_name}'
  [ "$status" -eq 0 ]
  [ "$(printf '%s\n' "$output" | grep -c '^repo-feat-x$')" -eq 1 ]
}

# bats test_tags=real-tmux
@test "real tmux: rm -y kills the session" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  tmux has-session -t repo-feat-x
  run_wt rm feat-x -y
  [ "$status" -eq 0 ]
  run tmux has-session -t repo-feat-x
  [ "$status" -ne 0 ]
  [ ! -d "$REPO/.worktrees/feat-x" ]
}
