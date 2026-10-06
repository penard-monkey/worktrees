---
title: "2026-10-06 — always-visible + and ⌘N"
---

# Always-visible + on project rows, and ⌘N for a new worktree

- **Date:** 2026-10-06
- **Worktree / branch:** `nav-plus-cmdn`
- **PR:** #446, squash-merged as `34dc5af`
- **Planning files:** none (the brief was `.planning/brief.md`)

## What shipped

- `app/src/App.css`: `.mini.pnew { opacity: 1 }` for the project header's `+`;
  `.project-h:focus-within .mini` reveals the other actions for keyboard users.
  They keep their slots at opacity 0, so nothing shifts when they appear.
- `app/src/App.tsx`: the `+` is `mini pnew`, title "new worktree (⌘N)". ⌘N is
  bound in the one global chord handler, after the modal guard, through
  `newWorktreeRef`. `NewPlaceDialog` takes optional `choices`/`onPickProject`
  and renders a project `<select>` in its header; picking re-keys the dialog.
- `app/src/SettingsSheet.tsx`: ⌘N in the shortcuts list.
- `app/scripts/newworktree-check.mjs`: pins the `.pnew` rule, the focus-within
  reveal, the ⌘N-after-guard order and the full modifier condition.
- CHANGELOG entry under Added.

## Decisions

- **Current project** is `sel.repo` when that project is still in the
  workspace (`sel`, not `selected`, which is null before the first list). Home
  has none. The project sheet paints a scrim, so the modal guard already
  blocks ⌘N there.
- No current project: several projects → picker in the dialog (one modal, not
  two); one → open directly; none → the Add project folder picker.
- ⌘N fires from text fields, as in most Mac apps; the modal guard keeps it from
  stacking on another dialog.
- `lib.rs` builds no native menu, so nothing swallows ⌘N.

## Dead ends / gotchas

- **The first report said Shift and Alt were excluded; the code did not.** ⌘N
  sat above the meta-only gate that drops shifted chords, so ⌘⇧N fired it, and
  ⌥⌘N did through the `KeyN` fallback (`e.key` is "˜" there). Found in review;
  the fix and a guard (shown red against the old file) landed in the same PR.
  A summary of what a condition does must be read off the condition.
- The harness's Esc closes the revealed sidebar, so a script must ⌘2 before it
  looks for rows. Removing "the first project" in the mock removed the two good
  ones and left the broken one, which is the zero-OK-projects path rather than
  the one-project path; target the broken one explicitly.

## Verification

Mock harness, Playwright Chromium and WebKit, identical: `+` opacity 1 and
hit-testable at rest; other minis 0 at rest, 1 on hover and on focus-within;
⌘N with a place selected, on Home with 3 projects (picker), on Home with 1
(direct), with a modal open (nothing), at the xterm helper textarea (opens);
the picker switch resets the dialog. Gates: bats 1..476 with 0 `not ok`, lint,
cargo core 707 / cli 70 / app 161, tsc, `cargo check -p app`, `make
test-frontend`; CI green on both pushes.

## Follow-ups

In ROADMAP: the resting `+` contrast, the zero-projects ⌘N path in a browser,
and the `dead_code` warning (`viewer.rs` `claim`, from #311, not this PR).
