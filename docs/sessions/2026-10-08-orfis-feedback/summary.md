---
title: "2026-10-08 — Send feedback to Orfis"
---

# Send feedback to Orfis

- **Date:** 2026-10-08
- **Worktree / branch:** `feat-orfis-feedback` / `feat/orfis-feedback`
- **PR:** #466, squash-merged as `c360e08`
- **Planning files:** `planning.tar.gz` beside this summary
- **Scratch:** `~/.cache/worktrees/worktrees/feat-orfis-feedback/`

## What shipped

A feedback form backed by the Orfis web SDK, reachable from Settings →
Data & Logs and from a nav-rail trigger between add-project and the gear.

- `app/src/feedback.ts` — the singleton adapter: one widget, main window only,
  started once outside StrictMode, teardown beats a slow start, Escape on the
  app's one stack, background inerting, the two queue notices.
- `app/src/feedbackConfig.ts` — release-owned routing, validated.
- `app/src/feedbackWidget.ts` — the SDK bridge; both capture opt-outs passed
  explicitly and typed as literal `false`.
- `app/src/feedbackStyle.ts` — **new**; the spacing override (see below).
- `app/src/FeedbackSection.tsx` — the Settings row, the notice, and
  `FeedbackRailButton`.
- `app/src/icons.tsx` — `MessageSquare`. `app/src/App.tsx` — the rail mount
  and the `DIALOG_OPEN` chord-guard selector.
- `app/src/tokens.css` — `[data-orfis]` maps the SDK's eight custom properties
  onto this app's tokens, so the widget follows every theme with no JS.
- `app/src/vendor/orfis/` — pinned bundle at Orfis `3f8c14b` (MIT), its
  `LICENSE`, the narrow `.d.ts`, and the provenance `README.md`.
- `app/.env.production` / `app/.env.development` — routing by vite mode.
- `Makefile`, `README.md`, `app/README.md` — the `make install-app` /
  `make dev-app` fix (unrelated to Orfis; it was blocking the packaged build).
- `docs/orfis-feedback.md`, `docs/orfis-sdk-requests.md`.

## Decisions

- **The `pk_` keys are COMMITTED, in vite's two mode files.** A publishable
  key ships inside the bundle wherever it is stored — `strings worktrees.app`
  finds it — so a repo secret protects it from nobody while making a local
  `tauri build` silently lack a feature CI builds have. The mode split is what
  stops a dev session posting to production, and it is fail-safe rather than
  tidy: `feedbackConfig` allows `http://` only for a loopback host in a dev
  build, so the laptop key cannot ship. Measured against the real validator,
  all four combinations.
- **The acceptance gate was REMOVED, not set to `false`.** A dead flag reads
  like a switch someone might flip back with nothing guarding it. What holds
  the SDK shut in an unconfigured build is configuration itself.
- **The optional email field is on, and the disclosure changed with it.**
  Settings said "no email address is asked for", which would have become
  untrue. `feedback-check.mjs` now asserts the flag and the disclosure
  together so they cannot drift. The two CAPTURE opt-outs stay off: a user
  typing an address is not ambient collection.
- **`make install-app` overrides go in the Makefile, not `tauri.conf.json`.**
  release.yml builds through that file and CI, where pnpm is on PATH and the
  updater key exists, is correct as written.

## Dead ends / gotchas

- **A `calc()` whose custom property does not resolve is INVALID AT
  COMPUTED-VALUE TIME.** The declaration is dropped and the property takes its
  *initial* value — `gap` becomes `normal`, i.e. 0. The first spacing fix was
  `calc(n * var(--ob))`; padding and borders survived, every gap vanished, and
  the dialog looked like a layout nobody had styled rather than a variable
  nobody had defined. It looked exactly like the bug it was meant to fix, and
  cost two rounds of "still wrong". Literal px cannot fail that way.
- **The reference integration had the answer the whole time.** The form's rows
  have NO spacing upstream: `gap: 1rem` is on `.panel`, and the entire
  `<form>` is one of its children, so the type picker, both fields and the
  buttons sit flush. jobmepls (`src/renderer/src/orfis/styles.ts`) carries
  four `margin-block-start` rules its own comment calls "worth upstreaming".
  Three rounds were spent re-deriving from our vendored bundle; a `diff`
  against the working integration would have ended it immediately.
- **`rem` in a vendored shadow-DOM widget resolves against the APP's root.**
  The SDK sizes in `rem`; this app pins `html` to `--ui-rem` (15px), not the
  16px the dialog was drawn for, so everything rendered ~6% tight. `rem`
  cannot be re-based per subtree.
- **`inert` leaves an element visible and drops its events.** `opened()` was
  handed `document.documentElement`, which is not a child of `<body>`, so the
  "exempt the dialog" filter matched nothing and inerted the dialog with the
  app. A perfect-looking form that refused every click.
- **`.modal-scrim` is a styling class, not a marker.** Borrowed as one for the
  chord guard, it painted a second full-screen scrim over the SDK's own
  backdrop and trapped the dialog in a `z-index: 100` stacking context. A
  shadow-hosted dialog cannot paint a light-DOM class; it needs a style-free
  marker (`[data-dialog-open]`).
- **Three independent reasons `make dev-app` was broken, each hidden by the
  one before.** `pnpm --dir app` from the root (corepack fixes its version
  from the cwd first); a bare `pnpm` may not exist at all (nvm installs it per
  NODE VERSION and the pinned one ships only `corepack node npm npx`, so
  `nvm use` REMOVES it from PATH); and tauri's own
  `beforeDevCommand`/`beforeBuildCommand` are a bare `pnpm` too.
- **Verifying in a shell that has used another node version hides all of
  that.** `$PATH` keeps the old version's bin further down, so `pnpm` resolves
  anyway and a broken target looks fixed. `env -i` with only the pinned node's
  bin reproduced the user's exact error.
- **A source assertion matched the COMMENT, not the code.** The mirror check
  for `[data-dialog-open]` passed on the prose above `DIALOG_OPEN` while the
  real selector had dropped it. Match the constant's value, not the file.
- **`pkill -f "tauri dev"` is too broad.** Cleaning up a test killed the
  user's sandbox app and its tmux session. Kill by PID tree.
- **`git add <file>` sweeps unrelated hunks.** Staging `App.tsx` for the guard
  fix also committed the rail-button mount, leaving a commit that referenced
  an export from an uncommitted file — it would not have built on a fresh
  checkout.
- **A background task has a 2h ceiling.** The sandbox app died when its task
  hit it; a detached tmux session on a private socket (`-L`) has no such cap.

## Verification

Run and green on the merged tree: bats 478, `worktrees-core` 722,
`worktrees-cli` 70, `app --lib` 162, `make lint`, `tsc --noEmit`,
`cargo check -p app`, all 37 `app/scripts/*-check.mjs`.

New assertions were each confirmed RED first: the vendored LICENSE and its
notice line, the README's recorded hash, the mode-split routing, background
inerting (both directions), the style-free dialog marker, and the adapter's
choice of shadow host. `zoom-check.mjs` went red on the selector refactor and
was re-pinned to the constant's value, then re-confirmed against both a
dropped scrim class and a state mirror.

**Done by hand, not by the list:** the form was opened from both triggers
against the laptop API and a report sent. **NOT done:** a packaged build
against production, a queued report surviving quit and relaunch, `429`,
offline retry, duplicate submission, or a stored row inspected against the
documented fields.

## Follow-ups

- Release verification owes the list above (`docs/orfis-feedback.md`).
- The four form-spacing rules are jobmepls' local deviation and "worth
  upstreaming". If Orfis adopts them, the vendored sheet will carry them and
  our copy becomes redundant — check at the next re-vendor.
- `feedbackStyle.ts` is generated from the vendored sheet (rem → px at 16).
  Regenerate on re-vendor; the check pins that the sheet still uses `rem`.
- `setTheme()` is effectively a no-op: it only swaps the SDK's own eight
  colour properties, and our `[data-orfis]` rule beats `:host` in every case.
  The `[data-theme]` observer is harmless but buys nothing.
