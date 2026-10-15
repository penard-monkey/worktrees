# rules-review-lane — a review lane for every `agent-instructions` branch

- **Date:** 2026-10-15
- **Place:** `rules-review-lane` (lane launched by `(main)` against a brief)
- **Branch:** `rules-review-lane`
- **PR:** #502 (squash-merged as 554ad99)
- **Release:** none yet (under `[Unreleased]` → `### Added`)
- **Planning files:** none (no `task_plan.md`/`findings.md`/`progress.md`); the brief stays in `.planning/brief.md`. Proposal: `~/.cache/worktrees/worktrees/rules-review-lane/proposal.md` (scratch); the fable review: `~/.cache/worktrees/worktrees/main/review-502.md`

## Why

David applied the rules pack in anime-song-remover from the app. Apply
committed `agent-instructions` locally, as designed, and then nothing pointed
at it:
- no place was opened for it;
- the only trace was an info To do row that said "push it and open a PR" in a
  repo with **no remote**;
- the pending branch hid the rules rows, which removed "Repair / upgrade…"
  from the menu.

## What shipped

- **`crates/worktrees-core/src/review.rs`** (new):
  - `brief`: a fixed template, `guidance/review-brief.md`, filled with computed
    facts: base ref, default branch, origin or not, commit count, and
    `git diff --raw -z -M <base>...agent-instructions`.
    - Each changed path goes through a fixed table (`what_is`): rules block,
      close-out skill, `close-out.md` ("GENERIC defaults"), CLAUDE.md stub,
      and `.agents/skills` **symlinks only**.
    - Anything else is flagged "NOT written by worktrees" and counted.
    - Paths are `tame`d and fenced as data; `{files}` is filled last.
  - `open_lane`: `cmd_new agent-instructions --brief … --no-attach --no-spare
    --no-install`, plus `--no-tmux` for the CLI.
  - `cli_lane` / `cli_outcome`: the CLI's half. With `--json` the lane is
    captured into the one object as `lane`.
  - `land` / `land_with`:
    - **No origin:** fast-forward main in the root. Main must be checked out
      in the root with no tracked changes, the place must be clean, and the
      branch strictly ahead. The tip SHA is read once and that SHA is merged.
      If main already equals the tip, `land` retires the place instead.
    - **Origin:** push + `gh pr create --fill`, and the place stays. While the
      PR is OPEN, `land` re-pushes; once it is MERGED, it retires.
  - `abandon --yes`, and `retire_merged` (the app's button, which re-asks
    `gh` and refuses a dirty place).
  - `retire`: the shared exit.
    - Reads `own_session()` first.
    - Runs `ops::remove_tree`, then `branch -d/-D`, then the origin delete
      (only when the origin tip equals the local tip).
    - A "remote ref does not exist" counts as deleted, and the tracking ref is
      dropped.
    - Prints a farewell built from what actually happened, then kills the
      other sessions and its own session **last**.
- **`ops::remove_tree`**: removes a place and RETURNS its sessions instead of
  killing them. `remove_one` kills first, so it can't be used from inside the
  place.
- **`agentfiles::Report`** gains:
  - `has_origin`;
  - `pending_place`;
  - `pending_merged`, filled by the app only, from a `gh` call cached for
    5 minutes (`pr_merged_cached`).
- **CLI:**
  - `rules review|land|abandon`;
  - `rules apply` / `agent-setup fix` open the lane (place + brief, no agent;
    `--review` starts one).
- **MCP:** `apply_rules` opens no lane; its notes and tool description carry
  the two commands.
- **App:**
  - Tauri commands `rules_review` and `rules_lane_remove`.
  - `useAgentSetup(…, onCommitted)` opens the lane after a LOCAL commit.
  - `projectTodos` `agent-pending` rows have three actions: `review`,
    `open-lane` and `remove-lane`. The `repair` flag brings back uncounted
    Repair for a local branch with no place, and for a merged PR.
  - `PendingHint` wording no longer tells a repo with no remote to push.
  - The mock gained `rules_review` / `rules_lane_remove`, plus the
    `?noorigin` and `?prmerged` URL parameters.

## Decisions

- **The two decisions are core commands, not prose in the brief.** The agent
  can't safely improvise "merge main from a side worktree" or "delete my own
  place". The commands carry the guards, and the brief only says which one to
  run, on the user's explicit word.
- **Kill last.** The agent runs inside the place it removes. Every git step
  and the final line come first, then the other sessions die, then its own.
- **The CLI opens a place + brief but starts no agent.** A CLI run is often a
  script or an agent in a shell, and a second agent spending quota behind it
  is the wrong default. `worktrees open` already launches on the brief.
- **MCP opens no lane.** The caller is already an agent with the user's
  attention.
- **"Merged" is GitHub's word (`gh pr view`), never a subject match.** A
  squash merge leaves the branch looking unmerged to git. The app's cached
  "merged" is harmless because removal asks again.

## Dead ends / gotchas

- **`worktrees rm` cannot remove the place you are in.** It refuses a cwd
  inside the place, and it kills the session before `git worktree remove`.
  That finding shaped the whole design (`remove_tree` + `retire`).
- **GitHub's auto-delete of head branches** (review blocker):
  - the merged branch disappears from origin;
  - our `push --delete` fails, and the local tracking ref SURVIVES a failed
    push;
  - `pending_state` reads exactly that ref, so the project stayed "waiting to
    merge" forever, and `abandon` failed the same way.
  - Fix: treat "remote ref does not exist" as done and `update-ref -d`.
- **`--name-status` has no mode.** A regular file under `.agents/skills/`
  passed as "a link"; `--raw` gives the mode.
- **`git_status_captured` writes git's stdout through to OUR stdout.** That
  broke `--json` purity until `git::set_stdout_is_protocol()` ran before the
  lane.
- **A Rust farewell assertion proves nothing about ORDER.** `CaptureUi`
  records no kills. The bats test logs git calls into the fake tmux log
  instead, and asserts every `kill-session` comes after the last git call and
  that the caller's own session is the final line.
- **Harness:**
  - Playwright's `dispatchEvent("contextmenu")` carries no coordinates, which
    produces a `NaN` CSS `left` console error; it's harness-only.
  - A two-click arm needs a pause between the clicks inside one evaluate,
    because the second click in the same tick sees the unarmed closure.
- **A pty test flaked once:** `a_real_size_change_still_reaches_the_shell`
  ("the trap shell never came up") failed in one full `cargo test -p app
  --lib` run and passed on rerun. It's on ROADMAP.

## Verification

- Every new test was shown red first, by mutating the code under test (or,
  for `todos-check.mjs`, by running against the old `projectTodos.ts`).
  - core `review::tests` (13): dirty root and dirty place, the
    root-holds-branch case, OPEN vs MERGED, the origin tip, the foreign count,
    the skills symlink mode, the no-place farewell, the auto-deleted branch,
    a dirty `retire_merged`, `pending_place`.
  - bats: land from INSIDE the place with git-before-kill and own-last order;
    `--json` is one object; abandon needs `--yes`.
  - MCP unit test; `todos-check.mjs` §4b.
- Gates: bats 516/516, core 795, cli 68+1, app 163, lint, tsc, all
  `app/scripts/*-check.mjs` from the root. CI was green on both pushes.
- Mock harness (port 1437, headless Playwright):
  - Review from the To do row selects the lane, and Repair clears;
  - Fix on casa-del-valle opens the lane automatically;
  - `?noorigin` wording;
  - `?prmerged` → Remove the place.
- **Not run:** the real app. That is a ROADMAP item for David to do by hand.

## Follow-ups

- Drive the lane once in the real app (ROADMAP).
- `ops::write_brief` follows a tracked `.planning` symlink (ROADMAP; it is
  older than this PR but now runs automatically on Apply).
- The flaky pty test (ROADMAP).
- anime-song-remover's existing branch: David decides. The new Review button
  covers it.
