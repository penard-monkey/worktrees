---
title: "Session — hourly release check and the update bubble"
---

# Session — hourly release check, a dismissable update bubble, the gear's dot

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/update-notify`
- **Branches:** `update-notify` (the feature), `update-notify-close-out` (this
  archive)
- **PRs:** [#413](https://github.com/penard-monkey/worktrees/pull/413),
  squash-merged by the orchestrator in `(main)` as `e2fd7e3` after review
  ("merge after fixes": a CHANGELOG conflict with #411 and one wording nit,
  both fixed by a rebase and force-push with lease).
- **Release:** none. The entries sit under `[Unreleased]`.
- **Planning files:** only the handover `.planning/brief.md`, in
  `planning.tar.gz` here.

## Why

David asked for releases to be polled periodically (hourly), with two marks
when one is out: a dismissable "conversation bubble" at the bottom left, and a
blue dot on the Settings gear. He asked for a design pass first. Before this,
`check_update` ran once, 3s after launch, so an app left open for a week never
heard of a release made on day two.

## What shipped

- `app/src/updatepoll.ts` (new, import-free). `startUpdatePoll(check, env)`:
  - first check 3s after start, then hourly from when the last check finished;
  - a check that comes due while hidden is held, and runs as soon as the
    window is visible again;
  - on failure, retries after 5, 10, 20 and 40 minutes, then hourly;
  - only one check in flight; `stop()` leaves nothing armed.

  Also `browserPollEnv()`, and `bubbleTag()`: the bubble shows only for a
  real newer release, per tag, and never for a missing CLI.
- `app/src/UpdateBubble.tsx` (new). The bubble is fixed beside the gear, with
  its tail on the gear's centre, measured from the gear's rect and re-measured
  on rail resize. It follows the gear to the right when Places is mirrored. It
  has Update… (opens Settings → Updates), What's new ↗ (the GitHub release
  page) and ×. It is `role="status"`, takes no focus and has no `useEscape`.
  `onLift` reports its height so App can raise the error/undo float-stack
  above it.
- `app/src/App.tsx`:
  - `checkUpdate(keepLatest)`: the poll keeps the last tag on a failed check,
    while the manual check replaces it;
  - the poll effect gated on `update_auto_check`;
  - `gearEl`, `bubble`, `dismissBubble`, and `onSettingsCat`, which retires
    the bubble whenever Updates is shown, by any route;
  - the float-stack lift.
- `app/src/settings.ts`: `update_dismissed: string` (the tag).
- `app/src/SettingsSheet.tsx`:
  - `onCatShown`;
  - `cat` reset on close (see the gotchas);
  - `RELEASES_URL` exported;
  - "Check for updates automatically" plus an "Hourly while the window is
    open" hint.
- `app/src/App.css`: `.upd-bubble*`, with the tail as a rotated square in the
  bubble's own fill and borders.
- `app/src/mock/install.ts`:
  - the default is now UP TO DATE, so no harness session or README recording
    grows a bubble;
  - knobs `?latest=`, `?cli=x|none` and `?offline`, plus
    `__mock.release(tag|null)` to publish mid-session.
- `app/scripts/updatepoll-check.mjs` (in `make test-frontend`): evaluates the
  real module on a virtual clock and checks App's wiring.
- `app/scripts/update-bubble-browser.mjs`: an optional gate in Chromium and
  WebKit. Covers:
  - geometry on both sides at 900 and 1280px, reachability via
    `elementFromPoint`, and the float-stack lift;
  - dismissal flows;
  - the real poll on Playwright's fake clock;
  - six-theme contrast.

## Decisions

- **Two surfaces, and the update is not an Offer.** Offers are setup
  suggestions, fingerprinted on machine state and listed behind the sparkles
  button. A release is time-bound news. The dot is the durable mark and the
  bubble a one-shot announcement pointing at it. The dot is still the only
  mark on the gear, so "one mark per fact" (#384) holds.
- **The dot stays `--accent`**, the app's "pending" token. David said "blue",
  and it is blue in four themes, but Frost in nord and orange in gruvbox. A
  hard-coded blue would clash with those themes. The bubble's dot uses the
  same token, so the two read as one fact.
- **The bubble follows the gear rather than staying literally bottom-left.**
  With Places mirrored, a left bubble would point at nothing. This is flagged
  in the PR for David to confirm.
- **Actions are deep links, not the act.** Update… opens Settings → Updates,
  where the buttons and logs already live (the offers.ts rule). What's new
  opens the GitHub release page, because the compiled-in changelog does not
  have the new version yet.
- **Dismissal is per version** (`update_dismissed`). Showing Settings →
  Updates by any route counts as seen. Otherwise the bubble would come back
  when you close the sheet you just opened to read the same news.
- **No Escape.** The bubble is not modal, and `useEscape` would take Esc from
  the terminal while it is up.
- **Keep the last tag on a failed background check.** Otherwise an hour
  offline blinks the dot and bubble out and back. The manual check still
  replaces it, so Settings' "couldn't reach the feed" stays true.
- **The curl is fine hourly.** It is a HEAD of the releases/latest redirect,
  with no API and no rate-limit bucket.

## Dead ends / gotchas

- **The float-stack sits at the same corner.** Errors, undo and notices are
  pinned to `left: rail-w + s3; bottom: s3`, which overlaps where the bubble
  goes. It was found by reading the CSS, not by measuring the rail. The fix
  lifts the stack above the bubble while it shows (AGENTS.md: two fixed
  elements at the same coordinates).
- **The stale-category edge.** SettingsSheet resets `cat` in an effect when
  it opens. A sibling effect reporting `cat` ran in the same commit with LAST
  visit's value, so reopening at Appearance after an earlier Updates visit
  reported "Updates" and retired a brand-new bubble. `cat` now resets on
  close too. The browser gate's fake-clock case was shown red by reverting
  the fix, and so was the offline keep-last-tag case.
- **`--txt-dim` on `--bg-panel` is 3.6:1 in tokyo-day and 3.8:1 in nord.** It
  was the first cut for the bubble's second line and link; both now use
  `--txt`. The pre-existing gear dot is 2.4:1 on tokyo-day's rail, because
  `--accent` is low in that theme app-wide. Left as is, since a fix is a theme
  change. The gate asserts "exactly `--accent`" there and prints the ratio.
- **Another lane's vite took the harness port.** Port 1437 was free when this
  harness was killed for a restart, and guidance-editable's harness bound it
  in the gap. Grepping the served file for the edit (0 hits) is what caught
  it. Moved to 1471. Check `lsof … -sTCP:LISTEN`'s cwd, not just the port.
- **The red-first for the poller used a stub of the OLD behaviour** (one
  `setTimeout` at 3s) as the module under test: 17 failures. A missing file
  would only have shown that the check runs.
- **A CHANGELOG conflict after rebase.** #411 opened `### Added` under the
  same `[Unreleased]`. Kept one header and both sets of items, then counted
  the headers.

## Verification

- `updatepoll-check.mjs` passes on the real module and fails 17 cases on the
  old-behaviour stub.
- `update-bubble-browser.mjs` passes in Chromium and WebKit, in all six
  themes.
- Gates on the feature branch:
  - release build;
  - `make test` 447/447, 0 `not ok`;
  - `make lint`;
  - cargo test: core 648, cli 70, app 151;
  - `tsc`, `cargo check -p app`, `make test-frontend`.

  After the rebase onto #411, `test-frontend` (28/28) and `tsc` were re-run.
- CI on #413: all 9 jobs green on both OSes.
- **Never run in the real app.** Only the mock harness has been tried.

## Follow-ups

In ROADMAP:

- the real-app pass;
- the bubble's z-index against the hover-reveal nav overlay;
- caching the `cli_binary()` probe that now runs hourly;
- an optional Esc-when-focused dismiss;
- the tokyo-day accent contrast.
