---
title: "Session — the place header names its project"
---

# Session — project name in the place header

- **Date:** 2026-10-03
- **Worktree:** `.worktrees/topbar-project` (kept; not removed).
- **Branches:** `topbar-project`, the feature; `topbar-project-closeout`, this archive.
- **PRs:** [#430](https://github.com/penard-monkey/worktrees/pull/430),
  squash-merged by the orchestrator as `f852af4`.
- **Release:** none yet; the entries sit under `[Unreleased]`.
- **Planning files:** none. The brief was `.planning/brief.md`.

## What shipped

- `app/src/App.tsx`: a `.topbar-proj` crumb at the start of `.identity`
  (`<name> /`, full root path in the `title`), before the place name.
- `app/src/App.css`: `.topbar-proj`, `.topbar-proj-name`, `.topbar-proj-sep`.
- `app/scripts/identity-keys-check.mjs`: pure source check that `TitleEditor`
  and `HeaderBranch` in the identity row never share a React `key`.
- `CHANGELOG.md`: one `### Added` (the crumb) and one `### Fixed` (the stale
  rename box).

## Decisions

- **Name = `basename(sel.repo)`.** The nav's project header renders
  `basename(pv.root)`, not `ProjectView.name`, so the crumb copies that; the CSS
  comment says both must change together if the nav ever shows the registry name.
- **`(main)` shows the crumb unless its name equals the project's.** Main's name
  is normally `(main)`, so the dedupe rarely fires; `project / ◆ (main)` is not
  redundant.
- **Display only.** No click, so no nav-history question.
- **Words in `--txt-dim`/`--txt-mute`, no hue.** The name is below 4.5:1 at 11px
  in tokyo-day (3.57), nord (3.80) and catppuccin-latte (4.37), the app's own
  dim-text tokens; consistent with the rest of the app rather than fixed here.

## Dead ends / gotchas

- **`flex: 0 8 auto` did not make the crumb shrink first.** Flex-shrink is
  weighted by basis × factor, so the slug shrank at the same time (crumb began
  ellipsising at 1070px, slug at 1065px). `0 999 auto` makes the crumb absorb the
  whole deficit. Found in review by measuring a width sweep, not by reading.
- **`min-width: 0` on the name left a bare `/`** for a ~10px band. `2ch` makes the
  container clip the separator first.
- **A harness "pinned" run that is not pinned.** The mock's stored settings
  without `settings_rev: 3` are migrated back to unpinned, so my first
  pinned/mirrored measurements were all the unpinned layout (identical numbers in
  every mode was the tell). Seed `{settings_rev: 3, nav_pinned: true, places_side}`.
- **A pre-existing bug the crumb review surfaced:** `TitleEditor` and
  `HeaderBranch` shared the key `sel.repo|sel.slug`, so after Enter/Esc on a
  rename a stale `.title-input` stayed in the header. Reproduced on the old key
  (1 left after Enter, 2 after Esc, 3 duplicate-key warnings); gone with a
  distinct key.
- **Vite for the harness must be a real background task.** `setsid` is absent on
  macOS and a `( … &)` in a tool call dies with the call; `exec vite` under
  `run_in_background` held. Source edits need a restart (HMR is dead in
  `.worktrees/`).
- **A conflicted PR gets no CI.** #430 went `DIRTY` after another entry landed in
  `CHANGELOG.md`; no run started until a rebase. Resolving kept both items under
  one `### Added`.

## Verification

- Playwright Chromium and WebKit against the mock harness, 1280/900px, unpinned/
  pinned/mirrored: order (`.navhist` < name < `.slug`), `elementFromPoint` on the
  header controls, no overflow past them, contrast in all six themes (opaque
  `.topbar` bg).
- After review: width sweep 1500→700px in both engines (crumb ellipsises at
  1325px; slug at 1280 Chromium / 1185 WebKit; no bare `/`), rename Enter/Esc
  leaves 0 `.title-input`.
- Gates: bats 456 / 0 not ok, lint, test-frontend, cargo test core 667 / cli 70 /
  app 151, tsc, `cargo check -p app`; CI green on the tip `b9c69ee`.
- Not done: the real app (the mock is Chrome/WebKit, not the shipped WKWebView
  build) — nobody has looked at the crumb there.

## Follow-ups

- At 900px pinned the collapsed crumb still costs one 12px flex gap before the
  place name (ROADMAP).
- Contrast of the crumb in the three light/nord themes, if the dim-text tokens are
  ever raised.
