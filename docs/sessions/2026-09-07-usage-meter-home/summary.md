# Session: the usage meter's new home — strip, footer or rail — and the branch chip as the switcher

- **Date:** 2026-09-06 → 2026-09-07
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branch:** `ui-tweaks-usage-meter` off a freshly fetched `origin/main`, deleted after merge
- **PR:** [#194](https://github.com/penard-monkey/worktrees/pull/194) → `ccf7375`
- **Release:** none — the entry sits in `[Unreleased]` on top of v0.21.0
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress
- **Design canvas:** four placement options + the compact line at 1:1 — a Claude
  Design canvas, private to David's account, id `44e3c1a7-4c5c-4581-9716-d05400a4d8a0`
- **Screenshots:** `strip-webkit-before.png` / `strip-webkit-fixed.png` — the
  headless-WebKit reproduction of the real-app bug and its fix

## Context

v0.21.0 made the sidebar auto-hide. David's annotated screenshots the next day:
the Claude plan usage bars ("I like it always visible") had gone with it; the
sidebar's "+ Add project" footer was redundant; the terminal footer (branch
switcher + tmux facts) "needs to be removed/hidden or be somewhere else".

## What shipped

- **`usage_place` setting** — `"strip" | "footer" | "rail" | "off"`, default
  `strip`, Settings → Appearance → Usage meter (`app/src/settings.ts`,
  `app/src/SettingsSheet.tsx`). Frontend-only key; `loadSettings` merges over
  `DEFAULTS`, so no migration and no Rust change. Off stops the poll itself.
- **One poller, three hosts** (`app/src/App.tsx`): `UsageWidget` became
  `useUsage` (App-owned; `enabled` reaches the effect), `UsageRows` (the old
  three-row block, now the hover detail) and `UsageMeter` (compact trigger +
  fixed-position popover: hover after 140ms, focus, click pins, Esc unpins,
  outside click unpins). Strip = `.usage-strip`, a sibling BELOW `.app` inside a
  new `.appwrap` flex column; footer = `.statusbar`, rendered only in that mode;
  rail = a 32px tile above ＋/⚙.
- **The compact line** is the existing `.usage-row`s laid across
  (`.usage-row.usage-seg`), so the warn/over colour rules apply unchanged.
- **Branch chip = switcher.** `BranchSwitcher` became `HeaderBranch`: the
  header chip is a button that pops the same `BranchCombo` (`drop="down"`),
  icon-only when the branch equals the slug, plain span on `(main)`.
- **Deleted:** the status bar's tmux facts (nav dot, agent dot and header
  already said each), the sidebar's "+ Add project" (the rail's folder-plus
  opens the identical menu), the `.add-footer`/`.sb-*`/`.switch-wrap` CSS.
- `CHANGELOG.md` `[Unreleased]` entry (Added / Changed / Removed).

## Decisions

- **Talk first, then a canvas, then a setting.** David wanted to *see* the
  compact-line-with-hover in place before choosing. Four full-window mockups
  (A strip / B footer / C rail / D header) plus a 1:1 sheet of three line
  shapes. He picked A + B + C behind a setting "to see them in action for a
  while"; D (header cluster) was dropped — it fights the identity/status
  cluster on a narrow window.
- **"Global strip" was explained visually, not in words.** He did not follow the
  phrase; the canvas made it a one-look decision.
- **The footer's other tenants move, they do not stay.** Asked explicitly
  (AskUserQuestion with ASCII previews): the branch switcher goes to the header
  chip, and the footer row exists only when hosting the meter. The alternative
  (keep the footer always, usage joins it in mode B) was offered and declined.
- **Icon-only chip when branch === slug.** `showBranch` is false for most
  worktrees; a switcher that only exists on diverged places is not a switcher.
- **`Off` is a real off.** An "off" that still fetched every 180s would be a lie;
  proved by counting `claude_usage` calls across window-focus events.
- **`--bg-panel` for every new surface**, per CLAUDE.md; measured against
  app/rail/nav in all six themes (≥16/16/8 RGB units; border-top 18–35 from fill).
- Model split: opus wrote the implementation from the brief; fable reviewed
  twice (David's real-app screenshots drove both passes).

## Dead ends / gotchas

- **The mock harness runs in Chrome; the app is WKWebView — and WebKit sizes a
  `<button>` differently.** David's sandbox showed bars and percentages with NO
  labels after the harness had passed at 1280px. Reproduced in headless
  Playwright WebKit (installed into the session scratchpad): every label 0px
  wide, button 201px vs Chrome's 321. Two separate gaps: (1) a `<button>` that
  is itself a flex container is shrink-wrapped without its `overflow: hidden`
  children, so the labels — the only shrinkable items — lose everything;
  (2) even with an inner span as the flex container the button was exactly
  120px short: an empty span sized ONLY by `flex-basis` contributes 0 to
  WebKit's intrinsic width. Fixes: `.usage-shape` span is the flex container,
  `.usage-label { flex: none }`, and the bar carries `width: 40px` as well as
  `flex: 0 0 40px`. Each step measured before/after (201 → 321). The probe
  scripts (`probe*.mjs`, Playwright + `webkit-2203`) lived in the scratchpad;
  the shape is: `addStyleTag` a candidate rule, re-measure, no edit until it
  moves the number.
- **A class name that already exists in the sheet.** The compact segments were
  `usage-row seg`; `.seg` is the Settings segmented control (border, `overflow:
  hidden`, `width: fit-content`). Bordered pills, and the hidden overflow made
  each segment a flex item that can shrink to 0 — in David's narrow window the
  labels vanished and the percentages clipped. Renamed `usage-seg`. grep the
  sheet for a new class before using it.
- **A popover positioned inside `.identity` (`overflow: hidden`) is clipped to
  17px and its autofocus SCROLLS the header.** The branch list rendered at
  y=39..239 with a plausible rect and was invisible and unclickable
  (`elementFromPoint` → `.menu-catch` at every row); focusing the input
  scrolled `.identity` 34px and pushed the place name off the left. Fixed
  positioning from the chip rect + `focus({ preventScroll: true })`. Exactly the
  "a rect is not reachability" rule in CLAUDE.md.
- **Escape at window CAPTURE with `stopPropagation` steals the key from xterm.**
  The first pinned-panel handler did that; while pinned, vim's Escape would have
  died. Bubble phase, no stop.
- **Combo rows commit on `mousedown`, not `click`.** A synthetic `.click()` on a
  `.combo-item` did nothing and read like a broken switcher. Check what the row
  listens to before calling it a bug.
- **`usage-check.mjs` is about metrics, not the plan meter — and it was live.**
  Removing the branch button's constant `data-testid` makes it fail on the
  interpolated branch name; that is a real metrics-key leak the testid prevents.
- **Vite inside `.worktrees/` never sees an edit** (chokidar ignores
  dot-directories). Every re-test needed `kill` + `--force` restart, and a
  content grep of the served file (a CODE string — esbuild strips comments).
  Also true of the sandbox app's own vite: David had to relaunch `sandbox.sh
  --app` to see each fix.
- **A python edit script that asserts mid-way leaves a half-applied change.**
  The TSX half of one edit landed and the CSS half did not (a two-line comment
  sat between the lines the `old` string expected). tsc passed on the TSX
  alone. Re-read the exact on-disk block and replace by line range.

## Verification

- Gates on the PR branch: bats 335/335 · `cargo test -p worktrees-core` 285 ·
  `-p worktrees-cli` 7 · `-p app --lib` 47 · `make lint` · `tsc --noEmit` ·
  `cargo check -p app` · all 9 `app/scripts/*-check.mjs`. CI: 9/9 green.
- Mock harness (chrome-devtools MCP, one click per evaluate): strip spans the
  full window and `.app` shrinks 820→793; rail tile 32×32 with three 18×3 bars;
  footer spans the main column only, no tmux facts; branch chip text/icon/plain
  across messaging / kitchen-sink / (main); a switch to `develop` committed;
  Off → zero `claude_usage` calls across two focus events, re-enable → exactly
  one; add menu still opens from the rail with all three items reachable;
  every visible combo row hit-tests to `.combo-item`.
- Headless WebKit: strip 321px with labels 13/12/27 (Chrome's numbers); rail
  tile + hover panel correct; before/after screenshots archived here.
- Real app: David ran `sandbox.sh --app`, toggled all placements, and confirmed
  the setting persisted (`usage_place` in the sbx `ui-state.json`) and the final
  fix ("that fixed it").

## Follow-ups

- Turn the WebKit probe into a check script (`app/scripts/webkit-check.mjs`)
  or at least a documented recipe — the harness cannot see engine differences
  and this class of bug passed every gate. → ROADMAP.
- David is trying A/B/C "for a while"; the losing placements can be removed
  once he settles (and the `Off` option kept). → ROADMAP.
- The old `.usage-eta` column and `anyEta` logic are only in the popover now;
  fine, but the popover has no "polled Ns ago" freshness line beyond
  live/snapshot. → ROADMAP (minor).
