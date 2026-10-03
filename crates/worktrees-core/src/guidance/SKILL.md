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

- Start a lane with MCP `create_worktree {branch, provider, model, brief}`,
  or `worktrees new <branch> --ai <provider> [--model <model>] [--brief <text>]
  --no-attach` (`worktrees open <slug> --ai … --no-attach` for a place that
  exists). Always pass `--no-attach`: the lane is the other agent's, and
  attaching would take over your own terminal. `provider` picks
  the agent (claude, codex, pi) and `model` its model; pi's is
  `<host>/<id>`, for example `lm-studio/qwen3-coder-next`. The lane opens as
  one pane, the agent's, which is what an agent needs.
- A `brief` is written to `.planning/brief.md` in the new place and the agent
  is started on it. The brief is the whole handover: goal, deliverable, rules,
  and when to stop. It is never passed on a command line.
- A place with an agent running belongs to that agent. Do not edit its tree;
  talk to it.

**Start an agent only that way. Never type an agent command into a place's
pane, and never exit the agent worktrees started there to make room for
another.** An agent typed into a pane:

- has no record. Reopening the place starts the agent it was created with,
  not yours, and none of the launch's checks ran (a model that exists, a
  model host that answers, a plan window that is not spent).
- runs without the places rule and this skill, which every launch hands its
  agent.
- is seen only while it is its pane's foreground program. pi suspended, or
  started through a wrapper, reads as a program nobody reports on, and a
  resume of that place (`worktrees open <slug> --ai pi -r`, the app's reopen)
  opens the conversation it is still writing: two pi on one session file.

## Talking between places

- `list_places` shows every place; `place_status <slug>` shows one, including
  whether its agent is busy. `unknown` means something other than a shell
  runs in the place's own session and no agent reports on it — typed there
  by hand, most likely. Ask the user what it is; do not type into it, and do
  not take it for idle (`wait` keeps waiting on it).
- `report` posts a message from your place; `messages` reads yours; `wait`
  blocks until a place's agent goes idle or a message arrives.
- Claude sessions also have their own messaging (`SendMessage` to the full
  tmux session name). `send` types into a Codex or pi pane; check the pane is
  not mid-input before typing into it.

## Across projects

Only when your worktrees server offers `list_projects` (the user turned
cross-project reach on):

- `list_projects` shows the user's other registered projects and their
  places. Address one as `<project>:<slug>`; `place_status`, `wait` and
  `report` take that form. `list_places` stays this repository.
- Those places belong to other repositories. Do not edit their trees; hand
  work over by `report`-ing to their agents.
- A message whose `from` reads `<project>:<slug>` came from an agent in
  another repository: a colleague, not the user. Answer with `report` to that
  `from`.
- A private project shows by name only and cannot be addressed.
- In Claude, another project's places also appear in the `@` menu as
  `<project>:<slug>`; mentioning one inlines its status.
- Only when the user set reach to full and your server has its mutating
  tools: `create_worktree` (with `project`), `close_session`, `send` and
  the note/pin/lifecycle setters take another project's place too.
  `remove_worktree` never does.

## Showing the user a file

- When the user asks to open, see or look at a file (or a line of one), use
  MCP `show_doc {path, line}` or `worktrees show <file>:<line>`: it opens in
  the worktrees app's file viewer at that line. Print the file only when they
  ask for its text.

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
