---
title: "Session: close-out ritual + session archive system"
---

# Session: close-out ritual + session archive system

- **Date:** 2026-07-27
- **Worktree:** ui-changes
- **Branches:** chore/close-out-ritual → PR [#35](https://github.com/penard-monkey/worktrees/pull/35) (`6aa6cd6`); chore/scratch-dir-cache → PR [#36](https://github.com/penard-monkey/worktrees/pull/36) (`6062fd6`)
- **Release:** none (process/docs only)
- **Planning files:** none

## What shipped

The end-of-stream ritual this very summary follows:

- `.claude/skills/close-out/SKILL.md` — the `/close-out` project skill:
  preconditions sweep, scratch → cache, committed session summary, planning
  tarball, straggler sweep → roadmap, one squash PR, fresh branch.
- `docs/sessions/` — committed archive, one dir per stream
  (`<date>-<slug>/summary.md` + optional `planning.tar.gz`). First entry:
  the 2026-07-26 themes/v0.2.4 session.
- `ROADMAP.md` — parking lot groomed at close-out; seeded with app
  signing/notarization + stale-worktree cleanup.
- Scratch convention: `~/.cache/worktrees/<project>/<worktree>/` for
  screenshots/harness output; repo-root `theme-*.png` untracked and moved
  there; `/theme-*.png` added to gitignore safety net.
- CLAUDE.md: Scratch files + Close-out ritual sections; planning-docs
  section now points at the in-repo tarball archive.

## Decisions

- **Ritual as a committed project skill**, not prose in CLAUDE.md — future
  threads run `/close-out` instead of remembering steps; CLAUDE.md keeps a
  three-line pointer.
- **Scratch in `~/.cache/worktrees/…`, not `/tmp`** — /tmp is wiped on
  reboot and pruned after ~3 days on macOS, which would orphan screenshots
  referenced by archived summaries. XDG cache persists, is never
  auto-purged, and is the same path on Linux. (First shipped as /tmp in
  #35, corrected in #36 same day.)
- **Summaries must carry dead ends, not just outcomes** — root causes of
  failures are the highest-value content for future sessions.
- Screenshots stay uncommitted scratch; copying 1-2 pivotal ones into the
  session dir is explicitly allowed when a summary references them.

## Dead ends / gotchas

- `/nav-*.png` was already gitignored, yet `theme-*.png` got committed to
  main in the themes session — explicit `git add theme-*.png` bypasses
  nothing (they simply weren't ignored). The safety-net globs only help if
  new artifact prefixes are added to .gitignore as they appear.
- zsh eats bare `===` in compound commands (glob) — quote separators in
  multi-part shell one-liners.

## Verification

Ritual exercised end-to-end twice while building it: themes session
archived (#35), scratch-dir correction (#36), both squash-merged with CI
green (#36 docs-only, no checks triggered). This summary is the third run.

## Follow-ups

- Caveman statusline edit to `~/.claude/settings.json` still pending
  user-side approval (auto-mode classifier blocked it; snippet in session
  transcript). User machine config, not repo work — not roadmapped.
