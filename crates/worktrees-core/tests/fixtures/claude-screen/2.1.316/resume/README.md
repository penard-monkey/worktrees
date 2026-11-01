Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.316** (native install, macOS 27.0,
`DISABLE_AUTOUPDATER=1`) on 2026-11-01, for `prompt::claude`'s resume
picker and serve's `agent_prompt` / `agent_answer` on it
(`fix/chat-resume-picker`).

**Launched the way worktrees resumes a lane.** A scratch repository under
`/private/tmp/claude-501/wtrp/`, this branch's CLI with its own
`XDG_CONFIG_HOME`, a PRIVATE tmux server (`TMUX_TMPDIR`, `$TMUX` unset) and
`WORKTREES_PREFIX=wtrp`, every `CLAUDE_*` variable of the capturing session
unset. `worktrees new lane --ai claude --no-attach` made the lane (80×24,
the size a detached lane gets), Haiku 5.5 in manual mode made the scratch
conversations (`Reply with only the word apple.` and so on, `/clear` between
them), and `worktrees close lane; worktrees open lane -r --no-attach`
resumed it, which runs exactly:

    claude --plugin-dir <guidance> -r --name 'wtrp-lane' --plugin-dir <claude-prompt>

Bare `-r` is Claude's interactive PICKER. Only the scratch conversations
are listed: the picker lists the folder's own, and nothing here pressed
`Ctrl+A` (all projects). Trailing blank rows removed; every visible
character verbatim.

Screens (80×24 unless noted):

- `picker` (+ `.e`): three conversations. Under the column-0 rule: `Resume
  session`, a rounded search box (`⌕ Search…`), a dim scope row `repo ·
  lane`, then per conversation a title row (`❯` in column 2 on the
  highlighted one, the title from column 4) and a dim row `10 seconds ago ·
  lane · 355.6KB` (age · branch · size), blank rows between, and the dim key
  hints, WRAPPED over three rows: `Ctrl+A to show all projects · Ctrl+B to
  only show current branch · Ctrl+W to show all worktrees · Space to preview
  · Ctrl+R to rename · Type to search · Esc to cancel`. No name tag under
  it: the picker comes before the composer.
- `picker-wide` (+ `.e`, 140×40), `picker-wide-moved` (+ `.e`): the same
  list, then Down once (the highlight on row 2). The hints wrap over two
  rows. **The resize recomputed every age** (`10 seconds ago` → `50 seconds
  ago`); moving the highlight does not.
- `picker-long` (+ `.e`): seven conversations, a window of four. The head
  becomes `Resume session (1 of 7)`; the window's last row has `↓` in column
  2.
- `picker-long-last` (+ `.e`): Down ×3. `(4 of 7)`, and the `❯` on the last
  row REPLACES its `↓`: nothing on screen says more rows are below but the
  count. The window did not scroll.
- `picker-long-scrolled` (+ `.e`): Down ×4. The window scrolled by one: `↑`
  on its first row, `(5 of 7)`.
- `picker-long-end` (+ `.e`): Down ×6, `(7 of 7)`. One more Down WRAPS to the
  top.
- `picker-search` (+ `.e`): Up from the first row. The keyboard is in the
  search field: no `❯` anywhere, no count, no scope row, and the hints are
  `Type to Search · Enter to select · Esc to clear`. Down goes back to the
  list.
- `picker-titled` (+ `.e`, `(1 of 8)`), `picker-titled-wide` (+ `.e`,
  160×40, all eight fit, no count): an eighth conversation, started with
  plain `claude` (no `--name`) on a long first message. Claude titled it
  with a summary, `Resume picker title wrapping test`; every lane
  conversation is titled by its `--name`, `wtrp-lane`. At 160 columns the
  hints' last row is `Esc to cancel` alone, which `prompt::claude`'s
  permission reader took for a permission footer before the picker was
  known: a "permission" of the first conversation's two rows.

What was measured, and the code relies on:

- Up/Down move the highlight; Enter resumes the highlighted conversation.
  Typing goes to the search field (`Type to search`), so a digit answers
  nothing here: the answer walks with arrows, as everywhere.
- **Esc EXITS Claude** (the pane falls to its shell). The picker draws no
  "new conversation" choice.
- Rows can read ALIKE to the byte (three `wtrp-lane` · `349KB`, three `1
  minute ago`): a lane's titles are all its session name. Where the window
  stands is `(k of N)` less the highlight's row.
- While the picker is up Claude HAS written `~/.claude/sessions/<pid>.json`,
  with `pid`, `cwd`, `tmux`, `name` and a fresh `sessionId` whose transcript
  does not exist — and NO `status`. worktrees' probe reader requires
  `status`, so that file is not a probe: `agent_turns` answers "the claude
  in wtrp-lane has written no session file yet".
- Answered end to end through an isolated `worktrees serve` (this branch):
  `agent_prompt` read the picker while `agent_turns` still said "no session
  file yet"; `agent_answer` choice 2, then choice 4 (a window's last row,
  `↓` under it), each `confirmed`, resumed that conversation; `agent_turns`
  read it about half a second later (not on the read straight after).

Not captured: the question Claude can ask AFTER a pick, for an old, large
conversation — a dialog titled `This session is <age> old and <tokens>
tokens.`, choices `Resume from summary (instant, recommended)` (or `…
(recommended)`), `Resume full session as-is`, `Don't ask me again`. It is in
2.1.316's code behind a server-side flag, then
`CLAUDE_CODE_RESUME_THRESHOLD_MINUTES` (default 70) and
`CLAUDE_CODE_RESUME_TOKEN_THRESHOLD` (default 100000). With both thresholds set to 1 on this account it did not appear, so
the flag is off here, and no reader is built for a screen nobody has seen.
