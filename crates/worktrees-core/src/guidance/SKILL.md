---
name: worktrees
description: Use BEFORE any branch work in a repository managed by worktrees (it has places under .worktrees/ or a .worktrees.toml) — creating a branch, fixing something for a PR, handing work to another agent, releasing, or cleaning up. Covers places, briefs, lanes, messaging, finishing, and what never to touch.
---

# Working in a worktrees-managed repository

A **place** is a git worktree under `.worktrees/<slug>` with its own tmux
session. It is durable; a branch is work that flows through it. `(main)` is the
base checkout at the repository root. Declared state (lifecycle, pin, note)
lives in `.worktrees.places.json`; everything else is read live from git and
tmux. The repository's own AGENTS.md / CLAUDE.md wins on repo-specific rules
(gates, branch names, release steps).

## Doing branch work yourself

- Create a place: MCP `create_worktree {branch}` or `worktrees new <branch>`.
  Then work in its directory, `.worktrees/<slug>`.
- Never `git worktree add` by hand: the tree gets no session, and outside
  `.worktrees/` it is invisible to the app and to `list_places`.
- Never switch branches in `(main)` or in a place that is not yours.
  Moving YOUR OWN place to another branch is fine; it is what a place is for.
- Branch off a freshly fetched default branch. A branch can be checked out in
  only one worktree at a time; never force it into a second.

## Handing work to another agent (a lane)

- `create_worktree` with a `brief` writes `.planning/brief.md` in the new place
  and starts an agent on it. The brief is the whole handover: goal,
  deliverable, rules, and when to stop. It is never passed on a command line.
- `provider` picks the agent (claude, codex, pi) and `model` its model.
- A place with an agent running belongs to that agent. Do not edit its tree;
  talk to it.

## Talking between places

- `list_places` shows every place; `place_status <slug>` shows one, including
  whether its agent is busy.
- `report` posts a message from your place; `messages` reads yours; `wait`
  blocks until a place's agent goes idle or a message arrives.
- Claude sessions also have their own messaging (`SendMessage` to the full
  tmux session name). `send` types into a Codex or pi pane; check the pane is
  not mid-input before typing into it.

## Finishing

- Commit, push and open the pull request from the place, not from `(main)`.
- After it merges, remove the place (`remove_worktree`, destructive tools need
  `confirm: true`) or park it on a fresh branch.
- If the repository has a close-out or release ritual, it is in its own docs.

## Never touch

- Another place's working tree while its agent runs.
- A branch that is checked out in another worktree.
- Any harness's own config or state files (for example `~/.claude.json`):
  read them if you must, never write them.
- A running desktop app you did not start.

## When it has gone wrong

- You committed in `(main)` on a new branch: create a place for that branch
  (`worktrees new <branch>` adopts an existing branch after you switch
  `(main)` back to its default branch), and continue there.
- A worktree outside `.worktrees/`: move it with
  `git worktree move <path> .worktrees/<slug>`, or finish there and remove it.
