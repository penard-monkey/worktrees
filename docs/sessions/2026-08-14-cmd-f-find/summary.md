---
title: ⌘F finds — in the terminal, and in the file you have open
---

# ⌘F finds — in the terminal, and in the file you have open

- **Date:** 2026-08-14
- **Worktree:** `.worktrees/ui-tweaks`
- **Branches:** `ui-cmd-f-find` → merged; archive on
  `ui-close-out-cmd-f-find`; tree parked on `ui-next`
- **PRs:** [#131](https://github.com/penard-monkey/worktrees/pull/131)
  `feat(app): ⌘F finds — in the terminal, and in the file you have open`
  (squashed as `be8b930`)
- **Release tag:** none — sits in `[Unreleased]` after v0.13.0
- **Planning files:** `planning.tar.gz` beside this summary
- **Screenshots:** `find-term-dark.png` (terminal, Tokyo Night),
  `find-file-dark.png` / `find-file-light.png` (Files viewer, both themes)

## What shipped

Searching meant leaving. The terminal had no find at all — scroll and hope — and
the Files viewer sent you to an editor to look for a string in the file already
on screen. ⌘F now opens a find bar on whichever surface you are working in.

**In a terminal** — the place's tmux pane (`TerminalPane`) or a dock shell tab
(`ShellPane`) — it searches the xterm buffer through `@xterm/addon-search`:
every match tinted, the active one brighter, ⏎ / ⇧⏎ and ⌘G / ⇧⌘G stepping. It
can only see what the terminal has RECEIVED since it attached, because tmux
keeps its own scrollback and does not replay it into xterm. That is a real
limit, so the field says it on hover rather than leaving it to be discovered.

**In the Files viewer and in the ⌘⇧E reader** it searches the open file —
rendered markdown as readily as source — and follows you when you switch files.
`Aa` toggles case. Esc closes and hands the keyboard back.

Also filled in Settings → Shortcuts, which had been listing ⌘B/⌘K/⌘,/⌘1-4/⌘E and
omitting ⌘J, ⌘⇧E and ⌘⇧T.

### Files

| File | What |
|------|------|
| `app/src/Find.tsx` (new, 356 lines) | `FindBar` (the shared bar), `findColors()`, `useFileFind` — DOM walk → `Range`s → CSS Custom Highlight API |
| `app/src/TerminalPane.tsx` | search addon, `allowProposedApi`, `guard()`, new `TermSurface` shared by both pane kinds, `.term-wrap` |
| `app/src/FilesPane.tsx` | `FindInFile` strip between the viewer header and body |
| `app/src/App.tsx` | `findOn` / `findToken` state, `lastSurface` ref, `findTarget()`, the ⌘F chord, four teardown effects |
| `app/src/App.css` | `.term-wrap`, `.findbar` (float over a terminal / strip in the viewer), `::highlight()` rules |
| `app/src/tokens.css` | `--find-hit` / `--find-on` / `--find-on-ink` across all six themes |
| `app/src/SettingsSheet.tsx` | the shortcuts list |
| `app/package.json` | `+ @xterm/addon-search@0.15.0` |

## Decisions

**File find searches the rendered DOM, not the file's source text.** The
markdown preview is built from `marked` tokens, so source offsets do not map to
what is on screen at all; the code view is tokenised into thousands of spans, so
injecting `<mark>` during render would re-reconcile the whole file on every
keystroke. Walking the rendered text gives ONE implementation for code, markdown
preview and SVG source, and leaves `highlight.ts`, `CodeView.tsx` and
`markdown.tsx` completely untouched.

**Painting goes through the CSS Custom Highlight API** (`CSS.highlights`,
`Highlight`, `Range`) rather than DOM mutation — React never sees it, so a
re-render cannot fight the highlight and there is nothing to clean up in the
tree. The fallback when the registry is missing (WKWebView older than Safari
17.2) is to select the active hit, which at least shows where it is. `var()`
inside `::highlight()` was the one thing Chromium could not vouch for; the real
app confirmed WKWebView resolves it, so the per-theme literal-hex fallback was
never needed.

**`@xterm/addon-search@0.15.0`, not 0.16.0.** 0.15.0 declares
`peerDependencies: {"@xterm/xterm": "^5.0.0"}`; 0.16.0 declares no peer range and
targets the 5.6/6.0 line. Do not bump it without checking the terminal still
renders. The addon is not a "UI library" in the sense CLAUDE.md forbids — it is
the same family as the `addon-fit` already in use.

**Exactly one find bar exists app-wide** (`findOn: null | "main" | "dock" |
"read"`). The highlight registry is global, so two live bars would overwrite
each other's hits, and a bar left open behind the ⌘⇧E reader would be tinting a
file nobody can see.

**Routing follows what the user last TOUCHED, not DOM focus.** Every terminal
calls `term.focus()` when it mounts, so a focus-based rule is decided by mount
order — opening the dock's Terminal tab silently takes ⌘F away from the pane you
are working in. `pointerdown` + `keydown` in the capture phase are the only
marks. (Focus alone is also wrong in the other direction: the file tree's rows
are plain divs, so clicking a file to read it leaves `activeElement` on `<body>`.)

**The terminal bar floats; the viewer's is a strip.** Not cosmetic — xterm sizes
the pty from `.term-host`'s box, so a bar in the flow there refits the grid every
time it opens. Hence `.term-wrap`, a positioned wrapper that took over the flex
role. In the viewer there is nothing to refit and a floating bar would cover the
first lines of the file, so it sits between the header and the body — and
crucially OUTSIDE `.viewer-body`, which is the search root: a bar within it would
offer its own "3/12" and button glyphs up as matches.

**Esc closes the find bar first — except inside `.term-host`.** There Escape is
the user's (vim, a menu, a prompt), and swallowing it would be a worse bug than a
bar left open.

## Dead ends / gotchas

**The search addon's decorations are gated behind `allowProposedApi: true`, and
it only throws when you SEARCH.** The terminal renders perfectly right up until
the first ⌘F, at which point `registerDecoration` throws — from inside an effect,
which unmounted the whole pane. Two lessons: set the flag, and wrap every addon
call (`guard()` in TerminalPane.tsx) so an addon failure logs via `log_event`
instead of taking the terminal down with it. Also load the addon AFTER
`term.open(host)`.

**`SearchAddon` caches its last search and ignores changed decoration colours.**
`_didOptionsChange` compares only `caseSensitive` / `regex` / `wholeWord` — never
`decorations` — so a theme switch left every non-active match painted in the OLD
theme's hex on the NEW theme's background. Verified in the vendored source
(`node_modules/@xterm/addon-search/lib/addon-search.js`). The fix is
`clearDecorations()` first, which drops `_cachedSearchTerm` and forces a full
re-highlight; it also makes `_findNextAndSelect` measure from the selection's
START rather than its end, so the user stays on the match they were on.

**A `useCallback` in an effect's dependency array silently advanced the user a
match.** The search options callback closed over the theme, so any theme change
gave it a new identity → the live-search effect re-ran → `findNext` moved on.
Every `findNext` moves the selection, so an effect that re-runs "harmlessly" is
not harmless here. Caught in the harness (1/8 → 2/8 on a theme switch) and fixed
by putting the callback behind a ref; the effect now depends on
`query`/`caseSensitive`/`epoch` and nothing else.

**Reading live state from a ref via a render-time snapshot defeats the ref.**
`keyRef.current` is assigned during render, so copying `lastSurface.current` into
it re-staled the value the capture-phase listeners existed to keep fresh. Any
interaction that changes no React state — clicking into a terminal to focus it,
clicking the open file's text — left the snapshot behind. The fix is to read the
ref at keydown time. **Worth recording: this could NOT be reproduced in the mock
harness.** The mock polls `list_workspace` on a timer, so a render lands within a
second of any click and refreshes the snapshot before ⌘F can catch it stale — a
deliberate probe that restored the bug still routed correctly. It is fixed on
reasoning, not on a regression test.

**Escape on a `<input>` is not Escape on the bar.** The key handler started on
the field, so clicking `Aa` or `›` (which moves focus to that button) left
Escape doing nothing. It belongs on the `.findbar` container, with Enter still
guarded to the field so a focused button does not fire twice.

**Case folding is not length-preserving.** The first implementation lower-cased
both haystack and needle and guarded against a length change by falling back to
case-sensitive — meaning one `İ` anywhere in a file turned the whole search
case-sensitive, with the term visibly on screen and "no results" underneath.
Replaced with an escaped `RegExp` over the ORIGINAL text, which removes the
problem instead of degrading around it.

**The code gutter's line numbers are real text nodes.** Without excluding
`.code-gutter` in the DOM walk, searching a digit matches the margin first and
the hits look like nonsense — `1` in Cargo.toml found 8 instead of 3.

**Two vite instances can hold one port, and `pkill -f vite` leaves one.** The
survivor kept serving the PRE-edit module while the new instance died with "Port
5199 is already in use" — so an applied fix read as not applied. Already in
CLAUDE.md; `kill -9 $(lsof -ti:5199 -sTCP:LISTEN)` is the fix. Two related traps
cost time on top: **grepping the served file for a COMMENT proves nothing**
(esbuild strips them — grep for code), and **a vite started with `nohup … &`
inside a Bash tool call gets SIGTERM'd** when a later call's process group is
cleaned up. That last one produced the session's most convincing false alarm: the
server died mid-run, React Fast Refresh reset App's state, and the app jumped to
the Home screen immediately after a ⌘F — which read exactly like ⌘F clearing the
selection. Start the harness with the tool's own background flag.

**The rebase was not a formality.** `origin/main` moved two commits ahead
(#129 markdown reading size, #130 its close-out) touching the same four files.
Three conflicts, all additive, plus the part that mattered: checking for real
interference rather than trusting a green `tsc`. ⌘F is not in `ZOOM_KEYS`, and
markdown zoom is a CSS variable on `.scroll` — a style, not a re-render — so it
never replaces text nodes and the find `Range`s survive it. Confirmed live:
zoom to 110% with find open keeps all 6 hits and the active one.

## Verification

Driven in the mock harness (Playwright, port 5199) with assertions rather than
screenshots:

| Check | Result |
|-------|--------|
| routing to each of the four surfaces | correct, including dock-click with no intervening render |
| terminal search | `1/8`, 9 xterm decorations; ⏎ / ⇧⏎ step 1→2→1 |
| opening the bar does not refit the pty | `.xterm-screen` 704×690 / 46 rows, identical with the bar up |
| markdown preview | count = DOM occurrences = `CSS.highlights.size` = 6 |
| step to the last hit | `.scroll` scrollTop 851 of 1194 — revealed |
| gutter exclusion | `1` in Cargo.toml → 3 (code), not 8 (with line numbers) |
| match across syntax spans | `members = ["crates` → `startContainer !== endContainer` |
| regex escaping | literal `.` → 18 dots, not 1051 characters |
| case toggle | `WORKSPACE` with `Aa` on → "no results" |
| switching file with the bar open | query kept, count recomputed |
| ⌘⇧E over an open dock find | dock bar closed, nothing left painted |
| Esc, focus outside the bar | find closed, reader survived |
| Esc, focus in `.term-host` | bar stays — Escape reached the terminal |
| theme switch with find live | all 9 decorations re-created, new token, position held |
| ⌘K palette open | ⌘F swallowed |
| ⌘F twice | re-focuses the field, does not toggle shut |
| zoom to 110% with find open (post-rebase) | 6 hits + 1 active, count unchanged |

Then run for real — `app/scripts/sandbox.sh --app` — which is what closed the one
question the harness could not answer (`var()` inside `::highlight()` on
WKWebView) and exercised find over actual scrollback instead of the mock's
three-line banner.

Gates green before and after the rebase: `make test` (288 bats, 0 `not ok`),
`make lint`, `cargo test -p worktrees-core` (208), `-p worktrees-cli` (6),
`-p app --lib` (29), `cargo check -p app`, `tsc --noEmit`,
`node app/scripts/race-check.mjs`. CI green on both OSes.

## Follow-ups

- **⌘F on an image or binary viewer** opens a bar that answers "no results".
  Harmless, slightly sloppy — could be suppressed by file kind.
- **Terminal find only covers what xterm received since attach.** Raising
  xterm's `scrollback` from the default 1000 lines would widen it; reaching
  tmux's own history would mean driving copy-mode, which takes over the user's
  pane and was deliberately not attempted.
- **The no-Highlight-API fallback is untested.** It exists for WKWebView older
  than Safari 17.2 and has never been exercised — there is no `minimumSystemVersion`
  in `tauri.conf.json` to say whether anyone can reach it.
