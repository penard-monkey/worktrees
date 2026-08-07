---
title: "Session: creating a place says so, reopening one stops shouting"
---

# Session: creating a place says so, reopening one stops shouting

- **Date:** 2026-08-07
- **Worktree:** ui-changes
- **Branches:** `fix/new-place-feedback` (feature), `chore/close-out-new-place-feedback` (this archive)
- **PRs:** [#89](https://github.com/penard-monkey/worktrees/pull/89) (squash-merged → `48467f3`)
- **Release:** unreleased — sits in `[Unreleased]` after v0.9.0
- **Planning files:** planning.tar.gz alongside this summary (task_plan / findings / progress)

## Where this started

Two sentences, reported as one complaint:

> when creating a new worktree, there is a delay and it appears stuck … so I
> keep getting the message `tmux session 'xxx' already in the worktree --
> attaching.` I get it every time I open the worktree. Happens after the new
> release.

The "happens after the new release" pointed at a regression. It wasn't one, and
chasing it as one would have wasted the session.

## The actual diagnosis

### The warning was never new — its AUDIENCE was

`git log -L` on the line settles it: `ops.rs`'s
`ui.warn("tmux session '{session}' already in this worktree — attaching.")` has
been there since **#6**, the original bash→Rust port. Nothing about it changed.

What changed is that **#58** taught the app to route and log warnings.
`run_op` (`app/src-tauri/src/lib.rs`) logs warnings *even for commands that
returned 0*, so a message that had been a harmless line in a terminal became a
`[warn]` on every single `open`. Straight from the user's app.log:

```
15:15:55Z [info] open ui-changes fresh=false ok repo=/Users/davidpena/workspace/worktrees
15:15:55Z [warn] open ui-changes fresh=false warnings repo=…: tmux session 'worktrees-ui-changes' already in this worktree — attaching.
```

Two defects in the one line:

1. **Wrong severity.** Finding the session already up is what a durable place
   *is* — the expected, healthy outcome of reopening one. Warn is for things
   needing a human.
2. **Wrong text on the app path.** `open_place` calls
   `launch(…, do_attach=false, …)` — the app embeds the session in its own PTY
   and never attaches. It said "attaching" regardless.

Worth noting the frontend never bannered it (`App.tsx` only sets `err` when
`!r.ok`). The user was reading it in **Settings → Logs**, where it had crowded
out everything else.

### The delay was silence, not slowness

Measured, rather than guessed:

| step | time |
|---|---|
| `worktrees new <new-branch> --no-tmux` end-to-end | **2.07 s** |
| `git fetch origin refs/heads/<new-branch>:…` — **cannot succeed** | 0.80 s |
| `git fetch origin <base>` | 0.80 s |
| `git worktree add` (worktrees repo) | ~0.5 s |
| `git worktree add` (monorepo, 1944 files, 300M `.git`) | 0.51 s |
| `tmux list-panes -a` / `list-sessions` (20 live sessions) | 0.006 s |
| `worktrees --json ls` (what `refresh()` uses) | 0.05 s |

tmux and the workspace sweep were never the problem — worth stating because
"20 tmux sessions must be slow" is the intuitive wrong answer. The real cause was
that **nothing in the UI acknowledged the click**: `NewPlaceForm.submit()` fired
`onCreate` un-awaited, the button stayed live, and `runCmd` held no pending
state. `InitRepoPrompt`, thirty lines below it in the same file, already had a
`busy` flag — the pattern existed and this form just lacked it.

The network half was real too: creating a branch that does not exist yet fetched
`refs/heads/<branch>` — a request guaranteed to return
`fatal: couldn't find remote ref` — and then fetched the base separately.

## What shipped

| File | Change |
|---|---|
| `crates/worktrees-core/src/ops.rs` | `launch()`: `ui.warn` → `ui.info`; tail follows `do_attach` ("attaching." / "reusing it."). `cmd_new`: two targeted fetches → one **guarded** `git fetch origin` |
| `app/src/App.tsx` | `pendingNew` / `pendingSeq` / `newDraft` state; `PendingRow` at module scope; `createPlace` dismisses the form up-front, retires the ghost in `finally`, restores the form on failure |
| `app/src/App.css` | `.status-dot.pending` (accent + `breathe`), `.row.pending` |
| `app/src/mock/install.ts` | `?slowcreate[=ms]` latency knob; a branch containing `fail` returns `ok:false` |
| `test/new.bats` | golden update + 2 new tests |
| `CHANGELOG.md`, `ROADMAP.md` | `[Unreleased]`; `do_switch` follow-up |

## Decisions

- **Optimistic ghost row over an inline busy form.** Offered both; the user
  picked the ghost. The form dismisses on submit and a placeholder place appears
  in the nav. `runCmd` awaits its own `refresh()` before returning, so the ghost
  hands over to the real row with no empty frame between them.
- **One `git fetch origin` over parallel targeted fetches.** Parallel would have
  won the same wall-clock, but message ordering becomes racy and the bats suite
  gates CLI output. One fetch is also simply less machinery.
- **Accepted narrowing:** the single fetch honours the repo's *configured*
  refspec, so a `--single-branch` clone no longer force-materializes a remote
  branch it was set up not to see. Documented in the code, the CHANGELOG, and a
  test. Forcing `+refs/heads/*:…` on everyone to serve that case would fetch
  refs those clones exist to avoid.
- **`do_switch` left alone.** Same doomed two-fetch pair, same three-line fix,
  but it changes `switch`'s own semantics and goldens. Deferred to ROADMAP
  rather than smuggled into a `new` change.

## Dead ends / gotchas

- **"Happens after the new release" was a red herring — and also true.** The
  line was untouched since #6; the *plumbing that surfaces it* was new. When a
  user says a message is new, check whether the message changed **or** whether
  its transport did. `git log -L <range>:<file>` answers the first in one
  command.
- **I introduced a regression and the review caught it.** Collapsing the two
  fetches, I dropped the `!show-ref refs/remotes/origin/<branch>` guard the old
  first fetch carried. When the tracking ref is already on disk (a background
  fetcher, a previous `new`, an `rm` that kept it), the old code took the
  tracking checkout with **zero** network calls. Unguarded, that path went 0 → 1
  — and offline, that is a DNS/TCP timeout before producing an identical
  worktree. **Collapsing 2 into 1 must not raise the floor from 0 to 1.**
  Guard restored, plus a test that points `origin` at an unreachable path so any
  network attempt fails loudly instead of silently costing time.
- **Dismissing the form on submit quietly cost the user their input.** The old
  code only dismissed inside `if (r?.ok)`, so a rejected create left the fields
  intact to correct. Moving the dismissal up-front deleted three typed fields on
  every failure — a typo'd base meant a full retype. Fixed with `newDraft` +
  `initialBranch`/`initialName` props. A UX decision ("close the form
  immediately") had a second-order cost that wasn't in the decision.
- **The mock harness resolved instantly, so the ghost was unobservable.** A row
  that renders for one frame cannot be tested. Added `?slowcreate[=ms]`,
  following the existing `?notmux` / `?empty` query-knob idiom.
- **Playwright refs (`e123`) go stale between snapshots.** Two failed clicks
  before switching to xpath/CSS selectors. Also: the MCP tools refuse to write
  outside the repo, so artifacts land in `.playwright-mcp/` and get moved to the
  cache at close-out (now written up in CLAUDE.md).
- **`make test` piped three times in one command** to grep for counts — ~15
  minutes of the session for output a single logged run gives. Run it once,
  redirect to a file, grep the file.
- **`worktrees rm --delete-branch` is not a flag.** It is `--branch`.

## Verification

| Gate | Result |
|---|---|
| `make test` | 288 ok, 0 failures |
| `cargo test -p worktrees-core` | 205 passed |
| `cargo test -p worktrees-cli` | 6 passed |
| `make lint` | clean (shellcheck + bash-3.2) |
| `tsc --noEmit` | clean |
| `cargo check -p app` | ok |
| CI on #89 | 9/9 pass, both OSes |

New tests:

- `new: tracking ref already present → no fetch at all (works with origin
  unreachable)` — points `origin` at `/nonexistent/origin.git`, so a network
  attempt cannot pass silently.
- `new: restricted refspec → remote-only branch is not materialized, falls
  through to base` — locks in the documented narrowing.

Mock harness (Playwright, port 5199):

- `?slowcreate=60000` → form dismisses, ghost renders (`listitem "creating
  feat-ghost-row…"`), resolves into the real selected row.
- `?slowcreate=1500` + branch `feat/fail-me` → error banner, form returns with
  all three fields intact (`feat/fail-me` / `my-topic` / `devlop`), no stranded
  ghost.

The xterm `Cannot read properties of undefined (reading 'dimensions')` console
error in the harness is **pre-existing** (hidden terminal, zero dimensions), not
from this change.

Screenshots in `~/.cache/worktrees/worktrees/ui-changes/`
(`ghost-row.png`, `ghost-row-inflight.png`).

## Follow-ups

- **`do_switch`'s two-fetch pair** — in ROADMAP. Reached from `cmd_new` on the
  holder-reuse path, so `new` can still pay two network waits there.
- **The ghost is suppressed on that same holder-reuse path** — the slug dedup
  hides it because the place already exists. Marking the *existing* row busy is
  the fix; a second ghost is not.
- **Mock fault injection is `new_place`-only** — the roadmap item is struck as
  done, with a note that other commands still always succeed.
