Captured with `tmux capture-pane -p` (and `-e -p` for the `.e.txt` files)
from Claude Code **2.1.318** (native install, macOS 27.0, Opus 5.5, auto mode
from the user's settings) on 2026-11-04, for slash commands in the chat
(`docs/proposals/chat-slash-commands.md`). Launched as worktrees launches a
lane (`--plugin-dir <claude-guard> --name scA-main --plugin-dir
<claude-prompt>`) in a PRIVATE tmux server (`-L wtsc1`, 120×40), a scratch
repo under `/private/tmp/claude-501/…/scratchpad/repos/scA` holding
`.claude/commands/hello.md`, `preexec.md` (a `` !`date +%Y` `` line with
`allowed-tools: Bash(date:*)`), `sub/nested.md` and a project skill. Every
`CLAUDE_*` variable unset, `DISABLE_AUTOUPDATER=1`. The account's email is
rewritten to `u@example.com` and `/Users/<name>` to `/Users/u`; nothing else
changed. Every line was PASTED the way `agent_send` pastes (a bracketed
`paste-buffer -p`).

The menu, before Enter:
- `menu-status`, `menu-model`, `menu-context`: the `/` menu with the exact
  match highlighted (`❯ /status`), the composer holding the line.
- `menu-nested`: `/nested` pasted — the menu highlights `/sub:nested`, a
  DIFFERENT name: Enter would run it.
- `menu-slash`: `/` alone.
- `composer-compact-args`: `/compact keep only the facts` — no menu.

What appears:
- `model-picker` (+ `-moved`, Down once): "Select model … Your pick becomes
  the default for new sessions", footer `Enter to set as default · s to use
  this session only · Esc to cancel`, the current model `2. Opus 5.5 ✔`
  highlighted, `↓ 10.` and `… +3 models` below the window.
- `model-session-only`: after `s` on the current model: `Set model to Opus
  5.5 for this session only`. `~/.claude/settings.json` kept its SHA-256 and
  mtime. Enter was never pressed on the picker.
- `status-view`, `usage-view`: the full-screen Settings dialog (tabs
  `Settings  Status   Config   Usage   Stats`, footer `Esc to cancel`) on
  its Status and Usage tabs. `status-dismissed`: after Esc — "Settings
  dialog dismissed", the composer back, nothing in the transcript.
- `context-inline`: `/context`'s output inline, the composer back.
- `busy`: a turn running (`esc to interrupt`); `busy-menu-context`: `/context`
  pasted over it; `busy-queued-context`: after Enter — queued (`Press up to
  edit queued messages`); `busy-queued-ran`: after the turn, it ran.

The lists (not screens):
- `help-commands-pages.txt`: `/help` → Commands, paged with Down at 120×60:
  108 default commands, each with its description (the source of
  `slash::CLAUDE_2_1_318`).
- `help-custom-pages.txt`: `/help` → Custom commands: user and synced skills,
  `(project)` commands, plugin skills and the user's MCP prompts
  (`/mcp__kicad__…`, no descriptions).
