---
title: "2026-10-10 — remove automations"
---

# Remove automations — close-out

- **Date:** 2026-10-10
- **Worktree:** `remove-automations`
- **Branches:** `remove-automations` (the removal), `remove-automations-close-out` (archive)
- **PR:** [#477](https://github.com/penard-monkey/worktrees/pull/477), squash-merged
  as `f1e291b` after a fable review (MERGE); CI 9/9.
- **Release:** none yet (under `## [Unreleased]` → `### Removed`).
- **Brief:** [`brief.md`](brief.md) beside this file (from `.planning/brief.md`).
- **Planning files:** none.

## What shipped

Automations (v0.29.0, #325 core + #326 app) removed end to end. David's reason was
that they were "not very useful and confusing"; the plan is to revisit later.

- **Core:** deleted `crates/worktrees-core/src/automation.rs` and `runs.rs`.
  `runs.rs` had no user outside automations, the MCP server and the app's
  automation commands.
- **CLI:** the `automations`/`auto` verb and its help lines are gone. The verb is
  now an unknown command and exits 1.
- **MCP** (`crates/worktrees-cli/src/mcp.rs`):
  - the eight tools are gone: `list/get/upsert/delete/run_automation`,
    `list_runs`, `get_run`, `apply_proposal`;
  - the whole in-run apparatus is gone: `WORKTREES_RUN_ID` → `in_run`,
    `HIDDEN_IN_RUN`, `Role::Automation`, and `reach::Inputs::in_run`.
- **Store:** `exclude_app_state` no longer adds
  `/.worktrees.automations.json`. Existing exclude lines stay, because the
  function only appends.
- **App:**
  - `AutomationsPane.tsx`, `automations.ts`, `Icons.Zap`, the `.autopane`/`.auto-*` CSS
    and the rail entry are gone;
  - so are the 7 Tauri commands and the mock handlers;
  - `settings.ts` gains `DOCK_TABS`, and `loadSettings` sanitises `dock_tab`
    globally and per `place_panels` entry, so a removed tab loads as Files
    (pinned by `dockrail-check.mjs`).
- **Tests:** deleted `test/automations.bats` and `install_fake_claude`, and
  replaced the mcp.bats automation tests with a "tools are not advertised" guard.
- **Docs:**
  - the proposal is marked removed (also the index row);
  - the CHANGELOG `### Removed` entry says where the user's data still lives;
  - a ROADMAP "revisit later" item records the lessons;
  - manual-checks §12 is reduced to a pointer.

## Decisions

- **Shared helpers were kept and their comments corrected:**
  `proc::run_deadline`, `profile::claude_launch_check`, the app's
  `project_at` / `EMPTY_PLACES`, and `store::LIFECYCLE_LABELS`.
- **User data is left on disk, unread.** That covers
  `<repo>/.worktrees.automations.json` and `~/.local/state/worktrees/runs/`.
  It is the user's data, and the CHANGELOG says where it is.
- **There is nothing to unschedule.** `When` was stored and validated but never
  evaluated, so no launchd or cron path ever existed.
- **The in-run guard was removed rather than kept.** Its only source was the
  automation runner. Keeping it would have been dead code that
  reads as a live security boundary.
- **The `dock_tab` sanitiser only fixes a value that is present.** An absent
  per-place value stays absent, so the global seed is not frozen into a place
  (the `place_panels` seed trap in AGENTS.md).
- **No guidance VERSION bump.** No shipped guidance or skill text ever named
  the tools.
- **"Removed in v0.41.0" in the proposal assumes the next release is a minor
  bump.** Correct it at release time if it is not.

## Dead ends / gotchas

- **The headline promise was false for eleven releases.** The automations editor
  offered "Every morning" / "Weekly" over a hint that "scheduled runs arrive in
  the next version". The clock (phase 2) was never built. This is the main
  lesson in the ROADMAP item.
- **A test fixture never matched.** `store.rs`'s exclude-header fixture had a
  run of spaces inside its string literal, so its `places.json` line could not
  match. The test passed only because automations was the entry it was
  "missing". It was rewritten to drop `/.worktrees-sync/` instead.
- **A comment was orphaned.** The app subagent first inserted `DOCK_TABS`
  between `placeKey`'s doc comment and `placeKey`. Fixed before commit.

## Verification

- **Rust:**
  - the release build ran first, and the binary was confirmed newer than the sources;
  - core 738 passed;
  - CLI 66+1 passed;
  - app `--lib` 162 passed.
- **Shell and frontend:**
  - bats 481/481;
  - lint clean;
  - tsc clean;
  - `test-frontend` 38/38;
  - `cargo check -p app`: one warning (`viewer.rs` `claim`), already on main.
- **Contracts, against v0.40.1:**
  - `ls --json` and MCP `list_places` have identical key sets to `~/.local/bin/worktrees`;
  - `place_status` loses nothing and only gains `plan.*` from owned planning.
- **Red-first:** the dock_tab sanitiser was removed and `dockrail-check.mjs`
  went red (3 failures). It was restored, touched, and the check went green.
- **Mock harness:** run on port 1477 with `--force`, served content checked.
  There is no Automations tab, and a persisted global plus per-place
  `"automations"` opens on Files.

## Follow-ups

- AGENTS.md: the note that `settings.ts` cannot gain a relative import now
  also applies to `dockrail-check.mjs`, not only `zoom-check.mjs`. The fable
  review raised it. It was not added in this archive PR because it touches the
  agent instructions, and that is left for David to do.
