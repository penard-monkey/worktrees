---
name: close-out
description: Session close-out ritual before /clear — archive the session (summary + planning tarball), sweep stragglers into the roadmap, verify nothing is left uncommitted, then park the place on a fresh branch. Use when the user says "close out", "wrap up the session", "archive this session", or wants to /clear and start a new thread in this repo.
---
<!-- worktrees:managed v1 sha=90bfffeda4620ef0 — written by `worktrees rules apply`; edit it and it stops updating -->

# Close-out ritual

Run this top to bottom, from the place the work happened in (never from
`(main)`). Goal: a future session (or a human) can reconstruct how we got here
from the committed archive alone, and the place is clean for a fresh branch +
`/clear`.

## 0. Load project config

Read `.claude/close-out.md` in the repository if it exists — it overrides the
defaults below (archive dir, scratch dir, gate commands, roadmap file, merge
style, branch naming). Everything it does not mention keeps the default.

| Setting | Default |
| --- | --- |
| archive dir | `docs/sessions/<YYYY-MM-DD>-<slug>/` |
| scratch dir | `~/.cache/worktrees/<project>/<place>/` |
| planning files | `.planning/<topic>/` or the place root: `task_plan.md`, `findings.md`, `progress.md` |
| roadmap | `ROADMAP.md` |
| gates | whatever AGENTS.md / CLAUDE.md says to run before a PR |
| merge | squash |
| idle branch | `<place>-next` |

No config file and no `docs/` convention in the repo? Say so and propose the
defaults before writing anything — do not invent a directory layout silently.

## 1. Preconditions

- All feature/release work for the stream is merged: `git status`,
  `git log origin/<default>..HEAD`, `gh pr list --author @me`. Nothing open
  from this place.
- **Check the base is current BEFORE editing anything:**

  ```sh
  git fetch origin
  git rev-list --left-right --count origin/<default>...HEAD   # left=behind, right=ahead
  ```

  A clean `git status`, no unpushed commits and no open PRs all look identical
  on a tree that is twelve commits and a release behind. If the left number is
  non-zero, branch off the fetched default branch first.
- Gates green if any code changed since the last merge.
- Lanes this place started are finished or handed back: `list_places` /
  `place_status`. Do not archive while one of them is still writing.

## 2. Scratch files → cache

Scratch artifacts (screenshots, harness output, one-off scripts) leave the
repository and go to the scratch dir. Create it if missing; move any strays out
of the place. They are NOT committed. If the summary references one or two
pivotal screenshots, copying just those into the archive dir is allowed.

## 3. Session summary (committed)

Create `<archive-dir>/summary.md` on a new branch off a freshly fetched default
branch, in this place. Sections:

- header list: date, place, branch(es), PR links, release tag, planning files
  (or "none")
- **What shipped** — with file paths
- **Decisions** — each with the why
- **Dead ends / gotchas** — what failed and the root cause (the most valuable
  section for future sessions)
- **Verification** — what was actually run and observed
- **Follow-ups** — anything not finished

## 4. Planning files → tarball next to the summary

If planning files exist (gitignored working memory), tarball them INTO the
archive dir and remove the originals:

```sh
tar czf <archive-dir>/planning.tar.gz -C <planning-dir> task_plan.md findings.md progress.md
rm <planning-dir>/{task_plan,findings,progress}.md
```

Leave `.planning/brief.md` alone: it belongs to the place, not the session.

## 5. Straggler sweep

Look for work that got mentioned but not done: summary follow-ups, TODOs in the
diff, half-finished branches and places (`list_places`, `git branch -a`),
comments like "later" in touched files.

## 6. Roadmap

Anything worth keeping goes into the roadmap file (create if missing — short
bullets, link the session summary for context). Dross gets dropped
deliberately, not silently.

## 7. Commit → PR → merge

One PR for the archive (+ roadmap + any AGENTS.md learnings): summary dir,
tarball, roadmap. Title `chore: close out <slug> session`. Merge per the
project's style (default: squash). A merge command run from a place may fail
AFTER the merge landed (its local checkout step finds the default branch
checked out in `(main)`): check the PR's state before retrying.

## 8. Park the place

After merge, park this place on its idle branch — never switch `(main)`:

```sh
git fetch origin
git checkout -B <place>-next origin/<default>
git branch --unset-upstream    # the idle branch must not track the default branch
git rev-list --left-right --count origin/<default>...HEAD   # expect 0 0
```

Every place has its OWN idle branch: a branch checked out in two worktrees
leaves one of them with a stale tree that reads as the inverse of everything
that landed. Confirm `git status` is clean, then tell the user the place is
ready and they can `/clear`. If the place is not coming back, remove it
instead (`remove_worktree`, or `worktrees rm <slug>`).
