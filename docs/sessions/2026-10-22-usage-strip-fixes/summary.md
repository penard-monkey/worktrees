# Usage strip: a false "unavailable", logos beside the names

- **Date:** 2026-10-22
- **Place:** `usage-strip-fixes` (a lane briefed by `(main)`)
- **Branch:** `usage-strip-fixes`
- **PR:** [#542](https://github.com/penard-monkey/worktrees/pull/542), squash-merged as `4dd8c77`
- **Release:** none (under `[Unreleased]`)
- **Planning files:** none (only `.planning/brief.md`, which stays with the place)

## What shipped

- **A fresh Codex/kimi reading is never "unavailable"** (`app/src/planUsage.ts`,
  `app/src/App.tsx`). The readers stamp `fetched_at` when they FINISH, and
  `useUsage`'s `nowSec` moved only on its focus tick and a 15s interval. A reply
  that landed after the focus tick therefore sat a few seconds "in the future",
  and `viewUsage` took `age < 0` as a bogus stamp: `unavailable / expired`
  until the next tick. Claude is not an AGED provider, so it never showed this.
  The fix has two parts:
  - `FUTURE_SLACK_SECS = 60`: a stamp up to that far ahead reads as fresh.
  - `useUsage` re-reads the clock whenever a provider's reading changes, so the
    slack only has real skew to cover.
- **Harness logos in the strip and in the panel headings** (`UsageMark` in
  App.tsx, which reuses `HARNESS_MARK` and the `.g-agent-*` sizes). The marks are
  `aria-hidden`, and the trigger's aria-label is unchanged. The 2px `|` rule
  between providers is now plain spacing. At ≤480px the logo replaces the name
  (`.usage-provider-text`).
- **The rail tile's bare `▥` is now `Icons.ChartColumn`** (Lucide
  `chart-column`). Its bars go `--warn` when any strip window is warning or
  over, and the axis stays neutral (`tileSeverity`, `.usage-tile-icon`).
- **Checks:**
  - `plan-usage-check.mjs` now covers a reading 5s ahead and at the slack edge,
    for codex and kimi. It also slices `USAGE_TICK_MS` out of App.tsx instead
    of using a literal 15000.
  - `plan-usage-browser.mjs` asserts a sized, visible logo per provider, and
    names hidden only at the narrow width. That replaces the removed divider's
    assertion.

## Decisions

- **A slack in `viewUsage`, plus a re-tick, rather than only one of them.**
  The slack is the smallest pure fix and the place the test reaches. The
  re-tick came from the review: without it the slack covers the race as well
  as skew. `fetched_at` is the ENGINE's clock, so a remote serve skewed by more
  than 60s still reads "unavailable". The comment says so, so that nobody
  tightens the slack later.
- **`HARNESS_MARK`, not `AgentMark`.** `AgentMark`'s label says
  "<harness> session live", which is false in a usage meter.
- **The tile's bars stop at amber, even for an over-limit window.** Red on the
  tile already means the service-status `.usage-badge`, and the status block
  comment in App.tsx already reasoned that two colours 4px apart cannot carry
  two meanings. Whether a window is near its limit or over it is for the
  panel to say.
- **The light-theme amber is left as is.** The bars' `--warn` measures
  2.3–2.9:1 in tokyo-day and catppuccin-latte. That is the same fill the
  strip's own bars use, and the axis keeps the icon legible (≥5.5:1). Fixing
  only the tile would make it inconsistent with the rest of the app (AGENTS.md:
  check against the app's own tokens first).

## Dead ends / gotchas

- **app.log has no usage lines, by design.** `codex_usage`/`kimi_usage` log
  their failure reasons through core's logsink, which is serve.log, the
  ENGINE's log. app.log is the shim's. serve.log had no usage failures at the
  time of the screenshots, which confirmed the issue was in the frontend.
- **The first red was red for the wrong reason.** The test referenced
  `FUTURE_SLACK_SECS` before it existed, so `99 - undefined` gave `NaN`, and
  `NaN < 0` is false, so the case read "ready". Exporting the constant first,
  with `viewUsage` unchanged, gave the honest red
  (`actual: 'unavailable', expected: 'ready'`).
- **`plan-usage-browser.mjs` was already red on main, and not because of the
  divider.** It predates kimi, and the mock enables kimi by default, so its
  window list gained three Kimi rows. Nothing in CI runs it, so nobody saw it.
  An init script now defaults `usage_kimi` to false unless a step sets it.
- **The cached webkit-2215 browser was half-downloaded and locked.**
  `playwright install` hung on another process's `__dirlock`. Pinning
  Playwright 1.59.1, whose WebKit revision (2272) was already complete in
  `~/Library/Caches/ms-playwright`, avoided the download.
- **This lane's environment sets `WORKTREES_VIEWER_JS`.** Two tests refuse to
  run under it: `open_docs_viewer_without_a_bundle_is_a_clear_refusal` (cli)
  and `a_build_with_no_viewer_bundle_refuses_the_open` (api). They failed the
  gate with a message that names the cause. They pass under
  `env -u WORKTREES_VIEWER_JS`.

## Verification

- Mock harness (`dev:mock`, ports 1437 and 1438), headless Playwright WebKit
  1.59.1, and Chromium for comparison:
  - logos 10.8–12.2px at every width, and WebKit sizes the labels in the
    button correctly;
  - at 460px the names are hidden and the logos keep their size;
  - name and logo contrast 7.0–14.3:1 in all six themes, tile axis ≥5.5:1.
- `plan-usage-browser.mjs` passed under both engines.
- Screenshots are in `~/.cache/worktrees/worktrees/usage-strip-fixes/`: the
  strip in tokyo-night, in tokyo-day and at 460px, and the rail in all six
  themes, plus the warning and over states.
- Mutations shown red:
  - `viewUsage` without the slack fails the 5s-ahead case;
  - `USAGE_TICK_MS = 90_000` fails "the slack must cover a whole USAGE_TICK_MS".
- Gates, all green:
  - release build, `make test` (540 tests, 0 `not ok`), `make lint`;
  - cargo tests: core 912, cli 122 and api 189, cli and api re-run with the
    env var unset; app lib 36;
  - `tsc`, `cargo check -p app`, and all 49 `*-check.mjs` from the repo root;
  - CI 9/9 on both heads.
- Not driven in the real app: the false "unavailable" is a timing bug, and the
  mock's instant replies cannot reproduce the late reply.

## Follow-ups

- See ROADMAP for:
  - `plan-usage-browser.mjs` running nowhere;
  - a lane's `WORKTREES_VIEWER_JS` failing two gate tests;
  - checking the fix in the real app.
