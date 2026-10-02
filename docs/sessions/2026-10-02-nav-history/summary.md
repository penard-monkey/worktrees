---
title: "Session — back / forward navigation history"
---

# Session — back / forward through what the two panes showed

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/nav-history`
- **Branches:** `nav-history` (the feature), `nav-history-close-out` (this
  archive)
- **PRs:** [#416](https://github.com/penard-monkey/worktrees/pull/416),
  squash-merged by the orchestrator in `(main)` as `a325db6` after review
  ("merge after fixes": two wiring bugs, both fixed with a regression guard).
- **Release:** none. The entry sits under `[Unreleased]` → `### Added`.
- **Planning files:** `.planning/brief.md` and `.planning/nav-history/`
  (`task_plan.md`, `findings.md`, `progress.md`), in `planning.tar.gz` here.
- **Design:** [`docs/proposals/nav-history.md`](../../proposals/nav-history.md),
  approved the same day with four decisions recorded in it.

## Why

David asked for browser-style history over what the app shows. That covers
the place picked in the left nav, whatever the right side is on, and which
terminal is in front. ‹ › buttons and ⌘← / ⌘→ walk it. He asked for a
design to review first, then small atomic slices, each one pushed, because he
might stop the lane partway through. He did pause it once.

## What shipped

- `app/src/navHistory.ts` (new, pure). It holds the rules: `record` (push or
  amend), the 600 ms sliding coalesce, the same-location no-op, truncate on
  push after Back, `reachable`/`step` skipping dead entries, `prune`, a cap of
  100, `normalize` (a file only counts on Files, a shell only on Terminal),
  and `label`.
- `app/src/App.tsx`:
  - **Observer.** One effect derives `navLoc` and records every change. A
    location is the place or Home, plus `dock: {tab, file{path,line,col},
    shell}`. Declared below the launch restore, so the restore coalesces into
    entry 0.
  - **Apply.** `navGo` selects the place, then parks the dock half in
    `navPending` until `sel` lands (the `pendingDoc` shape). Only what differs
    is written, so no `place_panels` seed is frozen. The `files_open` restore
    stands down for a place history is applying. `navApplying` ignores
    observations until the target shows, or for 1.5 s, after which whatever
    landed amends the entry. A pointer press, or a selection history did not
    ask for, ends it early. `navApplyGen` tickets drop a `file_readable`
    answer for an entry history has already left.
  - **Shells.** `TerminalTabs` gained `onFront` (the tab actually in front;
    `undefined` while it restores or is unmounted) and a `goto` token.
    `navSeenFront` remembers the last front tab per place for a closed dock.
  - **UI.** `NavButtons` (module scope) leads the topbar, with
    `.navhist-home` at the top left on Home. Tooltips name the destination;
    right-clicking lists up to 12 entries in a `CtxMenu`. `navChordDir` and
    `isTextField` at module scope; the chord branch sits in the window `onKey`
    below the modal guard, with repeat allowed. Mouse buttons 4 and 5 are
    handled on `mouseup`. Removing a place or project prunes its entries.
- `app/src/App.css`: `.navhist`, `.navhist-home`.
- Checks:
  - `app/scripts/navhistory-check.mjs` evaluates the real module.
  - `app/scripts/navwire-check.mjs` slices the real `frontHere…navLoc` lines
    and the parked-apply effect out of App.tsx and drives them with stubs and
    a delayed stat, the race-check shape.
  - `app/scripts/zoom-check.mjs` inlines the real `navChordDir` and drives the
    history chord in the same handler slice it already ran.
- `CHANGELOG.md` `[Unreleased]` → Added; `docs/proposals/nav-history.md` plus
  a row in the proposals index.

## Decisions

- **Observe rather than instrument.** Pushing is the default and the app's own
  moves amend. A missed system path costs one extra entry, while a missed
  user path under instrumentation is a silent hole. So a way of navigating
  added later is recorded without anyone remembering to.
- **⌘[ / ⌘] always, ⌘← / ⌘→ outside text fields, the terminal included.**
  Evidence from xterm 5.5's `Keyboard.ts`, `case 37/39: if (ev.metaKey)
  break;`: ⌘← / ⌘→ send no bytes and the event is not cancelled, so in a
  terminal they did nothing before. Text fields keep line start and end.
  `.xterm-helper-textarea` is a textarea and has to be special-cased. Chords
  are matched on `e.key` with an `e.code` fallback.
- **Approved by David:**
  - the Project sheet is not an entry;
  - `dock_open` (⌘J) is not part of a location;
  - the coalesce window is 600 ms;
  - all chords are unbound under a modal.
- **The agent is not part of a location**, dropped while building. A place
  runs one live agent (`App.tsx` reconciles a second away), so a switch
  replaces the session and there is nothing to go back to.
- **Not persisted.** The history lives in memory and is per window.
  `restore_last` plus `files_open` already return you after a relaunch, and
  `ui-state.json` is a whole-blob file.
- **Below the place, degrade; at the place, skip.** A removed place is skipped
  through `navAlive` and pruned on removal here. A deleted file lands on the
  Files tab with a notice. A closed shell leaves the current front tab.
- **Labels leave out a bare "Files".** With ⌘J outside the location, every
  place's default tab said nothing.

## Dead ends / gotchas

- **A stale front shell truncated forward history.** On a remount the App's
  first render still held the previous front tab, so the apply "landed"
  early. The strip's restore then reported nothing, which was recorded as a
  push that cut off everything forward. Fix: the strip reports `undefined`
  while restoring and on unmount, and the location is not observed while the
  front shell is unknown.
- **That fix created review bug 1.** It waited for a front shell whenever the
  tab was Terminal, but only the OPEN dock mounts a strip. A place visited
  with the dock closed on Terminal was never recorded, and because
  `updatePanels` spreads the dock state over the globals, neither was any
  place visited after it. Now it waits only while `stripUp` (`dockShown` and
  Terminal). The first version of that fix still made ⌘J a step: the strip's
  restore never writes `term_tab_active`, only a pick does. Hence
  `navSeenFront`.
- **Review bug 2.** The apply's `file_readable` `.then` had no cancellation,
  although the `files_open` restore four hundred lines up guards the same race
  with `alive`. Two quick Backs whose first stat answered last opened one
  place's file inside another, and the observer pushed it.
- **`zoom-check.mjs` broke on the new chord branch.** It slices the real
  handler up to the meta-only gate, and the history branch sits inside that
  slice. Inlining the real `navChordDir` was the right fix, rather than a
  stub. Its first fake element did not model xterm's sink as a textarea, so
  the xterm mutation stayed GREEN until the fake did.
- **A hidden nav's `input.search` cannot take focus.** The first chord test
  "in a text field" really ran from `body`. Dispatching at the element is
  what tests the target.
- **Pre-existing harness page errors:** xterm `dimensions` and
  `unregisterListener` on place and tab switches. Confirmed on `main`'s
  App.tsx with the same clicks, so they are not this change's doing.
- **Coalescing changes what a repro means.** The reviewer's `race.mjs` records
  "P2" and "P2·Cargo.toml" as separate entries, because its waits exceed
  600 ms. Its two quick Backs therefore land on "P2, no file", which is
  correct, and the stale answer is dropped there: the same-place case.

## Verification

- `navhistory-check` was red against 4 mutations. `zoom-check`'s chord cases
  were red against 3. `navwire-check` gave 7 not ok against the pre-fix
  App.tsx; reverting only fix 1 left 4 failures, and only fix 2's guard, 3.
- The mock harness was driven in headless **Chromium and WebKit** (scripts in
  `~/.cache/worktrees/worktrees/nav-history/`). It covered places and Home,
  buttons, every chord dispatched at the focused element (search field, body,
  xterm sink), mouse button 4, dock tabs, files, `show_doc`, `?stalefile`,
  shell tabs and the right-click list. Buttons were hit-tested with
  `elementFromPoint`. The reviewer's `closeddock`, `twoback`, `race`,
  `shells` and `removed` all behaved after the fixes.
- Gates on `e29992f`, whose tree is byte-identical to `a325db6`:
  - release build fresh;
  - `make test` 1..451 with 0 not ok;
  - lint;
  - core 665, cli 70, app 151 tests;
  - `tsc`, `cargo check -p app`, and 30 of 30 frontend checks.
- CI run 37034260519: all 9 jobs succeeded, read raw.

## Follow-ups

- Real-app pass: whether a real mouse's side buttons (4 / 5) reach WKWebView
  as `mouseup` `button` 3 / 4. Only synthetic events have been seen.
- The Home arrows (`.navhist-home`, top left of `.main`) sit under the
  hover-reveal Places overlay when the sidebar is unpinned and revealed.
- Not in v1, by design: the Docs, Plan and Automations panes' inner
  selection. Anything they open lands in Files, which is recorded.
