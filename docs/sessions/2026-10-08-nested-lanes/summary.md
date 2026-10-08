---
title: "2026-10-08 — optional hubs and nested lanes (proposal)"
---

# Optional hubs and nested lanes — design close-out

- **Date:** 2026-10-08
- **Worktree:** `nested-lanes`
- **Branches:** `nested-lanes` (proposal), `nested-lanes-close-out` (archive)
- **PR:** [#458](https://github.com/penard-monkey/worktrees/pull/458),
  squash-merged as `38161f92866bba3e80abed0518f6a029996e8d5a`.
- **Release tag:** none; this stream shipped documentation only.
- **Planning files:** no `task_plan.md`, `findings.md` or `progress.md` existed.
  `planning.tar.gz` preserves the original `.planning/brief.md`, the session's
  actual planning input; no retrospective working-memory files were invented.
- **Scratch:** `~/.cache/worktrees/worktrees/nested-lanes/` holds validation logs.

## What shipped

[The nested-lanes proposal](../../proposals/nested-lanes.html) and its row in
`docs/proposals/index.md`. No code, app prototype or runtime migration shipped
in this stream.

The proposal covers declared parentage in a physically flat worktree layout,
parent-aware creation/health/PR mapping/close-out, scoped MCP mutations, an
optional hub's development environment, navigation and lifecycle, tmux server
routing/recovery, quota and polling costs. Source claims are pinned to
`34ad3ee`; Graphite, Git Town and GitHub's gh-stack documentation informed the
Git-flow comparison.

Fable's first review requested changes; the revision separated tmux routing
from recovery, corrected Git/MCP facts and made implementation witnesses
explicit. Re-review approved the proposal, with one final wording correction
so the hub checklist follows the owner's review policy. Main merged #458;
this lane did not merge it.

## Decisions

David's decisions recorded in the proposal:

- Hubs are opt-in. Both `main → children` and `main → hub → children` are
  first-class; two edges below main is the depth cap. Omitted MCP `parent`
  still means main, preserving the flat flow.
- Close-out happens at both levels: child into hub, hub into main.
- Place/lane should have one user-facing word; the recommendation is **lane**,
  with existing `place` API identifiers kept compatible. The word choice itself
  remains an open decision.
- One tmux server per project is wanted. The non-launch `-N` hotfix is decided
  and proceeding in its own `tmux-no-start` lane, separately from this proposal.
- The project owner chooses reviewers. The tool supplies reviewer-launch and
  coordination capabilities, never a hard-coded reviewer, model or approval
  policy. Fable review per child is optional advice only.
- The app stays read-only toward GitHub in every phase.

Recommendations still distinct from those decisions: squash child/hub PRs,
merge main into a stable published hub while child owners rebase, refuse hub
removal with any children, and use MCP subtree scope with honest limits around
shell/GitHub authority. Phase 1a is project routing, sandbox namespace and a
restart-based drain with no adoption UI. Phase 1b is lease/SIGUSR1 recovery;
nesting depends on 1a only. The proposal's §11 retains the remaining decisions.

## Dead ends / gotchas

- **A raw base is already supported.** `cmd_new` can start from an explicit
  base; the missing feature is durable parent/target intent, not simply a new
  Git start-point argument. Likewise PR mapping already retains the PR base;
  it needs validation against the expected parent target.
- **Do not invent `Project::snapshot`.** Core's typed sweep is `Project::ls`,
  serialized by `ls_json`, with `place_json` currently receiving one base ref
  for the project. The app's `snapshot` and MCP's `place_snapshot` are separate
  consumers. New structural JSON fields must reach all three and the mock.
- **`git branch -d` is not integration proof.** A pushed child matching its
  upstream can pass deletion after a squash, regardless of whether its work
  reached the intended hub. Require target-specific PR/content evidence.
  Current MCP removal passes neither `--branch` nor `--force`, so it cannot
  delete branches; the dual-purpose force hazard belongs to CLI/app removal.
- **Hub branch deletion can retarget child PRs.** Check open PRs with the hub
  as base even if no child worktrees remain. Guard `--delete-branch` and
  automatic head deletion; GitHub can otherwise move those PRs onto main.
- **Leading tmux flags break the existing fake.** The Bats shim identifies
  `$1` as subcommand and checks the copy-mode chain before parsing options.
  The first `-L`/`-N` change must update that parser and its argv assertions.
- **`-N` and recovery are separate.** In the inspected tmux 3.2 source,
  `-N` sets `CLIENT_NOSTARTSERVER`, blocking the unlink/start path; it does not
  literally clear `CLIENT_STARTSERVER`. It protects non-launch clients, not
  intentional launches. It arrived in 3.2 while this project's README still
  recommends tmux ≥1.9, so compatibility must be explicit.
- **Scope is not a shell sandbox.** MCP can deny a sibling mutation, but cannot
  constrain Claude's own messaging or the user's shared GitHub credentials.
- **Core's process-table test needs process access.** The initial sandboxed
  run failed only the real `ps` fallback test; the complete unsandboxed rerun
  passed. Close-out uses that known requirement rather than calling it a bug.

## Verification

The proposal PR passed the release CLI build, all 478 Bats tests, lint, core
(722 passed, one ignored), CLI (70 passed), app library (162 passed), TypeScript
and app cargo check. All subsequent proposal edits were prose-only and received
link/code-fence and whitespace checks.

Close-out repeated the repository-required local gates from freshly fetched
`origin/main` at `38161f9`: release CLI build, all 478 Bats tests (none skipped),
lint, core (722 passed, one ignored), CLI (70 passed), app library (162 passed),
TypeScript and app cargo check all passed. Summary/index links, whitespace and
tarball byte equivalence passed too. The archive is docs-only, so GitHub CI is
expected to skip by repository policy.

This was source/documentation research. A read-only check found about 20 Claude
Unix sockets under `/tmp/cc-socks`; no fresh cross-server delivery test was
claimed. No existing tmux sessions or installed app were modified. No nested
lane or socket-loss experiment was run on the user's servers.

## Follow-ups

- Track implementation and remaining decisions through the proposal, not this
  archive. `tmux-no-start` and `tmux-routing` are separate active workstreams at
  close-out; this lane neither implements nor takes over their work.
- The roadmap now tracks phase 1b recovery and the shallow-tree vertical slice,
  linked back here. Do not silently make recovery a prerequisite for nesting.
- The existing concurrency-cap roadmap item now includes cross-process launch
  reservations and shared provider/account accounting before larger fan-out.
- The archive PR is handed to main for review/merge. No merge or final parking
  is performed here; a fresh `<slug>-next` base is a post-merge step.
