---
title: "Session — research: context usage per agent session"
---

# Session — research: how full is each agent's context window

- **Date:** 2026-10-03
- **Worktree:** `.worktrees/context-usage`. It was removed before close-out, so
  this archive was written from `.worktrees/context-usage-closeout`.
- **Branches:**
  - `context-usage`, the proposal;
  - `context-usage-closeout`, this archive.
- **PRs:** [#427](https://github.com/penard-monkey/worktrees/pull/427),
  squash-merged by the orchestrator as `15b5ccb`.
- **Release:** none. This was research only.
- **Planning files:** `planning.tar.gz` here. It holds `task_plan.md`,
  `findings.md`, `progress.md` and `.planning/brief.md`. All four were
  **reconstructed** from the session after the worktree that held the
  originals was removed. Every number in them is also in the proposal.

## Why

David wants each place's agent to show how full its context window is, for
example "62% of 1M". The orchestrator should be able to see the same thing
through MCP, so it can tell a lane near its limit to `/compact` or hand off.
The brief was research and planning only.

## What shipped

`docs/proposals/context-usage.md`, plus its row in `docs/proposals/index.md`.
It contains:

- a per-harness table: source, numerator, window, freshness, compaction signal,
  auto-compact point, tail fit and confidence;
- redacted real records;
- what is impossible or unreliable;
- a shared core shape (`ContextUse`);
- surfaces drawn in ASCII as a %, a bar and tokens;
- thresholds and three phases;
- six open questions (§11).

## Decisions

All of these are proposed; none is built.

- **Each harness's numerator is the CLI's own figure:**
  - **Claude:** `input + cache_creation + cache_read` from the last real
    main-chain assistant `usage`.
  - **Codex:** `last_token_usage.total_tokens` from the last `token_count`
    event.
  - **pi:** `totalTokens` of the last assistant reply after the latest
    compaction.
- **Show each CLI's own arithmetic**, including Codex's 12K baseline, so the
  chip never disagrees with the lane on screen. This is open question 5.
- **Claude's window is inferred, in this order:**
  1. the `[1m]` model passed at launch;
  2. a table of model defaults;
  3. a ratchet: if any prompt was over 200K, the window is 1M;
  4. otherwise show tokens and no %.

  A wrong denominator is worse than none.
- **Ride the existing 256 KiB tails.** These are the reads that already find
  the model: `cached_model`, `codex_tail` and `tail_info`. The app gets a new
  `sessions:context` event, not `places:changed`, because context moves on
  every message and a re-list is a git fan-out.
- **Primary surface: a mark on the nav row that appears only near the limit.**
  The question is cross-place ("which lane is about to compact?"). The header
  shows the detail at all times, and MCP `place_status` gets it in phase 1.
- **Warn on headroom against each harness's auto-compact point, not on a % of
  the window.** Claude 200K compacts at 83.5% of its window and Claude 1M at
  96.7%.

## Dead ends / gotchas

- **Claude 2.1.288's `/model X` writes `model` into
  `~/.claude/settings.json`** ("saved as your default for new sessions").
  - The probe's `/model sonnet[1m]` changed David's default. It was restored
    at once with `/model opus`, through the CLI, not by editing the file.
  - No other Claude session started in that window of about one minute: I
    checked `startedAt` in the session probes.
  - In probes, pass `--model` at launch instead.
  - Recorded in ROADMAP and in `docs/ai-profiles-manual-checks.md`.
- **The window is nowhere on disk.**
  - The transcript's `message.model` is the bare id, and in 2.1.288 the
    `/model` echo no longer says "(1M context)".
  - Opus 5.5 runs at 1M with no suffix. The live `/context` said
    "82.1k/1m", and "Autocompact buffer: 33k".
  - `~/.claude.json`'s `lastModelUsage` keys sometimes carry `[1m]`, but they
    are cumulative per-project totals, not a current reading.
- **`compactMetadata.postTokens` is not the new context size.** It counts
  messages only (8–50K). The first real reply after a compaction showed
  54–127K, once the system prompt, tools and memory are added back.
- **The first assistant entry after a compaction is often `<synthetic>` with
  zero usage** (6 of 33). Read naively, it gives a false 0%.
- **Codex `total_token_usage` is cumulative** (20M+), so it is billing, not
  context. Codex Desktop placeholder rollouts have a null window.
- **pi's footer adds an estimate of the trailing messages, and a tail read
  does not.** Ours reads slightly low in the middle of a turn.
- **The context-usage worktree and its local branch were deleted before
  close-out, although the message asking for close-out said not to remove the
  place.** The gitignored planning files went with them. They were rebuilt for
  this archive from the session's own notes.

## Verification

- **Real files read:**
  - 251 Claude transcripts from the last 14 days, and 37 compaction boundaries
    from the last 30;
  - 180 Codex rollouts from the last 30 days;
  - all 27 pi sessions.
- **Tail fit:** the last usage line was ≤ 154 KB from the end of the 40
  largest Claude transcripts, ≤ 2 KB for Codex CLI rollouts and ≤ 2.7 KB for
  pi.
- **Live probe:** run in a throwaway `worktrees new` place on
  `tmux -L ctxprobe-cu` and `tmux -L ctxprobe-cx`. Both servers were killed
  and the place was removed afterwards.
  - Claude haiku: 63,859 = `/context` "63.9k/200k (32%)".
  - `/clear` changed the probe's `sessionId` at once.
  - Codex: 30,888 / 258,400 = `/status` "92% left (30.9K used / 258K)".
- **Formulas read in the installed sources:** Claude 2.1.288's window
  function (`vv`) and pi 0.99.1's `getContextUsage`, `shouldCompact` and
  footer.
- Scratch scripts are in `~/.cache/worktrees/worktrees/context-usage/`.

## Follow-ups

- §11's six questions need David's answers before phase 1. This is parked in
  ROADMAP.
- The `/model` hazard is in ROADMAP. A probe line was added to
  `docs/ai-profiles-manual-checks.md`.
