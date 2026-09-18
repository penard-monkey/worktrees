# close-out overrides — worktrees

Project config for the global `close-out` skill (`~/workspace/claude-skills`).
Only the deltas from that skill's defaults live here.

| Setting | This repo |
| --- | --- |
| archive dir | `docs/sessions/<YYYY-MM-DD>-<slug>/` (default) — ALSO add a row at the top of the table in `docs/sessions/index.md`, linking `<slug>/summary.html` (Jekyll site: `.md` → `.html`) |
| scratch dir | `~/.cache/worktrees/<project>/<worktree-name>/` — e.g. `~/.cache/worktrees/worktrees/ui-changes/` |
| planning files | `task_plan.md`, `findings.md`, `progress.md` (gitignored) |
| roadmap | **GitHub issues, not a file** — see “Roadmap → issues” below. `ROADMAP.md` is now an index over them plus the deliberately-not-issues; it is edited only when a straggler belongs in one of those sections |
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

## Roadmap → issues (replaces the skill's step 6)

Stragglers become **GitHub issues**, not ROADMAP.md bullets. The roadmap file is
the index; `gh` is the parking lot.

For each thing worth keeping:

1. **Is it a task?** If it is a decision already made, a note waiting on
   evidence, or a chore that belongs to a machine rather than the codebase, it
   goes in the matching section of `ROADMAP.md` — *Decided and declined*,
   *Parked with a reason*, *Waiting on evidence*, *Local chores*. Those sections
   keep their full prose. Do not file them.
2. **Otherwise file it:** `gh issue create`, carrying the full prose (the
   context is the point — do not compress it into a bullet), and linking the
   session summary at the bottom the way the existing issues do.
3. **Label it.** One `area:*` (`app` / `core` / `cli` / `ci-tooling` / `docs`),
   plus `bug` / `enhancement` / `tech-debt` as it fits. Add `good first issue`
   **only** if it is self-contained, needs no hardware beyond a checkout, and
   the CLAUDE.md gates are the whole bar — that label is how an outside
   contributor finds a starting point, so a wrong one costs them an evening.
4. **`needs-real-mac`** for anything only confirmable by hand in the real app.
   Prefer appending a checkbox to the existing tracker
   ([#294](https://github.com/penard-monkey/worktrees/issues/294), or
   [#295](https://github.com/penard-monkey/worktrees/issues/295) for
   `docs/ai-profiles-manual-checks.md`) over filing a new issue — twenty issues
   that all say “open the app” help nobody.
5. **Add a row** to the matching table in `ROADMAP.md`.
6. **Close what the session finished.** `gh issue close <n> --comment` with the
   PR that did it. This is the step that keeps the list honest, and it is the
   one the old append-only file never had.

⚠ Cross-references inside an issue body: `#N` resolves against this repo, and
PR numbers and issue numbers share one sequence. Check what a number actually
points at before writing it — `#49` is a PR about the busy dot, not the
note-focus bug.

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
- **The archive PR is docs-only and CI skips it by design** (`ci.yml`
  `paths-ignore`: `docs/**`, `ROADMAP.md`, `CLAUDE.md`, `.claude/**`, root
  prose docs). Zero checks on the PR is the expected state — merge without
  waiting for CI. The local gates in step 1 still run.
