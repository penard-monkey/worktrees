---
title: "Session: an agent launching a place must not take over the terminal view"
---

# Session: an agent launching a place must not take over the terminal view

- **Date:** 2026-10-03
- **Worktree:** no-steal-client
- **Branches:** no-steal-client → PR [#439](https://github.com/penard-monkey/worktrees/pull/439) (squash `18a4385`)
- **Release:** none
- **Planning files:** none

## What shipped

- `crates/worktrees-core/src/tmux.rs`: `may_attach(stdin_tty, stdout_tty)`; `attach_or_switch` returns false and does nothing unless both are ttys.
- `crates/worktrees-core/src/ops.rs`: `launch` prints "Session ready (detached). Attach with: tmux attach -t …" when it returns false. `launch` is the only caller, so `new` and `open` are both covered; `switch` never attached.
- `crates/worktrees-core/src/guidance/SKILL.md`: `open <slug> --no-attach` shown for own-work places. No `guidance::VERSION` bump.
- `test/open.bats` (4 tests) and `run_wt_tty` in `test/helpers/common.bash`; unit test `only_a_person_at_a_terminal_may_attach`.
- CHANGELOG: one `### Fixed` line.

## Decisions

- **No `-c` on `switch-client`.** With a tty the caller is a person in their own client; `display -p '#{client_tty}'` has the same which-client ambiguity as the bare form.
- **Gate on stdin AND stdout.** Piped output is not someone watching.

## Dead ends / gotchas

- **Root cause:** an agent's Bash tool has no tty but inherits `$TMUX` from its pane. A bare `switch-client` there picked tmux's "current client" for the session, which was the app's embedded one, so the new lane appeared inside `(main)`'s view.
- **BSD vs util-linux `script`:** `script -q /dev/null cmd` is macOS only. The first push failed 3 tty tests on ubuntu; the helper now branches on `uname` (`script -qec "…" /dev/null` on Linux).
- **Flaky pty tests on macOS** until the helper's stdin was redirected from `/dev/null`; `script` otherwise inherits the caller's stdin.

## Verification

- Fail-first on the unfixed binary: `not ok 19` and `20` (the no-tty open/new tests); the two tty tests passed.
- `make test` plan 1..476, 0 not ok; `make lint`; `cargo test` core 707, cli 70, app 154; `ls --json` identical to the installed CLI; CI green on all jobs.
- No real client was switched or attached.

## Follow-ups

- An agent whose shell allocates a PTY (Codex's exec can) passes `may_attach`. See ROADMAP.
