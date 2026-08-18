---
title: Remove-worktree dialog + context-menu clamp
---

# Remove worktree became a dialog, and the menu stopped hiding its last item

- **Date:** 2026-08-18
- **Worktree:** `left-nav-menu-bug` (`.worktrees/left-nav-menu-bug`)
- **Branches:** `left-nav-menu-bug` → `left-nav-menu-bug-closeout`
- **PR:** [#156](https://github.com/penard-monkey/worktrees/pull/156), squashed as `b2c97f4`
- **Release tag:** none (lands in `[Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this summary

## What shipped

Two things, reported as one bug.

**1. `CtxMenu`'s clamp re-runs on resize** — `app/src/CtxMenu.tsx`.

The shared right-click menu shell clamped itself into the viewport in a
`useLayoutEffect` keyed `[x, y]` — cursor coords, frozen for the menu's whole
life. A menu that GREW after opening kept the `top` computed for its old
height, so one already flush against the bottom pushed its new last row off the
screen: unreachable, and with no scrollbar to admit it. The nav's "Remove
worktree…" arming into two danger buttons is exactly that shape, and is how it
was found (screenshot in the scratch dir).

Now: a `ResizeObserver` on the menu element plus a `window` resize listener,
both re-running the same clamp, so it covers callers that do not exist yet. It
measures `offsetWidth/Height`, **not** `getBoundingClientRect()` — the latter
measures through the `pop` keyframe's `scale(0.98)` and reports a box ~2% small
on the first frame. `.ctxmenu` gained `max-height: calc(100vh - 16px)` +
`overflow-y: auto` (`app/src/App.css`) as the belt: a clamp can only slide a
menu that already fits.

**2. Removing a worktree is a dialog** — `RemoveDialog` in `app/src/App.tsx`,
styles `.rm-*` in `app/src/App.css`.

Replaces the inline armed two-button pair at BOTH entry points (nav right-click
menu, topbar ⋯). Reuses the existing modal shell (`.scrim-center` +
`.sync-modal` / `.sync-h` / `.sync-body` / `.sync-foot`) — no new shell, no new
colour tokens. Names the worktree, path and branch, then warns only about what
is actually at risk, with two checkboxes (`--force`, `--branch`) rather than
extra danger buttons.

**3. `app/scripts/ctxmenu-check.mjs`** — evaluates the REAL `CtxMenu.tsx` source
under React/DOM stubs, same slice-the-real-source shape as `race-check.mjs`.

## Decisions

- **Every line of the dialog is pinned to `ops.rs::remove_one`, not to
  intuition.** This was re-derived twice (see dead ends). Final mapping:

  | State | What it says | Why |
  |---|---|---|
  | dirty | ⚠ *refused unless you discard them* + `--force` tick | `ops.rs:1014` refuses; nothing is destroyed by a plain remove |
  | live tmux | ⚠ *will be killed* | `ops.rs:1040` |
  | ahead, branch survives | plain note, *"not in the base branch"* | branch holds them; `ahead` is base-ref divergence (`project.rs:367`), not `@{u}` |
  | ahead + detached | ⚠ *no branch keeps them* | `ops.rs:999` clears the branch; nothing holds the commits |
  | ahead + force + delBranch | ⚠ *force-deleting discards them* | `ops.rs:1055` picks `-D` |

- **`force` is a checkbox the user ticks, never inferred from `place.dirty`.**
  It is the only flag here that destroys work.
- **`place.dirty` (≤3s stale) decides what to SAY, never whether the button
  works.** The backend is the authority on a refusal; disabling client-side
  would block a tree the user just committed.
- **A refusal keeps the dialog OPEN** with the reason inline, directly under the
  force checkbox that answers it. `runCmd` also puts it in the app-wide banner;
  the inline copy is the one you can act on without reopening anything.
- **`delBranch` alone stays safe** (`git branch -d`), so its own checkbox is not
  a warning — until `force` joins it.

## Dead ends / gotchas

**The dialog shipped a false warning through every gate, twice.**

*First draft* said "9 uncommitted files will be lost." Every gate passed. Driving
the mock harness returned the backend's actual answer instead:

```
Worktree 'feat-redesign' has uncommitted changes:
Refusing to remove. Commit/stash, or pass --force.
```

`ops.rs:1014` refuses a dirty remove outright. Nothing was about to be lost — the
warning was crying wolf while hiding the only action the user needed. Rewrote
against `remove_one`.

*Second draft* — caught by the fable review, not by any gate — hit the real trap:

```rust
// ops.rs:1055
let flag = if force { "-D" } else { "-d" };
```

**`force` is two permissions wearing one flag.** Line 1014 reads it as "remove a
dirty tree"; line 1055 reads the same bool to pick `git branch -D`. So
force+`--branch` force-deletes an UNMERGED branch — the only combination in this
path that can destroy commits — while the checkbox still read *"(only if
merged)"* and the note said the commits were safe. The inline arm this repo
shipped for a year was immune **only** because it hardcoded `force: false`, and
the docstring's "del_branch is safe by construction" was true *of that call
site*, not of the command. Exposing a force checkbox is what put it in reach.

Nothing automated can catch this: bats does not model branches, and the mock says
so out loud (`install.ts`: "delBranch is state-invisible here"). Recorded in
CLAUDE.md.

**Four more from the same review, all real:**

- `force` survived its own checkbox's unmount. The box renders only under
  `place.dirty`, but `force` is `RemoveDialog` state and the component stays
  mounted across refresh ticks — commit the tree under an open dialog and the
  flag is set, hidden and unturnoffable, silently upgrading `-d` to `-D`. Now
  cleared by an effect on `place.dirty`.
- `ahead` is divergence from the **base ref**, not `@{u}` (`project.rs:359-373`,
  deliberate — see its comment). "Not pushed to origin" was simply false for a
  fully-pushed unmerged branch.
- Detached HEAD: the note said commits "stay on the branch". There is no branch;
  that is the one case they actually die. Backwards in exactly the wrong place.
- The vanish-effect keyed on `!rmPlace`, but `list_workspace` returns a null
  snapshot for ANY failed sweep and `refresh` commits it wholesale — so one
  errored tick would dismiss an open destructive-action dialog, wiping checkbox
  state and any refusal text mid-read. Now keyed on "snapshot listed the
  project, slug absent".

**A test that passes is not a test that works.** `ctxmenu-check.mjs` registered
a `window` resize listener it never fired, and its "window shrinking" case
spread the window object with identical dimensions and shrank nothing. Deleting
the listener from `CtxMenu` still passed. Now mutation-verified against three
distinct mutants.

**A stub that answers only one measurement API turns a failed assertion into a
TypeError.** First run of the check against the pre-fix component died with
`el.getBoundingClientRect is not a function` — a crash, not a demonstrated
failure. The stub now answers both, and the check is about WHEN the clamp runs,
not which probe it uses.

**Merging took four rebases.** Main absorbed #155, #157–#159, #160–#162 while
this was in flight. Two merge attempts failed outright — `Base branch was
modified`, then `the merge commit cannot be cleanly created` — and both times
`gh pr view --json state,mergeCommit` confirmed nothing had half-landed (the
CLAUDE.md rule about `gh pr merge` reporting failures it did not cause cuts both
ways: verify before retrying AND before assuming failure). The CHANGELOG
conflicted twice; the second time #160's close-out had **relocated** the entries
mine sat beside, so the fix was to rebuild that section from `origin/main`'s
version rather than resolve markers in place. Ended by enabling `--auto` instead
of continuing to race.

**`grep --include=*.tsx` under zsh** → `(eval):1: no matches found`. Quote it.

## Verification

Gates re-run in full at every rebase; final pass on `3af4b91`:

| Gate | Result |
|---|---|
| `make test` | exit 0, 314 tests, 0 `not ok` |
| `make lint` | exit 0 |
| `cargo test -p worktrees-core` | 251 passed |
| `cargo test -p worktrees-cli` | 7 passed |
| `cargo test -p app --lib` | 42 passed |
| `tsc --noEmit`, `cargo check -p app` | exit 0 |
| `ctxmenu` / `race` / `dnd` / `relpath` / `termfit` `-check.mjs` | all exit 0 |
| CI on the PR head | 9/9 green |

`ctxmenu-check.mjs` shown RED first, per the repo rule, on three mutants:

```
pre-fix CtxMenu   FAIL re-clamps when the menu grows — top 602 → 602, bottom 1100 > 1076
                  FAIL the grown menu is observed at all — no ResizeObserver
no-listener       FAIL registers a window resize listener — none registered
                  FAIL re-clamps when the window shrinks under it
                  FAIL removes its window listener on unmount
```

Driven for real in the mock harness (`:5200`, viewport 1100×760, served source
verified by CONTENT not by port):

- menu opened at cursor y=635 → clamped to top 274, **bottom 752 ≤ 760**, last
  item fully visible, `max-height: 744px`, `overflow-y: auto`
- dirty remove without force → backend refusal, dialog stays open with the
  reason inline; tick force → remove lands, place gone, dialog + scrim closed
- topbar ⋯ → same dialog, popover closed; a clean tree shows no force box
- Esc dismisses without removing; 0 console errors from this change

## Follow-ups

- **The reworded dialog has never been seen rendered.** The harness run above
  covered the PRE-review wording. After the copy rewrite, verification was tsc +
  gates + reading only — starting vite was blocked by the permission classifier.
  The unrendered branches are the three new conditionals: the force-on branch
  label, and the detached / force-delete variants of the commits note.
- **`remove_place`'s overloaded `force` is a core-level footgun**, not just a UI
  one. Any future surface that exposes it inherits the same trap. Worth
  considering splitting it (`--force` for the tree, `--force-branch` for `-D`).
