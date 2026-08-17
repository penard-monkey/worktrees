---
title: "2026-08-17 — emoji width: tmux says 2, xterm said 1"
---

# Session: emoji garbled the terminal whenever Claude ran in tmux

- **Date**: 2026-08-17
- **Worktree**: `.worktrees/bug-fixes`
- **Branches**: `bug-fixes-emoji-width` (fix), `bug-fixes-close-out-codesign`
  (rebased + merged as part of this close-out — #128 had sat open for 2 days)
- **PRs**: [#143](https://github.com/penard-monkey/worktrees/pull/143) (fix,
  squash `88c955e`), #128 (previous session's stranded archive)
- **Release**: none — ships with the next one
- **Planning files**: `planning.tar.gz` beside this summary

## What shipped

Emoji-prefixed lines from Claude Code (or anything else) inside tmux no
longer shred in the app's terminal ([evidence](garbled-status.png) —
"Phase 15 DBacklo" where the pane held "Phase 5: Backlog QA").

- `app/src/TerminalPane.tsx` — load `@xterm/addon-unicode-graphemes`,
  `term.unicode.activeVersion = "15-graphemes"`, before `term.open()`.
  `allowProposedApi: true` shared with the search addon (both need it;
  `term.unicode`'s getter throws at load without it, not at use).
- `app/package.json` + lockfile — `@xterm/addon-unicode-graphemes@^0.4.0`.

## Root cause

Width disagreement per emoji between tmux's grid and the renderer, measured
on both hops:

| Char | tmux 3.6a (utf8proc) | xterm.js 5.5 default (Unicode 6) |
|---|---|---|
| ✅ U+2705, 🔄 U+1F504, 📋 U+1F4CB | 2 | 1 |
| ⚠️ U+26A0+VS16 | 2 | 1 |
| ⚠ bare U+26A0 | 1 | 1 |

tmux keeps an authoritative grid and repaints damaged cells with absolute
cursor moves; 1 column of skew per emoji makes every partial repaint
interleave with the previous paint, and Claude Code's spinner forces
constant partial repaints. `tmux capture-pane -p` shows clean text while the
pane renders garbage — that pair is the diagnostic that isolates the hop.

Measurement method (reusable): tmux side —
`send-keys "clear; printf '…'; sleep 3"` then `tmux display -p '#{cursor_x}'`
mid-sleep; xterm side — node with
`term._core.unicodeService.wcwidth(cp)` under `allowProposedApi`.

## Decisions

- **Graphemes addon over Unicode11Addon.** Homebrew tmux links utf8proc,
  which upgrades VS16 sequences (⚠️) to wide; Unicode11Addon counts
  per-codepoint (VS16 = 0) and would leave those still 1-off. Claude Code
  emits ⚠️-class chars, so the conservative option would not fully fix the
  bug being fixed.
- **Fix applied at the renderer, not upstream.** tmux and Claude Code agree
  with each other; xterm was the odd one out. Any other tmux client with old
  wcwidth tables (Terminal.app) still skews for itself — tmux tracks widths
  per grid, not per client; nothing app-side can help that.

## Dead ends / gotchas

- **The Node width probe for the graphemes addon LIES about astral emoji.**
  Under Node it reports ✅=2 but 🔄=📋=1 — looks like the fix half-failed.
  Addon bug, browser unaffected: `_dec()` takes the `typeof Buffer` branch,
  `Buffer.from(b64,'base64')` returns a POOLED buffer (byteOffset 8), and
  `unicode-trie` builds its `DataView` without the byteOffset, so `highStart`
  misreads and every supplementary-plane codepoint falls to narrow. Probe
  under Node only after `delete globalThis.Buffer`.
- **`tmux send-keys` mangles pasted VS16/ZWJ sequences** — measured width 0
  for ⚠️ and 👨‍👩‍👧 until re-sent as UTF-8 byte escapes
  (`printf '\342\232\240\357\270\217'`), which measured 2. Byte escapes for
  anything beyond a lone codepoint.
- **The spec omitted `allowProposedApi` and would have bricked every pane.**
  `term.unicode` is proposed API in xterm 5.5; both new lines throw inside
  `useTerm`'s effect, before `open()`. The implementing agent caught it by
  testing against the app's exact constructor options. (Upstream's search
  addon hit the same flag for `registerDecoration` — throws only on first ⌘F.)
- **The worktree was parked 14 commits behind, on a branch whose close-out PR
  (#128) never merged.** Upstream had meanwhile added the SearchAddon to the
  exact lines this fix touches. Stash → branch off fresh `origin/main` → pop
  gave 3-way conflicts; lockfile resolved by taking main's and re-running
  `pnpm install`, never by hand-merging. And `gh pr merge --delete-branch`
  fails in a side worktree ("'main' is already used by worktree at …/worktrees")
  AFTER the remote merge succeeds — merge state must be verified, then the
  remote branch deleted manually.

## Verification

- Buffer-level: all emoji 1→2 cells with addon active (browser decode path).
- David eyeballed a live claude-in-tmux session in the sandbox app
  (`app/scripts/sandbox.sh --app`) — no garbling. Mock harness cannot show
  this bug at all: no tmux.
- Gates on the rebased base: `tsc --noEmit`, `cargo check -p app`,
  `cargo test -p app --lib` 34/34; PR CI 9/9.
- Caveat recorded: the sandbox eyeball predated the rebase onto the
  SearchAddon change. Logically independent (graphemes loads before `open`,
  search after), but ⌘F + emoji together deserves one glance post-release.

## Follow-ups

- Post-release: one glance at ⌘F highlights on emoji-bearing lines (see
  caveat above).
- Rebuild + `make install-app` to get the fix into the daily-driver app
  before the next release ships it.
