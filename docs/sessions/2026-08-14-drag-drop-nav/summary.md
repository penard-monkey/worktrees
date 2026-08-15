---
title: drag a place into another group to put it there
---

# drag a place into another group to put it there

- **Date:** 2026-08-14
- **Worktree:** `.worktrees/reorder-places-projects`
- **Branches:** `reorder-places-projects` → merged; archive on
  `reorder-places-close-out`; tree parks on `reorder-places-next`
- **PRs:** [#133](https://github.com/penard-monkey/worktrees/pull/133)
  `feat(app): drag places between tier groups, drag projects into order`
  (squashed as `58ad9ef`)
- **Release tag:** none — sits in `[Unreleased]` after v0.13.0
- **Planning files:** `planning.tar.gz` beside this summary
- **Screenshots:** none (the feature is a gesture; the harness assertions in
  `## Verification` are what stands in for them)

## What shipped

Dragging a row in the nav used to be a sorting gesture and nothing else: it
worked only in Manual sort mode, and only inside the group the row already sat
in. A row's GROUP — pinned, archived, saved — could be changed from the context
menu or the topbar, never by moving it to where you wanted it.

Now a drop applies the group. Dropping a row on Pinned pins it; on
Saved/Closed/Archived/Abandoned sets that lifecycle; in every sort mode. A gap
opens where the row will land — at the pointer under Manual, and under A–Z or
Activity at the slot those modes will actually produce, because a gap that
followed the cursor in a sorted list would be a promise the list does not keep.
Projects reorder by dragging their headers.

New files:

- `app/src/dnd.ts` — the PURE half. `predictTier`, `dropIntent`, `landingNote`,
  the three landing-slot functions (`alphaIndex` / `recentIndex` /
  `pointerIndex`), `naturalTop`, and the manual-order splicing. No React, no
  DOM, no clock of its own, which is what makes it checkable.
- `app/src/navdrag.ts` — the PLUMBING half. `useNavDrag`: a pointer-events drag
  controller with a 4px threshold, rAF-throttled hit-testing, edge auto-scroll,
  Escape cancel, and click swallowing.
- `app/scripts/dnd-check.mjs` — 60 checks over `dnd.ts`, plus assertions that
  re-read `store.rs` and `lib.rs` so a drift in the constants it mirrors fails
  HERE rather than as a drop that predicts the wrong group.

Changed: `app/src/App.tsx` (drop zones, gap, empty-tier stubs, spring-open,
ghost, flash, undo, project reorder), `app/src/App.css`, `app/src-tauri/src/lib.rs`
(`reorder_projects`, clearable `set_lifecycle`), `app/src/mock/install.ts`
(parity for both), `CHANGELOG.md`.

## Decisions

**Half the tier groups are not labels, and the feature is built around that.**
`store::reconcile` (`crates/worktrees-core/src/store.rs:101`) derives **Active**
from live tmux and **Idle** from `last_opened_epoch`, so there is nothing to
write when a row is dropped on them. Three options were on the table: pretend
(stamp `last_opened_epoch` — rejected, it lies to the Recent lens and
`activityAt` about when a place was last used), refuse both groups outright, or
be honest. Honest won: `dnd.ts::predictTier` mirrors `reconcile` client-side, so
the drag knows before it commits which group the row will really land in. The
gap opens THERE, the ghost names it, and when it differs from the group under
the pointer the app says why ("still Active — the tmux session is running").

**Dropping on Active is refused, with a reason.** It is the one group with no
honest interpretation — "make this have a live tmux session" is `Enter`, not a
drag. The refusal says so rather than silently doing nothing.

**A drop inside a row's OWN group writes nothing.** Without this, refusing the
derived groups also killed reordering inside Active and Idle — a regression paid
for a new feature. `dropIntent` takes the row's current tier for exactly this.

**Pointer events, not HTML5 drag-and-drop.** Three reasons, all pre-paid by this
repo: HTML5 DnD only works here because `tauri.conf.json` sets
`dragDropEnabled: false` (a WKWebView workaround that also costs the app any
chance of a Finder folder-drop); ROADMAP's "smoke the v0.5.0 dock + drag" item
records that it is undrivable by the mock harness, because Playwright cannot
drive a WKWebView dataTransfer; and a gap opened under the cursor changes what
is under the cursor, which with dragenter/dragleave is a feedback loop.

**The badge is patched optimistically to the PREDICTED tier.** The first cut
patched only `pinned` optimistically, on the documented ground that
`lifecycle_effective` is reconciled server-side. Review pointed out that the
rationale predated `predictTier`: with the mirror in place and drift-guarded by
`dnd-check`, the prediction IS the server's answer. Without it, the real app —
where `list_workspace` is a git fan-out of seconds — closes the gap, fires the
flash in the group the row is LEAVING, and can expire the 1.6s flash before the
row ever moves.

**Empty groups appear while dragging.** A tier with no rows renders nothing, so
before this you could not pin the first place in a project, or reach a tier you
had emptied. Same reasoning made the padding above the first project a drop
target (see gotchas).

**Project order lives in `projects.json`, so `reorder_projects` rewrites it.**
The incoming order is treated as a PREFERENCE over the roots that exist, not as
truth: roots the file no longer has are dropped, roots the dragger never saw are
kept. A stale window can permute the workspace; it can never delete from it.
The alternative — a separate order field in `ui-state.json` — would have
duplicated a fact the file already carries, and `ui-state.json` is
frontend-owned whole-blob, which the backend must not write.

## Dead ends / gotchas

**`spliceOrder` sent a row dropped on ITSELF to the end of the list.** Removing
the moved slug first makes `indexOf(beforeSlug)` miss, and the `-1` fallback is
"append". Caught by the check script before any UI existed — the argument for
writing the pure half first.

**A gap under the cursor oscillates unless you compensate arithmetically.** The
open gap displaces every row below it, so a pointer resting INSIDE the gap sits
past the next row's displaced midpoint, which computes the next slot down, which
moves the gap, which moves the pointer back inside it. `naturalTop` puts rows
AND the pointer back where they would be without the gap, clamping the
inside-the-gap case to the gap's own top so the answer there is a fixed point.
The check that proves it goes red if the clamp becomes a plain subtraction.

**Undo could revert ANOTHER repo's manual order.** Found in review. The undo
closure captured `settings` at drop time and wrote the whole `manual_order` map
back; a pure reorder in a different project inside the 8s window does not
replace the undo banner (it takes the `!moved` early return), so clicking Undo
restored that repo's order from the stale snapshot — and `updateSettings`
persists the whole blob, so it was lost on disk too. Same family as CLAUDE.md's
`ui-state.json` whole-blob trap. `updateSettings` now accepts a functional
patch; undo merges only its own repo's key against current state, and deletes
the key when there was no prior order.

**An Escape-cancelled drag still ENTERED a place on release.** Also from review.
The click swallow was armed and then cleared on a 0ms timeout — but on Escape
the button is still DOWN, so the timeout fired long before the eventual pointerup
and the click that followed passed straight through to `enterPlace`. The user
aborted the gesture and the app opened a place. Swallowing is now a window-level
capture-phase one-shot that waits for the pointerup when the button is still
held, which also covers drops released over BUTTONS — so the five per-handler
guards are gone.

**The undo banner painted over the landing explanation.** Both toasts were
`position: fixed` at the same offset, and a mismatch drop sets BOTH — so the
message explaining where the row really went was covered by the message
announcing the move. The Playwright checks had verified the notice's STATE, not
that it was visible. They now render in one bottom-anchored `.float-stack`.

**A reported regression that was not one.** Mid-verification I reported project
reorder "broken by the refactor" and started debugging the refactor. It was
correct: my first test aimed at a position that was already a no-op (the dragged
project was ALREADY before the target), and the second aimed at the scroller's
top edge. Check what the test asks for before blaming the code. The second bad
target did find a real bug, though —

**The strip above the first project took no drop.** `elementFromPoint` over the
scroller's own padding returns `.nav-scroll`, whose `closest('[data-project-root]')`
is null, so no gap and no drop — and that strip is exactly where you aim to make
a project FIRST. Project drags now accept anywhere inside `.nav-scroll`, which
also makes the empty space below the last project mean "send it to the end".

**The harness's own leftovers can eat the hit-test.** A drag driven by
synthetic pointer events bypasses hit-testing on the way IN, so it can start
while a full-screen `.menu-catch` overlay is up — and then `elementFromPoint`
answers with the overlay for the whole drag. A real pointerdown would have
dismissed the overlay instead of starting a drag, so this is a test artifact;
`body.dragging .menu-catch { pointer-events: none }` makes it a non-issue either
way.

**Two agents on one working tree.** A subagent that appeared stalled (no
transcript growth, no edits on disk for over a minute) was actually still
running; a replacement was spawned and both edited the same five files. They
converged only because the brief was identical and each re-read before writing.
Check `git status` mtimes AND transcript growth before concluding an agent is
dead — and prefer resuming one over spawning its twin.

**The two-vite-on-one-port trap is real and was hit.** Port 5199 ended up held
by another worktree's dev server. Every harness result in this session was taken
against a server whose served files were content-checked first (`curl … | grep`
for something the edit added, using CODE, never a comment — esbuild strips
comments).

## Verification

Gates, all green, on the rebased tree: release build first, `make test` (288
bats), `make lint`, `cargo test -p worktrees-core` (208), `-p worktrees-cli` (6),
`-p app --lib` (30), `tsc --noEmit`, `cargo check -p app`, `race-check.mjs` ALL
GREEN, `dnd-check.mjs` (60 checks). CI on #133: 9/9.

New tests were shown to FAIL first, per CLAUDE.md: the unpin-on-dormant rule
(reverted → 2 red), the gap clamp (plain subtraction → 3 red), the
project-order merge guard (dropped the known-roots filter → red with the exact
`/gone` root written back), the pin-precedence invariant the optimistic badge
leans on (reverted the pin override → 8 red), and tie order in `sortedIndex`
(`<=` → `<` → 1 red).

Driven in the mock harness with real pointer events: tier drops in all three
sort modes; Active rejected with its hint; a pinned row dropped on Idle landing
in Active with the explanation; dormant drop after spring-open; empty-tier stubs
appearing and disappearing with the drag; the alpha slot ignoring where the
pointer was; a gap held stable across 400ms of a motionless pointer; project
reorder including into the strip above the first project; Escape cancel followed
by a release over a different row NOT entering it; a plain click still entering.
Under `?slowlist=1200` the row lands in its destination immediately, the flash
fires there, and neither it nor the project order snaps back when the slow
refresh lands.

**Not verified:** a real-app (non-mock) drag. No tool in the session can drive a
pointer in a native WKWebView window. `app/scripts/sandbox.sh --app` is the
manual pass; a launch was made and exited on its own without being driven.

## Follow-ups

- **Real-app drag pass** — the one gate no automation here can run. Seven steps
  worth trying, ordered so the timing-sensitive ones come first, are in the
  roadmap entry.
- The `[Unreleased]` section now holds three features (⌘F, markdown zoom, this)
  — a v0.14.0 release when they have been used in anger.
