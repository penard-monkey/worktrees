---
title: "2026-10-07 — pull requests in the app (proposal)"
---

# Pull requests in the app — design only

- **Date:** 2026-10-07
- **Worktree / branch:** `pr-panel`
- **PR:** #452, squash-merged as `99dacc4` (docs-only)
- **Planning files:** none; the brief is `.planning/brief.md` (`planning.tar.gz`)
- **Scratch:** the captured GraphQL response (`gql.json`) is in `~/.cache/worktrees/worktrees/pr-panel/`

## What shipped

`docs/proposals/pull-requests.md` plus its row in `docs/proposals/index.md`.
No code. The proposal covers:

- a project's open PRs, seen from `(main)`;
- each lane's own PR;
- David's answers, in §10.

## Decisions

- **Data: one `gh api graphql` call per project.** It returns the open PRs,
  the last 20 merged or closed, and CI as a single `statusCheckRollup.state`.
  It does not use `gh pr list`, and the app holds no token of its own. `gh`
  already owns keyring, multi-account and Enterprise auth, and shelling out
  is the house rule.
- **Code split.** The parse, fork resolution, query and lane↔PR mapping go in
  `worktrees-core`. The cache and polling stay in the app, which polls the
  selected project only, every 120s while the window is visible. None of it
  goes in `snapshot()`, which must stay offline-fast.
- **UI.**
  - A PR chip in the lane header, with the state as a dot.
  - A sixth dock tab, "Pull requests". It shows the whole project list, with
    the lane's own PR pinned on top.
  - Attention goes on the rail icon's count badge (red for PRs that need
    you), not as a second dot in the nav row; the nav is already noisy.
  - Rejected: a sheet, a nav section, a Home card, and an in-app checks view.
- **Mapping uses the place's LOCAL branch, never its upstream.** This lane
  tracks `origin/main`; matching on the upstream would claim every PR aimed
  at main.
- **A merged PR stays on its lane until the place is removed.** Squash-merge
  hides the merge from git (`health.rs` `maybe_merged` is a guess), so the PR
  is the real signal.
- **Settings and detection.**
  - One global on/off switch. Off makes no `gh` calls at all.
  - Projects whose remote is not GitHub show nothing, so there is no
    per-project switch.
  - `gh` is detected at startup, and again on focus while it is missing. The
    tab shows `brew install gh` and `gh auth login` to copy; the app never
    runs them.
- **Read-only toward GitHub in every phase. The app never creates PRs.**
  Phase 2 (MCP `pr` field, merged → remove, open-in-a-place) is deferred.

## Dead ends / gotchas

- **`gh pr list --json statusCheckRollup` fetches every check of every PR.**
  That costs 1.2–1.9s, against 0.7s without it. One GraphQL query asking for
  the rollup `state` returns open and recent PRs in about 1.2s, at 1
  rate-limit point.
- **An exit code read through a pipe is the pipe's.** The first capture of
  "unreachable host" said exit 0, but that was `head`'s status. Measured
  again without the pipe, it is exit 1, and the doc was corrected before the
  PR. A GraphQL `NOT_FOUND` also exits 1, so classify by stderr or the
  `errors` array, not by the exit code.
- **Not logged in is exit 4.** `gh auth token -h <host>` is a 50ms offline
  probe that tells "not logged in" from "wrong host" before any network
  call.
- **Mid-session section renumbering broke a peer's references.** Adding §7
  moved Decisions to §10, so a `(main)` request that said "§9" had to be
  mapped by hand. Name sections in messages, not just numbers.

## Verification

- Every timing and auth state in §1 was measured against the real
  `gh` 2.101.0 on this repo. The failure states were reproduced in a scratch
  repo with an empty `GH_CONFIG_DIR`.
- The lane↔PR table in §3 is real `worktrees ls --json` output joined with
  the query.
- Nothing was built, and no harness prototype or screenshots were made.

## Follow-ups

- Phase 1 build (ROADMAP entry). David may want it in a fresh lane.
- Optional per-row hollow-ring nav mark: look at it in the harness first.
