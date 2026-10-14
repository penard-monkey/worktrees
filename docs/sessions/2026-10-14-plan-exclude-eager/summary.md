# plan-exclude-eager — `/.planning/` excluded the moment a project goes full

- **Date:** 2026-10-14
- **Worktree:** `.worktrees/plan-exclude-eager`
- **Branches:** `plan-exclude-eager` (squash-merged), `plan-exclude-eager-close-out` (this archive)
- **PRs:** [#483](https://github.com/penard-monkey/worktrees/pull/483), merged as `68b272d`
- **Release:** none. The fix sits under `## [Unreleased]` → `### Fixed`
- **Planning files:** none. `planning.tar.gz` holds the brief (`.planning/brief.md`)

## The bug

Settings → Planning said that `.git/info/exclude` gains `/.planning/`, and that
was false when you read it. `planning::set_project` only edited
`projects.json`, and the only caller of `ensure_planning_excluded` was
`worktrees new`. A project set to full had nothing excluded until it got its
first place. (main)'s own session at full could write `.planning/orchestrator/`
untracked and unignored, ready for a `git add -A` to commit it.

## What shipped

- **`crates/worktrees-core/src/planning.rs`**
  - `Exclude`, an outcome enum: `added | ignored | listed | tracked |
    failed{reason}`, with `words()` for the CLI line. `ensure_planning_excluded`
    returns it instead of a bool.
  - `heal(git_common, wt)` is the one guarded write. A tracked `.planning/`
    returns `Tracked` and nothing is written.
  - `exclude_for_root(root)` resolves the git common dir with `rev-parse
    --git-common-dir` and then calls `heal`.
  - `set_project_at(reg, &Global, key, choice) -> Set { entry, exclude }`
    excludes right after the registry write whenever the project is
    *effectively* full. That includes "inherit" under a full default.
  - `set_global(_at)` saves the default and, when it is full, sweeps every
    registered project the default makes full.
- **`ops.rs`**: `planning_for` checks the exclude at every full launch, for
  every harness, before the Claude-only plugin logic. `new`'s
  `write_plan_pointer` keeps it as a backstop. Both go through `heal`.
  `report_exclude` speaks only on an add or a failure.
- **`plancmd.rs`**: `plan level` and `plan default` print the outcome. The
  sweep prints per project, and only where something happened.
- **`app/src-tauri/src/lib.rs`**: the planning-status payload sent to the app
  gains an optional `exclude` field, present only on the status returned by a
  write. `ai_status_report` now logs the seam's info lines (review item).
- **`app/src/PlanningPanel.tsx`**: `excludeWords`; each `ProjectRow` shows its
  outcome after a write, using the error styling for tracked or failed. The
  "What changes in a repo" hint now says when the line is written.
- **`app/src/mock/install.ts`**: added-then-ignored idempotence, the global
  sweep, and `?planexclude=tracked|failed` to show the error styling.
- **Tests**:
  - six core tests in `planning::tests`, behind a guarded `Scratch` repo: full
    writes once and is idempotent; inherit under a full default; a covering
    `.gitignore` is skipped; `Listed` via a `!/.planning/` negation; the sweep
    with an explicit off and an unreachable entry; tracked is reported, not
    written.
  - five bats tests in `test/plan.bats`: `level full`, a `.gitignore` that
    already covers it, `default full`, a launch healing a removed line, and a
    tracked repo never getting the line from the default, `new` or a launch.
- `docs/proposals/owned-planning.md` §2.5's level table: the full row updated in this archive PR
  to say when the line is written.

## Decisions

- **One core function per act, not two call sites.** The app and the CLI call
  `set_project` and `set_global`, and the exclude lives inside them. Either
  front end would otherwise have been able to forget it again, which is how
  the bug happened.
- **"Effectively full", not "chose full".** Inherit under a full default
  leaves a project just as exposed. `set_project_at` takes the `Global` it
  resolves against, so its tests never read the real `planning.json`.
- **Sweep on global → full: yes.** The sweep checks every project whose
  *effective* level is full; explicit off and show only are left out. A
  tracked or unreachable repo is reported and never refuses the setting,
  because the default belongs to the machine and one stale registry entry
  must not block it.
- **Heal at every full launch, for every harness.** It costs one `git
  check-ignore` plus one file read per launch. It heals projects set to full
  before this fix (anime-song-remover, for example) on their next (main)
  launch, and (main) is exactly where the leak happens.
- **The exclude result belongs to the act, not the repo.** It rides on the
  status a write returns and is absent on plain reads. That avoids a git
  fan-out per project on every `planning_status`.
- **A tracked `.planning/` is never written over, anywhere.** This was a review
  must-fix. The first cut's launch heal skipped the tracked check, so a row
  that said "tracked, nothing written" was contradicted by the next launch.
  `heal` is now the only entry point.

## Dead ends / gotchas

- **A lenient assertion passed against a stub.** The first core tests
  accepted `Added | Ignored`, hedging against a machine-wide git ignore. With
  the exclude calls stubbed to return `Ignored`, every one of them stayed
  green. The fix was to pin `core.excludesFile=/dev/null` in the scratch
  repo's LOCAL config (repo config beats global) and assert `Added` strictly.
  After that, the stub turned three tests red. Same family as AGENTS.md's
  "a test can pass because its fixture omits the thing".
- **RED against the installed release proved nothing.** Running the new bats
  tests with `WT_BIN=~/.local/bin/worktrees` (v0.41.0) "failed" every one of
  them, but inside `setup` (`projects add`), not at the assertion. Read
  *where* a RED fails. The real RED came from stubbing the call sites in the
  tree and rebuilding release.
- **A mutation that did not mutate.** A perl edit meant to comment out the
  launch heal put `//MUT` on its own line above the call, so the call still
  ran and the "heals" test stayed green. Grep the mutated line itself before
  believing a green.
- **Chrome DevTools MCP's browser was held by another session**
  ("browser is already running for …chrome-profile"). I did not fight it: I
  used a cached Playwright from another tree's scratch dir
  (`createRequire(~/.cache/worktrees/worktrees/pr-view/node_modules/)`) to
  drive the mock headlessly.

## Verification

- RED first for every wiring test, by stubbing the call sites. The tracked
  test was also RED with ONLY the launch call reverted, so it pins the launch
  path on its own. The `Listed` and `.gitignore` tests went red when their
  guards were removed. Every mutation was restored by editing back and
  `touch`ed.
- Gates on the final commit, with a fresh release build: `make test` 486/486;
  `make lint`; core 744 passed (2 ignored); cli 66 + 1; app `--lib` 162;
  `cargo check -p app`; `tsc --noEmit`; all 38 `app/scripts/*-check.mjs` run
  from the repo root. Clippy showed no new warnings in the touched code.
- I drove the mock headlessly: full → "Added…", off → no line, full again →
  "git already ignores…", default full → each inheriting row reports.
  `?planexclude=tracked|failed` → `hint planning-err` on every row.
- I did not run the real app. This change has no timing-sensitive UI.

## Follow-ups

- The mock's `?planexclude=tracked` also answers on an EXPLICIT full, which the
  real core refuses with an error before writing. It is fine as a styling
  flag; don't read it as behaviour.
- A candidate AGENTS.md learning, which needs David's OK: "a RED run must fail
  at the assertion. Check the failing line, because an old binary that dies
  in `setup` reads exactly like a red test."
