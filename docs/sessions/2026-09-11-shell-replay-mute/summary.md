# Session: a dock shell's replay stops answering itself

- **Date:** 2026-09-11 (closed out 2026-09-16)
- **Worktree:** `ui-changes` (idle base `ui-changes-next`)
- **Branch:** `ui-changes-replay-mute` off a freshly fetched `origin/main`, deleted after merge
- **PR:** [#201](https://github.com/penard-monkey/worktrees/pull/201) → `4e12f34`
- **Release:** v0.23.1 (#202, same day)
- **Planning files:** none — a single-bug session, reasoning is in this file
- **Scratch:** `~/.cache/worktrees/worktrees/ui-changes/{zsh,vim}-query-probe.py` — the two pty probes that identified the asker

## Context

David's screenshot (`_tmp/Screenshot 2026-09-11 at 13.30.19.png`): the dock's
`sh 1` tab in the saywhat `(main)` place showed, on several consecutive
prompts, text he had not typed —
`2RR0;276;0c11;rgb:0f0f/0f0f/161610;rgb:c0c0/caca/f5f512;2$y` — "sometimes,
after navigating", never pinned down. One line had been executed (exit 130 =
his Ctrl-C).

## What shipped

- **`shell_open` answers `{ gen, replay }`** (`app/src-tauri/src/lib.rs`,
  `ShellAttach`): the attach generation as before, plus how many bytes of ring
  were pushed down the channel FIRST. Zero on a fresh spawn.
- **The pane mutes `onData` while the replay is parsed**
  (`app/src/TerminalPane.tsx`, `useTerm`): the channel's first message is the
  replay whenever `open` reported one, or when it lands before `open`
  resolves; `term.write(bytes, cb)`'s callback lifts the mute. A tmux pane
  reports `replay: 0` and is unchanged.
- **`app/scripts/termreplay-check.mjs`** — evaluates the real pane source
  under stubs (the `termresize-check.mjs` shape): replay before / after
  `open`, a live chunk queued behind the replay, a fresh shell, the tmux
  attach burst. Fails 6/8 on the pre-fix file, 8/8 after.
- The mock (`app/src/mock/install.ts`) and `termresize-check.mjs` follow the
  new `open()` shape.

## Diagnosis — how the garbage was read

The text decodes as terminal REPLIES, all of them things xterm.js 5.5 emits:

| fragment | reply |
|---|---|
| `2R`, `R` | cursor-position reports (`CSI row;col R`, prefix eaten by ZLE) |
| `0;276;0c` | Secondary Device Attributes — 276 is xterm.js's fixed firmware number |
| `11;rgb:0f0f/0f0f/1616` / `10;rgb:c0c0/caca/f5f5` | OSC 11 / 10 colour replies, in the app's tokyo-night terminal values |
| `12;2$y` | DECRQM reply for mode 12 (cursor blink) |

So an xterm.js instance answered a query burst, and the answers went down the
dock shell's pty as INPUT, where zsh echoed the printable tails. The burst is
vim's: the vim probe sends `ESC[6n`, `ESC[6n`, `ESC[>c`, then (once it hears the
DA2 reply) the colour and blink queries — the screenshot's order exactly. On
this machine vim is the only binary carrying the `?12$p` query; tmux 3.7b
and claude do not, and an interactive login zsh emits no queries at all. Every
`git commit` without `-m` runs vim.

The repeat is the replay ring: `shell_open` on a live shell replays 256K of
recorded output raw into a brand-new xterm on every re-attach (place switch,
dock re-open, tab flip). The recording still contains vim's queries, so each
fresh xterm re-answers them — until 256K of later output rolls them out, which
is why it came and went.

## Decisions

- **Mute replies in the frontend rather than scrub queries in Rust.** A scrub
  needs an escape-sequence parser and an allowlist of reply-soliciting
  sequences (DA1/2/3, DSR, DECRQM, DECRQSS, XTVERSION, XTGETTCAP, OSC colour
  `?`, window-op reports, kitty `?u`…) that has to stay complete; missing one
  leaves a hole. Dropping every reply while the recording parses is complete
  by construction and does not touch the live stream, so a program still
  running in the tab (vim itself) keeps getting its answers.
- **The replay is identified by channel POSITION, not by timing.** The first
  design was "anything that arrives before `open` resolves is replay". Wrong:
  Tauri 2.11's `Channel` sends a Raw payload above a size threshold through a
  separate `fetch` (`ipc/channel.rs`, `FETCH_CHANNEL_DATA_COMMAND`) that can
  land AFTER the invoke's own response. What IS ordered is the channel
  itself (`@tauri-apps/api` buffers by `index`). So the backend reports the
  replay length and the pane classifies the channel's first message.
- **The mute lifts on xterm's write callback, not on a promise.** `WriteBuffer._innerWrite`
  parses a chunk, runs THAT chunk's callback, then moves to the next — all
  synchronously within one loop pass. A microtask-based unmute could let the
  next chunk parse first.
- **When the first message beats `open`, presume replay.** The cost is the
  replies to a query in a fresh shell's first chunk, and zsh makes none.

## Dead ends / gotchas

- **`grep -c` and `strings` are only a hint.** The `strings` scan reported
  `[>c` in neither vim nor tmux (both build it from pieces); the pty probe was
  the evidence. Run programs, don't grep binaries, when the question is "what
  does it send".
- **Pre-open classification is a trap in Tauri.** See Decisions; would have
  fixed the small-ring case and left the >1KB case (every real one) broken.
- **The scratchpad is not durable.** `/private/tmp/claude-501/…` was pruned
  between the fix and the close-out five days later; the probe scripts were
  rewritten from the transcript. Anything a summary will cite goes to
  `~/.cache/worktrees/…` at the time it is made, not at close-out.

## Verification

- `termreplay-check.mjs`: 6/8 `not ok` against `origin/main`'s
  `TerminalPane.tsx`, 8/8 `ok` on the fix. `termresize-check.mjs` still green.
- `tsc --noEmit`, `cargo check -p app`, `cargo test -p app --lib` (47 passed).
- bats / lint / core+cli tests not re-run: nothing under `crates/`, `bin/`,
  `test/` changed; CI ran the full matrix on the PR, all nine checks green.
- **Not observed in this thread:** the by-hand check in the real app (run
  vim in a dock tab, `:q`, switch place and back). The mock cannot express
  the timing.

## Follow-ups

- The by-hand real-app check above, if not already done on v0.23.1.
- The tmux pane has a latent twin: `onData` is wired only after `term_open`
  resolves, so tmux's attach-time queries parsed before that get NO reply
  (no listener). Probably harmless (tmux falls back to defaults for fg/bg)
  but worth a look — in ROADMAP.
