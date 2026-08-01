# Session: right panel, icon set, owned dock shells, nav fixes → v0.5.0

- **Date**: 2026-07-29 → 2026-07-30
- **Worktree**: `.worktrees/ui-changes`
- **Branches**: `ui-next` (pushed as `ui-right-panel`), `nav-fixes`, `release-0.5.0`
- **PRs**: [#62](https://github.com/penard-monkey/worktrees/pull/62) right panel ·
  [#63](https://github.com/penard-monkey/worktrees/pull/63) nav fixes ·
  [#64](https://github.com/penard-monkey/worktrees/pull/64) release
- **Release**: `v0.5.0` (tag on `9272820`, all 10 assets published)
- **Planning files**: `planning.tar.gz` (task_plan / findings / progress)
- **Models**: Opus implemented #62's five slices; Fable reviewed the diff
  pre-merge (3 findings, all fixed in `423e231`) and did #63 + the release.

## What shipped

Driven by 3 annotated screenshots (2026-07-29) + 1 more (16.10.42) for the nav
bugs. One commit per slice inside #62; #63 as two fixes.

1. **Viewport-fit columns** — `app/src/settings.ts` (`fitLayout`, viewport-aware
   `clampNav`/`clampDock`), `app/src/App.tsx`, `app/src/App.css`. Flat caps
   (nav 460 / dock 680) replaced by ceilings derived from the live viewport with
   a 420px center floor; degrade order dock-shrink → dock-hide → nav; prefs stay
   intent and are never rewritten by fitting.
2. **Icon set** — new `app/src/icons.tsx`: Lucide geometry (MIT) at native
   24-box/stroke-2, which renders within 0.03px of the two hand-drawn 16-box
   originals at a 15px draw. All chrome glyphs swapped; row-level data markers
   (◆ ★ ⚑ ● ↑↓) deliberately stay Unicode.
3. **Right activity rail** — permanent 44px column owning the dock's
   Files/Terminal selection; active icon collapses the dock; topbar `▧` deleted.
4. **Owned dock shells** — `app/src-tauri/src/lib.rs`: `Shells` registry keyed
   (repo, slug, index); login-shell PTYs, 256KB replay ring, detach-not-kill on
   unmount, attach generations, liveness in `list_shell_sessions`; swept on app
   exit / place close / place rm; one-time `~term` sidecar cleanup for
   upgraders. Frontend: `useTerm` transport split in `app/src/TerminalPane.tsx`
   (tmux hero vs owned shell), dead-tab + Restart UI in `TerminalTabs`.
5. **Branch combobox** — `Project::branch_names()`
   (`crates/worktrees-core/src/project.rs`), `list_branches` command,
   `BranchSwitcher` module-scope component. Combobox, NOT `<select>` — the DWIM
   create path survives as an explicit `create <name> off <base>` row.
6. **Nav divergence vs base** (#63) — `Project::base_ref()`; ↑↓ now measured
   against `origin/<default_base>` (else local base), resolved once per
   snapshot. `test/json.bats` contract updated + a scenario test (238 bats).
7. **Drag-sort fix** (#63) — `"dragDropEnabled": false` in
   `app/src-tauri/tauri.conf.json`.

## Decisions (why)

- **D1 right rail always visible**: a toggle that vanishes exactly when you'd
  reach for it is worse than 44px of chrome; also deletes one of the two
  confusable box glyphs (▤ vs ▧) by removing the second one's reason to exist.
- **D2 Lucide at native 24-box**: hand-rescaling ~20 paths to the 16-box house
  style is a transcription-error generator for a 0.03px stroke difference.
- **D4/D5 fit as pure function of (settings, viewport, selection)**: prefs are
  intent; fitting must never write them back, or a narrow window would
  permanently eat the user's dock width.
- **D6 owned PTYs for dock shells only**: kills the C-b steal, copy-mode
  scrollback, co-client resize clamp, and the tmux hard-dependency — priced at
  "shells die with the app". **D7**: the hero session stays tmux; Claude lives
  there and must survive quit / stay `tmux attach`-able.
- **↑↓ vs base, not @{u}**: the arrows answer "how far from main" — @{u} showed
  ↑494 on a branch that had *just* synced with main (merged commits unpushed to
  its own remote), and showed nothing on upstream-less branches.

## Dead ends / gotchas (read these first next time)

- **Tauri `dragDropEnabled` defaults true and eats HTML5 DnD in WKWebView.**
  Feature worked in the browser harness (no Tauri layer), dead in the app.
  Anything drag-based must be tested in a real build.
- **Keyed detach is not id'd detach.** Porting tmux-attach (fresh id per
  attach) to a registry keyed by place+index silently changed detach semantics:
  under StrictMode, unmount №1's detach could clear mount №2's live sink.
  Fixed with attach generations. Sibling bug: check-unlock-spawn-insert let two
  concurrent opens both spawn (leaked orphan shell) — hold the lock across the
  whole open.
- **`shell:exit` is transient**; dock closed = no listener = dead shell
  resurrects as live on reopen. Liveness must ride the restore
  (`list_shell_sessions` + `try_wait`), not the event.
- **`window.innerWidth` counts the scrollbar gutter** (1284 vs 1269
  clientWidth) — silently ate the center-pane reserve in the harness.
- **First topbar overlap "fix" relocated the overlap**: `.status-cluster`
  escaped `.identity` onto `.controls`. Flex shrink needs the full
  min-width:0 + overflow chain at every level; verified by measuring
  `getBoundingClientRect`, not by looking.
- **A pseudo-element 1px outside its rail's content box** (`right:-6px`) put a
  real horizontal scrollbar on the whole window. App shells should be
  `overflow:hidden` at the document level regardless.
- **`gh pr merge --delete-branch` in a worktree** fails its local-checkout step
  ("main is already checked out") — merge succeeds; delete the remote branch
  explicitly.
- Old remote `ui-next` was stale/divergent (its commits squash-merged as #56) —
  pushed to fresh `ui-right-panel` instead of force-pushing over it.

## Verification

- Playwright against `pnpm dev:mock` (port 5199): column sums exact at
  1920/1600/1284/900; overlap checks via getBoundingClientRect; dock
  collapse/reopen keeps 2 shell tabs; exit-while-closed → reopen shows dead tab
  → Restart revives (needed new `__mock.exitShell`); combobox click / ↓+Enter /
  Esc+Enter-raw all switch.
- Gates per PR: 237→238 bats, 137 core tests, tsc, `cargo check -p app`,
  `make lint`. CI ×9 green on #63/#64.
- **NOT verified** (mock has no PTY / no Tauri layer): real drag-sort, real
  shell replay + detach under `tauri dev` / the 0.5.0 build. Flagged to the
  user for a manual pass.

## Follow-ups

- Manual smoke of drag-sort + dock-shell replay/restart in the real 0.5.0 build.
- Review nits deliberately left: `--nav-w`/`--dock-w` CSS vars still written but
  unconsumed (grid is inline px); `place_session_cwd` doc comment still
  mentions sidecar names; combobox Enter with a stale highlight falls back to
  raw text.
- PR #60 (`fix/remove-place-delbranch`, everything-settings worktree) still open
  — different stream, untouched here.
