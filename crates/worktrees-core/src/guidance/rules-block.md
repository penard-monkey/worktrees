## Working with worktrees

This repository is managed by [worktrees](https://github.com/penard-monkey/worktrees):
every branch lives in its own PLACE, a git worktree under `.worktrees/<slug>`
plus a tmux session. `(main)` is the repository root and stays on the default
branch. If worktrees launched you, you also have the `worktrees` skill: it is
newer than this section and wins where they differ (`worktrees guide` prints
it). Without worktrees installed, ignore this section.

- **Branch work:** MCP `create_worktree {branch}` or `worktrees new <branch>
  --no-attach`, then work in `.worktrees/<slug>`. Never `git worktree add`;
  never switch branches in `(main)` or in a place that is not yours.
- **Spin up a lane** (hand work to another agent): `create_worktree {branch,
  provider, model, brief}` or `worktrees new <branch> --ai claude --brief
  "<text>" --no-attach`. The brief lands in `.planning/brief.md` and is the
  whole handover: goal, deliverable, rules, and when to stop and report. Never
  type an agent command into a pane.
- **Which tool:** `list_places` (every place) · `place_status` (one place: is
  its agent busy, what is its plan) · `report` / `messages` / `wait` (talk
  between places, wait for an agent to go idle) · `send` (type into a Codex or
  pi pane; check it is not mid-input first) · `show_doc` (open a file for the
  user) · `set_note` / `set_lifecycle` · `remove_worktree` (needs `confirm`).
  Claude sessions can also `SendMessage` the place's tmux session name.
- **A place with a running agent belongs to that agent.** Do not edit its
  tree; message it.
- **Planning:** plans are working memory in `.planning/` inside each place
  (`task_plan.md`, `findings.md`, `progress.md`). Not committed.
- **Finishing:** commit, push and open the pull request from the place. After
  it merges, remove the place or park it on a fresh `<place>-next` branch. At
  the end of a work stream, run the `close-out` skill.
- **Never touch:** another place's working tree, a branch checked out in
  another worktree, a harness's own state files (`~/.claude.json`), or a
  running app you did not start.
