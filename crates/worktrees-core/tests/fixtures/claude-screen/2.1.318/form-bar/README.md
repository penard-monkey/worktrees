Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.318** (native install, macOS 27.0, Opus 5.5, auto
mode from the user's settings) on 2026-11-04, launched as worktrees launches
a lane: `claude --plugin-dir <claude-guard> --name scA-main --plugin-dir
<claude-prompt>`, in a PRIVATE tmux server (`-L wtsc1`), a scratch git repo
under `/private/tmp/claude-501/…/scratchpad/repos/scA`, every `CLAUDE_*`
variable unset and `DISABLE_AUTOUPDATER=1`. Trailing blank rows are kept as
tmux printed them; every visible character is verbatim.

The form is the owner's, re-asked: Claude was told to call AskUserQuestion
with the exact input its PermissionRequest hook had kept for the form a lane
asked on 2026-11-04 — two single-select questions (`/model`, `MCP prompts`),
the first long enough to be drawn behind Claude's dim `│ ` bar, every option
with a description of two or three rows, then `Type something.`, the list's
rule and `Chat about this`. The chat answered it `readable: false` ("This
app can't read this prompt whole") in the owner's pane.

Why: in an **80×24** pane this form is exactly as tall as the pane. Claude
keeps its footer and name tag on the bottom rows, so the modal's own top rule
scrolls off and the tab bar is row 0 — and `prompt::claude` found its top by
that rule alone. A head on row 0 (the tab bar, or a lone question's `☐
header`) is the proof nothing of the modal is cut: its rule is the only row
above it.

- `first.txt` (140×52), `first-80.txt` / `.e` (80×52): the first question
  with its rule on screen. The same form, resized live (Claude redraws it).
- `first-80x24.txt` / `.e`: the owner's shape — tab bar on row 0, no rule
  above it, 24 rows ending on the name tag. Unread before the fix.
- `first-80x20.txt` / `.e`: 80×20, the form's head and the question's first
  rows are cut. Still no modal: nothing proves what is above.
- `second-80x24.txt` / `.e`: after `agent_answer` chose 2 on the private
  server (one Down, proved, then Enter): the second question, `☒ /model`.
- `review-80x24.txt` / `.e`: after `agent_answer_text` put `leave them out
  for now` into question 2's `Type something.`: the review screen, `Submit
  answers` / `Cancel`. `agent_answer` 1 submitted it; Claude carried on.
