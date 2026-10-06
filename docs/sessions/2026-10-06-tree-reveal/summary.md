---
title: "2026-10-06 — reveal the open file in the Files tree"
---

# Reveal the open file in the Files tree

- **Date:** 2026-10-06
- **Worktree / branch:** `tree-reveal`
- **PR:** #449, squash-merged as `28e8efe` (after v0.39.0)
- **Planning files:** `findings.md` plus the brief (`.planning/brief.md`), both in `planning.tar.gz`
- **Scratch:** the harness scripts (`measure.mjs`, `toggle.mjs`, `mutants.py`) are in `~/.cache/worktrees/worktrees/tree-reveal/`

## What shipped

A file opened from outside the tree now opens its folders, is highlighted
and scrolls into view. That covers a terminal ⌘-click, MCP `show_doc`,
Back/Forward, the place's remembered file, Docs/Plan "open", and a
re-open of the same file.

- `app/src/App.tsx`: `openDockFile` bumps `revealToken`, which is passed to
  `FilesPane` and then `FileTree`.
- `app/src/FilesPane.tsx`:
  - `FileTree` creates a `Reveal` request `{path, done}` for each
    (open path, token). It is kept in a hand-keyed ref, not `useMemo`,
    because the request is mutable.
  - The request is passed down every `TreeNode`. A directory on the path
    calls `setOpen(true)` from an effect that also runs on mount, so the
    lazy tree opens one level per listing.
  - The file's row, in a layout effect, marks the request done and calls
    `scrollRowIntoBox`. That function centres the row by moving
    `.dock-tree`'s `scrollTop` only.
  - Toggling a folder on the revealed path abandons the reveal. Toggling
    any other folder does not.
- `app/src/filekind.ts`:
  - `treePath(root, canon, path)` moves a path built from the registered
    place path onto the canonical root. Rows are canonical because
    `list_dir` canonicalises the directory.
  - `revealsThrough(dir, path)` is a prefix test that includes the
    separator.
- `app/scripts/treereveal-check.mjs`: covers the path rules, the wiring,
  source assertions sliced out of TreeNode/FileTree, and the real
  `scrollRowIntoBox` evaluated under a DOM stub.
- CHANGELOG: one item under `[Unreleased]` → `### Fixed`.

## Decisions

- **Always reveal, with no setting.** This is VS Code's default and David's
  stated lean. Revealing on a tree click costs nothing: the ancestors are
  already open, and the row is in view so nothing scrolls.
- **A file the tree can't reveal gets no hint.** That covers a file
  outside the place, one under a gitignored folder while ignored files are
  hidden, and one inside a followed symlinked folder (its rows carry the
  TARGET's canonical path). The viewer header already names the file, and a
  hint about an outcome nobody asked for is noise.
- **Scroll by hand, not with `scrollIntoView`.** `scrollIntoView` also
  scrolls every overflow ancestor (the dock, the space body), which is the
  same shape as the AGENTS.md autofocus/hidden-overflow note.
- **The trigger is a token, not just the path.** Opening the file that is
  already open changes no path, and that "where is this?" is exactly when
  it should scroll back.
- **A reveal is consumed once its row scrolls.** Otherwise collapsing a
  folder on the path, then re-expanding its parent, remounts the chain and
  springs it all open again.

## Dead ends / gotchas

- **Root cause.** The row already counted as selected
  (`openPath === entry.path`). But expansion is per-node `useState(false)`
  that only a click sets, so the row never mounted. Nothing scrolled the
  tree.
- **The first version's toggle rule was too broad.** Any folder toggle set
  `done`, so expanding `src/` while the cascade was still loading `crates/…`
  stranded the reveal halfway. Review found it. The bug is invisible with
  the mock's instant `list_dir`; wrapping `__TAURI_INTERNALS__.invoke` with
  a 600 ms delay reproduced it in both engines.
- **The first check missed 4 of 6 review mutants.** Its scrollIntoView
  regex only matched the `{block:…}` form, and it never looked at the
  ancestor `setOpen`, the scroll call, the recursive prop or the memo deps.
  The check now takes a FilesPane path, so a mutant copy can be checked.
  Ten mutants all go red.
- **The CHANGELOG entry rebased cleanly into `## [0.39.0]`.** That release
  was cut while the PR was open. The #309 trap fired exactly as AGENTS.md
  describes; the entry was moved back by hand.
- **The harness logs an xterm page error** (`_renderer.value.dimensions`)
  each time a terminal mounts in headless Chromium/WebKit. It is unrelated
  to this change.

## Verification

- Mock harness on :1437 (the served file was grepped for new code after each
  restart), headless Playwright, Chromium AND WebKit.
- Five steps, each asserting:
  - the row is present and has `.sel`;
  - its rect is inside `.dock-tree`;
  - every ancestor's `.tree-kids` is mounted;
  - outer scrollTop sum is 0.

  The steps: a terminal ⌘-click on `src/lib.rs` (tree padded open);
  `show_doc` 3 levels deep; a re-open after scrolling the row out of view;
  `show_doc` into another place; ⌘[ Back. All passed. Forward still names a
  single entry, so the reveal pushed no history.
- With the pre-fix sources served: `present: false`.
- Toggle race: red before the fix (`docs: false`), green after, in both
  engines.
- Gates on the rebased tree, after a fresh 0.39.0 release build:
  - `make test`: 1..476, 0 not ok
  - `make lint`: passes
  - `cargo test`: core 707, cli 70, `app --lib` 161
  - `tsc`, `cargo check -p app` and `make test-frontend`: all pass
  - CI: 9/9 on both pushes.
- `canonicalize` case: `/bin/realpath /USERS/…/WORKSPACE` → `/Users/…/workspace`
  (macOS `realpath(3)` returns the on-disk case).

## Follow-ups

These are in ROADMAP:
- the Plan tab reveals nothing while gitignored files are hidden;
- the mock's `list_dir` never canonicalises;
- horizontal overflow for deep rows;
- a rapid A-then-B open leaves A's partly opened chain open.
