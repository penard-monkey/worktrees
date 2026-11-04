Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.318** (native install, macOS 27.0, `DISABLE_AUTOUPDATER=1`,
every `CLAUDE_*` variable unset) on 2026-11-04, for the owner's bug "Not
sent: the prompt changed" on `/model` → Sonnet from the chat. Launched as
worktrees launches a lane (`--plugin-dir <claude-guard> --name 'scA-(main)'
--plugin-dir <claude-prompt>`) in a PRIVATE tmux server (`-L wtmp1`, 139×52,
the owner's size), in a scratch repo an earlier lane had trusted. The
session's effort is the user's `xhigh` (their settings), Opus 5.5 in effect.
`/model` was opened through the real `agent_command` row; the highlight was
moved with Up/Down by hand for the captures. Trailing blank rows are trimmed.

**The effort row under the list follows the HIGHLIGHT**: it says the
highlighted model's effort, not the session's. Claude draws it as
`<glyph> <Level> effort[ (default)][ ←/→ to adjust][ · Ultracode on
(/effort)]` (glyphs `○` Low, `◐` Medium, `●` High, `◉` xHigh, `◈` Max), or
`○ Effort not supported for <model>`. A `Max` row carries Claude's warning
row under it, and once the effort is lowered again the box keeps that row's
height as a blank. The `/fast` row did NOT change on any of the 13 models.
The `❯` landing on a row replaces that row's `↓`/`↑`.

- `on-current`: `/model` as it opens — `❯ 2. Opus 5.5 ✔`, `◉ xHigh effort`,
  rows 1–10 of 13 (`↓ 10.`, `… +3 models`).
- `on-fable` (3, `◉ xHigh effort`), `on-sonnet` (4, `◐ Medium effort
  (default)` — the owner's case), `on-haiku-4-5` (6, `○ Effort not supported
  for Haiku 4.5`), `on-row-10` (10: `❯ 10.` where `↓ 10.` was, the window
  unmoved).
- `scrolled-12`: Down past row 10 scrolls the window (`↑ 3.` … `❯ 12.`,
  `… +1 model` below, `● High effort (default)`): other rows, another list.
- `window-2`: back up to row 3, the window stays on rows 2–11 (`↑ 2.`, `↓
  11.`, `… +2 models`), `◉ xHigh effort`. Then `→` once: `max` (`◈ Max
  effort` + the warning row; `→` again changed nothing); `←` three times:
  `medium-toggled` (`◐ Medium effort`, no "(default)", one extra blank row
  where the warning was). A toggled effort stayed toggled as the highlight
  moved (Haiku 5.5 showed `◐ Medium effort (default)`, Fable `◐ Medium
  effort` again).
- `cancelled`: Esc — `Kept model as Opus 5.5`, effort still `xhigh`.
- `sonnet-session-only`: `agent_answer 4` (Down, Down, `s`) after the fix —
  `Set model to Sonnet 5.5 for this session only`, no "with … effort": the
  picker passes an effort only when ←/→ toggled it (2.1.318's
  `modelPicker:thisSessionOnly` → `toggled ? effort : undefined`), so the
  session runs Sonnet 5.5 at its own default (badge `◐ medium`).
- `switch-confirm`: after one exchange (prompt cache warm), `agent_answer 2`
  → `s` on Opus 5.5 opened Claude's "Switch model? / Your next response will
  be slower and use more tokens … `❯ 1. Yes, switch to Opus 5.5` / `2. No, go
  back`". Its rows are the same whether the list was answered with `s` or
  with the Enter that saves the default (Claude keeps that in state), so it
  is not read: the chat says to answer it at the host. `switch-confirmed`:
  Enter on Yes at the host — `Set model to Opus 5.5 for this session only`.
- `owner-sonnet.txt`: the owner's own capture from the sandbox (139×52,
  2026-11-04, `capture-pane -p`), after the failed answer: a five-model list
  (`2. Opus ✔`, `❯ 4. Sonnet`), `◐ Medium effort (default)`.

`~/.claude/settings.json` kept its SHA-256 and mtime through every step,
←/→ and Esc included. Nothing here pressed Enter on the model list.
