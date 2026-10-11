# Brief: REMOVE the automations feature (to revisit later)

David: "We need to remove the automations feature. It hasn't been very useful and it is confusing. Let's
remove it and then we'll revisit later."

Automations shipped in v0.29.0: #325 (core: model, runner, CLI, MCP — phase 1a), #326 (app: Automations
dock tab — list, run, report, proposals — phase 1b), design `docs/proposals/automations.md`. Remove the
whole user-facing feature cleanly; keep the design doc and git history as the record for "revisit later".

## Inventory first (grep, don't trust this list)
Known touch points: `crates/worktrees-core/src/automation.rs`, `runs.rs` (run ledger + proposals —
check whether ANYTHING outside automations uses it before deleting), references in `lib.rs`, `reach.rs`,
`proc.rs`, `profile.rs`, `store.rs`; CLI `main.rs` verbs; MCP `mcp.rs` tools (list/get/upsert/delete/
run automation, list_runs/get_run, apply_proposal — confirm each is automation-only); app
`AutomationsPane.tsx`, `automations.ts`, `App.tsx`, `App.css`, `icons.tsx` (icon only if unused
elsewhere), `navHistory.ts`, `settings.ts`, `usage.ts`, `lib.rs` Tauri commands, `mock/install.ts`;
`app/scripts/dockrail-check.mjs` (and any other *-check.mjs that mirrors automations); tests
`test/automations.bats`, parts of `test/mcp.bats`, `test/helpers/common.bash`;
`docs/ai-profiles-manual-checks.md`; README / guidance / skill text that mentions automations (incl. the
shipped worktrees skill and the MCP server instructions line).

## Rules for the removal
1. **Persisted state must degrade, not break.** A saved `dock_tab: "automations"` (global and per place in
   `place_panels`, nav history entries) must fall back to Files on load — test it. An old
   `.worktrees.automations.json` in a repo, and the per-machine run ledger on disk, are the USER's data:
   never delete them; just stop reading them. Say in the CHANGELOG where they live in case someone wants
   them. If a schedule/runner could still fire from a launchd/cron entry or similar, find and disable that
   path (and say what an upgrading user must do, if anything).
2. **MCP removal + long-lived servers** (AGENTS.md): clients cache tool lists and old `worktrees mcp`
   processes keep running the old image. Removing tools is fine; make sure nothing in guidance/skill text
   still tells agents to call them, and bump guidance VERSION if the shipped text changes.
3. `ls --json` / MCP `list_places` / `place_status` shapes: diff against `~/.local/bin/worktrees`
   (v0.40.1); any field removed is a contract change — list it in the PR.
4. Keep `docs/proposals/automations.md` but mark it at the top: "Status: built in v0.29.0, removed in
   <next version> (David, 2026-10-10: not useful enough, confusing); kept as the record for a later
   revisit." Update its `docs/proposals/index.md` row the same way. Add one ROADMAP item: "Automations —
   removed 2026-10-10; revisit later", linking the proposal and the removal PR, with the lessons you can
   see (why it confused — read the code/UI and say what you think, briefly).
5. CHANGELOG under `## [Unreleased]`: a `### Removed` section (count headers; there is an `### Added`
   from owned planning already).
6. Don't break unrelated things that happened to share code (e.g. a proposal/apply mechanism, run ledger
   reused by MCP messages, any `proc.rs` helper). If something is shared, keep it and say so.

## Gates
All of AGENTS.md's gates, `cargo build --release -p worktrees-cli` FIRST; `make test-frontend` from the
repo root (several *-check.mjs scripts mirror the dock rail — update them, don't delete a guard that still
guards something). Drive the mock harness once (off port 1420; restart vite with `--force`) to confirm the
right rail no longer has Automations and a persisted `dock_tab: "automations"` lands on Files.

## Rules
- Never drive David's running app; sandbox by pid only. Any tmux use: `tmux -L <unique>` on EVERY call.
- Never `git worktree add`. Scratch → `~/.cache/worktrees/worktrees/remove-automations/`.
- No session URLs / Claude-Session trailers; keep Co-Authored-By.
- Open one PR (split only if huge), message `(main)` with the number, gates, the inventory of what was
  removed vs kept, and any contract changes. Don't merge — a fable review runs first.
