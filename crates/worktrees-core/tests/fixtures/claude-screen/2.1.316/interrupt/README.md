Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.316** (native install, macOS 27.0, Haiku 5.5,
`--permission-mode default`, `--name wtint-probe`, `DISABLE_AUTOUPDATER=1`)
on 2026-11-01, for `prompt::turn_shown` / `prompt::interrupted_shown` and
serve's `agent_interrupt` (`feat/chat-interrupt`).

As in `../`: a PRIVATE tmux server (`-L wtint`), an **80×24** pane (the size
`worktrees new --no-attach` gives a lane), a scratch git repository under
`/private/tmp/claude-501/wtint/`, every `CLAUDE_*` variable of the capturing
session unset, trailing blank rows removed, every visible character verbatim.
The Esc was `tmux send-keys -t <pane> Escape`, ONE key, as `tmux::press`
sends it.

Screens:

- `idle`: the composer of a fresh session (`Try "…"` placeholder, dim in
  `.e`). Status row `⏸ manual mode on · ? for shortcuts · ← 1 agent`.
- `after-turn`: the same once a turn has finished (`✻ Churned for 11s ·
  done 12:34 AM` above the box).
- `busy-bash`: mid-turn, running `for i in $(seq 60); do echo $i; sleep 1;
  done` in Bash (allowed at its prompt). The spinner (`✢ Cascading… (7s ·
  ↓ 129 tokens)`) is ABOVE the box; the busy hint is on the status row
  BELOW it: `⏸ manual mode on · esc to interrupt · ← 1 agent`.
- `busy-text`: mid-turn, streaming a long reply with no tool. No spinner is
  on screen at all; the status row's `esc to interrupt` is the one witness.
- `busy-permission`: mid-turn with the Bash permission prompt up (the same
  command, before it was allowed). The modal replaces the composer: no busy
  hint anywhere, and Esc there would cancel the prompt (`Esc to cancel`).
- `busy-after-esc`: 50 ms after the Esc on `busy-bash`: still the busy
  hint. The interrupt is not instant, so it must be WAITED for.
- `interrupted`: 100 ms after it, and every capture up to 2 s:
  `⎿  Interrupted · What should Claude do instead?` as the newest row above
  the box, and the status row back to `? for shortcuts`.
- `interrupted-text`: a text-only turn interrupted the same way (the mark
  under the half-written reply).
- `busy-draft`: mid-turn with `half typed draft` typed into the composer
  (`send-keys -l`, no Enter). **The status row drops every hint**: it reads
  `⏸ manual mode on` alone, so the screen cannot say a turn is running.
  Claude 2.1.314 does the same (`../../2.1.314/composer-busy-typed.txt`).
- `busy-draft-esc`: Esc on it: the turn IS interrupted (the mark), and the
  draft is kept.
- `idle-draft`, `idle-draft-esc`: an idle Claude with that draft; one Esc
  arms `Esc again to clear` (right-aligned above the box) and clears
  nothing. A second would clear the draft.
- `idle-esc`: one Esc on an empty idle composer: nothing changes. (Two open
  the rewind menu.)

What was measured, and the code relies on:

- The busy hint `esc to interrupt` is drawn on the row(s) under the
  composer box's closing rule, where history cannot reach; history CAN quote
  it (these very fixtures, a diff of `prompt.rs`), so it is read only there.
- The interrupt took 50–100 ms in four tries (Bash loop, text, text with a
  draft), and the screen did not change again in the 2 s after.
- The transcript records the interrupt as a user entry,
  `[Request interrupted by user]`, or `[Request interrupted by user for
  tool use]` when a tool was running (`turns::is_prompt` skips both).
