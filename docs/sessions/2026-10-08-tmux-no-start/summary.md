# Session: tmux `-N` on every call that must not start a server

- **Date:** 2026-10-08
- **Worktree:** `.worktrees/tmux-no-start`
- **Branch:** `tmux-no-start` → PR [#459](https://github.com/penard-monkey/worktrees/pull/459), squash-merged as `c785561`
- **Release:** none (in `[Unreleased]`)
- **Planning files:** none; `planning.tar.gz` holds the lane's brief (`.planning/brief.md`)

## Why

On 2026-10-07 the user's default tmux server (25+ sessions) lost its socket file
and a second server started on the same path. The old server was still alive and
unreachable, and the app relaunched duplicates of several agents. When a tmux
client whose command may start a server meets a refused or missing socket, it
takes the lock, unlinks the socket and starts a new server. `tmux -N` (tmux 3.2
and later) clears that permission, so the client fails instead.

## What shipped

- `crates/worktrees-core/src/tmux.rs`
  - `tmux()` adds `no_start_args()` (`["-N"]` or nothing), so every call site
    is safe by default.
  - `no_start_args()` parses `tmux -V` once. The verdict is cached in a
    `OnceLock`, but only after a probe succeeds, because the app re-checks
    after refreshing PATH.
  - `supports_no_start()` accepts `master` and `next-X.Y`, and compares the
    minor version as a number (`3.10` > `3.2`). Anything unparsable gets no
    `-N`, which keeps today's behaviour.
  - `tmux_launch()` is the only path without `-N`. Its one caller is
    `new_session`. Before launching, `wait_out_refusal()` runs
    `-N start-server` up to 5×200ms while the server refuses.
  - `attach_or_switch` (CLI) adds `no_start_args()`.
- `app/src-tauri/src/lib.rs` `term_open`: the pty `attach-session` adds
  `no_start_args()`.
- `test/helpers/common.bash`: the fake tmux consumes global options
  (`-N -u -2 -l -v`, plus `-L/-S/-f` with their argument) before the
  subcommand.
  - `$TMUX_LOG` lines are unchanged, so no existing assertion moved.
  - Each call's globals go to `$TMUX_LOG.globals` as `<globals>|<sub>`.
  - `-V` answers `tmux ${FAKE_TMUX_VERSION:-3.7c}`.
- `test/open.bats`: `-N` is on every call except `new-session`; on 3.1c it is
  on none.
- `test/real-tmux.bats`, the witness: a wrapper deletes the private socket just
  before `open`'s tty attach. The test asserts:
  - no server took the socket path;
  - the old pid is still alive;
  - `kill -USR1` brings its socket back, and it still has the session.

  Teardown kills only the pid the test recorded.
- AGENTS.md architecture rule; CHANGELOG `### Fixed`.

## Decisions

- **One wrapper, with the dangerous path named:** `tmux()` is safe by default
  and `tmux_launch()` is the opt-out. A new call site cannot forget the flag.
- **The refusal wait proceeds after about 1s rather than refusing to launch.**
  A crashed server leaves a socket that refuses forever, and tmux's own
  recovery (unlink and restart) has to keep working. A busy server usually
  accepts again within milliseconds.
- **Feature-detect, don't require 3.2.** The README still recommends ≥ 1.9.

## Dead ends / gotchas

- **Probing which commands start a server.** A server started over a missing
  socket with no sessions exits at once and removes its socket. A naive probe
  therefore reports "no new server" for every command. A config with
  `set -s exit-empty off` (passed with `-f`) is what made it visible.
- **`attach` starts a server only under a tty.** Without one it reported "no
  new server", so the witness needed `script`. Measured over a missing socket:
  only `new-session` and tty `attach` start one. list/has/display/capture/
  send-keys/kill/set-option/switch-client do not, even without `-N`. (The
  review notes attach may start one without a tty too; see Follow-ups.)
- **tmux reports ECONNREFUSED as `no server running on <path>`, not "Connection
  refused".** A missing socket prints `error connecting to <path> (No such file
  or directory)`. The first `is_refusal` matched the wrong string. Measuring a
  stale socket (server killed with `kill -9`) caught it.
- **What the hotfix cannot close:** the incident's first new server came from
  `new-session`, the one call that must be allowed to start a server. A missing
  socket looks the same as "no server", too. Per-project servers
  (`docs/proposals/nested-lanes.md`) are the structural fix.
- A nested `<<'EOF'` inside a `python3 - <<'EOF'` heredoc ends the outer one
  early. Use distinct delimiters.

## Verification

- Release binary built first, then `make test` 480/480 and
  `make test-real-tmux` 5/5.
- `cargo test`: core 724 (1 ignored), cli 70, app --lib 162.
- `make lint` and `cargo check -p app` clean. `tsc` was not run (no TypeScript
  changed).
- Red first:
  - the `-N` bats test, with the flag disabled;
  - the old-tmux test, with the version check ignored;
  - the real-tmux witness, with the flag disabled. It failed at
    `[ ! -S "$RT_SOCK" ]` because a second server started.
- Fable review: MERGE, and its mutations went red. CI green.

## Follow-ups

Review stragglers are in ROADMAP:
- the cold-start cost of `wait_out_refusal`;
- the AGENTS.md wording, which should name both direct spawn sites;
- the `no_start_args` doc on attach without a tty;
- the real-tmux skip pattern, which also skips 3.10+;
- doctor reporting the protection gap on tmux < 3.2;
- the extra `tmux -V` per process.
