---
title: "2026-09-23 — Restore the window on launch"
---

# The window reopens the way you left it

- **Date:** 2026-09-23 (archived 2026-09-26)
- **Worktree:** `.worktrees/ui-changes`
- **Branch:** `ui-changes-restore-window` (2 commits, squash-merged)
- **PR:** [#331](https://github.com/penard-monkey/worktrees/pull/331), merge `64d424f`
- **Release tag:** none yet. The CHANGELOG entry is under `## [Unreleased]`
- **Planning files:** `planning.tar.gz` beside this summary

## What shipped

Settings → Behavior → Startup has a new "Restore window on launch" switch, on
by default. The app reopens with the size, position, maximized state and full
screen it had, after a quit and after an in-app update's relaunch.

- **Backend:** `app/src-tauri/src/winstate.rs`. It tracks the main window from
  its Moved/Resized events and writes `window-state.json` in the app config
  dir. Setup reads `restore_window` from `ui-state.json` and applies position,
  size and maximize. Full screen is requested on `RunEvent::Ready`, checked
  1.5s later, and retried once.
- **Wiring:** `app/src-tauri/src/lib.rs` manages the tracker, calls `attach` in
  setup, `ready` on Ready and `stop` on Exit.
- **Frontend:** `restore_window` in `app/src/settings.ts` (default `true`) and
  the checkbox in `app/src/SettingsSheet.tsx`. No mock change was needed,
  because no new command was added.

## Decisions

- **Own module, not `tauri-plugin-window-state`.** The plugin's `Resized`
  handler records the size on every resize that is not a maximize, full
  screen included. After a full-screen quit, leaving full screen would give a
  screen-sized window. Here the frame is recorded only while the window is
  not full screen, maximized or minimized, and the flags are kept beside it.
- **Backend reads the setting straight from `ui-state.json`.** Restore has to
  happen in setup, before the frontend exists. The file is only ever read,
  because the frontend writes it whole-blob. A missing key means on, to match
  `DEFAULTS`.
- **Always record, restore only when on.** Turning the switch on later picks
  up the last real session.
- **Save while running, never at exit.** See the dead end below.
- **Default on.** Typical macOS behaviour, and the request implied wanting it.

## Dead ends / gotchas

- **Saving at `RunEvent::Exit` recorded the wrong state.** The first hand
  test (full screen, ⌘Q, relaunch) came back windowed. The file said
  `fullscreen: false` over the pre-full-screen frame, written at the quit.
  What the window reports during teardown is not the state the user left.
  The fix writes each change about a second after the window settles, and
  Exit only stops the writer. Quit, close, updater relaunch and a crash all
  stop mattering. The cost is a move made in the last second before quitting.
- **The first version logged nothing, so the failure could not be split.** The
  file and the log could not tell "restore failed" from "quit lost the flag".
  Every restore step and every save is now logged.
- **`set_fullscreen` returning `Ok` proves nothing.** It only queues the
  request to the main thread, hence the check and single retry.
- **macOS reports `maximized = true` while full screen.** tao's
  `is_maximized` is `isZoomed`, and a full-screen window reads as zoomed. The
  real log shows `fullscreen=true maximized=true`. Harmless, because restore
  maximizes only when full screen is false, but anything new reading those
  flags must not treat them as independent.
- **The sandbox writes its own `window-state.json` but the SHARED `app.log`.**
  Sandbox and installed-app lines interleave there (already noted in
  CLAUDE.md). Tell them apart by the `startup v…` lines.
- **The first merge attempt conflicted** on CHANGELOG `[Unreleased]` with #329.
  The entry went at the end of Added, above main's Fixed section.

## Verification

- 7 unit tests in `winstate`. The full-screen test was shown red under the
  plugin's rule, then restored.
- Gates before the PR: release build, `make test` (360, 0 failures),
  `make lint`, core/cli/app tests, `tsc`, `cargo check -p app`. After the
  rebase: app tests (144), `cargo check -p app`, `tsc`.
- Real app, from `app.log`: on 2026-09-24 a save recorded
  `fullscreen=true`. On 2026-09-25 launch restore read it, entered full
  screen, and logged `fullscreen confirmed (check 1)`.

## Follow-ups

- Not yet observed: leaving full screen after a restore returns to the
  pre-full-screen frame, the switch turned off gives the default window, and a
  frame on a detached display falls back to macOS placement.
- `window state saved: …` logs every settled change. Useful while the feature
  is new; consider dropping it to restore-only lines later.
