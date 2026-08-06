---
title: "Session: busy-only dots, nav with Home, system theme pairs"
---

# Session: busy-only dots, nav with Home, system theme pairs

- **Date:** 2026-07-27
- **Worktree:** ui-changes
- **Branch:** next → PR [#38](https://github.com/penard-monkey/worktrees/pull/38) (squash-merged)
- **Release:** none yet — everything sits in CHANGELOG `[Unreleased]` for v0.2.5
- **Planning files:** planning.tar.gz (task_plan / findings / progress) in this dir
- **Screenshots:** home-busy.png (dark, busy dots + folder badge), settings-system-pair.png (pair picker)

## What shipped

User asks: green dot was on everything (meaningless), purple ✦ AI dot on
everything (meaningless), sidebar restyle inspired by a Mux screenshot
(`_tmp/side-bar.jpeg`), a Home with logo + open-project button, and macOS
system light/dark. Plus a follow-up: reframe messaging around durable
work-stream places.

- **Busy ("churning") signal** — `crates/worktrees-core/src/tmux.rs`
  `session_activity()` (`tmux list-sessions -F "#{session_name}\t#{session_activity}"`).
  `app/src-tauri/src/lib.rs`: the existing 3s fingerprint poll thread also
  derives a busy set (activity within 10s) and emits `sessions:busy`
  (sorted `string[]`) only when the set changes. Frontend keeps a
  `Set<string>` from the event — zero new invokes, zero UI polling.
- **Dot semantics** — `app/src/App.tsx` / `App.css`: row `status-dot` renders
  green + breathe ONLY when busy; fixed-width slot so names never shift.
  Purple ✦ glyph removed from rows, topbar, and Home chips; lifecycle-colored
  always-on dots gone (`DOT_COLOR`, `isLive` deleted). Dirty ●N /
  ↑ahead / ↓behind / ⑂detached glyphs unchanged.
- **Nav restyle** — Home entry at the top (`.home-item`, house SVG; click =
  deselect → Home view). Project rows: folder SVG icon (`.picon`) with a
  pulsing corner badge when any child session is busy (replaces the rollup
  dot). `tokens.css`: `--ind` 10→14px, `--dotcol` recentred for the 16px
  icon. Right-aligned dim age column (`.row-age`, `ago(recencyOf(p))`).
- **Home view** — replaces "Welcome back" briefing: app logo
  (`app/src/assets/logo.png`, copied from `app/src-tauri/icons/128x128@2x.png`)
  + name + tagline + "＋ Open a project" + live/dirty chips + resume list.
- **System theme pairs** — `settings.ts`: `theme_light`/`theme_dark`
  (defaults tokyo-day/tokyo-night), `resolveTheme(settings)` new signature,
  `normalizePair()` guards appearance on load. `SettingsSheet.tsx`: two pair
  selects, shown only when theme = system.
- **Mock harness** — `app/src/mock/install.ts`: real `plugin:event`
  listen/unlisten registry + `emitEvent()`, plus a simulated busy cycle so
  `pnpm dev:mock` exercises the dots headlessly.
- **Work-stream framing** — tagline "a place for every work stream"
  (App.tsx), README intro rewritten (durable places, branches flow through),
  GitHub repo description updated via `gh repo edit`, CHANGELOG
  `[Unreleased]` filled.

## Decisions

- **Busy transport = piggyback + event push.** The 3s poll thread already
  exists; adding one tmux format field costs nothing. Emit-on-change keeps
  re-renders quiet. Considered a frontend-invoked poll — rejected (invoke
  churn, lag risk).
- **10s busy window.** Absorbs 3s poll jitter; decays fast enough that the
  dot means "working right now", not "was working a minute ago".
- **`ls --json` schema untouched.** Busy is app-side only; schema v1 stays
  byte-stable for bats + CLI consumers. A `busy` field in the CLI output was
  considered and deliberately dropped.
- **Rail kept.** The Mux borrow is nav-column only (nesting, padding,
  icons, Home); the rail still owns lenses/⌘B/settings.
- **"Red circle" read as the dirty ●N glyph** — the only red-ish circular
  mark in the nav; kept as-is either way.
- **Home keeps the resume list** — user asked for logo + button; resume rows
  were too useful to drop and match the Mux "recent work" feel.
- **System pair is user-configurable** rather than hardcoded tokyo pair —
  any light theme ↔ any dark theme, validated by appearance on load.

## Dead ends / gotchas

- **Settings-open screenshot looked like a broken theme** (nav dark, main
  light in system-light mode). False alarm: the settings `.scrim` overlay
  darkens the nav; computed styles confirmed `--bg-tree` was light. Don't
  screenshot theme states with the sheet open.
- **bats suite failed in this worktree** with
  `make: ./test/lib/bats-core/bin/bats: No such file or directory` — fresh
  worktrees don't have submodules; `git submodule update --init --recursive`
  first. Suite is 134 tests, not the 159 remembered from main.
- **tsc unused-var errors** after dot cleanup — removing the always-on dots
  orphaned `DOT_COLOR` and `isLive`; delete them with the feature.
- **Mock harness ate events silently** — `plugin:event|listen` resolved `0`
  without registering, so the busy dot was invisible in the harness until
  the emitter existed. Any new backend event needs a mock counterpart.
- **`--dotcol` centring** — the project row's leading mark grew from a 7px
  rollup dot to a 16px icon; the (main)-row dot column needed +4px
  (`s3 + caret 10px + gap + half-icon − half-dot`) to keep parent/child
  marks stacked.

## Verification

- `tsc --noEmit`, `cargo check -p app -p worktrees-core`,
  `cargo build --release -p worktrees-cli`, `make test` (bats 134/134),
  `make lint` — all green locally.
- Visual: mock harness on :1421 + Playwright — dark (Tokyo Night) and
  system-light (Tokyo Day) screenshots; busy cycle observed live (dot +
  folder badge appearing/decaying); theme pair picker exercised.
- PR #38: all 8 CI checks green (rust/test/app/install × ubuntu+macos,
  lint), squash-merged to main.

## Follow-ups

- Cut **v0.2.5** — `[Unreleased]` has shippable user-facing work.
- **Work-stream framing sweep** — DESIGN.md, install.sh output, and app
  copy still say "one worktree per branch" in places; align with the
  README/repo-description framing.
- Real-app (non-harness) observation of the busy dot under a live claude
  session — verified by design + harness, not yet eyeballed in the Tauri app.
