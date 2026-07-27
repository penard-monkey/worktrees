---
name: close-out
description: Session close-out ritual before /clear — archive the session (summary + planning tarball), sweep stragglers into the roadmap, verify nothing is left uncommitted, then hand the user a fresh branch. Use when the user says "close out", "wrap up the session", "archive this session", or wants to /clear and start a new thread in this worktree.
---

# Close-out ritual

Run this top to bottom. Goal: a future Claude thread (or human) can
reconstruct how we got here from the committed archive alone, and the
worktree is clean for a fresh branch + `/clear`.

## 0. Preconditions

- All feature/release work for the stream is merged (no open PRs from this
  worktree, no unpushed commits: `git status`, `git log origin/main..HEAD`,
  `gh pr list --author @me`).
- Gates green if any code changed since the last merge (see CLAUDE.md Gates).

## 1. Scratch files → ~/.cache

Scratch artifacts (screenshots, harness output, one-off scripts) live in
`~/.cache/worktrees/<project>/<worktree-name>/` — e.g.
`~/.cache/worktrees/worktrees/ui-changes/`. (XDG cache: survives reboots,
never auto-purged — unlike /tmp, which macOS wipes on reboot and prunes
after ~3 days.) Create the dir if missing; move any strays out of the repo
root (nav-*.png, theme-*.png and friends). They are NOT committed. If the
session summary references one or two pivotal screenshots, copying just
those into the session archive dir is allowed.

## 2. Session summary (committed)

Create `docs/sessions/<YYYY-MM-DD>-<slug>/summary.md` on a new branch off
origin/main. Sections:

- header list: date, worktree, branch(es), PR links, release tag, planning
  files (or "none")
- **What shipped** — with file paths
- **Decisions** — each with the why
- **Dead ends / gotchas** — what failed and the root cause (highest-value
  section for future sessions)
- **Verification** — what was actually run/observed
- **Follow-ups** — anything not finished

## 3. Planning files → tarball next to the summary

If `task_plan.md` / `findings.md` / `progress.md` exist (gitignored working
memory), tarball them INTO the same session dir and remove the originals:

```sh
tar czf docs/sessions/<date>-<slug>/planning.tar.gz task_plan.md findings.md progress.md
rm task_plan.md findings.md progress.md
```

The tarball rides with the summary so the reasoning trail survives.

## 4. Straggler sweep

Look for work that got mentioned but not done: summary Follow-ups, TODOs in
the diff, half-finished branches/worktrees (`git worktree list`,
`git branch -a`), comments like "later"/"task plan" in touched files.

## 5. Roadmap

Anything worth keeping goes into `ROADMAP.md` (create if missing — short
bullets, link the session summary for context). Dross gets dropped
deliberately, not silently.

## 6. Commit → PR → merge

One PR for the archive (+ roadmap + any CLAUDE.md learnings): summary dir,
tarball, ROADMAP.md. House style: squash-merge. Title
`chore: close out <slug> session`.

## 7. Fresh start

After merge: `git checkout -b <next-branch> origin/main`, confirm
`git status` clean, tell the user the worktree is ready and they can `/clear`.
