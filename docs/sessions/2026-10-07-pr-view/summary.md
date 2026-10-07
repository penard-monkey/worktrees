---
title: "Pull requests view, phase 1 — 2026-10-07"
---

# Pull requests view, phase 1 — 2026-10-07

- **Date:** 2026-10-07
- **Worktree:** `.worktrees/pr-view` (place `pr-view`), a lane driven by `(main)` from `.planning/brief.md`
- **Branch:** `pr-view` → squash-merged as 81173a8
- **PR:** [#455](https://github.com/penard-monkey/worktrees/pull/455) — fable review: MERGE-AFTER-FIXES, then a clean re-review
- **Spec:** `docs/proposals/pull-requests.md` (§9 phase 1, §10 decisions), designed in #452
- **Planning files:** `planning.tar.gz` (task_plan.md only)

## What shipped

- **`crates/worktrees-core/src/github.rs`** (new):
  - `parse_remote` / `web_base`: the app's old `normalize_remote` moved here, with its test cases unchanged, and URL credentials are now stripped.
  - `resolve_from` / `resolve`: which repo the PRs live on, in `gh`'s own default-repo order — `remote.<n>.gh-resolved` (`base` or the older `owner/repo` value), then `upstream`, `github`, `origin`. The push owner is origin's.
  - `probe_auth`: `gh auth token -h`, with stdout sent to `/dev/null`.
  - `fetch`: ONE `gh api graphql` query covering open PRs, the last 20 merged/closed, and `viewer { login }`.
  - `parse`; `pr_for`, which maps a lane by its LOCAL branch + head owner, open first, never the upstream.
  - `chip_of` (one severity order); `view`, which joins places to PRs and computes attention.
  - `diag_line`.
  - `WORKTREES_GH_BIN` is the seam. Fixtures: `tests/fixtures/github/` (a real capture of this repo, plus not-found).
- **`app/src-tauri/src/lib.rs`**:
  - `project_prs(repo, max_age_secs, places)` (async), backed by a `PrCache` type:
    - Per-project answers, plus a lock per project that callers queue on.
    - An answer is dated when the fetch FINISHES, so a forced ask queued behind an in-flight fetch shares it.
    - An offline failure keeps the last good list, marked stale.
  - One app.log line per real fetch.
  - `diagnostics` gained a `gh` line, gated on the setting.
- **Frontend**:
  - `prs.ts`: types plus the `useProjectPrs` poller. It polls the selected project only, while the window is visible, every 120s. Focus refreshes at 30s, or immediately while `gh` is missing or signed out. Becoming visible also refreshes; opening the tab refreshes at 15s; ↻ forces a fetch.
  - `PrsPane.tsx`: four groups, a CtxMenu, and the empty states with Copy buttons.
  - `App.tsx`:
    - The header chip, hidden in `fit.tight`.
    - The `prs` DOCK_RAIL entry, filtered by `prsOn`.
    - The rail badge: dim open count, or a red attention count.
    - The remembered-tab fallback to Files.
  - `icons.tsx`: `GitPullRequest`.
  - `settings.ts`: `pull_requests` (default on), and `dock_tab` gains `prs` (both unions).
  - `navHistory.ts` and `usage.ts` (`dock.prs`).
  - Settings → Behavior switch.
  - Mock: `mock/prs.ts` with `?prs=ok|gh_missing|logged_out|no_host_token|not_found|error|offline|empty|slow|not_github`.

## Decisions

- **The backend maps and decides; the frontend draws.** `project_prs` takes the places' current branches, so a branch switch re-maps against the cache without a fetch, and no `*-check.mjs` mirror is needed. (`mock/prs.ts` re-joins branches only to imitate this; its chip states are written in as fixtures.)
- **Attention counts only PRs that are in a place.** On this repo 7 of 8 open PRs were CONFLICTING, mostly other people's stale branches, so counting every PR would make the badge permanently red. The reviewer agreed.
- **An Enterprise host with no `gh` token counts as not-GitHub (§2).** It cannot be told apart from GitLab without a host list. github.com with a login to another host only still gets the `gh auth login -h` line.
- **`viewer { login }` is in the query** instead of a second `gh auth status` call. It is free, and it names the account on NOT_FOUND.
- **The badge's red fill is 32% `--danger`, not 55%.** At 55%, catppuccin-latte measured 2.4:1. At 32% the six themes measure 3.6–9.5, the same band as the offers badge and `--accent`.

## Dead ends / gotchas

- **`gh api -F` is typed.** It turns `2048`/`null`/`true` into JSON non-strings (GraphQL rejects them for `String!`), expands `{owner}`, and reads a FILE for a value starting with `@`. Names must go with `-f`. The first seam test asserted only the reply, so it stayed green with `-F` and even with `--hostname` deleted. The fake `gh` now records its argv. Review caught this; the first cut shipped it.
- **Dating a conclusion when the fetch STARTS breaks coalescing.** A caller that queued mid-fetch asked AFTER the start, so it saw the answer as older than itself and fetched again. Date it at completion. The `_in`-style seam (`PrCache` as a type, not two statics) is what made it testable without an order-dependent global.
- **`gh auth status` is not local.** It validates tokens against the API (~0.5s), so diagnostics must respect the off switch.
- **Playwright version vs cached browsers.** `npm i playwright` pulled 1.64, whose browser revisions were not in `~/Library/Caches/ms-playwright`. Pinning `playwright@1.63.0` matched the cache, with no download.
- **Headless harness noise.** An xterm `Viewport.syncScrollArea … dimensions` page error appears on a plain place select + Files tab in headless Chromium/WebKit, with no PR UI involved. It is not this feature's.
- **(Re-review, open) The remembered-tab → Files fallback is a WRITE** via `updatePanels`. It freezes a `place_panels` entry for a place never opened, and resets the global `dock_tab` seed. It also fires with the dock closed. That is the "`place_panels` seed" trap AGENTS.md already describes. See ROADMAP.

## Verification

- **Gates**, run twice with the release build first, both times all green: bats 478/478, lint, core 722, cli 70, app --lib 161 → 162, tsc, cargo check, every `app/scripts/*-check.mjs`. CI green on #455.
- **Failing first:** each new core test went red against a mutation of the rule it guards: dropping the owner match, attention without a place, recent before open, `-F`, and no `--hostname`. The coalescing test went red with the start-of-fetch date.
- **Live probe** (a scratch crate on core, real gh 2.101.0, this repo):
  - Auth 60–220ms; fetch 1.27–1.42s; 8 open / 20 recent.
  - `codex-skills-mcps` → #350 and `roadmap-and-github-issues` → #296.
  - Under `env -i` with only system PATH plus gh's absolute path, keyring auth works.
  - With gh off PATH it reports `gh_missing`.
- **Harness** (headless Chromium + WebKit):
  - The chip's parts measure the same in both engines.
  - Every group, chip state and auth state was rendered.
  - Contrast was measured in all six themes, with translucent fills composited.
  - The non-GitHub fallback was driven via `?prs=not_github`.
- **NOT done: `sandbox.sh --app`** against this repo, including a Finder launch. It needs a person; David will do it before v0.40.0.

## Follow-ups

In ROADMAP: six items — the four left from the first review plus two from the re-review. Not done from the proposal:
- The +20s refetch after an app push.
- The optional `offers.ts` "install gh" suggestion.
- Phase 2.
