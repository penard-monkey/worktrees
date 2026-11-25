---
title: "Workspace chats: what was measured"
---

# Workspace chats: what was measured

Claude Code 2.1.331 on macOS 27.2, interactive, in scratch folders outside
every registered project, 2026-11-24. One machine, one model (haiku), one or
two runs each unless a line says more. These decide the launch
(`worktrees_core::workspacechats`).

## `--add-dir`: not used

| Question | Answer |
|---|---|
| The added repository's `.claude/settings.json` hooks (`SessionStart`, `PreToolUse`, `UserPromptSubmit`), its `env`, `.mcp.json`, `CLAUDE.md`, a plugin its settings enable from its own marketplace | None took effect |
| Its skills, commands and agents | **Loaded**, and named to the model |
| A hook in a skill's frontmatter | **Ran**: the skill was invoked by `/name`, and the marker of its `PreToolUse` hook was there after the next Bash call |
| Folder trust | Asked again for each untrusted added directory, in every session ("Your answer holds for this session") |
| The flag's shape | Variadic: `--add-dir X 'prompt'` takes the prompt as a directory. `--add-dir=X 'prompt'` works |

A cloned repository would supply what runs (ADR 0001).

## The rules that are used instead

`--settings '{"autoMemoryEnabled":false,"permissions":{"allow":["Read(//<root>/**)"],"deny":["Edit(//<root>/**)"]}}'`

| Question | Answer |
|---|---|
| Read of the root, manual mode | No ask. No skills, commands or agents of it load |
| Edit, Write, NotebookEdit there | All refused by the one `Edit` rule; so is a path spelled in other letter case, and a root with a space in it |
| A nested `Read` deny inside an allowed root | Wins |
| The shell, `echo x > <root>/f` and `cat` of a Read-denied file | Refused |
| The shell, `python3 -c "open(<root>/f,'w')…"` (auto mode) | **Written. Other programs are not covered** |
| Read of a directory no rule names | Manual and accept-edits mode ask. **Auto mode reads it without asking** |
| Another settings source (tried: the cwd's `.claude/settings.local.json`) | Its deny narrows these rules; its allow cannot undo the deny; its allow can widen reading to directories these rules do not name |

Not tried: the user's own `~/.claude/settings.json` (the same precedence is
expected), managed settings, bypass-permissions mode.

## The rest

| Question | Answer |
|---|---|
| Folder trust for the cwd | Asked once per folder. The highlighted choice is "No, exit" |
| `claude --resume <id> '<prompt>'` | Takes the prompt |
| An environment variable set on the command line | Seen by a user-scope MCP server |
| A git repository below the cwd (`memory/`) | Not Claude's own: "Is a git repository: false", no status |
| "Write it in `memory/MEMORY.md`" | **Claude wrote its own auto memory** (`~/.claude/projects/<cwd>/memory/MEMORY.md`) and reported success, twice of twice. With `autoMemoryEnabled: false`, or `CLAUDE_CODE_DISABLE_AUTO_MEMORY=1`, it wrote the right file. A file that names the folder by its absolute path also worked |
