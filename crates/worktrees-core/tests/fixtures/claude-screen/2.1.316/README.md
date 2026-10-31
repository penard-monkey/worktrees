Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.316** (native install, macOS 27.0, Haiku 5.5,
`--permission-mode default`, `--name wtph-probe`) on 2026-10-31, for
`prompt::clipped` and `pending` — a prompt taller than its pane, and what
Claude's PermissionRequest hook is handed while it is up
(`feat/actionable-prompts`, step 3).

As in `2.1.314/`: a PRIVATE tmux server (`-L wtph`), a scratch git repo
under `/private/tmp/claude-501/wtph/`, every `CLAUDE_*` variable of the
capturing session unset, trailing blank rows removed, every visible
character verbatim. Claude ran with a throwaway `--plugin-dir` whose
`PermissionRequest` and `SessionEnd` hooks wrote their stdin to a file.

Screens:

- `permission-edit-tall.txt` (120×40): an Edit replacing lines 20–60 of an
  80-line file. The modal's top and the diff's head are gone; the rows on
  screen are the diff's tail (`  32 +row 32 changed` …), the dashed rule,
  `Do you want to make this edit to big.txt?`, the three choices, the footer
  and the name tag.
- `plan-approval-tall.txt` (120×40): ExitPlanMode with a 45-step plan. The
  sheet's top (`▔`) is there; the plan is clipped inside its box (one dashed
  edge, `↓` at the last row shown), footer `ctrl+g to edit in Vim ·
  ~/.claude/plans/plan-adding-a-script-shimmying-glade.md`.
- `permission-bash.txt` (120×40): a 50-line `printf` command that fits.
  **New in 2.1.316:** a multi-row command is drawn with a `│ ` gutter.
- `permission-bash-tall` (+ `.e`): the SAME live prompt after `tmux
  resize-window -y 18`: Claude re-lays it out, the title and description are
  gone, the command is whole between its two dashed edges.
- `permission-bash-taller` (+ `.e`): `-y 13`: the command's head is gone too;
  the rows on screen are its tail.
- `plan-approval-tall-80` (+ `.e`), `plan-approval-80` (+ `.e`): a 30-step
  plan in a lane `worktrees new --ai claude --no-attach` launched (an
  80×24 detached pane; `--name wtph-probe` and worktrees' plugins), first
  clipped, then whole after `resize-window -y 60`. **At 80 columns the
  footer wraps**: `ctrl+g to edit in Vim ·` on one row and the plan's file
  on the next. Until `prompt::plan_footer`, no plan read in such a pane,
  whole or clipped; found end to end, not from a fixture.

Hook payloads (`/Users/<name>/` rewritten to `/Users/u/`, nothing else):

- `hook-edit.json`, `hook-bash.json`, `hook-exit-plan.json`: the
  `PermissionRequest` stdin for the three prompts above, each written while
  its prompt was up. Fields: `session_id`, `transcript_path`, `cwd`,
  `scratchpad_dir`, `prompt_id`, `permission_mode`, `effort`,
  `hook_event_name`, `tool_name`, the WHOLE `tool_input`, and
  `permission_suggestions` (Edit's `setMode acceptEdits`). ExitPlanMode's
  input is `{plan, planFilePath}`; `plan` is the file's text but for one
  trailing newline.
- `hook-session-end.json`: `/exit`'s `SessionEnd` (`reason:
  prompt_input_exit`).
- `hook-exit-plan-80.json`: the 80-column plan's input as worktrees' own
  hook KEPT it (`pending`'s record, put back in the stdin's shape: only
  `session_id`, `hook_event_name`, `tool_name`, `tool_input`).

What was measured, and the code relies on:

- The hook fires while the prompt is up, once per prompt, and Claude draws
  the prompt as usual when the hook exits 0 with nothing on stdout.
- The probe file's `sessionId` (`~/.claude/sessions/<pid>.json`) is the
  hook's `session_id`, which is how serve finds a lane's records.
- A `--plugin-dir` hook with no `matcher` fires for every tool's prompt
  (Edit, Bash, ExitPlanMode here; AskUserQuestion on 2.1.314).
- The transcript still has no pending tool call while the prompt is up, as
  on 2.1.314.
