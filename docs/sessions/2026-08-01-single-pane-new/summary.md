# Session: app-created places open single-pane

- **Date:** 2026-08-01
- **Worktree:** bug-fixes
- **Branch(es):** fix/new-single-pane → PR [#70](https://github.com/penard-monkey/worktrees/pull/70) (squash-merged as `6e5e927`)
- **Release tag:** none (rides in `[Unreleased]`)
- **Planning files:** `planning.tar.gz` in this dir

## What shipped

- **crates/worktrees-core/src/ops.rs** — `cmd_new` gained `--no-spare`
  (parity with `cmd_open`): `launch(spare_shell=false)` and an EMPTY
  install_cmd (launch's contract: install only rides with the spare pane).
  A detected-but-suppressed install command is echoed as `then: <cmd>` —
  same phrasing as the `--no-tmux` branch — never silently dropped.
- **app/src-tauri/src/lib.rs** — `new_place` passes `--no-spare` alongside
  `--no-attach`, matching `open_place`'s single-pane choice.
- **test/new.bats** — 3 tests: default `new` still splits; `--no-spare` =
  single pane + no `split-window` in the shim log; suppressed install
  hinted. Existing `tmux_pane0/1_cmd` helpers + `$TMUX_LOG`, no shim edits.
- **README.md** — flag table documents `--no-spare` for `new`/`co`/`open`
  (was undocumented even for `open`).
- **CHANGELOG.md** — Fixed entry under `[Unreleased]`.

## Decisions

- **App new = single pane, NO auto-install** (David; over running install
  in pane0 before the AI, or keeping the pane only when a lockfile exists).
  Claude starts instantly at full width; deps install manually in the dock
  Terminal tab. The suppressed command must surface as a hint.
- **CLI untouched** — bare `worktrees new` keeps the spare pane +
  auto-install; that pane is deliberate for terminal users.

## Dead ends / gotchas

- The "2 tmux windows" were 2 **panes** (`split-window -h`), not windows.
- The asymmetry was one flag: `open_place` already passed `--no-spare`
  (why reopen looked right); `cmd_new` simply had no such flag and
  hardcoded `spare_shell=true` — pane1 existed as the install shell.
- Naive `--no-spare` would have silently killed auto-install for
  app-created worktrees; `launch`'s "install_cmd only ever WITH the spare
  shell" contract is what surfaced that trap.
- Materialization is unaffected: `.env` linking + port provisioning
  (`materialize_place`/`provision_place`) run before the session and were
  untouched — only dependency install moved to manual.

## Verification

- Gates green (implement agent, + independent fable re-run of full bats
  and lint): release CLI build, `make test` 241 ok, `make lint`,
  `cargo test -p worktrees-core` 137 ok, app `tsc --noEmit` +
  `cargo check -p app`. CI green on both OSes before squash-merge.

## Follow-ups

- Manual repro on David's mac: app "New" → confirm single pane (fix
  machine-verified via bats shims only).
- PR #60 (`fix/remove-place-delbranch`) still open — different worktree's
  stream, untouched by this one.
