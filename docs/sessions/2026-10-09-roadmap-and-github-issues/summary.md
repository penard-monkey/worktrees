# Session: the roadmap does not become GitHub issues

- **Date:** 2026-09-18 (filed) → 2026-10-09 (rejected and unwound)
- **Worktree:** `.worktrees/roadmap-and-github-issues`
- **Branch:** `roadmap-and-github-issues` → PR [#296](https://github.com/penard-monkey/worktrees/pull/296), **closed unmerged**; this archive lands from `roadmap-and-github-issues-close-out`, branched off `origin/main`
- **Release:** none
- **Planning files:** none — this tree never had `task_plan.md` / `findings.md` / `progress.md` or a `.planning/brief.md`, so there is no `planning.tar.gz`
- **Outcome:** nothing shipped. The experiment is fully reverted and the decision is recorded.

## Why it was tried

brethash started contributing. `ROADMAP.md` was a flat parking lot of 115
top-level bullets with no headings, each one a paragraph of prose ending in a
`_From: <session>_` attribution — readable start to finish by whoever wrote it,
unpickable by anyone else. The idea was to turn the actionable part into issues
so an outside contributor could take one.

Three decisions set the shape, all the user's:

1. A **curated actionable set** (~40 intended, 68 filed), not all 115 and not
   only the dozen that were obviously contributor-ready.
2. **Issues become the source of truth** — the prose moves out of the file and
   `ROADMAP.md` becomes an index over them. This was known at the time to
   require rewriting the close-out ritual's step 6, and that rewrite was part
   of the PR.
3. **Create area + type labels** beyond GitHub's nine defaults.

## What was built

PR #296, one commit (`2abd39b`), `+305/-1183` across three files:

- **`ROADMAP.md`** rewritten from 1258 lines to 343: a "how to use it" preamble,
  six tables (CI/gates/harness, core engine, app correctness and cost, app
  interface, distribution/CLI/docs, verification debt) linking all 68 issues,
  then a `# Not issues` section keeping the **full prose** of everything that
  was deliberately not filed.
- **`.claude/close-out.md`** — the `roadmap` row changed to point at issues, and
  a new `## Roadmap → issues (replaces the skill's step 6)` section with the
  triage rule, the labels, and a step the append-only file never had: close
  what the session finished.
- **`CLAUDE.md`** — three `ROADMAP` references repointed at issue numbers.

Off-PR: 68 issues `#228`–`#295` and seven labels (`area:ci`, `area:core`,
`area:app`, `area:cli`, `area:docs`, `needs-real-mac`, `tech-debt`).

### Triage judgements worth keeping

- **~25 items were manual verification passes needing the real app on a Mac** —
  no fake `claude`, no fake tmux, the harness is Chrome and the app is
  WKWebView, Playwright speaks CDP and the sandbox is a native window. Filing
  25 issues that each say "open the app and look" is noise, so they became two
  checkbox trackers (`#294` real-app debt, 18 boxes; `#295` the
  `docs/ai-profiles-manual-checks.md` §10/§11 debt).
- **Declined decisions were NOT filed.** A glob `path` in `[[file]]`, keychain
  GC, the Mac App Store, `doctor --strict` — an open issue invites exactly the
  work those entries exist to prevent.
- Personal chores (cdv migration, stale branch cleanup, `~/bin/sync-macs`) and
  watch-and-decide notes stayed in the file for the same reason.

### The one real trap

**GitHub shares a single number sequence between issues and pull requests**, so
`#N` written in an issue body resolves against the whole repo. Nine draft bodies
cross-referenced other drafts by draft-local number; `#49` would have linked to
an existing PR about the busy dot, not to the 49th draft. Caught by grepping
every `#[0-9]+` across the drafts before filing and classifying each one. Fix
was a two-pass file: `{{NN}}` placeholder tokens in the bodies, resolved by a
second `gh issue edit` pass once the real numbers existed, then verified with
`gh issue list --json body -q 'select(.body|test("\\{\\{"))'` returning nothing.
Any future bulk-filing hits this.

## Why it was rejected

The PR sat for three weeks. In that window:

- `ROADMAP.md` on `main` was groomed normally by the close-out ritual and grew
  **115 → 235 bullets** across ~160 commits. The file was in active use the
  whole time; the issue list was a snapshot of its 2026-09-18 state.
- **Nobody picked up an issue.** None was assigned, none was referenced by a PR,
  none was closed — with a contributor active on the repo for the whole period.

2026-10-09: *"So I think we can close out 296. We're not going to use github
issues for the roadmap."*

The reason the experiment failed is not that the issues were badly written. It
is that the roadmap's value is the dense prose explaining **why the obvious fix
is wrong**, and that prose is written and re-read during close-out, in one file,
by whoever is closing out. Splitting it across 68 issues created a second place
to groom and bought nothing. The read path never moved, so the write path
shouldn't have either.

## What was unwound

| | |
| --- | --- |
| PR #296 | closed unmerged, with a comment recording the reasoning |
| Issues #228–#295 | **deleted** (all 68) — `gh issue list --state all` returns 0 |
| The seven labels | **deleted** — `gh label list` is back to GitHub's nine defaults |
| `ROADMAP.md`, `CLAUDE.md`, `.claude/close-out.md` | untouched on `main`; the rewrite exists only on the abandoned `roadmap-and-github-issues` branch |

Deleting was chosen over closing: 68 closed issues are 68 rows of search noise
for a decision that is now written down in one place.

**Nothing was lost.** Before closing, `main`'s `ROADMAP.md` was checked against a
sample of the filed issues (`#228`, `#229`, `#248`, `#251`, `#254`, `#262`,
`#271`, `#283`) — all eight were still present in the file. The issues were a
duplicate representation of live work, not the only copy of anything.

Issue deletion cannot be undone, so every body was archived first:

```
~/.cache/worktrees/worktrees/roadmap-and-github-issues/deleted-issues-228-295.tar.gz
```

68 `.md` files, one per issue, each `# <title>` / `_labels: …_` / body. Per the
repo's scratch rule this lives in the cache dir, never in the repository — it is
insurance against a mistake, not an artifact worth committing. Note it is on
this machine only.

## Consequences

- **The numbers #228–#295 are permanently spent.** GitHub does not reuse them,
  so the shared issue/PR sequence has a 68-wide hole and PR numbers jumped
  accordingly. Any old note referring to an issue in that range is referring to
  something that no longer exists.
- `ROADMAP.md` stays a flat file groomed by `/close-out`, per the unchanged
  `roadmap` row in `.claude/close-out.md`.
- **Do not re-propose this.** If onboarding a contributor comes up again, the
  answer is something other than mirroring the roadmap into a tracker — point
  them at the file.

## Gates

Not run, deliberately. This branch is `origin/main` plus one docs-only commit
(`docs/sessions/**`), so the suite would be testing `main`, not a change. The
PR is docs-only and `ci.yml`'s `paths-ignore` covers `docs/**`, so zero checks
on it is the expected state.
