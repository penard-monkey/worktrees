---
title: "Session — open a file at a line, from the terminal or from an agent"
---

# Session — open a file at a line, from the terminal or from an agent

- **Date:** 2026-10-01
- **Worktree:** `.worktrees/open-file`
- **Branches:** `open-file-show-line` (core/CLI/MCP), `open-file-term-links`
  (app, stacked on it; main and the first branch were merged in, never
  rebased, so neither needed a force-push)
- **PRs:**
  - [#399](https://github.com/penard-monkey/worktrees/pull/399): `show_doc` /
    `worktrees show` take a line, squashed as `c5f3a72`
  - [#400](https://github.com/penard-monkey/worktrees/pull/400): ⌘-click or
    right-click a file path in the terminal, viewer at a line, squashed as
    `221c2a2`
- **Release tag:** none yet. Both are under `[Unreleased]`.
- **Planning files:** `planning.tar.gz` (`task_plan.md` only; the brief was
  `.planning/brief.md`).

## Why

Agents and shells print file paths all day (`ops.rs:1022`, `src/foo.ts:42:7`),
and there was no way to get from one to the file. "Open X" asked of an agent
ended in the agent printing the file. `show_doc` existed but was worded for
documents, took no line, and opened `(main)`'s copy of a relative path even
when a lane was asking.

## What shipped

**#399, core/CLI/MCP**
- `crates/worktrees-core/src/inbox.rs`: `Request` gains optional `line`/`col`,
  `#[serde(default, skip_serializing_if)]`, so a request without a line is
  byte-identical to the old format. `At::check` refuses 0 and a column with no
  line. `split_suffix` splits `file:42[:7]` only when the literal name does
  not exist. `cmd_show` takes `--line`/`--col`, and `--line` overrides a
  suffix completely, column included.
- `crates/worktrees-cli/src/mcp.rs`: `show_doc` takes `line`/`col` (a numeric
  string is accepted). Its description now covers any file the user asks to
  open, see or look at. A relative path resolves in the CALLER'S place first,
  then the repo root. It stays in the `--mutations` tier: it changes no file,
  but it takes over the user's window.
- `crates/worktrees-core/src/guidance/SKILL.md`: a "Showing the user a file"
  section. Guidance `VERSION` 1 → 2, which re-shows the agent-guidance offer
  once.

**#400, app**
- `app/src/termlinks.ts`: `findPaths` (relative, absolute, `~/`, `:l[:c]`,
  `(l,c)`, `#Ll`, `File "x", line N`, `x line N`; URLs masked), and
  `logicalLine`/`hitRange`, which join wrapped rows and map every string index
  back to its CELL so a wide glyph does not shift a link.
- `app/src/TerminalPane.tsx`: `termLinkProvider`. A row's candidates go to
  `resolve_term_paths` in one call and the answers are cached for 10s.
  ⌘-click opens; right-click opens a `CtxMenu` (Open in viewer, Reveal in
  Finder, Copy path, Copy relative path). `linkAt` hit-tests the event itself
  when xterm has no current link.
- `app/src-tauri/src/lib.rs`: `resolve_term_path` (a regular file,
  canonicalised, under a registered project; `~/`; git diff's `a/`/`b/`
  prefixes fall back to the bare path) and `resolve_term_paths`. Relative
  paths try the dock shell's live cwd (`proc_cwd`) or the tmux pane's
  `#{pane_current_path}` first, then the place root. `OpenDoc` carries
  `line`/`col`.
- `app/src/FilesPane.tsx` + `app/src/CodeView.tsx`: `at` on FileView shows the
  SOURCE with the line marked (a tint plus a 2px edge, the gutter number lit,
  the column underlined) and scrolled a third of the way down. This applies
  over markdown preview or the diff too, until a view is chosen. The mark is
  positioned from a Range over the rendered text and re-placed by a
  ResizeObserver.
- `app/src/App.tsx` + `app/src/settings.ts`: `files_open_at`, per place, beside
  `files_open`. The startup restore no longer lands on top of a file opened
  while its `file_readable` was in flight.
- `app/scripts/termlinks-check.mjs`: the detection table, wrapped and wide-glyph
  rows, and the real provider under stubs.

## Decisions

- **⌘-click, not a plain click.** In a terminal a plain click focuses the
  pane, starts a selection, and in claude is claude's own input. iTerm,
  Terminal and VS Code all use ⌘ or Ctrl.
- **A link only for a file the viewer can read.** That means a regular file,
  canonicalised, inside a registered project: `guard_under_projects`' own
  question. There is no "Reveal in Finder" for outside paths. Terminal output
  is the one input nobody chose, so it should not get to name arbitrary files
  for the app to act on.
- **Generous detection, strict resolution.** Detection only keeps obvious
  non-paths (URLs, `1.2.3`, `and/or`) from costing a stat; existence decides.
  The one trade-off: a name with no extension and at most one `/`
  (`Makefile`, `bin/worktrees`) is never a candidate, or prose would be.
- **A line is a position in the SOURCE.** So `at` overrides markdown preview
  and the diff. Choosing a view dismisses the mark rather than mapping a line
  onto rendered output.
- **`files_open_at` is its own record**, not a `place_panels` field: a line in
  one place's file means nothing in another, so there is no global seed.
- **No `open_file` alias for `show_doc`.** A second name for the same action
  costs every session a tool slot and makes the model choose between two.
- **Copy offers both forms**, absolute and relative to the place, as the
  Files tree's menu already does.

## Dead ends / gotchas

- **A throwaway tmux probe killed David's real tmux server.** Inside a lane
  `$TMUX` is set, so `TMUX_TMPDIR` is ignored, and the probe's `new-session`
  and `kill-server` hit the live server: every session and agent, this lane
  included. The "duplicate session" lines in the probe's output were the
  warning and were not read. #401 made `tmux -L <name>` on every call an
  AGENTS.md rule. The re-run used it and left the real server alone.
- **xterm's linkifier activates on ANY click that presses and releases on a
  link**: no modifier check and no button check. So ⌘-right-click opened the
  file under its own menu until `isOpenGesture` required button 0.
- **The linkifier re-asks only when the pointer reaches a different cell, and
  keeps the last cell across a `mouseleave`.** After a menu covers the pane,
  the pointer can be back on a link with no `hover` fired. That gave
  WebKit's native menu on right-click, Escape, right-click. Fixed by `linkAt`.
- **Stopping a press without breaking the linkifier:** both listeners sit on
  `.xterm-screen`, and listeners on one element all run in registration order
  regardless of `stopPropagation`. Ours, added after the linkifier's, keeps
  the press from `.xterm` (claude's mouse report, selection) while the
  linkifier still records it.
- **A Range rect is the GLYPH box, not the line box.** The first band was 5px
  short of its gutter row in WebKit. It is now centred on each rect and one
  computed line-height tall.
- **A Range started at a node's END measures zero width.** The column caret
  was 2px wide on every column that begins a token, until the start test
  became strict (`from < at + len`).
- **The startup restore could replace an explicit open.** A `showDoc` at reload
  + 1.2s showed the remembered file instead of the asked-for one. The restore
  now applies only to an empty viewer.
- **`tmux display-message -t =<unknown>:` prints an empty line and exits 0.**
  Measured on a private `-L` server, so empty means no answer. Also,
  `pane_current_path` follows a `cd` only once the shell has read it: the first
  probe's 0.5s was too short and read like it never tracked at all.
- **zsh expands `=word`**, so a quoted target is needed when probing tmux's
  exact-match `=session:` from the shell. Rust's argv is not affected.

## Verification

- Red-first. Core: the old wire format, shown red by dropping
  `skip_serializing_if`. termlinks-check: red on main's TerminalPane, and
  under five mutations (`stopPropagation`, the modifier test, the button test,
  the `linkAt` fallback, the session argument). The lib.rs resolver test: red
  with the `under_roots` check removed. The resize check: red with CodeView's
  ResizeObserver stubbed out.
- Mock harness in Chromium (DevTools MCP and Playwright) and headless WebKit
  (Playwright 1.63), with real mouse buttons, real `Meta` and real Escape:
  - a hover sweep links exactly the three existing banner paths (not the
    missing one, not the URL);
  - ⌘-click opens at line:col, and the press never reaches `.xterm`;
  - a plain click opens nothing;
  - the menu: every item hit-tested with `elementFromPoint`, both copies and
    the reveal call send the right path, and Escape and an outside click both
    close it;
  - the line mark survives a place switch, and Preview dismisses it for good;
  - the band follows a 222px re-wrap.
- Gates on the final state: release build, bats 428/428 (0 `not ok`), lint,
  core 635, cli 41, app 150, tsc, `cargo check -p app`, 26/26 check scripts.
  `ls --json` byte-identical to the shipped 0.34.1. CI green on every push of
  both PRs.
- A fable review passed both PRs with nice-to-haves only. All of them were
  folded in.

## Follow-ups

Real-app checks (ROADMAP). None of these can be reached by the mock, which has
no tmux, no Finder and no clipboard.
