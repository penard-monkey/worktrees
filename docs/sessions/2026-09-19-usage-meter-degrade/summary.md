---
title: "The widget that hid instead of ageing"
---

# The widget that hid instead of ageing

- **Date:** 2026-09-19
- **Working tree:** `.worktrees/bug-fixes`
- **Branches:** `bug-fixes-usage-meter-survives-a-failed-poll` (fix),
  `bug-fixes-close-out-usage-meter` (this archive)
- **PR:** [#309](https://github.com/penard-monkey/worktrees/pull/309) —
  squash-merged as `2038412`
- **Release tag:** none — sits in `[Unreleased]`, after v0.26.0
- **Planning files:** none. The session began as a one-line question about a
  screenshot and stayed small enough to hold in the thread, so there is no
  `planning.tar.gz` beside this summary.
- **Evidence:** the app.log extract that diagnosed it is kept outside the repo
  at `~/.cache/worktrees/worktrees/bug-fixes/2026-09-19-usage-429-episode.txt`;
  the reporting screenshot is `_tmp/Screenshot 2026-09-19 at 13.35.26.png`.

The session started with a screenshot and a question: *why did I temporarily not
see the usage-meter?* The answer was in `app.log` within two greps — the OAuth
usage endpoint returned HTTP 429 from 18:30:58Z to 18:35:41Z, and the screenshot
was taken at 18:35:26Z, four and a half minutes into it. What made it worth a PR
rather than a reply was the second half: the app had been actively extending the
outage it was hiding from.

## What shipped

**A failed poll degrades before it hides** (`app/src-tauri/src/lib.rs`,
`usage_degraded`). The last live reading stays on screen, re-labelled
`source: "cached"`, dimmed, and named in the panel ("last good reading") with the
time it was taken. Bounded at 30 minutes by `USAGE_STALE_MAX_SECS`. Past that,
the statusline snapshot takes over at any age, and failing that the widget hides
exactly as before.

**A failure buys the same quiet as a success** (`usage_in_backoff`,
`USAGE_FAIL_AT`, 60s). `USAGE_TTL_SECS`'s 120-second floor between real fetches
had only ever applied after a success, because nothing was written to the cache
on failure.

**The failure log line says what the user will see** — `— showing cached reading
from 137s ago` or `— widget hidden` — replacing a pair of lines that claimed the
widget was hidden unconditionally, which was about to stop being true.

**Frontend** (`app/src/App.tsx`): `stale` becomes `source !== "oauth"` rather
than a match on `"statusline"`, and `usageSourceLabel()` names the three states
in the tooltip and the panel head. `app/src/mock/install.ts` gains
`?usage=cached`.

## Decisions

**Degrade in the backend, not the frontend.** `useUsage` could have held its last
non-null `info` across a failed poll, which is fewer lines. It was rejected
because the backend answer then has to be re-derived by every caller, and because
the frontend's copy dies on a remount while the Rust static does not. The command
now owns one honest answer and every host renders it.

**The cached reading outranks a *fresher* statusline snapshot.** The one place
this code does not prefer newer data. The statusline rewrites
`~/.claude/widgets/rate_limits.json` on every Claude Code prompt, so on a machine
that runs one the snapshot is almost always newer than a cached reading that is
≥120s old by construction — but it is a poorer *shape*: two rows instead of
three, no severity tiers, no model bucket. Preferring it drops a bar and loses
the amber tier for the length of an outage and then restores them, which is the
layout moving precisely when the function exists to hold it still. Over a 5h and
a 7d window there is nothing to choose between a reading two minutes old and one
five seconds old; between three bars and two there is.

**Two constants, not one.** `USAGE_STALE_MAX_SECS` duplicates
`STATUS_STALE_MAX_SECS`'s value and its inclusive bound deliberately. They answer
for different data, and a future adjustment to one window must not silently move
the other.

**Suppressed pulls are not logged.** One line per real attempt. The burst of
pulls *is* the bug being fixed, and logging each suppression would reproduce its
shape in the file used to diagnose it.

## Dead ends / gotchas

**The sibling feature had already solved half of this, and nobody noticed.**
`claude_status` — in the same file, ~300 lines further down — implements the
identical "serve the last answer, bounded" pattern as `stale_or_silence` /
`STATUS_STALE_MAX_SECS`, at the same 30 minutes, with a comment giving the same
reasoning. It was found only because a new test name collided in the test-list
output while checking that the new tests failed first. The new code was then
written to match its boundary convention (`<=`, inclusive) rather than the
arbitrary one it started with. **When adding a degradation path, grep the file
for one that already exists.**

`claude_status` does *not* have the negative TTL, though — same missing floor,
same burst shape available to it. Left alone: it is an unauthenticated Statuspage
GET and it did not fire once during the episode. See *Follow-ups*.

**A clean rebase produced a wrong result.** v0.26.0 was tagged mid-session. The
CHANGELOG entry, written under `## [Unreleased]`, rebased *cleanly* into the
now-released `## [0.26.0]` section — the surrounding context still matched, so
git had no reason to conflict — and the fix would have been documented as part of
a release it is not in. Caught by reading the rebased file rather than trusting
the exit code. **After any rebase that touches CHANGELOG.md across a release
boundary, look at where the entry landed.**

**`origin/main` moved twice mid-session, once without a fetch.** #307 (the
release) between the first fetch and the first rebase, then #308 between that
rebase and the push — worktrees share refs, so another tree's fetch updated
`origin/main` under this one with nothing run here. #308 also added a `mermaid`
dependency, so `tsc --noEmit` failed on *its* files until `pnpm install` — a
failure that reads exactly like one the current change caused.

**`git checkout -B <idle-base> origin/main` sets the idle base to track
`origin/main`.** A later bare `git push` from the parked worktree would then
target main directly. Unset it: `git branch --unset-upstream`.

**`gh pr merge` failed and the merge had landed** — `fatal: 'main' is already
used by worktree at …`, from `gh`'s local checkout step, after GitHub had already
merged. The documented behaviour; `gh pr view 309 --json state,mergeCommit`
confirmed `MERGED` rather than a retry re-merging.

**The mock harness will not poll in a background tab, and that is correct.**
`useUsage` gates on `document.visibilityState`, so a driven tab that is not
frontmost never fetches and the widget never appears — which reads exactly like
the render being broken. Dispatching a synthetic `focus` at `window` forces the
pull. A harness affordance, not a claim about the real app.

**`.click()` opens the panel where `.focus()` and `pointerenter` do not.** In an
unfocused tab, `focus()` fires nothing and React's `onPointerEnter` did not fire
from a synthetic `pointerenter`; `element.click()` reaches React's delegated
`onClick` and pins the panel. (Consistent with the existing CLAUDE.md note that
`.click()` dispatches no `pointerdown` — useful here, since the pin handler is
the one that does not need one.)

## Verification

Both new behaviours were **shown to fail first**, per CLAUDE.md. Reverting
`usage_degraded` to the pre-fix order and stubbing `usage_in_backoff` to `false`
turned 4 of the 5 new tests red; the fifth
(`with_nothing_held_the_widget_still_hides`) passes either way by design, because
it pins the behaviour that did not change. The revised tie-break test was
separately shown to fail against the fresher-wins rule it replaced.

Rendering was **measured in the mock harness**, not eyeballed — the three states,
by computed style and DOM text:

| state | class | opacity | panel head |
|---|---|---|---|
| live | `usage-trig line` | 1 | "live" |
| cached | `usage-trig line stale` | 0.55 | "last good reading" |
| statusline | `usage-trig line stale` | 0.55 | "statusline snapshot" |

Gates: `cargo test -p app --lib` (109), `-p worktrees-core` (385),
`-p worktrees-cli` (14), `cargo check -p app`, `tsc --noEmit`, and all eight
`app/scripts/*-check.mjs`. CI green on both pushes, read from raw job conclusions
rather than the PR view. `make test` was **not** run for the fix PR — nothing
under `crates/` changed, so bats would have been exercising an untouched binary.

**Review found two real defects in the first commit**, both fixed in a follow-up
before the merge (see the PR comment on #309):

1. *The degrade read a pre-fetch snapshot of the cache.* `cached` was cloned
   before curl spent up to 15 seconds, and overlapping pulls are routine here —
   the focus listener and the interval both invoke the command, which is the very
   doubled-pull shape the new backoff exists to damp. So A gets 200 and writes a
   fresh reading, B gets 429 and degrades from a copy taken 15 seconds ago; the
   frontend takes whichever response lands last, so the *loser* of the race dims
   a live widget — or hides it outright on a cold start, where the pre-fetch copy
   is `None` — for up to a full poll period. The `Err` arm now re-reads
   `USAGE_CACHE` and, if what it finds is inside the positive TTL, answers as the
   top-of-function check would have.
2. *The tie-break preferred the fresher fallback.* Described under *Decisions*
   above. Caught because the CHANGELOG entry claimed behaviour the code did not
   implement — the prose and the code disagreeing is what surfaced it.

The race is guarded by the re-read rather than by a test: reproducing it needs a
controlled interleaving of two network results, which `claude_usage` cannot
express without being restructured around an injectable fetch.

## Follow-ups

- **`claude_status` has the same missing negative TTL.** Its cache is likewise
  written only on success, so a status-page outage bursts the same way. Lower
  stakes — unauthenticated Statuspage GET, did not fire during this episode —
  but it is the same defect in the sibling, and the fix is the shape already
  shipped here.
- **`claude_usage` is not testable around its fetch.** Both defects found in
  review live in the command body, where the network call is a direct
  `usage_from_oauth` rather than an injectable seam. If this area grows again,
  that seam is what would let the race be pinned by a test instead of a comment.
