# Chat composer scroll bars — 2026-11-01

- **Date:** 2026-11-01
- **Place:** `fix-chat-composer-scrollbars`
- **Branch:** `fix/chat-composer-scrollbars`
- **PR:** [#590](https://github.com/penard-monkey/worktrees/pull/590) (squash `d449db0`)
- **Release:** none (under `[Unreleased]`)
- **Planning files:** none (only `.planning/brief.md`, written from worktrees-ios `(main)` through cross-project reach)

## What shipped

The chat view's message box (`.chat-field`) showed a horizontal and a vertical
scroll bar inside itself at some pane sizes when macOS was set to always show
scroll bars. The scroll bars clipped "Message Claude" and covered the
"One line · Enter sends" row.

- `app/src/ChatPane.css`: `.chat-field` sets `overflow-x: hidden; overflow-y: hidden`.
- `app/src/ChatPane.tsx` (`Composer`): the auto-grow is one `fit` callback. It:
  - sets `overflowY = "hidden"` before measuring;
  - rounds the capped height up (`Math.ceil`);
  - turns `overflowY = "auto"` back on only when `scrollHeight` passes the six-line cap;
  - skips a field of width 0 (hidden).
- `fit` runs on `[text]` and on `uiRem`. A `ResizeObserver` on the field also
  runs it, on a WIDTH change only, because the height changes are `fit`'s own.
- `app/src/App.tsx`: passes `uiRem={settings.ui_rem}` to `ChatPane`. `App`
  writes `--ui-rem` inside the `setSettings` updater, so the CSS variable is
  already set when the layout effect measures.
- `CHANGELOG.md`: one `### Fixed` entry under `[Unreleased]`.

## Decisions

- **Hide overflow and decide scrolling in JS, rather than tolerate a
  sub-pixel slack.** A line-height of 1.45 gives a fractional line, while
  `scrollHeight` is an integer. With `overflow: auto` and classic scroll bars,
  a shortfall under 1px drew a vertical bar. That bar took about 15px of
  width, which then produced a horizontal bar.
- **The observer runs `fit` only when the width changes.** `fit` writes the
  height, so running it on every observer callback would loop.
- **Toggling a classic bar converges.** When the bar appears or goes, the
  width changes and `fit` runs once more. A narrower box can only raise
  `scrollHeight`, and a wider one can only lower it, so it doesn't oscillate
  (from the review).
- **`ui_rem` is passed as a prop, not observed.** The box's font is
  rem-based, but its width is not (`7cqi` gutters, px buttons, px nav). So a
  UI-size change re-wraps the text without any resize event. Before the fix
  this showed a scroll bar; after the first commit it CLIPPED the text. The
  review found this, and the second commit fixed it.

## Dead ends / gotchas

- **Driving the mock to the chat view.** It needs the `chat_view: true` key
  in the `wt-mock-ui-state` sessionStorage item and a place with a live agent.
  The nav starts collapsed, so clicking `li.row` matched nothing visible, and
  each Playwright click waited out its 30s default timeout. The Home screen's
  "Enter" button on catalog-import opens a place whose Claude is up.
- **Headless WebKit ignores "always show scroll bars".** The probes measure
  `scrollHeight` against `clientHeight` and the computed overflow. They don't
  measure the bars, so the hysteresis fix (measuring with the bar hidden) was
  never seen working on screen.
- **CI flake on the first head (`00977ec`):** `app (macos-latest)` failed
  `engine::tests::a_closed_port_under_a_pid_that_never_goes_falls_through_to_a_start_after_the_bound`
  (`engine.rs:1058`, "gave up before the bound (16.584µs)"). This PR doesn't
  touch that code, and the next run passed. It is in ROADMAP.

## Verification

- Playwright WebKit against `pnpm dev:mock` (VITE_MOCK, port 1437, restarted
  with `--force` per AGENTS.md's dead-HMR note). Probes are in
  `~/.cache/worktrees/worktrees/fix-chat-composer-scrollbars/` (`probe.mjs`,
  `probe2.mjs`). Each change was measured red on the code before it:
  - **Width, old code:** a long draft stays 90px tall from 600 to 1400px wide,
    with `overflow: auto` on both axes and a capped height of 128.19px.
  - **Width, fix:** the draft refits from 90 to 71 to 52px, an empty box is
    33px, a very long draft scrolls only past six lines, and nothing overlaps.
  - **UI size 15 → 22px, at `00977ec`:** the empty box stays 33px against a
    `scrollHeight` of 40, and a draft stays 47px against 96. Both are clipped.
  - **UI size 15 → 22px, at `d5d1fde`:** the boxes refit to 42px and 98px,
    with no clipping at 22 or 13.
- `chat-keys`, `chatsteps`, `chat-announce` and `chatmirror` checks pass, and
  so does `tsc --noEmit`.
- CI was green on the merged head `d5d1fde` (run 41355564820, all 9 jobs).

## Follow-ups

- Look at the real app with System Settings › Appearance › Show scroll bars
  set to "Always". No headless run can show the bars.
- The `engine.rs` flake (ROADMAP).
