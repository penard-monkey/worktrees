# close-out overrides — worktrees

Project config for the global `close-out` skill (`~/workspace/claude-skills`).
Only the deltas from that skill's defaults live here.

| Setting | This repo |
| --- | --- |
| archive dir | `docs/sessions/<YYYY-MM-DD>-<slug>/` (default) — ALSO add a row at the top of the table in `docs/sessions/index.md`, linking `<slug>/summary.html` (Jekyll site: `.md` → `.html`) |
| scratch dir | `~/.cache/worktrees/<project>/<worktree-name>/` — e.g. `~/.cache/worktrees/worktrees/ui-changes/` |
| planning files | `task_plan.md`, `findings.md`, `progress.md` (gitignored) |
| roadmap | `ROADMAP.md` (default) |
| merge | squash (default) |
| branch naming | per worktree — `<tree>-<something>`, idle base `<tree>-next`: `bug-fixes` → `bug-fixes-next`, `ui-changes` → `ui-changes-next`. ⚠ `ui-next` belongs to the **ui-tweaks** tree; two worktrees must never share an idle base (see the phantom-state note below) |

## Gates (step 1 — run before the archive PR)

```sh
cargo build --release -p worktrees-cli   # FIRST — stale release binary makes bats fail mysteriously
make test
make lint
cargo test -p worktrees-core
cargo test -p worktrees-cli
cd app && ./node_modules/.bin/tsc --noEmit && cargo check -p app
```

## Notes

- Playwright MCP artifacts land in `.playwright-mcp/` inside the repo (it
  refuses paths outside the roots) — move them to the scratch dir in step 2.
- `_tmp/` is a user symlink (iCloud) for screenshots under review; not repo
  scratch, leave it alone.
- Repo-wide conventions and the "hard-won rules" that belong in CLAUDE.md go
  in the archive PR alongside the summary.
- **Two worktrees on one branch ⇒ phantom staged changes.** Whichever tree moves
  the shared ref wins; the other keeps a stale working copy, and its index then
  reads as the *inverse* of everything that landed meanwhile. It looks like a
  huge accidental deletion. Before resetting, prove it is phantom —
  `diff <(git diff --cached) <(git diff <branch-tip> <the-commit-you-were-on>)`
  should be empty — and check for untracked files and stashes, because the
  reflog will NOT show the move (it only records this tree's own checkouts).
  Give every tree its own `<tree>-next`.
