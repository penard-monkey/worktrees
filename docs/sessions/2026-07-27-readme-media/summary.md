# README desktop-app media — session summary

- **Date:** 2026-07-27
- **Worktree:** ui-changes
- **Branches:** `ui-next` (feature work) → `chore/close-out-readme-media` (this archive)
- **PRs:** [#53](https://github.com/penard-monkey/worktrees/pull/53) — merged as `ea27947`
- **Release tag:** none
- **Planning files:** none

## What shipped

README now shows the desktop app. New **Desktop app** section in `README.md`
(between the intro and Install) with:

- `docs/media/desktop-flow.gif` — 920×575, ~13s loop of the core flow:
  list → new (`feat/checkout`, session spins up) → open (attach another place).
- `docs/media/desktop-overview.png` — workspace overview (retina 2×).
- `docs/media/desktop-session.png` — a place with its embedded terminal (2×).

Plus a **reproducible recorder** committed under `app/scripts/`:

- `record-readme.sh` — one command: boots the mock harness, drives the flow,
  ffmpeg-encodes the gif, copies all three assets into `docs/media/`.
- `record-readme.py` — Playwright driver (synthetic cursor + real typing);
  records the flow webm + captures the two stills.

## Decisions

- **Capture source = mock harness (`VITE_MOCK=1`), not the real app.** User
  chose deterministic + fully-scripted over authentic. Trade-off accepted: the
  mock terminal prints a `mock terminal — design harness` banner, captioned
  honestly in the README. Real-app re-cut is on the roadmap.
- **Content = core flow (list → new → open).** The app's actual value story,
  vs. the flashier theme-cycle option.
- **Python Playwright `record_video` → ffmpeg palettegen/paletteuse**, not
  MCP screenshot-stills stitched together. Gives smooth motion + typing
  animation and is re-runnable from a committed script. Two-pass palette
  (`stats_mode=diff` + bayer dither) keeps it ~3 MB at 15 fps / 920px.
- **Synthetic cursor injected** — Playwright does not render a pointer in
  recorded video, so a fake arrow div follows `mousemove`.
- **Recorder committed to `app/scripts/`** so the gif is regenerable after UI
  changes instead of being a one-off artifact.

## Dead ends / gotchas

- **`pnpm dev:mock` fails on this machine** — pnpm requires Node ≥ 22.13,
  system has 22.12. Workaround (used by the recorder): run the vite binary
  directly — `VITE_MOCK=1 ./node_modules/.bin/vite --port 1425 --strictPort`.
- **Playwright Python `add_init_script` takes a raw script STRING.** Passing
  `() => { … }` just *defines* a function that never runs — cursor silently
  absent. Must be an IIFE `(() => { … })()`.
- **Even as an IIFE, init scripts run at `document_start` before `<body>`
  exists.** Appending the cursor node to `<html>` gets discarded when the
  parser builds the real tree → node MISSING at load. Fix: defer the insert to
  `DOMContentLoaded` (guard on `document.readyState`).
- **`new_place` (mock) sets `tmux_session.up = true`** → a freshly created
  place lands live in the terminal immediately (no "Enter ▸ to start" step).
  And a place **row click = `enterPlace` = opens directly**. So the "open"
  verb is shown by attaching to an *existing* place, not by a separate button
  after create.
- **Fancy/`rtk`-decorated `ls` output poisoned `$(ls *.webm)`** (ANSI + column
  chrome captured into the var). Use `find … -name '*.webm'` for clean paths.
- **ImageMagick single-frame extract from an optimized gif returns delta
  frames** (mostly transparent) — must `magick core-flow.gif -coalesce` first
  to verify a composited frame.
- **`gh pr merge --squash --delete-branch` errored on the post-merge local
  step** — `fatal: 'main' is already checked out at …/worktrees` (worktree-per-
  branch keeps main checked out elsewhere). The merge itself succeeded; deleted
  the remote branch manually with `git push origin --delete ui-next`.

## Verification

- `app/scripts/record-readme.sh` ran end-to-end (~18s), regenerating all three
  assets deterministically.
- Eyeballed all three flow beats via extracted keyframes; confirmed the
  synthetic cursor renders (coalesced gif frame).
- PR #53 merged to `origin/main` (`ea27947`); remote `ui-next` deleted;
  `origin/main` fetched clean.

## Follow-ups

- **Better desktop-app README gif** (in ROADMAP) — re-cut from the real Tauri
  app so the pane shows a live tmux + AI CLI session; consider trimming the
  flow and resting on the overview.
- **Work-stream framing sweep** (existing ROADMAP item) touches README copy —
  adjacent to this work if picked up.
