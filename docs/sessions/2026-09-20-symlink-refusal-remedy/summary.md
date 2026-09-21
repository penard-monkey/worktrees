---
title: "The refusal that told you to do two impossible things"
---

# The refusal that told you to do two impossible things

- **Date:** 2026-09-20
- **Working tree:** `.worktrees/bug-when-making-work-worktree`
- **Branches:** `bug-when-making-work-worktree-symlink-message` (fix),
  `bug-when-making-work-worktree-close-out` (this archive)
- **PRs:** [#319](https://github.com/penard-monkey/worktrees/pull/319) here;
  [penard-monkey/saywhat#68](https://github.com/penard-monkey/saywhat/pull/68)
  downstream, squash-merged as `18885ce`
- **Release tag:** none — sits in `[Unreleased]`, after v0.27.0
- **Planning files:** none. It began as a screenshot and a question and stayed
  small enough to hold in the thread, so there is no `planning.tar.gz` here.
- **Evidence:** the reporting screenshot is
  `_tmp/Screenshot 2026-09-20 at 21.12.36.png`; gate logs are outside the repo
  at `~/.cache/worktrees/worktrees/bug-when-making-work-worktree/`.

The session started with a screenshot of a **saywhat** worktree being created,
a red line in the middle of it, and a question: *what happened?* Plus a second,
quieter one — *I thought I had already started this worktree but I guess I did
it somewhere else incorrectly.*

Both answers were "nothing is broken", and the interesting part is why it did
not look that way.

## What shipped

**The B3 refusal now names a remedy that exists**
(`crates/worktrees-core/src/materialize.rs:481`). Check B3 refuses a declared
file whose source is itself a symlink resolving outside the checkout — the
hostile `.env -> ~/.ssh/id_rsa` case. The rule is right and is unchanged, along
with its `Severity::Error` and `doctor`'s exit 2. What changed is that the
message used to say:

> refusing (link the real file, or point the config at it)

Both are impossible in the only case that reaches the check. `[[file]].path` is
repo-relative — `init.rs` refuses absolute paths, `~` and `$` — so it cannot be
pointed at a target outside the repo; and the real file is outside the repo by
definition, or B3 would not have fired. The new text states the consequence the
old one left you to guess (the worktree is otherwise complete) and names the way
out: commit the link instead of declaring it.

**The asymmetry is documented at the check rather than plugged.** A *tracked*
symlink reaches a worktree without passing B3 at all: `git worktree add` checks
out a `120000` blob with no Layer B check. That is not a hole — the checkout is
git's, and a hostile repo's tracked symlink is already in your main checkout
before we run — it is the remedy, and this repo already relies on it for its own
`_tmp`.

**Downstream:** saywhat stopped declaring `_tmp` and now tracks it
(saywhat#68). Its `worktrees doctor` went from eight errors to clean.

## Decisions

**Fix the message, not the rule.** Three options were on the table: fix only the
downstream config, relax B3, or fix the message. Relaxing B3 was rejected —
it deliberately reopens the credential case, and the asymmetry above is an
argument that the rule does less than it looks like it does, not that it should
do less still.

**`git add -f`, keeping the path gitignored, rather than un-ignoring it.** Both
make git track the symlink. Tracked-*and*-ignored means branches that predate
the change see no `git status` noise and adopt the link on their next rebase.
The plainly-tracked alternative (what this repo does for its own `_tmp`) would
have shown `_tmp` as untracked in seven saywhat worktrees until each rebased.

**The downstream fix went to saywhat's own Claude session, not done from here.**
`sw-(main)` is that project's orchestrator; it re-derived every claim before
changing anything and opened the PR. The brief had to be fully self-contained —
that session had no context on any of this.

## Dead ends / gotchas

**A refusal printed mid-create reads as a failed create.** The worktree had in
fact succeeded — branch made, slot 8 taken, `.worktree.env` written, tmux
session opened — and exactly one declared file was skipped. Everything after the
red line in that toast is success output. This is most of why the report existed
at all, and it is why the new message leads with the consequence.

**"I thought I already started this" was true, and the UI was right.** The place
had been created 18 minutes before the screenshot. Two things made it look
older: the toast still on screen was that create's log rather than a new one,
and the nav's `COMMIT` column shows the age of the last *commit* — a fresh
worktree branched off a day-old `main` always reads "1d" there.

**The entry had never worked once, and four worktrees disguised it.** Four of
saywhat's eight places had a working `_tmp`, which made the config look
functional. Their link mtimes were scattered across three days — they had been
symlinked by hand. The declaration landed 2026-09-14 12:08 and B3 predates it by
seven weeks (`eb32f7f`, 2026-07-28), so there was never a window in which it
worked. **A feature that is manually patched where it fails looks like a feature
that works.**

**Checking that the advice WORKS is a different test from checking the wording.**
The defect was the advice, so the fix was driven end-to-end through the real
release binary in a scratch repo: reproduce the refusal, follow its own
instructions (`git add -f`, drop the entry), create another worktree, confirm the
link is there and `doctor` is clean. A unit test asserting the string would have
passed against advice that was still wrong.

**A fresh worktree needs both bootstraps, and the first failure names the wrong
thing.** `make test` died with `./test/lib/bats-core/bin/bats: No such file or
directory` — the bats submodule, not a test failure. `app/node_modules` was
likewise absent, so `tsc --noEmit` could not run until `pnpm install` under Node
22.13. Both are in CLAUDE.md; both still cost a cycle.

**An `app --lib` failure that is not yours.** `proc_cwd_follows_a_live_shell_into_a_new_directory`
(`lib.rs:7780`) failed once and then passed in isolation and three full runs —
a real pty spawn, order-dependent under the parallel runner. Same family as the
`viewer::ISSUED` note in CLAUDE.md. Left alone; see Follow-ups.

**A tracked symlink is branch-dependent, which the hand-made one was not.**
Review surfaced this and it is the one behaviour change worth knowing: checking
out a branch that predates the tracking **removes** the link from that tree —
git deletes a tracked path absent from the target commit, and ignore rules do
not protect it. None of saywhat's other branches carry the blob, *including
every `-next` idle base*, so parking a place on its base drops `_tmp` there
until the base is rebased. Self-healing, and it was observed live while
returning saywhat's main checkout to `main`: the link vanished between the
`checkout` and the `pull`.

## Verification

- B3 test gains an assertion that the remedy is named, **shown red against the
  old wording first**, per the repo's rule for new tests.
- End-to-end through the release binary in a guarded scratch repo (every `git`
  call routed through a helper that hard-exits outside `$TMP`): refusal renders
  on `new`; applying its own advice produces a working link and `doctor` clean,
  exit 0.
- Gates: `make test` 340/340 rc 0 with no `Executed N instead of` warning,
  `make lint` rc 0, `worktrees-core` 388, `worktrees-cli` 11, `app --lib` 135,
  `tsc --noEmit` clean, `cargo check -p app` clean. Release binary confirmed
  newer than the edited source before trusting bats.
- Downstream verified independently rather than from the peer's report: blob
  `120000 885d08e3…`, `doctor` run from *inside* a worktree so the result does
  not depend on which branch the main checkout sits on, and all eight worktrees
  resolving `_tmp` after the merge.

## Follow-ups

- **saywhat's config comment tells only the forward half.** It says branches
  adopt the link on their next rebase; it should also say that checking out a
  pre-change branch removes it until then. One sentence, downstream.
- Two nits in that same comment, neither worth a PR alone: it cites
  "`.gitignore` line 22", which rots on the first insertion above it; and it
  implies B3 fires first when B8 (`materialize.rs:415`) runs before B3 (`:475`),
  so a re-added entry would trip the gitignore check first.
- `proc_cwd_follows_a_live_shell_into_a_new_directory` is order-dependent under
  the parallel runner. Not investigated here.
