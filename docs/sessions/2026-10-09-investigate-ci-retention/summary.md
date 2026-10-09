# investigate-ci-retention — session summary

- **Date:** 2026-10-08 → 2026-10-09
- **Worktree:** `investigate-ci-retention`
- **Branches:** `investigate-ci-retention` (no commits), `investigate-ci-retention-close-out`
- **PRs:** this close-out only
- **Release:** none
- **Planning files:** none

A scoping question from the `general-stuff-and-chats` orchestrator, after
~130G was reclaimed on the host: does worktrees create CI or per-branch
artifacts that pile up with no cleanup? Answer: **no** — ruled out. No code
changed.

## What shipped

Nothing in the repo. On the host, by David's instruction: every
`~/.cache/worktrees/<project>/<tree>/` folder with nothing modified in 14 days
was deleted — 50 folders, ~0.75 GB (15 sandbox, 20 saywhat, 15 worktrees).
The cache went from 12G to 11G; `mac-utilities/` (9.5G) is all from Oct 3–8,
so it stays.

## Findings

1. **`~/.cache/worktrees` is agent scratch, not tool output.** The tool writes
   only `codex-usage/`, `inbox/` (both empty) and `sandbox/` (small). The rest is
   the `<project>/<tree>/` scratch convention from each repo's AGENTS.md. Nothing
   reads it back and nothing deletes it, including `remove_worktree`.
2. **Compose volumes belong to places and go on removal.** cdv declares
   `[compose] project = "{prefix}-wt-{slug}"`, so `cdv-wt-*` volumes are
   worktrees-named. `remove_worktree` runs `docker compose -p <project> down -v
   --remove-orphans` (`ops.rs` `compose_down`, best-effort). `close_session`
   and `set_lifecycle closed` tear nothing down — a "closed" place keeps its
   stack by design.
3. **CI leaves nothing on the Mac.** It runs on GitHub runners. The only
   container is the `docker run --rm bash:3.2` parse gate, and release
   artifacts live on GitHub.

## Decisions

- **No age-based reaper in the tool.** David ran a one-off 14-day sweep by
  hand. A permanent one (or deleting a place's scratch on removal) changes what
  the tool deletes, so it is parked in ROADMAP.

## Dead ends / gotchas

- **The host's new podman GC deleted live place volumes — and a valuable one.**
  `podman-gc` (launchd, installed that day) pruned stopped containers older than
  48h, then pruned volumes that were now unreferenced, in the SAME run. That took
  the `cdv-wt-*` volumes of places that still exist, and `valleos_postgres_data`
  — unrecoverable (no dump, no APFS snapshot, no Time Machine, and `fstrim` ran
  11s later). This session flagged the container→volume sequence. The peer then
  made volume pruning opt-in (`PODMAN_GC_PRUNE_VOLUMES=1`, off). Rule: volume
  lifecycle belongs to `remove_worktree`, never to an age-based sweep.
- **BSD `find -newermt '-14 days'` matches nothing.** macOS find does not parse
  the relative date, so every folder read as stale — including ones written the
  day before. The first candidate list (141 folders, 11.9G) was wrong and caught
  only because it included folders known to be recent. Use `-mtime -14`.

## Verification

- Read `ops.rs::compose_down`, `provision::compose_down`/`compose_project_for`,
  and cdv's `.worktrees.toml` `[compose]` block.
- `docker volume ls` (podman-backed): no `cdv-wt-*` volumes remained.
- Sweep: listed candidates with `find -mtime -14`, then spot-checked that
  `mac-utilities/release-path` (Oct 3) was kept before deleting.

## Follow-ups

- ROADMAP: scratch-dir retention (an age sweep, or deleting a place's scratch on
  removal) — David's call.
