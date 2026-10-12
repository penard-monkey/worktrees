# Planning section in Project settings — 2026-10-12

- **Date:** 2026-10-12
- **Worktree:** `.worktrees/project-sheet-planning`
- **Branch:** `project-sheet-planning` (merged), close-out on `project-sheet-planning-close-out`
- **PR:** [#479](https://github.com/penard-monkey/worktrees/pull/479), squash-merged as `3b39f4b` after a fable review (MERGE, CI 9/9)
- **Release:** ships in v0.41.0
- **Planning files:** none. The work followed `.planning/brief.md`, with no `task_plan.md`.

## What shipped

Right-click a project → **Project settings…** now has a **Planning** section. This is owned planning's
phase-2 "ProjectSheet section", pulled forward because David asked for it before the release.

- `app/src/PlanningPanel.tsx`:
  - `ProjectRow` is exported with an optional `label` prop. It defaults to the project's name, so
    Settings → Planning renders exactly as before.
  - `LEVEL_WORDS` is exported.
  - The body of `setProject` became `writeProjectPlanning(root, level, path, scope, onStatus)`, the one
    `set_project_planning` write that both surfaces use. Busy state stays local to each caller.
- `app/src/ProjectSheet.tsx`: `PlanningSheetSection` is at module scope. It contains:
  - the same row, labelled "This project";
  - "In effect: X — set for this project / inherits your default: off|full|not chosen yet";
  - a **Default & how it works…** button.
- `app/src/App.tsx`: passes App's single `planning` state and `setPlanning` to the sheet. The button
  closes the sheet and opens Settings → Planning.
- Docs: proposal §1 table, §2.3, §2.5 and §9, the ROADMAP owned-planning item, and one CHANGELOG bullet
  under the existing `### Added`.

## Decisions

- **Reuse the row instead of building a sheet-specific control.** The brief required behaviour identical
  to Settings, including the refusal text and the show-only path validation. Sharing `ProjectRow` means
  the two surfaces cannot drift apart. The only fork is the heading, because the sheet's header already
  names the project.
- **One state, no new poll.** Writes land in App's `planning`, which Settings → Planning reads too. The
  sheet re-reads `planning_status` once on open, as Settings does on open, because
  `worktrees plan level` may have changed it from the CLI.
- **The button closes the sheet before opening Settings.** Two stacked scrims would be ambiguous for Esc
  and for the eye.
- **The button is a `ctrl sm` in `.ver-actions`, not an inline link.** That is the sheet's existing idiom.
  The app has no shared inline-link button class, so this avoided adding one.

## Dead ends / gotchas

- **The chrome-devtools MCP browser was held by another session** ("browser is already running for …
  chrome-profile"). Driving the mock with headless Playwright from a scratch dir under
  `~/.cache/worktrees/…/pw` was quicker than contending for it.
- **On the Home screen at 1400×900 the nav's `.project-h` is attached but not visible.** A Playwright
  right-click waits for it forever. `dispatchEvent("contextmenu")` on the header opens the real menu.
- **`getByText("Apply")` inside the row is ambiguous**, because the show-only hint mentions "apply" too.
  Use `getByRole("button", { name: "Apply" })`.

## Verification

- All gates ran: release build first, then `make test` (481 ok / 0 not ok), `make lint`,
  `make test-frontend`, `cargo test` for core, cli and app `--lib`, `tsc --noEmit` and
  `cargo check -p app`.
- Mock harness on :1488, restarted with `--force` after each edit, with a content check on the served
  file:
  - every level is correct under `?planning=unset|full|off`;
  - sheet → Settings: Full shows in both;
  - Settings → sheet: show only `docs/goals.md` shows in both;
  - with `?plantracked=1`, the refusal shows and the select falls back to the stored value.
- Screenshots are in `~/.cache/worktrees/worktrees/project-sheet-planning/` (not committed).

## Follow-ups

The two nits from the review are in ROADMAP:

- a generation guard on the on-open `planning_status` read, in both the sheet and `PlanningPanel`;
- `PlanningSheetSection`'s dead `open` prop.
