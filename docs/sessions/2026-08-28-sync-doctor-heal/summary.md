# 2026-08-28 — sync damage: field heal, doctor detection, healing on every edge

- **Dates**: 2026-08-27 → 2026-08-28 (close-out 2026-09-01)
- **Worktree**: `.worktrees/sync-macs` (same tree as the sync-macs feature
  session; this is that stream's aftermath)
- **Branch**: `sync-macs-doctor-heal`
- **PR**: [#167](https://github.com/penard-monkey/worktrees/pull/167), squash-merged as
  `08b9120`; shipped in v0.18.0
- **Planning**: `planning.tar.gz` beside this file holds `doctor_heal_brief.md`
  (the opus implementation brief — this stream ran brief-first, no
  task_plan/findings/progress of its own). The feature session's planning set,
  including all five phase briefs, lives in
  [2026-08-16-sync-macs](../2026-08-16-sync-macs/summary.html)'s tarball —
  the copy archived there (from the MBP) was *newer* than this tree's local
  copies, which were dropped at this close-out as stale duplicates.

## What shipped

Field incident first: 8 of the 10 linked worktrees on the origin Mac were
carrying walls of unstaged `deleted: docs/sessions/*/planning.tar.gz` — the
#146 damage class (excluded-but-tracked files absent from the working tree),
172 files total, dating from before the heal existed. Nothing ever said so,
because the repair only ran on the next successful **pull** of that project.
The damaged trees were healed by hand (the same batched `git checkout --` the
heal performs), then the gap was closed for good:

- **`doctor` gains `sync-skipped-files` (Warn)** — one finding per project:
  count, sample paths, remedy — judged by the project's own exclude set.
  `crates/worktrees-core/src/diag.rs` (`Code::SyncSkippedFiles`),
  `crates/worktrees-core/src/sync.rs` (`skipped_files_finding`,
  `project_extra_excludes`, and the `skipped_deletions`/`rebase` refactor that
  puts ONE scan behind both the finding and the heal),
  `crates/worktrees-core/src/ops.rs` (wired beside `hub_copy_finding`).
- **The heal runs on every sync edge** — before a push transfers, and after a
  pull whose transfer *failed*; the transfer's error stays the outcome, and
  `--dry-run`/declined-confirm still write nothing. `heal_pass` extracted from
  `post_pull`; both `sync_one` (CLI) and `sync_apply` (app) call it, so the
  surfaces cannot drift.
- **`.worktrees-sync/` stops polluting `git status`** — added to
  `exclude_app_state`'s `.git/info/exclude` entries
  (`crates/worktrees-core/src/store.rs`) and to this repo's own `.gitignore`.
  The live instance on the origin Mac (store predates the entry) was closed by
  a hand-append; other machines' existing stores wait on the ROADMAP sweep.
- Tests: 5 new bats (318 total, prior suite untouched-green), 3 new + 1
  extended core units — every one red-first or mutation-proved.

## Decisions

- **Sync repairs, doctor detects.** Doctor stays read-only (the relink/doctor
  split); no `doctor --fix`, no new subcommand. The finding's remedy is "run
  any sync of this project", which the every-edge heal makes true.
- **One scan behind both surfaces.** The finding does not reimplement "which
  deletions are ours" — `skipped_deletions` feeds both it and the heal, so
  the report and the repair cannot disagree the day one of them is edited.
- **Warn, not Error.** The blobs are safe in `.git`; the tree functions;
  doctor still exits 0 on Warn alone (§7 semantics).
- **Suppressed beside the hub-copy finding** (review catch, see below).
- **One finding per project, not per file** — the field shape is dozens of
  paths across many worktrees; a report that long is one nobody reads.

## Dead ends / gotchas

- **The finding fired inside hub copies** — the one real bug in the agent's
  otherwise-clean implementation, caught in review. A hub copy is missing
  every excluded-tracked file *by construction* (the push skipped them), so
  absence there is the pushed state, not damage; the remedy the finding names
  is exactly what the hub-copy guard refuses; and the copy's ferried
  `.worktrees/*/.git` files point at the origin machine's repo, so the
  per-worktree `git status` probes would read a stranger. Fixed with a
  red-first bats extension: `skipped` is `None` whenever `hub_copy` fired.
- **Damage archaeology: current pulls cannot cause this.** The apply uses
  `--delete` but never `--delete-excluded`, and rsync protects excluded files
  that already exist on the receiving side — so the worktree walls had to
  predate the heal (the first cross-Mac import era, exactly #146's field
  case), sitting invisible in parked trees for ~10 days. That "invisible
  between syncs" property is the entire reason the doctor finding now exists.
- **The 25 tarball deletions in THIS tree looked like intentional local
  edits** and sat in `git status` across several sessions. The tell: origin
  still had every file and no commit ever deleted them. Check upstream before
  honouring a deletion you don't remember making.
- **`worktrees.app/Contents/MacOS/app --help` launched a second app
  instance** — the CLAUDE.md "never probe the bundle binary" trap, hit again
  despite being documented, this time via `--help` instead of `--version`.
  The probe hung a 120s tool call and left a GUI instance to kill. Read the
  Info.plist instead.
- **`gh pr merge --delete-branch` fails in a worktree layout**: after a
  successful merge it tries to check out `main` locally, which the root
  worktree owns (`fatal: 'main' is already used by worktree at …`), and the
  failure aborts whatever shell chain it sits in. The merge itself succeeded;
  delete branches manually.
- **`make lint` cannot run here — shellcheck is not installed on this Mac.**
  None of its lint targets were touched and CI runs the real thing; the
  bash-3.2 gate stages were run by hand. Also `app/node_modules` does not
  exist in this worktree — `tsc` ran via a temporary symlink to the sibling
  `macbook-pro` tree's copy, removed afterwards.
- **zsh eats `===` and `==` as glob patterns** (`(eval):1: == not found`) —
  two separate tool calls died on decorative separators. Use `---`.

## Verification

- Agent's gates + 8 red-first proofs (each mutation and its exact failure
  line recorded in PR #167's description trail), then re-run independently
  after review + rebase: bats 318/0, core 254, cli 7, app --lib 42,
  `tsc --noEmit`, `cargo check -p app` — all green.
- Hub-copy suppression proven red (finding fired in a constructed hub copy)
  → fix → green.
- Real-binary check: doctor on this machine named a hand-deleted tarball with
  the correct worktree-scoped path, exit 0, tree left unrepaired.
- Post-merge CI on `main`: full suite green (run 33124088984), including the
  shellcheck stage local gates couldn't run.
- After the hand-heal + `info/exclude` append: all 10 worktrees and the root
  repo report a completely clean `git status`.

## Follow-ups

- Doctor's new scan costs one `git status --porcelain -z` per repo (root +
  each linked worktree) per whole-project run, including the app's badge
  path → ROADMAP.
- False-positive shape: a deliberate unstaged deletion of a tracked
  exclude-matching file reads as sync damage → ROADMAP.
- The existing-stores `info/exclude` sweep now covers one more entry
  (`/.worktrees-sync/`) → existing ROADMAP bullet amended.
