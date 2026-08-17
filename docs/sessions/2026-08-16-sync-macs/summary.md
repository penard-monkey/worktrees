# Session: `worktrees sync` — courier sync between Macs (plan → six merged PRs)

- **Dates**: 2026-08-15 → 2026-08-16
- **Worktree**: `.worktrees/sync-macs` · branches `sync-macs`, `sync-macs-guards`,
  `sync-macs-app`, `sync-macs-stream`, `sync-macs-import`, `sync-macs-heal`
  (each deleted after its squash-merge; idle base `sync-macs-next`)
- **PRs**: #136 (CLI), #137 (guards), #138 (app surface), #139 (streaming +
  hover menu), #141 (import + add-project menu), #146 (heal + mirrored-state note)
- **Release**: phase-6 heal shipped in v0.16.0 (cut by a parallel session, #147)
- **Planning files**: `planning.tar.gz` beside this file (task_plan, findings,
  progress, plus the six per-phase opus briefs)
- **Workflow**: opus subagents implemented from written briefs and ran gates;
  fable reviewed every diff before each squash-merge and patched three defects
  the agents missed (see Gotchas).

## What shipped

Port of `~/bin/sync-macs` (bash courier sync via SSD hub) into the engine + app.

- `crates/worktrees-core/src/sync.rs` (~2 800 lines by the end): rsync
  shell-out engine — probe/plan/apply split, hub autodetect (denylist, single
  candidate), strict data-only hub manifest `.worktrees-sync.toml`
  (`deny_unknown_fields`; ADR 0001 extended to removable media — a manifest may
  never supply argv; its `hint` is printed, never executed), exclude set with
  per-project `extra_excludes`, backups under `.worktrees-sync/backups/<stamp>/`,
  additive Claude-sessions ferry, `--install` from user config only,
  programmatic API (`SyncRequest`/`SyncSource::{Root,HubName}` →
  `sync_preview`/`sync_apply`/`sync_status_data`) that the CLI delegates to
  byte-for-byte, streaming progress (`--info=progress2,name1` parsed off a pipe,
  split on `\r` AND `\n`), and the post-pull `post_pull` pass: heal
  excluded-tracked deletions + mirrored-state note.
- CLI: `worktrees sync push|pull|status` in the pre-guard block (adoption pull
  and hub status run outside any repo). `sync pull <name>` adopts a project
  onto a machine that has never had it.
- Guards on every surface: mutating ops refuse inside a hub copy (CLI dispatch
  choke point, `run_op` for the app, `readOnlyHint`-keyed refusal in
  `worktrees mcp`); `sync pull` onto live tmux sessions lists them and refuses
  bare `--yes` with `EXIT_NEEDS_CONFIRM`; `doctor` reports `hub-copy` (Error).
- App (`app/src-tauri/src/lib.rs`, `app/src/App.tsx`): `sync_status` /
  `sync_hub_list` / `sync_preview` / `sync_apply(Channel<SyncProgress>)`;
  project ctx-menu Sync group + hover ⇄ popover; preview→confirm modal with
  danger-styled deletions, live-session warning, determinate progress bar,
  sessions + rebuild checkboxes; Import-from-hub picker; the add-project menu
  (New project… / Add existing… / Import from hub…) and the `NewProjectDialog`
  (name + typed/pasted/browsed location, `create_project` backend re-validating
  everything). Mock harness arms + knobs (`?slowsync` `?nohub` `?hubcopy`
  `?synclive` `?openrsync` `?noinstall`, `__mock.syncFail/syncLive`).
- Tests: `test/sync.bats` (fake rsync ALWAYS, pinned via `WORKTREES_RSYNC` —
  see Gotchas), 314 bats total by the end; core unit suite grew 224 → 248,
  including a regression test built from a verbatim capture of real rsync 3.4.4
  piped output.

## Decisions (and why)

- **Registry-free model**: sync-macs's project registry died; the workspace IS
  the registry, sync unit = project root dir. Push/status are cwd-scoped; only
  pull takes a name (adoption) — you push from the machine that has it.
- **Fresh hub layout, no sync-macs compat** (David): one re-push per project;
  old SSD data left inert. Lucky overlap: old and new tree paths coincide, so
  the first push was incremental.
- **Hub-only v1, SSH = v2** (David). Sessions ferry opt-in (David).
  `~/bin/sync-macs` retired at phase-1 merge (David) — which forced phase 1 to
  be full CLI parity in one PR.
- **ADR 0001 extended to media**: rebuild commands never ride the SSD; argv on
  `--install` comes from the user tier (`[sync.projects.<name>] install`).
- **`--yes` never mirrors over live sessions**: blanket consent given by a
  script before it could know is not an answer; the GUI's modal confirm maps to
  `confirmed`, and apply re-reads the session list (a stale preview is a fresh
  question).
- **Heal, don't shrink the excludes**: the exclude list keeps 47G → 8.5G; the
  post-pull heal makes it safe generically (restore from the local `.git`,
  which always ferries complete) instead of per-pattern whack-a-mole. Heal only
  unstaged deletions matching the patterns; staged/non-matching deletions are
  mirrored intent. `name/` matches ancestor components only, so a tracked FILE
  named `build` cannot be resurrected.

## Dead ends / gotchas (highest value)

- **`/opt/homebrew/bin/rsync` beats a bats PATH shim** — discovery prefers v3
  outright, so the suite would have run real `rsync --delete` on the
  developer's machine. `WORKTREES_RSYNC` pins the shim; `common.bash` also
  unsets `WORKTREES_SYNC_HUB`/`CLAUDE_PROJECTS` (an exported one aims tests at
  the real SSD / real transcripts).
- **Two piped children read sequentially = deadlock** (fable review catch): the
  streaming apply read stdout to EOF, then stderr — an rsync stderr storm
  (permission-denied spam) fills the pipe and blocks the transfer. stderr now
  drains on its own thread.
- **A parser proven only on invented bytes isn't proven**: real rsync
  `--info=progress2` writes `\r` BEFORE each line and rewrites the final line
  three times back-to-back; a `\n`-only reader sees nothing until EOF. A
  verbatim capture is now a unit test, fed in 7-byte chunks.
- **An Edit wrote a raw NUL into App.tsx** (via a `[\s\x00-\x1f]` regex) —
  `grep`/`rg` then say "binary file matches" and print nothing, which reads
  exactly like the edit never landed. And the first NUL "gate" was itself
  broken: bash cannot pass NUL in argv, so `grep -c $'\x00'` counts every line.
  Real check: `tr -dc '\0' | wc -c`.
- **Live-machine mtime churn**: a 0/0 pull dry-run became a 37-file transfer
  seconds later — the app's 3s poll re-touches `.git/index` mtimes. Harmless
  (same content) but it is the concrete argument for the live-session guard.
- **The field bug that motivated phase 6**: exclude patterns match TRACKED
  files (committed `*.tar.gz` planning archives — this repo tracks 25). A
  fresh-machine import opened on a wall of `deleted:`. Diagnosis came from a
  screenshot, not the code: the transfer was correct, the working copies were
  never sent.
- **PR numbers are not merge order, live edition**: main gained #140 mid-stack
  (same file we were editing — rebase produced an untested combination,
  re-gated), then #145/#147 while closing out. Refetch before EVERY branch.
- **`git checkout -b` transiently fails under the app's 3s poll**
  ("remove the file manually" = index.lock) — retry, don't diagnose.
- **Relative Location in a GUI resolves against `/`** (launchd cwd) — refuse
  with the rule, don't surface the permission error (fable review catch).
- Session-old traps re-confirmed: Bash tool cwd sticks (`cd app` broke later
  `make test`); `git diff -- <paths>` returns empty under the RTK hook (use
  full `git diff`); planning files must record what HAPPENED, never expected
  state (task_plan was twice written with pre-completed phases + fabricated
  user answers before the rule stuck).

## Verification

- Real SSD round-trip (SanDisk, phase 1): autodetect → push (3 796 files,
  68 stale hub deletions → 289 files into backups) → pull dry-run 0/0 →
  delete-restore ferry proof byte-identical → hub cleaned.
- Real hardware guard checks (phase 2): `doctor`/`new`/`sync push` refused
  inside `/Volumes/SanDisk/worktrees/worktrees`; `pull --yes` at the native
  path refused exit 4 naming all 10 live sessions.
- David's real cross-Mac import (Studio → SSD → MacBook) — surfaced the
  phase-6 bug; his sandbox click-throughs drove the phase-5 UI reshape (add
  menu) and the new-project dialog.
- Every phase: red-first or mutation-proved tests (12-mutation table in phase
  6), full gate list re-run independently of the implementing agent,
  Playwright harness checks with `getComputedStyle` assertions, zero console
  errors.

## Follow-ups

→ moved to ROADMAP.md: SSH targets, workspace-level push-all, scheduled sync,
heal-matcher anchoring limitation, import-picker edge (existing local dir not
in workspace), `~/bin/sync-macs` deletion reminder.
