---
title: One clock for every list of places
---

# One clock for every list of places

- **Date:** 2026-08-11 (merged 2026-08-11 23:22 local / 2026-08-12 04:22 UTC)
- **Worktree:** `.worktrees/ui-changes`
- **Branches:** `ui-cmdk-activity-order` → merged; archive on
  `close-out-cmdk-activity-order`; tree parked on `ui-changes-next`
- **PRs:** [#121](https://github.com/penard-monkey/worktrees/pull/121) —
  `fix(app): one clock for every list of places` (squashed as `ebc1cbf`)
- **Release tag:** none (CHANGELOG `[Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this summary

## What shipped

⌘K, the Recent lens and the home "Resume where you left off" list each ranked
places by a different notion of *recent*, and none of them matched the nav tree.
Same three places, three orders — and in ⌘K's case an order with no date on
screen to explain it.

| List | Was | Now |
|---|---|---|
| Nav tree | `activityAt` — work or commit, opens excluded | unchanged |
| ⌘K | `recencyEpoch` — opens ‖ last commit, **no age rendered** | `activityAt` + an age column |
| Recent lens | `usedEpoch` — opened or worked | `activityAt` |
| Home Resume | `usedEpoch` | `activityAt` |

`app/src/App.tsx`:

- `QuickSwitch` takes the sort key as a **`rank` prop** instead of importing
  `recencyEpoch` at module scope. It has to: `activityAt` closes over the
  `donePaths` state that carries live `sessions:done` events, so it cannot exist
  outside `App()`.
- New `.qs-age` column on each palette row, rendering `ago(rank(p))`.
- `recentItems` and `resume` sort by `activityAt`; both memos gained
  `donePaths` as a dependency.
- `FlatLens.ageOf` and `PlaceRow.ageEpoch` deleted — they existed only so a list
  could label rows with a clock it did not sort by. Nothing needs that now.
- `recencyEpoch` deleted. `usedEpoch` survives with exactly one consumer.
- Restore-on-launch split out of `resume[0]` into its own `restoreTarget` memo.

`app/src/App.css`: `.qs-age` (right-aligned, tabular-nums, `--fs-micro`), sitting
after the lifecycle badge that already holds the `margin-left: auto`.

`CHANGELOG.md`: new `[Unreleased]` section.

## Decisions

- **One clock for everything a user reads.** The clock is `activityAt` =
  max(Claude work, last commit) — opens excluded, so clicking a row never
  reshuffles anything. Scope was confirmed with the user mid-session: the home
  Resume list moves too, not just ⌘K and Recent.
- **Restore-on-launch keeps `usedEpoch`.** This is the one thing that did *not*
  move, and it is the reason `usedEpoch` still exists. `activityAt` counts
  commits, so a worktree created from the CLI and never opened — a commit from
  yesterday, no user history at all — would outrank a place actually opened last
  week and become the place the app launches itself into. A *list* can survive
  being wrong about that; it shows the next row too. The restore target cannot.
  Splitting it into its own memo is what let the Resume list move without taking
  the launch target with it.
- **⌘K gained an age column** rather than silently re-sorting. The codebase's own
  rule (the comment that used to justify `PlaceRow.ageEpoch`) is that a list must
  label rows with the clock it orders by, or its ages read as a broken sort. ⌘K
  had been ordering by a date it never showed.
- **`donePaths` in the memo deps is load-bearing, not defensive.** See below.

## Dead ends / gotchas

- **The worktree opened in a phantom state — 616 lines of staged deletions that
  were not real work.** `git status` showed the *inverse* of PRs #117–#120
  staged: the session archive deleted, the rename fix reverted. This is the
  hazard CLAUDE.md's close-out section describes, arrived at by a different
  route: it does not need a `git checkout -B`. This tree was parked on `ui-next`,
  which is **also the ui-tweaks worktree's branch** — when that tree moved the
  shared ref, this one was left with a stale working copy and an index that
  read as a giant deletion.
  Diagnosis before touching anything:
  `diff <(git diff --cached) <(git diff 403393d abac288)` — byte-identical, so
  the working tree was exactly `abac288`'s and nothing local existed to lose.
  With no untracked files and no stash, `git reset --hard 403393d` was provably
  lossless. **Do not reset on the reflog's word alone** — the reflog showed only
  this tree's own last checkout and gave no hint the branch had moved.
  Consequence: `ui-changes` must never park on `ui-next` again. Its idle base is
  `ui-changes-next`, recorded in `.claude/close-out.md`.
- **A mid-verification disagreement that looked like a bug and was the proof.**
  The first nav read showed `messaging` at `26d`; ⌘K, read moments later, showed
  it at `now`. The mock's `sessions:done` event had landed between the two reads.
  Re-reading the nav afterwards showed `now` as well — the lists agreed, and the
  episode demonstrated that `donePaths` genuinely has to be a memo dependency:
  without it, a task finishing while the home view is up cannot re-rank the list.
- **`recencyEpoch` could not simply be swapped for `activityAt` in place.**
  `QuickSwitch` is at module scope by deliberate design (CLAUDE.md: components
  defined inside `App()` remount every render, losing input focus — fatal for a
  palette). `activityAt` is App state. The prop is the only way through; a
  module-scope re-derivation would have silently ignored live completion events.
- **Playwright MCP refuses paths outside the repo and had to be swept.** One
  screenshot landed in the repo root (not `.playwright-mcp/` as the note in
  `.claude/close-out.md` predicts) — `find -mmin -5` located it. Moved to
  `~/.cache/worktrees/worktrees/ui-changes/cmdk-activity-order.png`.
- **Escape dispatched at `window` does not close the palette.** It is handled on
  the input's `onKeyDown`. Clicking `.scrim-center` is the reliable headless
  close.

## Verification

Mock harness on port 5178 (`VITE_MOCK=1 vite --force --strictPort`), driven
through Playwright MCP. HMR is dead inside `.worktrees/`, so the *served* file
was checked before trusting anything: `qs-age` present, `recencyEpoch` absent.

| List | Ages read top to bottom |
|---|---|
| ⌘K | now, now, 4m, 30m, 45m, 5h, 26d ×5 |
| Recent lens | now, now, 5m, 46m, 5h, 26d ×4 |
| Home Resume | now, now, 5m, 46m, 5h, 26d |
| Nav tree | agrees — `messaging`/`billing-refactor` now, worktrees `(main)` 30m |

Monotonic in all four, and the same places in the same order. `.qs-age`
computed style asserted rather than eyeballed: `text-align: right`,
`flex-grow: 0`, right edge 17px inside the panel.

Gates, all green: `tsc --noEmit`, `cargo check -p app`, release CLI build,
`make test` (288 bats, zero `not ok`), `make lint`, `cargo test -p
worktrees-core` (208), `cargo test -p worktrees-cli` (6). Chain exit 0. CI:
9/9 checks.

Screenshot: `~/.cache/worktrees/worktrees/ui-changes/cmdk-activity-order.png`
(scratch, not committed).

## Follow-ups

- The base-tip age edge, previously noted as nav-only, is now workspace-wide: a
  just-created worktree shows the age of the branch it came from until work or a
  commit lands there. Moved to ROADMAP as its own item, since the roadmap entry
  that carried it ("the activity clock stops at the nav tree") is now closed by
  this session.
- ⌘K lists `(main)` places; the Recent lens filters them out. Unchanged by this
  work and possibly correct — noted in ROADMAP so the asymmetry is a decision
  rather than an accident.
