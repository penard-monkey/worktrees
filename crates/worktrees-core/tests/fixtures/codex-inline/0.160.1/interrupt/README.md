Captured with `tmux capture-pane -p` from **codex-cli 0.160.1** on macOS
27.0, 2026-11-01, for `prompt::turn_shown` / `prompt::interrupted_shown` and
serve's `agent_interrupt` (`feat/chat-interrupt`). A PRIVATE tmux server
(`-L wtint`), an **80×24** pane, launched as worktrees launches it
(`-c forced_login_method=chatgpt --no-alt-screen`) with `-a on-request -s
read-only` so that it asks, in a scratch repository under
`/private/tmp/claude-501/wtint/`. Only trailing blank rows were removed. The
Esc was `tmux send-keys -t <pane> Escape`, ONE key.

- `idle`: the composer (`› Ask Codex to do anything`) and its status row.
- `busy`: mid-turn on `for i in $(seq 60); do echo $i; sleep 1; done`: the
  busy row `• Working (12s • esc to interrupt) · 1 background terminal
  running · /ps to vie…` sits ABOVE the composer.
- `busy-streaming`: mid-turn, streaming a long reply with no tool. **There
  is no busy row**: Codex hides it while text streams (sampled every 0.5 s:
  shown for the first 1.5 s of the turn, then gone until `Worked for 15s`).
  The screen reads exactly like an idle composer; only the rollout
  (`task_started` newest) says a turn is running.
- `interrupted`: 50 ms after the Esc on `busy`, and every capture up to 2 s:
  `■ Conversation interrupted - use /feedback if something went wrong`
  above the composer. The command Codex had started in a background
  terminal KEEPS running (`1 background terminal running · /ps to view ·
  /stop to close`): Esc ends the turn, not that terminal.
- `busy-approval`: mid-turn with an approval up (`touch b.txt` in a
  read-only sandbox): the modal replaces the composer, footer `Press enter
  to confirm or esc to cancel`. Answering it `No` wrote `turn_aborted` too.
- `busy-draft`: streaming, with `half typed draft` typed into the composer:
  the status row becomes `tab to queue message … 99% context left`.
- `busy-draft-esc`: Esc on it interrupted the turn and kept the draft.
- `idle-esc`: one Esc on the idle composer: its status row becomes `esc
  again to edit previous message` (a second Esc would open that edit) and
  stays so.

The rollout recorded every Esc as `{"type":"turn_aborted","reason":
"interrupted",…}` after the turn's `task_started`.
