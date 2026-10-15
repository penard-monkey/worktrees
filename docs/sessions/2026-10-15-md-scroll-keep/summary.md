# md-scroll-keep — rendered documents keep their place across refreshes

- **Date:** 2026-10-14 → 2026-10-15
- **Worktree:** `.worktrees/md-scroll-keep`
- **Branches:** `md-scroll-keep` (squash-merged); `md-scroll-keep-close-out` (this archive)
- **PR:** [#482](https://github.com/penard-monkey/worktrees/pull/482), merged as `4351a1a`
- **Release:** none. The entry is under `## [Unreleased]` → `### Fixed`
- **Planning files:** none. `planning.tar.gz` holds the brief (`.planning/brief.md`)

## The report

Ian: "the rendered markdown views refresh periodically and the view port for
the pane resets to the top of the document instead of keeping place … this is
definitely happening in the files view but maybe others".

## What shipped

- **`app/src/scrollkeep.ts`: `useScrollKeep(ref, key, content, blocks)`.**
  - Anchors on the block at the top of the view: a signature of `TAG:text`
    (clipped to 160 chars) plus the previous block's signature.
  - On new content it searches nearest-first within ±400 blocks and prefers a
    match on both signature and predecessor. A text-only match is a fallback,
    and the scroll pixel is the last resort.
  - Ignores the scroll events its own restore, or a clamp of that restore,
    fires.
  - Memory is a module-scope LRU (200 entries) keyed `path\0view`. Nothing is
    written to `ui-state.json`.
- **`FilesPane.tsx` `FileView`:**
  - "loading…" shows only for a NEW path (`readFor` ref).
  - `sameRead()` keeps the same `read` object for identical bytes, so an
    unchanged refresh re-renders nothing.
  - The hook covers the preview, source and svg-source scrollers.
- **`PlanPane.tsx`:** the same hook on the plan body, keyed `${docPath}\0plan`.
- **Mock (`app/src/mock/install.ts`):** `?slowread[=ms]` (default 300) and
  `__mock.editFile(path, content?)`.
- **`app/scripts/scrollkeep-check.mjs`** (25 assertions, run by
  `make test-frontend`):
  - slices the real `FileView` under a mini hooks runtime with a StrictMode
    remount;
  - runs the real `scrollkeep.ts` under DOM stubs whose scroll box clamps and
    fires `scroll` asynchronously;
  - pins PlanPane's wiring.
- **AGENTS.md:** a 7-line note under the Tauri hard-won rules ("A reused
  `<div>` keeps its scroll offset through a class flip — the reset came from a
  CLAMP").

## Decisions

- **Anchor on a block, not a pixel.** Markdown keys blocks by index
  (`markdown.tsx`), so an edit above the reader shifts every node. A text
  signature survives that, and a pixel does not.
- **Signature plus predecessor.** Fable's review found that plan files repeat
  `**Status:** pending` under every phase, so a text-only match landed the
  reader one phase up. Empty-text blocks (`<hr>`, image-only paragraphs) are
  never anchors, because they all match each other.
- **Keep the old anchor on an inexact restore.** A mid-write stub forces the
  view short of the anchor. Re-anchoring there would lose the real position
  when the full file returns. A restore that lands exactly on the old pixel
  does re-anchor, otherwise a block rewritten once drifts on every later
  insert.
- **No persistence.** Position survives tab, view and place switches within a
  run. Restoring across restarts was not worth a `ui-state.json` field.

## Dead ends / gotchas

- **The obvious cause was not the cause.** `FileView` flipped its `.scroll`
  box to "loading…" on every `places:changed`. Measured in Playwright Chrome
  and WebKit against the mock, React reuses the div and scrollTop survives the
  flip. Not measured in the app's WKWebView. The fix removes the flip anyway.
- **The real reset was a clamp:**
  - `read_file` is a plain open + read, so a truncate-then-write save can be
    read as a stub.
  - The stub clamps scrollTop to 0 and fires a `scroll` event of its own.
  - Anything that trusts every scroll event records 0, and the full file then
    comes back at the top.
  - Old code: `[6000,"Section 56"]` → stub → `[0,"Long doc"]`.
- **The mock answers before React paints**, which hides that stub frame
  entirely. `?slowread` exists for that reason.
- **The rebase onto #483/#485 conflicted only in CHANGELOG.** It resolved to
  one `### Added` and one `### Fixed` (counted).
- **The slot wrapper the orchestrator named was stale.** The main checkout's
  `target/release/worktrees` was v0.40.1, with no `run` command, so every
  wrapped gate failed with "Unknown command: run". I built the branch once
  without the wrapper and used its binary. Main's has since been rebuilt.

## Verification

- `scrollkeep-check.mjs` was shown red first:
  - 7 FAIL against main's `FilesPane.tsx`;
  - the stub case fails on a hook that trusts every scroll event;
  - each review case failed before its fix;
  - both PlanPane assertions failed when mutated.
- Gates after the rebase, all through `run --slot gate`:
  - bats 511, 0 not ok
  - lint
  - test-frontend, 0 FAIL
  - core 764
  - cli 67+1
  - `app --lib` 162
  - tsc and `cargo check -p app`
- CI run 38753303106: 9/9 green.
- Playwright WebKit, headless and headed, against the mock with `?slowread`:
  - every probe holds position;
  - insert-above lands on "Section 56" again.

## Follow-ups

- **Real-app sandbox pass before v0.42.0.** Nothing was driven outside the
  mock. It is in ROADMAP with the five steps from the review.
