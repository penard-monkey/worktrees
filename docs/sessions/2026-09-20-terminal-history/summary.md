---
title: "Session: a shell tab keeps what it was showing, and what it ran"
---

# Session: a shell tab keeps what it was showing, and what it ran

- **Date:** 2026-09-20 (work started 2026-09-19)
- **Worktree:** feat-terminal-history
- **Branches:** `feat/terminal-history` (merged), `feat-terminal-history-close-out` (this archive)
- **PRs:** [#312](https://github.com/penard-monkey/worktrees/pull/312) — squashed to `95aaaf2`; this archive PR
- **Release:** none — on main, unreleased
- **Planning files:** none (single-thread session)

## Where this started

> "investigate how to implement persistant terminal window history. shoudl
> account for each named terminal tab per place."

A dock tab already remembered its name, its position in the strip and the
directory it was left in — and then opened blank, because the PTY is the app's
own and dies with it. `lib.rs` stated the scope cut in as many words: *"We
remember the DIRECTORY of each tab — nothing else: no history, no scrollback,
no environment."*

## What shipped

**Scrollback that survives a restart** (`app/src-tauri/src/lib.rs`). One
directory per tab under `<app config>/term-history/<slug>-<index>-<hash>/`
holding `meta.json`, `scrollback.bin` and `zdotdir/`. Its own tree for
`shell-cwds.json`'s exact reason: `ui-state.json` is frontend-owned and written
whole. Written on the existing 15s cwd tick (dirty-gated), at exit, and once
more when a tab restarts. Retention mirrors the cwd map — dropped on an
explicit close and on `remove_place`, kept on `close_place` — plus a 30-day
horizon, because this tree holds CONTENT and nothing else bounds it.

**Per-tab command history via `ZDOTDIR`** (same file). Each tab gets a
generated ZDOTDIR whose four shims source the user's own rc files.

**Two fixes found on the way** (`app/src/TerminalPane.tsx`). xterm was on its
default 1000 lines of scrollback while the backend replays up to 256K — ~3200
lines at 80 columns — so the oldest two thirds of every replay were already
being dropped; now `TERM_SCROLLBACK = 5000`, mirror-checked against
`SHELL_RING`. And a replay laid out for another width stacked a fragment of
every full line: the pane now hands the recording its recorded width and puts
its own back, so xterm reflows it.

**A tab-restore bug that pre-dated all of it** (`app/src/App.tsx`). Reported
mid-session — *"when I close the worktrees app and reopen it, my named tabs are
missing"* — and unrelated to the feature. See Dead ends.

**Tests and harness.** 11 new in `cargo test -p app --lib`; new cases in
`termreplay-check.mjs` and `termresize-check.mjs`; new
`app/scripts/tabsrestore-check.mjs`. The mock gained a faithful ring model, a
`restoreScrollback` driver and `?slowsettings=<ms>`.

## Decisions

**`ZDOTDIR`, not `HISTFILE`.** macOS's `/etc/zshrc:16` sets
`HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history` unconditionally for every interactive
zsh — measured: export `HISTFILE`, ask an interactive shell, get
`~/.zsh_history` back. `ZDOTDIR` is the one lever that file honours. Same family
as the OSC 7 note that ruled out shell integration for the working directory.

**The shims hand `ZDOTDIR` back twice.** Once in `.zshenv`, because a
`~/.zshenv` that does `export ZDOTDIR=$HOME/.config/zsh` — the standard dotfiles
layout — would otherwise hijack the rest of the chain and silently put the tab
back on the global history; once in `.zshrc` before sourcing theirs, because a
config that does `source $ZDOTDIR/aliases.zsh` has to find its own files. The
second also leaves child shells on the normal history.

**`meta.json` is the identity, not the filename.** The directory name is an
FNV hash, so a collision or a stale directory reads as "not mine" rather than as
someone else's output.

**Flush on a tick, not on `shell_detach`.** A tab flip is not worth a 256K
write, and detach runs under the registry lock, which may never wait on a file
lock.

**Reflow only where the terminal is fresh.** The replay is always the first
thing written to a new xterm, so moving the grid can damage nothing. The
temporary width reaches neither `tx.resize` nor `sentRef`, so the pty never
takes a SIGWINCH for a size nobody is looking at.

## Dead ends / gotchas

**The partial-line trim compounded.** The ring drains from the front, so its
first bytes are routinely mid-escape-sequence — but a ring BELOW the cap begins
where the shell began, and trimming that drops a real line on every save. Worse,
it compounds: the next restore seeds from the trimmed copy and the save after
that takes the next line. One line per restart, off the oldest end, forever.
Caught by reading back my own code, not by a test. `rolled_ring` now trims only
at the cap.

**The seam was dead on arrival.** It was deliberately kept OUT of the ring, on
the theory that N restarts would stack N markers. But a re-attach replays the
ring and nothing else — so the seam was visible only until the first tab flip,
and under `React.StrictMode` the pane re-attaches before anyone has seen
anything. The mock reproduced it exactly. It is now seeded into the ring, which
makes it persistable, which is why it carries an ABSOLUTE local time rather than
an age that would freeze at "2h ago" — and why `trim_trailing_seam` replaces a
seam nothing was typed under. That trim must test the buffer's END (a seam is
fixed width) rather than search a window, or it eats a boundary that had a
session's work under it; a test caught that second mistake.

**The phantom-tab bug that was my own test harness.** Driving the mock through
add → rename → restart showed a spurious `sh 3` stealing focus on every
restart, accumulating into `term_tabs` — reproducible in a PRODUCTION build, so
not a StrictMode artifact. It was the test: the selector matched buttons by
`/terminal/i` on the title, and `.termtab-add` is titled "new terminal (⌘T)", so
once the dock was already open on restart the script clicked **+** and then
blamed the app. Found by logging `new Error().stack` in `addTab`, which said
`executeDispatch` — a click, not an effect. **A repro that accuses the code
should be made to name its caller before it is believed.**

**The xterm crash that was not ours.** `TypeError: … 'this._renderer.value.dimensions'`
appears 123 times in `app.log` and in the browser harness. Tempting to attribute
to the new reflow, which calls `term.resize` from a write callback. The log
settles it: first occurrence `2026-09-19 05:01:55Z`, ~13 hours BEFORE this
branch's code first ran (`17:57:57Z`, the `per-tab history: fish` line), with
coverage back to 2026-08-01. Pre-existing, still unexplained.

**A killed vite kept serving the pre-rebase file.** After a rebase, the harness
answered on the same port with content from before it — a survivor of an earlier
run that the first kill missed. This is the trap CLAUDE.md already documents
(check CONTENT, not the port); it nearly produced a "the mock is broken"
diagnosis. Killing the listener AND its parent, then re-verifying by `curl |
grep` for something the edit added, is what settled it.

**`$SHELL` is fish on this machine, and the probe said zsh.** The interactive
probe ran under the tool's shell and reported `/bin/zsh`; the app resolves
`std::env::var("SHELL")` from its GUI launch context and gets fish. So the
per-tab history half of the feature does nothing here — it logs and skips, by
design, but the whole ZDOTDIR design was built for a shell this user does not
use interactively.

**A batch break-verification harness reported "no teeth" falsely.** Looping
apply-break → run test → restore over several breaks said one test still passed
when broken. Run by hand, the same break failed it correctly. Cause not pinned
down (most likely mtime granularity letting cargo reuse a stale build). **A
negative result from a test-the-tests harness deserves the same scepticism as a
positive one.**

## The tab-restore bug (reported mid-session)

Symptom: reopen the app, and the strip collapses to a single unnamed `sh 1`
while `ui-state.json` still holds `term_tabs [1,2]` and the name.

The settings layout effect AWAITS an invoke. A place entered before it resolves
reads DEFAULTS' empty maps, unions nothing with the (also empty, post-restart)
live shell list, and falls back to `sessionUp ? [1] : []`. Two things then hid
it: the restore deliberately never writes its fallback back, so the real strip
stayed on disk correct and unread; and nothing re-ran the restore once settings
landed.

`hydratedTick` already existed for exactly this — added with the Files viewer's
restore (#302), carrying a comment describing this failure verbatim. It had ONE
consumer. Now two.

No existing test could see it: the mock answers `get_settings` in a microtask,
so the window does not exist there. `?slowsettings=<ms>` opens it.

## Verification

- Full gates green on the merge base: bats 0 not-ok, core 388, cli 14, app 135,
  `tsc` clean, `cargo check` no new warnings, **19 guard scripts**.
- CI on #312: 9/9 across macOS and Ubuntu.
- Every new Rust test shown to FAIL first, including a real login zsh on a pty
  with a temp `$HOME` broken three ways (no ZDOTDIR, rc not sourced, ZDOTDIR not
  handed back) — string assertions on generated shims pass just as happily if
  zsh ignores them.
- New check-script cases fail on the pre-change source;
  `tabsrestore-check.mjs` fails on both `HEAD` and `origin/main`.
- Browser probes (`playwright-core` driving installed Chrome against the mock):
  restore renders with its seam, Settings surfaces render, tabs survive four
  restarts.
- **NOT done: the real-app pass.** `app/scripts/sandbox.sh --app` was never run.
  CLAUDE.md is emphatic that the mock answers in a microtask and models no
  shells, and that three v0.12.x bugs passed every gate and were found only by
  running the app. Everything here lives in timing the harness cannot express.

## Follow-ups

- **Run the real app.** The matrix that matters: a tab with `vim` open when you
  quit (the replay-mute regression, now carried across a restart); quitting wide
  and reopening narrow; `exit` then Restart; closing a tab; arrow-up in two tabs;
  `kill -9` to confirm `INC_APPEND_HISTORY` held.
- **fish support for per-tab history.** `fish_history` in the environment
  probably selects a session file; unverified, so it was not guessed at.
- **The xterm `dimensions` crash.** 123 occurrences, pre-dates this branch,
  unexplained. An unhandled throw inside an effect unmounts the whole surface.
- **`fn claim` is never used** in `app/src-tauri/src/viewer.rs` — arrived with
  #311, not touched here.
- **The `close-out` skill had to be reconstructed.** `~/workspace/claude-skills`
  was gone and the 2026-08-10 session recorded it as local-only with no remote.
  Rebuilt from the pre-move `SKILL.md` (`ff5ebf8^`), that summary's account of
  what the move changed, and the step numbering `.claude/close-out.md` refers
  to. **Give that repo a remote.**
