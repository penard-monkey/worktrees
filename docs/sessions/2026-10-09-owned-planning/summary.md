---
title: "2026-10-09 — worktrees owns planning (proposal)"
---

# Worktrees owns planning, opt-in — design close-out

- **Date:** 2026-10-09
- **Worktree:** `owned-planning`
- **Branches:** `owned-planning-proposal` (proposal),
  `owned-planning-close-out` (archive)
- **PR:** [#469](https://github.com/penard-monkey/worktrees/pull/469),
  squash-merged as `b4f6f74`.
- **Release tag:** none. This stream shipped documentation only.
- **Planning files:** none were written. `planning.tar.gz` keeps the
  original `.planning/brief.md`.
- **Scratch:** `~/.cache/worktrees/worktrees/owned-planning/` holds:
  - `survey.txt`, the per-place planning layout across worktrees, valleos
    and cdv;
  - the PR-body and section drafts.

## What shipped

[The owned-planning proposal](../../proposals/owned-planning.html), its row in
`docs/proposals/index.md`, and a ROADMAP entry. No code shipped. Phase 1 is
being built in the `owned-planning-p1` lane.

The proposal covers:
- an opt-in that is offered after an update and is user-tier only;
- three levels per project: **off** (today exactly), **show only** (your own
  planning files at a path and scope you choose, read-only, no hooks) and
  **full** (the `.planning/<topic>/` + `.active_plan` layout, `new` writes the
  pointer, Claude hooks per launch);
- one resolver (`plan::resolve`) shared by the Plan tab, MCP, the CLI and every
  hook;
- a `[plan] project` key (a path, so ADR 0001 allows it);
- concrete migration for the pilot (casa-del-valle-monorepo) and for valleos
  and this repo.

## Decisions

- **Wrap planning-with-files, don't replace it.** The skill stays the method.
  Worktrees owns where the files live, which plan is active, and what is
  injected.
- **The level is user tier only.** It decides whether hooks run, so a
  cloned repo may only suggest it (`[plan]` pre-ticks the level in Add
  existing; Clone asks after the clone).
- **Full mode removes the newest-directory guess.** A valid pointer whose
  plan is not written yet yields to a root plan, otherwise means "not written
  yet". A non-plain or symlinked pointer is `invalid_pointer`, and the hook
  never echoes its value or tells anyone to write through it.
- **Phase 1 hooks are SessionStart and UserPromptSubmit only.** A Stop hook's
  plain stdout never reaches the model.
- **The planning folder stays `.planning/`.** The skill's scripts and the fixed
  `BRIEF_OPENER` depend on it, and show only is the escape hatch. Full is
  refused at adoption where `.planning/` holds tracked files, and every
  `new` warns about them.
- **Show only has a scope.** `place` reads each lane's own copy. `main`
  reads main's working copy, for a goals file, labelled as main's copy in
  lanes.
- **cdv is the pilot** (David, relayed). #684 stays a draft. Phase 1 was
  re-sized to what cdv needs, plus show only (cheap, read-only) and an
  app-vs-CLI version check.
- **The `/close-out` skill does not ship with the app.** A `worktrees plan
  archive` primitive (phase 2) is the thing every user needs.

## Dead ends / gotchas

- **The tracked-copy hijack is live.** `valleos:investigate-ci-rentention` (no
  `.active_plan`) showed main's project goals as its own plan
  (`how_resolved: newest`, `.planning/orchestrator/task_plan.md`). A tracked
  file under `.planning/` is checked out in every lane, its directory's mtime
  is the checkout time, and the guess picks it.
- **valleos's `plan-context.sh stop` has never reached a model.** Stop-hook
  stdout goes to the debug log. Its budgets were tuned for size only.
  (Caught in review; the first draft repeated the mistake.)
- **"The same code" is only true at the same version.** The Plan tab runs core
  inside the app; a hook runs the CLI. The app's `cliStale`
  (`App.tsx:3642`) compares the CLI with the latest release, not with the
  app.
- **`safe_under` accepts regular files only** (and refuses a trailing slash),
  so a show-only *directory* needs a `safe_dir_under` sibling.
- **A pointer that is missing its plan is not the same as an invalid
  pointer.** The first draft would have told an agent to write its plan
  through a symlinked topic dir.
- **The "additive" contract change wasn't additive.** New `how_resolved`
  *values* break a closed TS union (`PlanPane.tsx:32`). Keep `root`, and add
  `level`.
- **Clone cannot be pre-ticked from the repo, and neither can New project.**
  Nothing is on disk yet.

## Verification

- **Read:**
  - `plan.rs`, `ops.rs`, `guidance.rs`, `projcfg.rs`, `registry.rs`,
    `offers.ts`, `projectTodos.ts`, `PlanPane.tsx`, `docserver.rs`, ADR 0001
    and the agent-guidance proposal;
  - the valleos and cdv hooks, scripts and `.gitignore`s;
  - cdv PR #684;
  - the planning-with-files 2.37.0 scripts;
  - Claude 2.1.296, codex 0.161.0 and pi 0.99.1 `--help`, and pi's extension
    docs.
- **Surveyed:** all 41 places across the three repos (`survey.txt`). One
  cross-project `place_status` confirmed the hijack.
- **Reviewed by fable:** four rounds, ending APPROVE. Each round's findings
  are folded into the proposal.
- **No gates were run.** The PR was docs-only, and CI skips it by design.

## Follow-ups

- **Phase 1** is being built in the `owned-planning-p1` lane. Two nits from
  the final review went there: `main` scope reads `safe_under` of main's root
  (its working tree as-is), and the "main unreadable" failure reason.
- **Phases 2 and 3,** and the open §11 questions, are in `ROADMAP.md`.
- **cdv, outside this repo:**
  - add `.planning/` to `.dockerignore` (#684's Finding 5) as its own PR now;
  - close #684 once the pilot runs.
