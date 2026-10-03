#!/usr/bin/env bats
# Integration smokes against REAL tmux, on a PRIVATE server. Always --no-attach
# (no tty).
#
# Isolation is the whole point of this setup, and TMUX_TMPDIR is NOT it: inside
# a tmux pane (every lane runs in one) a bare `tmux …` talks to the server
# `$TMUX` names — the developer's real one — and TMUX_TMPDIR only picks the
# socket dir when `$TMUX` is unset. A bare `tmux kill-server` in teardown has
# killed a user's whole tmux server that way (AGENTS.md). So:
#   - `$TMUX` is unset (common_setup does it too; this file does not rely on it);
#   - the fake shim is replaced by a WRAPPER that pins every call to this test's
#     own `-S` socket. worktrees shells out to plain `tmux` from PATH, so the
#     binary under test lands on the same private server as the assertions;
#   - teardown names that socket explicitly and never runs a bare kill-server.
# The socket lives in a short mktemp dir under /tmp, not $BATS_TEST_TMPDIR:
# macOS caps a unix socket path at 104 bytes, and the bats tmpdir is longer.

load 'helpers/common'

setup() {
  common_setup
  unset TMUX
  remove_fake_tmux                        # fall through to the real tmux binary
  REAL_TMUX="$(command -v tmux || true)"
  RT_DIR="$(mktemp -d /tmp/wtrt.XXXXXX)"
  RT_SOCK="$RT_DIR/s"
  export TMUX_TMPDIR="$RT_DIR"            # third layer: even a stray default socket is ours
  if [ -n "$REAL_TMUX" ]; then
    printf '#!/bin/sh\nexec "%s" -S "%s" "$@"\n' "$REAL_TMUX" "$RT_SOCK" > "$SHIMS/tmux"
    chmod +x "$SHIMS/tmux"
  fi
  export SHELL=/bin/bash                  # deterministic pane shell
  # ($REPO physicalized centrally in make_repo; symlinked-path handling is
  #  fixed in bin/worktrees via pwd -P — regression test in misc.bats.)
}

teardown() {
  # This test's private server only, by its own socket — never a bare kill-server.
  if [ -n "${REAL_TMUX:-}" ] && [ -n "${RT_SOCK:-}" ]; then
    "$REAL_TMUX" -S "$RT_SOCK" kill-server 2>/dev/null || true
  fi
  case "${RT_DIR:-}" in /tmp/wtrt.?*) rm -rf "$RT_DIR" ;; esac
}

# bats test_tags=real-tmux
@test "real tmux: new --no-attach creates a single-pane session" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --no-install --no-attach
  [ "$status" -eq 0 ]
  # The binary's session is on THIS test's private server, by its socket.
  [ -S "$RT_SOCK" ]
  "$REAL_TMUX" -S "$RT_SOCK" has-session -t repo-feat-x
  tmux has-session -t repo-feat-x
  run tmux list-panes -t repo-feat-x -F '#{pane_id}'
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 1 ]
}

# bats test_tags=real-tmux
@test "real tmux: new --spare --no-attach creates a session with 2 panes" {
  command -v tmux >/dev/null || skip "no real tmux"
  run_wt new feat-x --spare --no-install --no-attach
  [ "$status" -eq 0 ]
  run tmux list-panes -t repo-feat-x -F '#{pane_id}'
  [ "$status" -eq 0 ]
  [ "${#lines[@]}" -eq 2 ]
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
