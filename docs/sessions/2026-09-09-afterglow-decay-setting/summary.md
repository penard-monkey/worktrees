# Session: the afterglow's horizon and step count become a setting

- **Date:** 2026-09-08 → 2026-09-09 (closed out 2026-09-16)
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branch:** `ui-tweaks-afterglow-decay` off a freshly fetched `origin/main` (v0.22.0), deleted after merge
- **PR:** [#199](https://github.com/penard-monkey/worktrees/pull/199) → `074f1c6`
- **Release:** shipped in v0.23.0 (#200, cut from another tree the same day)
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress
- **Cost split:** an opus agent implemented from a brief; fable reviewed the diff, fixed one blocker inline, committed and merged (the pattern from the session that produced #41–#43)

## Context

David: the purple dot "is a bit too short" and should become a slider so the
interval can be tuned and tracked. He did not know how many steps or how long,
so the first ask was for the current numbers and a proposal. Two decisions came
back the next day: **geometric** spacing, and **keep the dot's meaning** — it
stays "Claude finished a task here", never opens or commits.

## What shipped

- **`app/src/afterglow.ts`** (new, pure, React-free): `DONE_FIRST_SECS` (15m,
  pinned), `DONE_HORIZONS` (1h · 2h · 4h · 8h · 12h · 24h · 2d · 3d · 7d),
  `DONE_STEPS_MIN/MAX` (2–6), `snapHorizon`, `clampSteps`, `doneBounds`
  (geometric: `b[i] = FIRST · (H/FIRST)^(i/(n−1))`), `doneTier` (0 = off,
  1..n), `doneOpacity` (linear 1 → 0.3 across steps), `fmtSecs`.
- **`app/src/settings.ts`**: `done_horizon_secs` (default 12h) and
  `done_steps` (default 3), snapped/clamped in `loadSettings` next to the
  theme normalisers. Frontend-only keys; the blob merges over `DEFAULTS`, so
  no `SETTINGS_REV` bump and no Rust change.
- **`app/src/App.tsx`**: the three constants and the local `doneTier` are
  gone; `bounds` is one `useMemo`, `doneOf` returns a number, `dotClass` emits
  `done` + `t1` (ring) only for tier 1, and a new `dotStyle` writes the
  opacity as an inline style at BOTH render sites (nav row, home Resume list).
  The project-folder rollup lights on tier 1.
- **`app/src/App.css`**: `.done.t2` / `.done.t3` removed; `.done` keeps the
  hue and `.t1` the box-shadow ring. The stylesheet cannot know the step count.
- **`app/src/SettingsSheet.tsx`**: Settings → Navigation → Afterglow — a
  horizon slider over the `DONE_HORIZONS` INDEX (same reason the zoom slider
  walks `ZOOM_STEPS`) and a steps slider; the hint prints the boundaries from
  `doneBounds` itself so it cannot go stale.
- **`app/scripts/afterglow-check.mjs`** (new): drives the real module — 45
  horizon×step curves, tier edges, opacity monotonicity, the CSS/App.tsx
  mirror (no `.done.t2` rule, no `DONE_T1_SECS` literal left in App.tsx).
- **`app/scripts/zoom-check.mjs`**: inlines `afterglow.ts` ahead of
  `settings.ts` — see gotchas.
- `docs/ai-profiles-manual-checks.md` §10 and `CHANGELOG.md` updated;
  `app/src/mock/fixtures.ts` comments name the default curve's bounds.

Defaults reproduce the old behaviour as closely as the rule allows: 12h in
3 steps → **15m · 1h 44m · 12h** at opacities 1 / 0.65 / 0.3. The old middle
boundary was 2h; that drift was accepted.

## Decisions

- **Geometric, not even.** Even spacing over 24h puts the first boundary at
  8h and the "just finished, go look" signal dies. Geometric keeps the fresh
  end fine and the tail coarse; today's 15m / 2h / 12h was already roughly
  that (×8, ×6).
- **First boundary pinned at 15 minutes, not derived.** Tier 1 wears the ring
  and is the only tier the project-folder rollup lights on; both are
  statements about a quarter of an hour, and stretching the horizon to a week
  must not stretch them.
- **Horizon is a stop table, steps a small integer range.** A linear range
  over 1h..7d spends most of its travel on moves nobody can see.
- **Opacity is an inline style, computed per tier.** Fixed `.t2`/`.t3`
  classes cannot express a variable step count. Discreteness is preserved
  (the three reasons in the App.tsx comment survive: relative contrast,
  reduced-motion freezing long animations, assertability via
  `getComputedStyle`).
- **Meaning unchanged.** David asked what the dot means today; the answer
  ("Claude finished here", from the sessions:done event or the persisted
  `last_worked_epoch`) is what shipped. `IDLE_WINDOW_SECS` (7d lifecycle
  reconciliation) is a different concept and was not touched.
- **Defaults reproduce today** rather than jumping to a longer horizon. David
  is the one user; the slider is where he tunes it, and the changelog says so.

## Dead ends / gotchas

- **Adding an import to `settings.ts` broke `zoom-check.mjs`.** That script
  loads `settings.ts` as a `data:` URL module, which has no base to resolve a
  relative `./afterglow` against (`ERR_UNSUPPORTED_RESOLVE_REQUEST`). Fixed by
  inlining the real `afterglow.ts` source (exports stripped) ahead of
  `settings.ts` and removing the import line — NOT by stubbing the two
  functions, which would be a second answer to `snapHorizon`, the exact drift
  the check scripts exist to catch. Any future import into `settings.ts`
  hits the same wall.
- **`fmtSecs` carried minutes into a 24th hour.** Rounding the minute part of
  86399s gave "24h"; 3599s gave "60m". Caught by the check script while it
  was being written, before the harness ever rendered it.
- **The Settings section used two heading-level labels.** The sheet's
  convention is one `<label>` title plus `<label className="sub">` rows;
  the review caught it, fixed inline before commit.
- **Short horizon × many steps is legal but thin.** 2h in 6 steps puts three
  boundaries inside 40 minutes, the dimmest two ~9 minutes apart — under the
  60s decay tick nothing skips, but the tiers are barely distinguishable.
  Left as is; trim `DONE_STEPS_MAX` or the low stops if it bites.
- **`snapHorizon`'s NaN fallback hardcodes `DONE_HORIZONS[4]`.** It equals the
  default, but nothing ties the index to `DEFAULTS.done_horizon_secs` because
  `settings.ts` imports `afterglow.ts` and the dependency cannot point back
  without a cycle. The check asserts every stop round-trips; it does not
  assert that pairing.
- **`gh pr merge --delete-branch` from a side worktree** failed its LOCAL
  checkout step after the merge had landed ("'main' is already used by
  worktree"), and the remote-branch delete never ran. Confirmed with
  `gh pr view --json state,mergeCommit`, then deleted the remote branch by
  hand. Known trap, now hit twice.

## Verification

- `tsc --noEmit`, all ten `app/scripts/*-check.mjs`, `cargo check -p app`
  green locally; CI 9/9 on #199.
- `afterglow-check.mjs` shown to FAIL first: with `doneBounds` made linear it
  reported 217 failures; restored, 45 curves verified.
- Mock harness (Chrome, port 5199, `--force`, served file grepped for a code
  token): the three fixture dots measured by `getComputedStyle` at defaults —
  `search-index` (4m) opacity 1 with a 3px ring, `hotfix-login` (45m) 0.65,
  `fix-flaky-ci` (5h) 0.3; steps 3→5 re-tiered live (0.825 / 0.65 / 0.3),
  horizon 12h→2h made the 5h dot disappear (tier 0). Both render sites
  checked. Zero console errors.
- NOT run: the real app. The change is timing-free (no refresh / optimistic
  UI), which is the CLAUDE.md criterion for needing `sandbox.sh --app`.

## Follow-ups

- Watch whether 12h still feels short; 24h is the next stop. If a finer or
  longer table is wanted, `DONE_HORIZONS` in `afterglow.ts` is the one place.
- The 2h × 6-step corner (see gotchas) if it ever reads as noise.
