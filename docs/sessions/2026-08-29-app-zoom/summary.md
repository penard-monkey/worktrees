---
title: "App-wide zoom: ⌘+/⌘− reaches the terminal"
---

# App-wide zoom — ⌘+/⌘− scales the whole window, tmux included

- **Date:** 2026-08-29
- **Worktree:** `~/workspace/worktrees/.worktrees/ui-changes`
- **Branches:** `ui-changes-app-zoom` (feature), `ui-changes-close-out` (this archive)
- **PRs:** [#169](https://github.com/penard-monkey/worktrees/pull/169) — squash-merged as `ce6351c`
- **Release tag:** none (lands in `[Unreleased]`)
- **Planning files:** `planning.tar.gz` beside this summary

## The report

Testing worktrees through the virtual display on an Apple Vision Pro. Two
complaints: there is no ⌘+/⌘− at all, and Settings' UI-font slider maxes at
18px, which "isn't enough and it doesn't seem to impact the tmux sessions or
the terminal".

The last clause was the real finding, and it was working as designed.
`tokens.css` keeps `--term-size` **deliberately independent** of `--ui-rem` so
UI zoom never disturbs the xterm grid. So the slider *could not* enlarge a
tmux pane, by construction — and it also never reached the ~33 hardcoded
`Icon size={13}` px props. Two whole categories of pixel the one advertised
size knob does not own.

## What shipped

**WKWebView page zoom**, not a CSS multiplier.

- `app/src-tauri/src/lib.rs` — `set_zoom` async command → `window.set_zoom()`
  → `setPageZoom`, with an `is_finite` guard and `.clamp(0.5, 3.0)` (an IPC
  boundary does not trust its caller; a NaN there is a window nobody can zoom
  back out of).
- `app/src/settings.ts` — `app_zoom`, the `ZOOM_STEPS` table
  (`0.8 … 3`, ten steps), `clampZoom`, `stepZoom`, and `applyZoom` (a
  module-level dedupe so an unchanged factor costs no IPC).
- `app/src/App.tsx` — ⌘+/⌘−/⌘0 is app zoom; ⌘⌥ the same keys is the markdown
  reader's size, where the unmodified chord used to live. Plus the
  `useEffect(… , [settings.app_zoom])` that pushes the factor to the webview.
- `app/src/SettingsSheet.tsx` — an **Overall size** row at the top of
  Appearance (a range over the step table *by index*, so it can only land on a
  legal step), the shortcuts table, and the diagnostics dump.
- `app/src/FilesPane.tsx` — the reader's +/− button titles now say ⌘⌥.
- `app/src/mock/install.ts` — a `set_zoom` sink.
- Base sliders raised: `ui_rem` 13–**22**px (was 13–18), `term_size` 10–**24**px
  (was 10–20). These set the base that Overall size multiplies.
- `app/scripts/zoom-check.mjs` — new guard, see Verification.

## Decisions

**Page zoom over a CSS `--ui-zoom` multiplier.** A multiplier stays inside the
existing token architecture and carries no blur risk, but it leaves behind
exactly what the user complained about: the px-sized icons, and the terminal
(unless `--term-size` is folded in, which undoes a deliberate separation).
Page zoom shrinks the CSS-px viewport, so `TerminalPane`'s ResizeObserver
refits and re-cols the tmux pane for free. Offered both to the user with the
trade-off stated; they picked page zoom.

**⌘+/⌘− is the APP's, always — markdown moves to ⌘⌥.** The alternative was to
keep markdown's priority whenever a rendered `.md` is on screen, which
preserves existing behaviour but makes ⌘+ mean two different things depending
on what is visible. User's call.

**Not tauri's `zoomHotkeysEnabled`.** Read the vendored source rather than
trusting the name. `tauri/src/manager/webview.rs:555` injects
`webview/scripts/zoom-hotkey.js`, a `window` keydown listener that (a) never
checks `defaultPrevented`, so it fires *alongside* the app's own handler;
(b) keeps the level in a script-local variable that desyncs from any
programmatic `set_zoom`; (c) steps by a coarse 0.2; (d) forgets the level on
every restart. One config line would have looked like the cheap answer and
been wrong four ways.

**The chord is deliberately ungated on the Settings sheet and the ⌘K palette**,
unlike every other chord in that handler. "Make everything bigger" is exactly
what you reach for while squinting at a sheet, page zoom cannot disturb what
either surface is holding — and it means no zoom level can trap the user.

**`app_zoom` is a flat setting, not a `place_panels` field.** One window, one
page zoom. (CLAUDE.md's seed-freeze trap only applies to per-place fields
whose global twin is a seed.)

## Dead ends / gotchas

**⌥ chords cannot be matched on `e.key`.** The highest-value find of the
session, and it was caught by reasoning about the code rather than by any
test. macOS composes Option with the keyboard layout: ⌥- arrives as `"–"` (an
en dash), ⌥= as `"≠"`, ⌥0 as `"º"`. The first implementation matched the
character, so ⌘⌥+/⌘⌥− would have been **silently dead on every US Mac** — the
chord fires, matches nothing, the reader never resizes, and nothing logs.
`e.code` (`Minus`/`Equal`/`Digit0`, plus the Numpad triplet) is the physical
key and is immune. Merge the two tables with `??`, never `||`: 0 is a legal
direction (reset) and `||` swallows it. Both regressions are now covered.

**A check can pass on a completely disconnected feature.** Raised by the
review. Delete the `useEffect` that pushes the factor to the webview, or drop
`set_zoom` from `generate_handler!`, and tsc, cargo and every behavioural
assertion stay green — while `applyZoom`'s `.catch()` keeps the app silent at
runtime too. Three presence checks now cover the wiring itself.

**A comment that states a false premise is worse than no comment.** The review
caught this: the note on the captured `updateSettings` claimed the keydown
effect is "registered once". Its deps are
`[toggleNav, toggleDock, updatePanels, fail, settings.nav_collapsed]`, so it
re-registers whenever the nav collapses. The safety *conclusion* held —
`updateSettings` touches only functional `setSettings`, refs and module
functions, so every copy behaves identically — but the next editor would have
inherited the wrong invariant and applied it to a value that genuinely needs
freshness.

**Three browser-harness attempts failed; the fallback was better anyway.**
chrome-devtools MCP refused to launch (profile locked by another session);
claude-in-chrome loaded `chrome-error://chromewebdata` for both
`localhost:5251` and `127.0.0.1:5251` while `curl` served the page fine.
Stopped after the third failure per the rabbit-hole rule and verified the
chord with `race-check.mjs`'s technique instead — slice the real handler out
of App.tsx, drive it under node with stubs. That turned out to be *more*
useful than a browser run: it can exercise en-dash and `"≠"` key events that
would be awkward to synthesise in a real browser.

**Port 5199 was already serving another worktree's vite.** `curl` answered
happily and reported a plausible-looking file — the exact trap CLAUDE.md
documents. Moved to 5251 and confirmed by grepping the *served* file for
something the edit added.

**`gh pr merge` fails in a side worktree.**
`fatal: 'main' is already used by worktree at ~/workspace/worktrees` — gh tries
to switch the current tree to `main` after merging. The merge had already
landed server-side; only gh's local post-merge step failed, so nothing was
half-done and the shared `main` ref was not moved. Verify with
`gh pr view <n> --json state,mergeCommit` rather than trusting the exit code,
and delete the remote branch by hand.

**`make lint` cannot run on this machine** — shellcheck is not installed at
all. Pre-existing; the diff contained no shell files. Ran the target's other
two halves by hand (`bash -n`, the bash-3.2 gate) and let CI cover shellcheck,
which it did.

## Verification

- **`app/scripts/zoom-check.mjs`** (new). Three layers: the `ZOOM_STEPS` table
  against the Rust clamp; the chord itself, sliced out of App.tsx between
  stable markers and driven with stubs; and the wiring (the `applyZoom`
  effect, `set_zoom` in `generate_handler!`, the mock's case). **Shown RED nine
  ways** before being trusted — a 4× step past the clamp, `1` dropped from the
  table, the Rust clamp deleted, the keyRef fast-double-tap mutation removed,
  the sheet-ungating undone, ⌥ polarity flipped, ⌘0 resetting to the wrong
  value, the `e.code` fallback removed, `??` swapped for `||`; then the three
  wiring deletions. Note it is wired into nothing — see ROADMAP.
- Gates: bats 318 ok / 0 not ok · core 254 · cli 7 · app `--lib` 42 ·
  `tsc --noEmit` clean · `cargo check -p app` clean · `dnd-check` and
  `race-check` green (unaffected).
- CI on #169: **all 9 checks pass**, `lint` included.
- **Real app:** the user ran `app/scripts/sandbox.sh --app` and confirmed it
  works. This was the one thing no automated check here could establish —
  whether page zoom actually moves the CSS-px layout viewport in WKWebView and
  fires the ResizeObserver that re-cols the tmux pane.
- **Review:** fable, against the full diff, briefed on nine specific attack
  surfaces. Verdict *merge with two conditions* (the false comment; confirming
  the real-app run). Both met. It independently confirmed the chord shadows
  nothing (enumerating every other keydown listener in `app/src`), that no
  `migrate` step is needed for a new key, and that `set_zoom` needs no
  capability entry.

## Follow-ups

All in `ROADMAP.md`:

- The column floors (`RAILS_W 88 + NAV_MIN 220 + MAIN_MIN 420` = 728 CSS px)
  are px constants and page zoom moves the viewport under them. 3× wants a
  ~2200px physical window; below that `fitLayout` drops the dock but never the
  nav, and the topbar piles up. Unreachable on the display this was built for,
  and ⌘− is ungated so nothing traps you. Wants one decision about what the
  floors mean when the viewport is elastic — probably deriving them from
  `--ui-rem` — not a patch per symptom.
- The raised 22px/24px caps have no `getComputedStyle` layout assertion, which
  is CLAUDE.md's own rule for exactly this.
- A chord pressed in the pre-hydration window records into `preHydration` and
  overwrites the loaded value, so ⌘+ at launch can land you *below* a persisted
  2×. One fast IPC wide, and shared by every pre-hydration chord (⌘B has it).
- `zoom-check.mjs` joins the four other static checks that are wired into
  neither CI nor the Makefile.
