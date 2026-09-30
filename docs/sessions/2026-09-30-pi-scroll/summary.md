---
title: "Session — the wheel scrolls a pi pane"
---

# Session — the wheel scrolls a pi pane

- **Date:** 2026-09-29 → 2026-09-30
- **Worktree:** `.worktrees/pi-scroll`
- **Branch:** `pi-scroll-wheel`
- **PR:** [#373](https://github.com/penard-monkey/worktrees/pull/373), squashed as `ca4081a`. It is two commits: the fix, then the review fixes.
- **Release tag:** v0.34.0 (#379)
- **Planning files:** `planning.tar.gz` (the lane's brief, `.planning/brief.md`; no task_plan/findings/progress).

## What shipped

**Root cause.** Scrolling a pi lane did worse than nothing: it sent ↑/↓ keys into pi.
- The app's terminal is xterm.js attached to a tmux *client*. The client always draws on the alternate screen, so xterm never has scrollback of its own.
- tmux passes a pane's mouse request through to the outer terminal even with `mouse off`. Claude asks for the mouse, so xterm reports the wheel to it and Claude scrolls itself.
- pi draws on the pane's MAIN screen and never asks for the mouse. For an alternate screen with no mouse mode, xterm.js 5.5 turns each notch into `ESC [ A`/`ESC [ B` (`Terminal.ts:809`).
- pi reads ↑ as editor history, and a shell in the main pane reads it as history recall. pi's real history is in tmux's scrollback, where nothing could reach it.
- Measured on a throwaway tmux server: three notches over a `seq 1 300; cat -v` pane arrived as `^[[A^[[A^[[A`.

**Fix.**
- `app/src/TerminalPane.tsx`: `attachCustomWheelEventHandler`. When xterm has no mouse mode and is on the alternate buffer, the wheel goes to the transport's `wheel`. Only the tmux transport has one; the dock's owned shells keep xterm's own scrollback.
  - `wheelLines` turns deltas into lines, using the cell height of `.xterm-screen`.
  - `wheelPump` keeps one invoke in flight and sums the rest into the next.
  - A failure is logged as a warning once, until a scroll succeeds again.
- `app/src-tauri/src/lib.rs`: `term_wheel` resolves `=session:` to `#{pane_id} #{alternate_on} #{pane_mode}`. `wheel_plan` then picks one of:

  | Pane | Plan |
  |---|---|
  | copy-mode or view-mode | scroll |
  | main screen, wheel up | `copy-mode -e` + `scroll-up` |
  | main screen, wheel down | nothing |
  | alternate screen, or a menu mode (choose-tree) | `send-keys Up/Down` |

- `Term` now records the session and a `scrolled` pane.
  - `term_write` runs `send-keys -X cancel` before the first keystroke after a scroll. This happens under the terminals lock, so no later key can land in copy-mode first. Esc is swallowed if it cancelled.
  - `term_open` seeds `scrolled` from `#{pane_mode}`, because copy-mode outlives the client.
- `crates/worktrees-core/src/tmux.rs`: `leave_mode` puts `copy-mode -q -t <pane> ;` at the head of `send_literal_args`, `press_enter_args` and the paste in `paste_commands`. That covers Codex/pi `send`, the drag drop and the Plan-tab prompt.
- `test/helpers/common.bash`: the fake tmux logs that head as its own line, then handles the rest. `test/mcp.bats` asserts the leave is logged directly before both the typed line and the Enter.
- `app/scripts/termwheel-check.mjs`: evaluates the real `TerminalPane.tsx` under stubs.

## Decisions

- **Per-wheel tmux commands, not `mouse on`.** `mouse on` would move text selection into tmux copy-mode and away from xterm's native selection and the clipboard path. brethash's #357 proposes it and stays open as a possible later layer.
- **The decision is made in Rust from one `display-message`, then one action invocation.** A single nested `if-shell` would need quoting of session names inside tmux strings: `(main)`, `~agent~pi`. Two spawns and a pure `wheel_plan` are testable. The target is `=name:` (exact), never a bare name, which prefix-matches.
- **An alternate-screen pane with no mouse still gets arrows**, through `send-keys` so they use the pane's own cursor-key mode. That is what xterm sent before, and such a pane (less, vim, an alt-screen Codex) has no tmux history to scroll. The routing is on `#{pane_mode}`, not `#{pane_in_mode}`, because choose-tree is a mode too and `scroll-up` fails there with "not in a mode".
- **Typing returns to live output; Esc while scrolled back is swallowed.** Esc is pi's and Claude's abort key, and "stop reading" should not interrupt the agent.
- **New output does not yank a scrolled-back view.** tmux keeps it anchored on what you are reading (measured).
- **Every write into an agent's pane leaves copy-mode first, in the same tmux invocation.** `-q` exits 0 when there is no mode, which matters because a `;` list stops at the first failing command.

## Dead ends / gotchas

- **The review's xterm claim was wrong, and acting on it would have broken Claude.** It said xterm 5.5 never reaches the custom wheel handler while wheel reporting is on, citing `Terminal.ts:802`. That early return belongs only to the arrow-making listener. xterm registers a second listener (`requestedEvents.wheel`), whose `sendEvent` → `case 'wheel'` (`Terminal.ts:642`) asks the handler first. A handler that returned `false` without checking the mouse mode would swallow Claude's scroll. The comment now cites both lines.
- **copy-mode outlives the tmux client.** The first cut kept the scrolled flag per `Term`. Scroll back, switch place, come back, and every key was a copy-mode command (Space selects, Enter copies and leaves, the text is lost). This is fixed by seeding at attach. The probe reproduces the detach.
- **`send-keys -l` into a pane in copy-mode is dropped**, while `paste-buffer -p` reaches the program behind the history. Scrolling in the app made this reachable for every orchestrator `send`.
- **The flag has to be set on both sides of the spawn.** A key that lands while copy-mode is being entered takes the flag. Its cancel may run before copy-mode is in, and the pane would then sit in history with no flag.
- **`#{pane_mode}` is empty outside a mode.** Put it LAST in the format; in the middle it shifts the whitespace-split fields.
- **tmux holds a lone Esc for `escape-time`** (500ms under `-f /dev/null`). A probe that drains for 300ms reads "Esc not delivered".
- **A fresh worktree's `pnpm install`**: corepack's pnpm 12.8.1 was missing its `bin/pnpm.cjs`, and `nvm use 22.23.2` (AGENTS.md) is not installed on this machine. The cached `~/.cache/node/corepack/v1/pnpm/11.5.2/bin/pnpm.cjs` on node 22.19.0 worked.
- **zsh `=word` expansion** turns an `echo ======` or a `t=…` helper line into "not found". Put probes that need bash in a `#!/bin/bash` file.

## Verification

- `termwheel-check.mjs` fails on the pre-fix file with 11 failures, including arrows reaching the program. It also fails on each of these mutations:
  - a handler that ignores the mouse mode;
  - a pump without the in-flight guard;
  - host-based cell height;
  - an error per flush.
- Rust tests, each red under a mutation back to the old rule:
  - app crate: `wheel_scrolls_history_only_where_there_is_history` and `attach_to_a_scrolled_back_pane_arms_the_cancel`;
  - core: `a_send_leaves_copy_mode_first`;
  - the mcp.bats leave assertion, against a release binary whose Enter skipped it.
- A throwaway-server probe (`tmux -L piscroll-probe -f /dev/null`, killed afterwards) mirrors the Rust argv and passed 27/27. It covers:
  - scrolling, the `-e` exit, and a prefix-sibling session left untouched;
  - typing and Esc after a scroll, a stale flag, and anchored output;
  - detach and re-attach with the seeded flag;
  - send and paste into copy-mode, before and after the fix;
  - choose-tree;
  - Claude-shaped and less-shaped panes.
- Gates after the review fixes:
  - bats 1..395, 0 not ok; lint 0
  - core 563 passed; cli 37 passed; app 165 passed, 1 ignored
  - tsc and `cargo check -p app` clean; `make test-frontend` 25/25
- CI run 36732928251: 9/9.
- **Not done:** the real app. David's running app was off limits, and no sandbox run was made.

## Follow-ups

In ROADMAP:
- A hand test in the real app, including switch-and-return and a send into a scrolled-back pane.
- Split panes scroll the active pane, not the pane under the pointer.
- #357 (`mouse on`) as a possible later layer.
