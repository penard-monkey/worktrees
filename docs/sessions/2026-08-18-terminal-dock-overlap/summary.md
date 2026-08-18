---
title: "2026-08-18 — the terminal ran on under the Files dock"
---

# Session: a terminal that could grow but never shrink

- **Date**: 2026-08-18 (work started late 2026-08-17)
- **Worktree**: `.worktrees/overlay-issues`
- **Branches**: `overlay-issues-termfit` (fix),
  `overlay-issues-closeout` (this archive). The tree arrived parked on a bare
  `overlay-issues`, which is neither a work branch nor an idle base under this
  repo's `<tree>-<something>` / `<tree>-next` rule; it now parks on
  `overlay-issues-next`.
- **PRs**: [#157](https://github.com/penard-monkey/worktrees/pull/157) (fix,
  squash `9dc108b`)
- **Release**: none — sits in `[Unreleased]` for the next one
- **Planning files**: `planning.tar.gz` beside this summary

## What shipped

The place's terminal and the dock's shells stop painting underneath the Files
dock. Reported from a screenshot ([overlap-reported.png](overlap-reported.png),
red annotation "We have an overlap here"): tmux's status line and every line
reaching the right margin were cut off mid-glyph at the terminal/dock seam.
[fixed.png](fixed.png) is the same layout after.

- `app/src/App.css` — `.term-host` gains `min-width: 0` (the fix) and
  `overflow: hidden` (backstop), with the reasoning inline.
- `app/scripts/termfit-check.mjs` — new static guard on those two
  declarations plus the premise they rest on.
- `CHANGELOG.md` — `[Unreleased] / Fixed`.
- `CLAUDE.md` — a hard-won rule: an xterm host is a ratchet without
  `min-width: 0`.

## Root cause

`.term-host` is a **row** flex item (`.term-wrap` is `display: flex` with no
direction). It declared `min-height: 0` but not `min-width: 0`, so its computed
`min-width` was `auto` → the automatic-minimum-size rule floored it at its
**min-content** width. That floor is not small: xterm writes an explicit
`width: <cols × cell>px` onto `.xterm-screen`, so the floor *is* the grid the
terminal is painting at that moment.

The host could therefore only ever grow. It is also the box
`TerminalPane.tsx`'s ResizeObserver watches, so the failure is self-sealing:

```
dock opens → .main narrows 1212→852 → .term-host stays 1206 (floored)
           → RO sees no change → no fit() → no term_resize
           → tmux still paints 1190px of columns → the last ~50 sit behind the dock
```

Measured in the mock harness (`getBoundingClientRect`, viewport 1600, place
`messaging`):

| | `.main` | `.term-host` | `.xterm-screen` | `.dock` |
|---|---|---|---|---|
| dock closed | 1212 | 1212 | 1190 | — |
| dock open, **before** | 852 | **1206** | **1190** (unchanged) | 1196–1556 |
| dock open, **after** | 852 | 852 | 830 (re-fit) | 1196–1556 |

Before: `.term-host` overhung the dock's left edge by **354px**. After: 0.

Both terminal kinds share the rule via `TermSurface`, so the dock's own shells
had the identical ratchet when the dock was dragged narrower — one fix covered
both.

## Decisions

- **Fix at `.term-host`, not at the observer.** The tempting alternative is to
  observe `.term-wrap` (which does track `.main`) instead of the host. That
  papers over it: the host would still overflow its wrapper and still paint over
  the dock, only now with a correct column count — a subtler version of the same
  screenshot. The flex floor is the actual defect.
- **`overflow: hidden` as well as the fix.** Not needed in steady state — the
  fit addon floors cols, so the screen is never wider than its box, and
  ResizeObserver callbacks run before paint. It is there so that *no* future
  mid-resize frame can paint a grid over a neighbour again. Cheap, and this
  class of bug is invisible until someone screenshots it.
- **A static guard script, not a test.** No suite in this repo can see this
  bug: the size written is always correct *at the moment it is written*, and
  only becomes wrong when something else takes width away. `make test`, the 295
  Rust unit tests and every mock harness check pass with the bug in.
  `termfit-check.mjs` asserts the CSS declarations instead — same idiom as
  `dnd-check.mjs` guarding the tier mirror.
- **The guard checks its own premise.** It fails if `.term-wrap` stops being a
  row flex container, rather than silently continuing to assert a rule that no
  longer carries anything. That is the failure mode of a stale guard, and this
  repo has been bitten by it before (the dnd mirror's own unit tests kept
  testing the OLD rule).

## Dead ends / gotchas

- **The mock harness reproduced this perfectly** — worth recording, because
  CLAUDE.md's standing warning is the opposite ("the mock answers INSTANTLY, and
  that hides a whole class of bug"). That warning is about *timing*. This was
  pure CSS: same stylesheet, same xterm, same flex algorithm, so Chromium and
  WKWebView agreed on the broken behaviour and agree on the fix. Rule of thumb:
  a layout bug is mock-visible, a refresh/ordering bug is not.
- **A survivor vite held port 1431 after `pkill -f "vite --port 1431"`
  reported success.** The real cmdline is
  `node ./node_modules/.bin/../vite/bin/vite.js --port 1431` — the pattern
  matched nothing, `lsof` still answered, and the survivor kept serving
  pre-rebase modules from its cache while `curl` of the *CSS* looked correct
  (that file had been edited before the survivor started). Kill by PID from
  `pgrep -fl vite`, and check what the server serves for the file you actually
  changed. Note `pgrep -fl vite` also lists **other worktrees'** harnesses —
  the ui-tweaks tree had one on 5199; do not kill by name across trees.
- **A backgrounded relaunch inherits the session cwd, not the last `cd`.** Two
  harness starts died with exit 127 (`no such file or directory:
  ./node_modules/.bin/vite`) because the Bash tool's cwd had moved back to the
  repo root. Put an absolute `cd` inside the same command.
- **`gh pr merge` reported a failure it did not cause** — the documented
  *fatal: 'main' is already used by worktree at …*. That is `gh`'s local
  checkout step, after the merge landed. `gh pr view 157 --json state` said
  `MERGED`. Do not retry.
- **A guard has to be shown red.** Written green, `termfit-check.mjs` proves
  nothing. It was run against the pre-fix rule (2 failed, exit 1) before being
  trusted, and the review's added case (`flex-flow`) was likewise mutation-tested
  before the widening was believed.

## Verification

- Mock harness, `getBoundingClientRect` on every path that changes the
  terminal's width — not eyeballed:

  | path | before | after |
  |---|---|---|
  | open dock | 354px overhang | overhang 0, screen re-fit 1190→830 |
  | close → re-open dock | ratchets wide, never returns | 1212 → 852, tracks exactly |
  | window 1600→1150 | overhangs | main/host 420, screen 399, no body h-scroll |
  | dock drag 360→660→310 | dock shell ratchets too | both hosts track, spill 0 |

  Re-run after rebasing onto #155, which also touched `App.css`/`App.tsx`.
- `overflow: hidden` side-effects: host `scrollWidth == clientWidth` (nothing
  clipped), xterm viewport fully inside the host, ⌘F opens, searches (1/8, 9
  decorations) and the find bar still renders outside the host box — it anchors
  on `.term-wrap`, not the host.
- Gates: bats 314 / 0 `not ok` · core 251 · cli 7 · app `--lib` 37 ·
  `tsc --noEmit` · `cargo check -p app` · `make lint`. CI 9/9 twice (once after
  the review fix).
- **Fable review of the diff before merge.** Verdict merge-ready; it read the
  FitAddon source and xterm's stylesheet to rule out the clip harming the helper
  textarea, char-measure element, decorations or scrollback, swept every
  `display: flex` in `App.css` for sibling ratchets (none — `.dock-tree`,
  `.dock-content`, `.code-text` already declare `min-width: 0`), and
  mutation-tested the guard against seven CSS edits. One finding taken: the
  premise check knew only `flex-direction: column`, so rewriting `.term-wrap`
  with the `flex-flow: column` shorthand would have made the width assertions
  vacuous while the script still printed "all good". Widened to
  `flex-(?:direction|flow)` and shown red against that mutation.

## Follow-ups

- **The fix has not been seen on WKWebView.** Everything above is Chromium via
  the mock harness; the original report came from the real app.
  `app/scripts/sandbox.sh --app` would close that gap. Low risk (the flex rule
  and xterm's inline width are engine-independent, and both engines agreed on
  the *broken* behaviour), but unobserved is unobserved.
- **The static check scripts run only when someone remembers.**
  `termfit-check.mjs`, `dnd-check.mjs` and `relpath-check.mjs` are pure Node
  with no browser and no fixtures, and none is wired into `ci.yml` or the
  Makefile. Raised in review as informational; deliberately left out of the fix
  PR because it is a repo-wide call. In ROADMAP.
