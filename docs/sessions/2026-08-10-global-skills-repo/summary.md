---
title: "Session: close-out leaves the repo — a personal skills repo"
---

# Session: close-out leaves the repo — a personal skills repo

- **Date:** 2026-08-10 (work started the evening of 2026-08-09)
- **Worktree:** bug-fixes
- **Branches:** bug-fixes-global-skills (this archive)
- **PRs:** this one
- **Release:** none — documentation and tooling only, no code touched
- **Planning files:** none (single-thread session)
- **Companion repo:** `~/workspace/claude-skills` — commit
  `4230781 feat: personal skills repo with symlink installer + global close-out`
  (local only, no remote at time of writing)

## Where this started

> "We have a close-out skill in this repo. I want to make it global but I want
> to make it in a way that makes more sense. I was thinking maybe we should
> make a claude-skills project on my machine … So in ~/workspace/claude-skills
> I made a repo so we can put them there."

The close-out ritual was good enough to want everywhere, but it lived at
`.claude/skills/close-out/SKILL.md` inside this repo, so only this repo had it.
Copying the file into every project would fork it immediately.

## What shipped

**A personal skills repo — `~/workspace/claude-skills`** (outside this
repository; not a submodule, deliberately — see Decisions):

```
skills/close-out/SKILL.md   the ritual, repo-agnostic
install.sh                  symlinks skills/* → ~/.claude/skills/*
README.md                   layout + the per-project-config pattern
```

`install.sh` is idempotent, bash-3.2 compatible and shellcheck-clean. It
refuses to clobber an existing `~/.claude/skills/<name>` — whether a real
directory or a symlink pointing somewhere else — unless given `--force`, and
exits non-zero when it skipped something so a silent partial install can't
pass for a good one.

**In this repo:**

- `.claude/skills/close-out/` — deleted. The global skill supersedes it.
- `.claude/close-out.md` — new. Project overrides the global skill reads:
  scratch dir (`~/.cache/worktrees/<project>/<worktree-name>/`), the full gate
  list, the `docs/sessions/index.md` row requirement, branch naming
  (`bug-fixes-<something>` in this worktree, idle base `bug-fixes-next`), and
  the Playwright-MCP / `_tmp/` notes.
- `CLAUDE.md` — the Close-out ritual section now points at the global skill and
  says to edit `.claude/close-out.md`, not the skill.

## Decisions

**Symlinks, not copies.** `install.sh` links `skills/<name>` into
`~/.claude/skills/<name>`. Editing a skill in the repo is live in the next
session with no reinstall step, which is the whole reason the skill stayed
current while it lived in-repo. It also matches the existing arrangement on
this machine: third-party skills are real directories in `~/.agents/skills`
symlinked into `~/.claude/skills` (tracked by `~/.agents/.skill-lock.json`).
The installer does not touch those.

**A plain skills repo, not a plugin marketplace.** Claude Code can consume a
repo as a local plugin marketplace (`.claude-plugin/marketplace.json` +
`plugin.json`, added with `/plugin marketplace add`), which buys versioning and
cross-machine sync and can carry hooks/agents/commands too. Rejected for now:
it namespaces every skill (`/dp-skills:close-out` instead of `/close-out`) and
adds scaffolding for a one-skill repo. The layout here doesn't block that move
later.

**Ritual in the skill, paths in the repo.** The global SKILL.md holds the eight
steps and a defaults table; anything repo-specific comes from an optional
`.claude/close-out.md` in whichever repo the skill runs in. A repo with neither
that config nor a `docs/` convention gets an explicit "propose the defaults
first" instruction rather than an invented directory layout. This is the part
that made the skill portable at all — the old one hardcoded `docs/sessions/`,
`~/.cache/worktrees/`, `ROADMAP.md` and this project's gate commands.

**Not a git submodule of this repo.** The skills repo is per-machine
configuration, not a dependency of worktrees. Vendoring it would drag the
worktrees repo into every unrelated skill edit.

## Dead ends / gotchas

**The tree was 12 commits behind, and nothing said so.** `bug-fixes-next` looked
like a clean, current base — `git status` clean, no unpushed commits, no open
PRs. `git rev-list --left-right --count origin/main...HEAD` returned `12 0`:
all twelve on origin/main's side, including a **v0.11.0 release**. The first
edits of the session were made against a stale `CLAUDE.md` that had since grown
39 lines of hard-won rules on origin/main. Recovery was cheap because the work
was three small changes: `git checkout -- CLAUDE.md`, branch off freshly
fetched `origin/main` (the staged deletion and the untracked config file carry
across a checkout when they don't conflict), then re-apply the CLAUDE.md edit
against the new text. Had that edit been large it would have been a manual
merge.

This is precisely what the ritual's own closing paragraph warns about, and it
still bit on the very first run — the warning was in step 8 (fresh start),
which is where you branch, but the damage happens in step 0 when you start
editing. The check belongs at the top. **Fixed:** the divergence check now runs
in step 1 (preconditions) of the global skill, not only at the end.

**A generic skill can't discover a project's conventions.** `.claude/close-out.md`
was written from CLAUDE.md, so it inherited CLAUDE.md's blind spot: it never
mentioned `docs/sessions/index.md`, a hand-maintained Jekyll table that every
prior session updated. Running the skill surfaced the omission, and the config
gained the row (with the `.md` → `.html` link note). A per-project config is
only as good as the run that exercises it — the first real run is the test.

## Verification

- `shellcheck install.sh` — clean.
- `./install.sh` — `linked close-out`; symlink verified pointing at
  `~/workspace/claude-skills/skills/close-out`.
- `./install.sh` again — `ok close-out`, `0 linked, 0 skipped` (idempotent).
- The skill registered globally: `/close-out` appeared in the session's skill
  list immediately after install, and **this archive was produced by running
  it** from `~/.claude/skills/close-out`.
- Gates not run: no Rust, TypeScript or shell in this repo changed — the diff
  is markdown plus a skill directory removal.

## Follow-ups

- The skills repo has no remote. It is one `gh repo create --private` away;
  until then it exists on this machine only.
- Only `close-out` has moved. Other rituals worth globalising as they prove
  themselves (release, gate-runner) can follow the same shape.
- If a second machine ever needs these, revisit the plugin-marketplace option —
  that's the case it's actually better at.
