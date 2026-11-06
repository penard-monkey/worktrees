Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.319** (native install, macOS 27.0, Haiku 5.5,
`--permission-mode default`, `--name wtask-probe`) on 2026-11-06, for
`prompt::call_agrees` and serve's `asked` field on `agent_prompt`
(`feat/prompt-asked`).

A PRIVATE tmux server (`tmux -L wtask-probe`, 120×45) in a scratch git repo
under `/private/tmp/claude-501/wtask/`, `DISABLE_AUTOUPDATER=1`, no
`XDG_DATA_HOME`. Claude ran with a throwaway `--settings` file whose
`PermissionRequest` hook wrote its stdin to a file (`hook-*.json`, the home
directory rewritten to `/Users/u`). Not launched with worktrees' plugin dirs:
the hook is the same event, and the screens do not depend on them.

- `permission-bash.txt` / `hook-bash.json`: `touch pc.txt && printf "%s\n"
  one two three > pc.txt`, whole, with its description above the dashed rule.
- `permission-edit.txt` / `hook-edit.json`: `hi` → `hello` in `README.md`.
- `question-first.txt`, `question-second.txt` / `hook-question.json`: one
  AskUserQuestion of two single-select questions (Colour, Size); the second
  screen is after answering Red, its tab bar `☒ Colour  ☐ Size`.
- `permission-webfetch.txt` / `hook-webfetch.json`: a WebFetch asked by a
  sub-agent (`agent_type: web-fetch`). **Nothing reads it**: it has no key
  hints, and its url row is `https://example.com/` where the call has
  `https://example.com`. So WebFetch has no agreement check.
- No WebSearch screen: with the permission mode used, the search ran
  without asking, so there was no prompt to capture.

Measured, and the reason an MCP (or any other input-less) tool carries no
`asked`: two Bash calls in ONE message, `touch pa.txt` and `touch pb.txt`.
The hook fired once, for the call whose prompt was up (14.70 s). The second
call's hook fired 0.47 s after the first was answered (34.65 s), as its own
prompt came up — not at once. So the newest record is the prompt on screen,
but only while the hook ran: a record that was not written (an input over
512 KiB, a hook that failed, a Claude started before the plugin) leaves the
last same-tool record, and an MCP tool's input is on no row to tell them
apart.
