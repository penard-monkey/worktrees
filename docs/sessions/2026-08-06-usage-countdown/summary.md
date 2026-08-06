---
title: "Session: time-until-reset on the usage bars"
---

# Session: time-until-reset on the usage bars

- **Date:** 2026-08-06
- **Worktree:** ui-tweaks
- **Branches:** ui-tweaks (feature), ui-closeout-usage-countdown (this archive)
- **PRs:** [#82](https://github.com/penard-monkey/worktrees/pull/82) (squash-merged → `4d8bde8`)
- **Release:** none — lands in `[Unreleased]`, next tag picks it up
- **Planning files:** planning.tar.gz alongside this summary (task_plan / findings / progress)

## Where this started

The nav-footer usage widget (shipped v0.7.0, see the
[2026-08-02 usage-widget session](../2026-08-02-usage-widget/summary.html)) drew
three bars with a percentage each. The observation that opened this session:
a percentage says how much of a window is *spent* but never how long until it
comes back — and the second half is the one that decides whether to keep working
or wait it out. Asked for the 5h session countdown, the weekly one in days, and
"if possible for Fable".

The recon answered the "if possible" immediately: **`resets_at` was already
there.** It is parsed for every kind in one shared loop (`lib.rs:904`), including
`weekly_scoped`, crosses into the frontend on the `UsageLimit` type
(`App.tsx:797`), and was being used for exactly one thing — the row tooltip. The
whole feature was a rendering change. No Rust, no new command, no new parse.

## What shipped

A fourth column on each usage row (`app/src/App.tsx`, `app/src/App.css`):

```
5h     ▬▬▬▬▬▬▬▬▬▬▬▬▬  35%   3h 02m
7d     ▬▬▬▬▬▬▬▬▬▬▬▬▬  59%    2d 5h
Fable  ▬▬▬▬▬▬▬▬▬▬▬▬▬  80%    2d 5h
```

- **`fmtEta()`** (`App.tsx`) — seconds-until-reset → two units, biggest first:
  `<1m` / `47m` / `3h 02m` / `2d 5h`, and `3d` rather than `3d 0h`. Returns `""`
  at or below zero.
- **A 15s local tick** in `UsageWidget` — its own `setInterval`, separate from
  the 180s poll effect. A poll also re-zeros the clock.
- **`.usage-eta`** (`App.css`) — `min-width: 4.1em`, right-aligned, tabular
  figures. Rendered only when at least one row has a live reset.
- **Narrow-nav guards** — `.usage-label` became `flex: 0 1 auto` (it could not
  shrink before) and `.usage-bar` gained a `2.5em` floor, so at the 220px nav
  minimum the label ellipsizes before the bar collapses.
- **`?usage=edge`** (`app/src/mock/install.ts`) — a fixture driving every
  formatter branch at once: sub-minute, minutes-only, and a reset already in the
  past. The default fixture's resets moved off round numbers (`t + 3h02m`,
  `t + 2d5h`) so the harness shows the real column shapes instead of a tidy `3d`.
- **CHANGELOG** — `[Unreleased] / Added`.

## Decisions

- **The clock is local, not a faster poll.** `resets_at` is absolute, so a 15s
  `setInterval` keeps the minutes honest while the undocumented, rate-limit-prone
  endpoint keeps its 180s frontend poll and 120s backend cache untouched. Polling
  harder to animate a clock would walk straight into the rate limit that shaped
  this widget in the first place. Drift between polls is zero by construction.
- **Two units, not three.** `2d 5h 12m` was on the table (and is closer to how
  the ask was phrased) but it is the widest possible string in the narrowest
  possible column — at the 220px nav floor it eats the bar. Minutes are noise
  three days out. The hour-scale form keeps its padded `00m` because minutes are
  live at that scale and an unpadded one would jitter the column.
- **`Xd 0h` collapses to `Xd`.** Caught by looking at the rendered harness, not
  by reasoning about the format: `3d 0h` reads as a bug.
- **A passed reset renders blank, not negative.** The statusline fallback's
  `resets_at` is often hours stale. The column stays reserved so the three ETAs
  keep their right edge; if *no* row has a live reset the column disappears
  entirely rather than leaving a strip of blanks.
- **The ETA is never tinted by severity.** Amber/red stay on the bar and the
  percentage. Time-to-reset is the same neutral fact at 5% or 95% — colouring it
  would say "this countdown is alarming", which is not a thing a countdown can
  be.
- **Layout: inline 4th column over a two-line row.** The two-line variant gave
  the bar full width at every nav size and room for "3h 02m left", at ~40px of
  extra footer height. The widget is ambient chrome; height is the thing it is
  least entitled to spend.
- **Absolute reset time stays in the tooltip**, now alongside "Nh Nm left". The
  inline countdown is the glanceable form, the tooltip the precise one.

## Dead ends / gotchas

- **Vite's watcher is dead in this worktree — HMR silently serves stale files.**
  This cost the most time and looked like three unrelated bugs. `.worktrees/` is
  a dot-directory and chokidar ignores it by default, so edits never invalidated
  the module graph: the dev server kept serving the pre-edit file, a full page
  reload did not help, and `touch` did not help. Fixes applied to disk appeared
  to do nothing. **Restart the harness with `--force` after every source edit in
  a `.worktrees/` checkout**, or diff what the server actually serves
  (`curl -s localhost:PORT/src/App.css`) against the file on disk before
  believing a change failed.
- **A stray `*/` swallowed a whole CSS rule, silently.** An edit left comment
  text after a comment close, so `.usage-eta` parsed as garbage and never
  applied. The bars still *looked* plausible — only the column width was wrong,
  which is not something you notice by eye. Caught by asserting
  `getComputedStyle(el).minWidth` in the harness. Layout claims want a measured
  assertion, not a screenshot.
- **`make test` failed with a bare "No such file or directory".** The bats
  submodules are not checked out in a fresh worktree; the error names the missing
  `bats` binary and nothing about submodules. `git submodule update --init
  --recursive`.
- **`pnpm install` refused to run.** Needs Node ≥ 22.13; the login shell here
  defaults to v22.12.0. `nvm use 22.23.2`.
- **CI never ran the PR, and `--auto` merged it anyway.** GitHub's queue was
  backed up (unrelated runs sat queued 47m and 1h44m), so `gh pr checks 82`
  reported "no checks reported" throughout — with no required check to wait on,
  `gh pr merge --auto` merged immediately rather than waiting for one to appear.
  Auto-merge waits on *existing* required checks; it does not wait for checks to
  be created. The local gates cover everything ci.yml does except the app-crate
  build on ubuntu.
- **`gh pr merge --delete-branch` errors in a worktree setup**: "fatal: 'main' is
  already checked out at …/worktrees". The merge itself lands; only the local
  branch-cleanup step fails, and the remote branch survives. Delete it by hand.
- **Playwright MCP writes screenshots into the repo root** and its own output dir
  is not on this filesystem (`find` for the files it reports comes up empty).
  Swept to `~/.cache/worktrees/worktrees/ui-tweaks/` before committing. Its
  inline image return is the reliable way to actually see a shot.

## Verification

Full gate set, all green before the PR:

| Gate | Result |
|---|---|
| `cargo build --release -p worktrees-cli` | ok |
| `make test` | 248 ok |
| `make lint` | clean |
| `cargo test -p worktrees-core` | 143 passed |
| `tsc --noEmit` | pass |
| `cargo check -p app` | pass (no Rust touched) |

Driven headlessly in the mock harness (`VITE_MOCK=1 vite --port 5199`) at nav
widths 220 / 300 / 460px across all three sources, with DOM assertions on the
column geometry rather than eyeballed screenshots:

| Fixture | Observed |
|---|---|
| default (oauth) @ 300px | `5h 35% 3h 02m` · `7d 59% 2d 5h` · `Fable 80% 2d 5h` |
| `?usage=edge` @ 220px | `<1m` · `47m` · blank cell; ETA column holds 42px, bars 75–90px |
| `?usage=stale` @ 460px | dimmed as before, `40m` / `1d 23h` |

GitHub Actions did **not** run on #82 (see above). The next run on main is the
first CI signal for this change.

## Follow-ups

- **Confirm CI on main.** #82 merged without a run. The only gate not covered
  locally is the app-crate build on ubuntu.
- **Delete the remote `ui-tweaks` branch** (`4acf077`) — `--delete-branch` could
  not.
- **Smoke-test the countdown on a real launch.** Everything here was verified
  against the mock; the oauth path's `resets_at` has only ever been read through
  the tooltip. Folds into the existing "smoke-test the usage widget on a real
  launch" roadmap item.
