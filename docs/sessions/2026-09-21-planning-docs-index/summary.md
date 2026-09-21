---
title: "The plan was in the one directory the walk could not enter"
---

# The plan was in the one directory the walk could not enter

- **Date:** 2026-09-20 → 2026-09-21
- **Working tree:** `.worktrees/planning-files-in-docs`
- **Branches:** `planning-files-in-docs-working-memory` (the change),
  `planning-files-in-docs-close-out` (this archive)
- **PRs:** [#318](https://github.com/penard-monkey/worktrees/pull/318),
  squash-merged as `60a8b2d`
- **Release tag:** none — sits in `[Unreleased]`, after v0.27.0
- **Planning files:** `planning.tar.gz` beside this summary
- **Evidence:** gate logs and probe output are outside the repo at
  `~/.cache/worktrees/worktrees/planning-files-in-docs/`. Nothing was left in
  the repo root; there were no screenshots this session.

The report was one sentence: *some projects put the planning files in
`.planning/` — it happened on valleos — and I can't see those files in the docs
viewer, and I should.* The reporter could not say why it happened on some
places and not others.

It did not happen on some places and not others. It never worked.

---

## What shipped

`docs::walk` (`crates/worktrees-core/src/docs.rs`) gains **step 4**: the
place's working memory, `.planning/**/*.md`, listed between the root-file block
and the repo's own `docs/` tree, grouped by directory.

| file | what changed |
| --- | --- |
| `crates/worktrees-core/src/docs.rs` | step 4; `subtree()` extracted from the inline tree loop and shared; the `.planning` lstat gate; the declared-path skip; 6 new tests |
| `crates/worktrees-core/src/ops.rs` | `PLANNING_DIR` beside `BRIEF_PATH` |
| `app/src-tauri/src/viewer.rs` | one test — a plan document derives into the browser tree with its `../../` links intact |
| `app/scripts/docs-check.mjs` | §3's fixture now carries a real `.planning/<slug>` group |
| `app/src/mock/install.ts` | five planning rows in the mock index |
| `app/src/doctree.ts`, `app/viewer/{contract.ts,IndexView.tsx}`, `app/scripts/viewer-boundary-check.mjs` | comments that reasoned from the removed fact |
| `docs/proposals/place-docs.md` | §18, and an amendment to §11.1 |

Measured on the repo it was reported against:

| place | planning rows listed, before | after |
| --- | --- | --- |
| `valleos/.worktrees/ssdlc` | 1 of 9 | 9, in 3 workstream groups |
| `valleos/.worktrees/docs` | 1 of 19 | 19 |
| `valleos` (empty `.planning/`) | — | unchanged |

**Neither frontend needed a change**, and that is the interesting part rather
than a convenience: both surfaces nest on the backend's `group` and have no
opinion about dots, both path validators already allowed a leading-dot name
(`.planning/brief.md` was in both of their tests), and `fingerprint_with`
shares `walk` with `index_with`, so the recency machinery picked the new step
up for free. A change that reaches this many surfaces through one function is
the seam the proposal's §6 was cut for, working.

---

## Decisions

**Step 4 sits between the root block and `docs/`.** The brief says what this
worktree is *for*; the plan says how it is going; the repo's committed
documentation follows both. It is the most current material a place has and the
least committed.

**The plan rows are grouped; the brief stays ungrouped.** §11.1 gave three
reasons for pinning the brief with the root files. Two survive — it is the most
place-specific document that can exist, and it is tool-owned, a constant this
module can name without guessing. The third, *"a group of one under a dotted
directory name reads as an accident"*, was **retired**: a loose
`.planning/review-pr7.md` is exactly that, and it is grouped. The amendment is
written into §11.1 so the two sections do not contradict each other.

**Convention, not configuration — and the first draft got the reason wrong.**
See the dead ends.

**`subtree()` extracted rather than copied.** A second tree walk cut for
`.planning` would have been a mirror of a rule living six lines away. The
module's own note is about exactly that failure, so writing a fresh one
underneath it was not an option.

**The general symlink hole is recorded, not fixed.** See below.

---

## Dead ends and gotchas

### The test asserted the bug away, and it took an adversarial reviewer to see it

`a_symlinked_planning_directory_is_walked_as_nothing` was written to pin the
new guard, and it passed. It passed because the link target happened to contain
no `brief.md`.

`symlink_metadata` refuses to follow only the **final** component of a path. So
step 2's `is_regular_file(".planning/brief.md")` resolves a
`.planning -> elsewhere` on its way past and reads a brief from outside the
place — while step 4's walker, which lstats its own start, correctly refuses
the same link. The result *looks* guarded: the plan rows are absent, exactly as
a working guard would leave them, and the one row that leaked is the row that
was always there.

That split is worse than either answer on its own. Both steps now gate on one
lstat of `.planning`, and the test writes a `brief.md` behind the link and
asserts its absence.

The general rule this is an instance of: **if a test pokes or omits state to
reach the thing it is asserting, ask what the omission is standing in for.**
This repo already has that lesson written down about a recovery test that
hand-cleared its own blocker; this is the same shape wearing a symlink.

### Five confident sentences, all false

The branch said, in three places in `docs.rs`, in the proposal and in the
CHANGELOG, that *a repo could not declare `.planning` in `[docs] paths` — it is
gitignored, so the repo does not know it is there*.

`.planning` is a perfectly legal `RelPath`. `parse` refuses `.`, `..`, `.git`
and `.worktrees` as components; nothing touches a leading dot. The
declared-paths loop stats its start directly, and the dotted-name refusal
applies only on the way **down**, to entries inside a tree already being
walked. `paths = [".planning"]` listed the plan sets before any of this work
existed — measured on `origin/main`, not reasoned about.

The honest reasons are that `paths` **replaces** the documentation tree (so a
repo declaring its working memory trades `docs/` away to get it), that the
directory is the tool's own, and that it is gitignored — which makes it the one
directory a committed config has no business having an opinion about, not a
directory a config is unable to name.

Worth noticing *how* it got written: the true fact ("the walk cannot descend
into a dotted name") and the false one ("so nothing can reach it") sit one
inference apart, and the false one is the more satisfying sentence. Five sites
is what happens when a satisfying sentence is written once and then propagated
for consistency.

### `paths = [".Planning"]` listed everything twice

Step 4 always walks `.planning`, and the walk's dedupe is on `rel` **as
spelled**. On APFS a case-variant declaration opens the same directory and
emits every row again under a second group name, which `write_tree` then
collapses to one file. `check_docs` refuses case-only duplicates *within* the
declared list and cannot see a collision with a tree that is not in the list.

A declared path folding to `.planning` is now skipped in step 5 — step 4 has
already walked it. The test asserts `.planning`, `.Planning` and `.PLANNING`,
because the exact spelling always deduped: a fix keyed on it would have passed
half the test and left the real shape open.

### A conflicted PR gets no CI, and nothing says so

The review fixes were pushed and got **no run at all**. `gh run list` came back
empty for the new commit while the old commit's run sat there green. GitHub
cannot build `refs/pull/N/merge` while the branch conflicts, so a `DIRTY` PR
simply has no checks — and #319 and #320 had landed on main while the review
was running. CLAUDE.md records this trap; it still cost a confused minute,
because "no run" reads as a queue problem, not a conflict.
`gh pr view <n> --json mergeStateStatus` is the question to ask.

### Two `### Fixed` blocks under one `[Unreleased]`

The rebase conflicted in `CHANGELOG.md` because #319 had opened its own
`## [Unreleased]`. Resolving it into one section left **two `### Fixed`
headers** — each side's, stacked. Nothing complains about that: the file
renders, the rebase exits 0, and the malformation is three lines below where
the eye stops. Found by counting headers rather than by reading the resolution.

This repo already carries a ROADMAP item about guarding against a second
`## [Unreleased]` header. The duplicate *subsection* is the same hazard one
level down, and neither is caught by anything but looking.

### `gh pr merge` printed nothing at all

Expected from a side worktree — it dies on its local checkout step *after* the
merge has landed. This time it printed no error either. Verified against
GitHub (`state: MERGED`, `mergeCommit: 60a8b2d`) rather than against an exit
code, which is the only thing that works here.

---

## What is deliberately not fixed

**An intermediate directory symlink is still resolved for `[docs]` paths.**
`resolve_rel`'s per-component `read_dir` follows a directory link, and
`root.join(rel)` + lstat resolves intermediates — so a `[docs] index` or a
declared `docs/api` resolves through a `docs -> ../shared-docs` and lists files
from outside the place.

It pre-dates this work, and it is a behaviour **decision** rather than a patch:
a repo symlinking its docs directory into a monorepo sibling is a reasonable
thing to do, and refusing it silently drops that repo's whole tree. The
observable symptom today is also milder than the security framing suggests —
`docserver::safe_under` canonicalises and already `403`s exactly those rows, so
the bug the user meets is *a row that will not open*, and the index is the half
that is wrong.

ROADMAP carries it with the mechanism and a sketch of the fix (resolve each
component with lstat; allow a link whose target canonicalises back under the
place root — which is `safe_under`'s own rule, one layer earlier). The module
note says which entry is shut rather than claiming the class is.

`ops::write_brief` also still writes through a `.planning` symlink at
`cmd_new`. That is creation, not listing, and is noted in the same item.

---

## Verification

- **Each of the six new tests shown RED first**, which is this repo's rule and
  earned its keep twice here: four by disabling step 4, the symlink one by
  removing the step-2 gate (it fails on the brief behind the link), the
  declared-path one by removing the skip (it fails on `.Planning`, not on
  `.planning`), and the viewer's by making `safe_rel` refuse a dotted
  component. `docs-check.mjs` §3 was shown red against a path-derived `tree()`,
  failing with both of its messages.
- **The real binary against the real repo.** A throwaway `cargo --example`
  (deleted afterwards) ran `docs::index` over `valleos/.worktrees/ssdlc`,
  `.worktrees/docs` and `valleos` itself. The numbers in the table above are
  from that, not from fixtures.
- **All three review findings reproduced before being fixed**, on constructed
  fixtures, and re-measured after.
- **Gates, run twice** — once before the PR and again after the rebase:
  `make test` 340/340 (plan line checked, `not ok` counted), `make lint` clean,
  `worktrees-core` 394, `worktrees-cli` 11, `app --lib` 136, `tsc --noEmit`
  clean, all 12 `app/scripts/*-check.mjs` green.
- **CI green on both OSes**, 9 jobs, on the rebased head `58c2b46`.
- **Review by fable**, two passes: findings, then a confirmation pass on the
  fixes. Verdict MERGE, with three accuracy sharpenings that were applied
  (the ROADMAP item's real symptom, the `write_brief` write side, and an
  over-narrow "refuses only …" in §18.1).

---

## Follow-ups

- The intermediate-symlink item above, in ROADMAP.
- The docs viewer has still never been opened in a real browser from a real
  app — the existing ROADMAP item, untouched by this session and now with one
  more thing worth looking at in it (a `.planning/<slug>` group in the nav).
