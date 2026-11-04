---
title: "Proposal — slash commands in the chat view"
---

# Proposal — slash commands in the chat view

**Status:** measured and built 2026-11-04 on `feat/chat-slash-commands`,
verified end to end on a private tmux server (§5). Not merged, not
reviewed; an automated security review of each commit found four fail-open
spots in the tiering, all closed (§2.5).

**The ask** (the owner, 2026-11-04): "I want to enable slash commands in the
chat view (e.g. /model)" — every slash command the session actually has,
on the desktop and the phone, with the same list. On the phone a command
needs `instruct` plus step-up, like a message, and is audited by name only.

**Two decisions the owner made after the measurements** (asked 2026-11-04):

1. **`/model` from the chat is for this session only.** Both harnesses' model
   pickers SAVE the pick as the default for new sessions on Enter (§1.3), and
   only their `s` key is "this session only". The chat answers the picker
   with `s`; `/model <name>` with an argument (which saves the default in
   Claude) is refused in the chat. Defaults stay a terminal act.
2. **MCP prompts are left out.** No file lists a session's MCP prompts, and
   the one listing API ran an untrusted repository's hook (§1.5).

## 1. Measurements

A PRIVATE tmux server (`-L wtsc1`), scratch repositories, every `CLAUDE_*`
variable unset, `DISABLE_AUTOUPDATER=1`. Claude Code **2.1.318** launched as
worktrees launches a lane (`--plugin-dir <claude-guard> --name <session>
--plugin-dir <claude-prompt>`); codex-cli **0.160.1** launched as worktrees
launches it (`-c forced_login_method=chatgpt -c project_doc_fallback_filenames
--no-alt-screen`) in a scratch path an earlier lane had already trusted, so
`~/.codex/config.toml` was never written. Fixtures:
`crates/worktrees-core/tests/fixtures/claude-screen/2.1.318/slash/`,
`…/codex-inline/0.160.1/slash/`, `…/transcripts/claude-2.1.318-commands.jsonl`
(their READMEs say what each is).

### 1.1 What typing does

| | Claude 2.1.318 | Codex 0.160.1 |
|---|---|---|
| Paste `/status` | the `/` menu opens, the exact match highlighted | its popup opens, the exact match highlighted |
| Enter on it | runs THAT command | runs it |
| Paste `/nested` (not exact) | the menu highlights `/sub:nested` — Enter would run another name | — |
| With arguments (`/compact keep …`) | no menu (a space ends it); Enter runs it | runs it |

So a name is sent only exactly as the session lists it, and the send proves,
on a fresh capture before Enter, that the composer holds exactly the line and
that the menu (if up) highlights exactly that name.

### 1.2 What appears, and how it goes away

| Command | Claude 2.1.318 | Codex 0.160.1 |
|---|---|---|
| `/model` | picker "Select model", footer `Enter to set as default · s to use this session only · Esc to cancel` | picker "Select Model and Effort" (`enter select · esc back`), then "Select Reasoning Level for X" (`enter default · s session · esc back`) |
| `/status` | full-screen **Settings** dialog, tabs Settings · Status · Config · Usage · Stats; Esc → "Settings dialog dismissed" | inline history cell |
| `/usage` | the same dialog on its Usage tab | picker: View analytics · **Redeem reset** (spends a usage reset) |
| `/config` | the same dialog on its Config tab | — |
| `/context` | inline output, composer back | — (no such command) |
| `/compact [text]` | a turn ("Compacting conversation…"), then "Compacted" | a turn, "Context compacted" |
| `/clear` | no confirm; screen cleared; the probe's `sessionId` changes | `/clear` and `/new`: picker "Where should the new conversation run?" — Current checkout · **New worktree** (a Codex-managed checkout) |
| `/help` | full-screen dialog, tabs General · Commands · Custom commands | — |

### 1.3 The `/model` trap, measured without writing anything

Claude's picker says it on screen: "Your pick becomes the default for new
sessions." The binary agrees (2.1.318's strings): Enter calls the picker's
`onSetDefault` and writes `userSettings.model` ("…and saved as your default
for new sessions"); its `s` binding is `modelPicker:thisSessionOnly`; and
`/model <name>` with an argument saves the default in any local interactive
session. Codex's effort step says `enter default · s session`, and its
strings hold "Failed to save default model". So Enter was never pressed on
either picker. `s` on the model already in effect: Claude "Set model to Opus
5.5 for this session only", Codex "Model changed to gpt-6-sol medium for
this session only"; `~/.claude/settings.json` and `~/.codex/config.toml` kept
their SHA-256 and mtime throughout.

One more trap: a Codex model with a single effort level skips the effort
step, so Enter on it in the FIRST step would apply it and save it. The levels
are in `~/.codex/models_cache.json` (every listed model has five or six
today); a model there with fewer than two, or missing, is host-only.

**Claude's effort row follows the highlight** (found by the owner,
2026-11-04: picking Sonnet from the chat said "the prompt changed"; fixtures
`claude-screen/2.1.318/model-effort/`). The row under the list —
`◉ xHigh effort ←/→ to adjust` — is the HIGHLIGHTED model's effort: `◐
Medium effort (default)` on Sonnet 5.5, `○ Effort not supported for Haiku
4.5`, and a `◈ Max` row carries a warning row under it. The first e2e picked
the model in effect, where the row stays put, which is why it passed. `s`
passes an effort only when ←/→ toggled it, so a pick from the chat runs the
new model at the effort Claude gives it (Sonnet 5.5: `medium`, its default)
and says only "Set model to Sonnet 5.5 for this session only"; nothing the
chat sends can toggle it (`tmux::Key` has no ←/→). ←/→ and Esc write nothing
(settings hash and mtime kept).

**In a conversation the prompt cache holds, `s` on another model is not the
end.** Claude asks "Switch model? Your next response will be slower and use
more tokens … `1. Yes, switch to <model>` / `2. No, go back`". Its rows are
the same whether the list was answered with `s` or with the Enter that saves
the default, Claude keeping which in its own state, so the chat does not read
it (`readable: false`, answer at the host). Yes at the host after the chat's
`s`: "Set model to Opus 5.5 for this session only", settings unchanged.

Codex's effort list depends on the model picked, but it is a prompt of its
own, read after the model list's Enter, so the chat shows that model's
levels. Only its hints follow the highlight: `enter select · esc back` on
`More reasoning…`.

### 1.4 What the transcript or rollout records

Claude 2.1.318 (`transcripts/claude-2.1.318-commands.jsonl`):

- A local command with output (`/context`, `/clear`) is now a **`type:
  "system", subtype: "local_command"`** pair — `<command-name>…` and
  `<local-command-stdout>…` (with ANSI) — plus an `isMeta` user copy written
  for the model. `turns::claude` skipped every `system` line, so the chat
  never showed these; it reads them now.
- A picker command (`/model`, `/compact`) is still a `user` pair, as in
  2.1.316.
- A prompt command (`/hello world`) is a `human`-origin user entry whose
  wrapper puts `<command-message>` BEFORE `<command-name>`; the human-origin
  rule took it for a person typing XML. It reads as the command now. Its
  expansion (the file's text, a `` !` `` line's output included) is an
  `isMeta` entry and stays unread.
- A dismissed dialog (`/status`, `/help`) writes nothing.

Codex 0.160.1: `/compact` writes `compacted` and a `ContextCompaction` item;
a model change writes `thread_settings_applied`; `/status`, `/usage` and the
command lines themselves are written nowhere. So Codex's inline output is read
from the pane once, right after the command, and handed back with the send's
answer (§2.2).

### 1.5 Which commands a session has

- **Built-ins** come from the binary, not from files. Measured lists:
  Claude 2.1.318's `/help` → Commands (108, descriptions on screen); Codex
  0.160.1's popup (53). They are tables in `worktrees_core::slash`, keyed by
  the version they were measured on; a session on another version gets the
  nearest table and the answer says which (`measured`).
- **Claude's own files**, read the way Claude names them (checked live):
  `~/.claude/commands/**.md` and the place's `.claude/commands/**.md` (a
  sub-folder is a namespace: `sub/nested.md` is `/sub:nested`, "(project)"),
  `~/.claude/skills/*/SKILL.md` and the place's `.claude/skills/*/SKILL.md`,
  the account's synced skills (`~/.claude/skills/synced/*/manifest.json`, as
  `/anthropic-skills:<name>`), and every plugin the session runs — enabled in
  `~/.claude/settings.json` (`installed_plugins.json` gives the path) or
  given with `--plugin-dir` on its command line (read from its process): a
  plugin's `commands/` and `skills/` as `/<plugin>:<name>`. Description:
  the front matter's `description`, else the first line of the body.
- **MCP prompts** (`/mcp__<server>__<prompt>`) exist only inside the running
  Claude. Claude's SDK `initialize` request (`claude -p --input-format
  stream-json`) lists commands in 0.26 s without a model call, but in a
  scratch repository nobody had trusted it **ran that repository's
  SessionStart hook** — a cloned repo's code, started by worktrees (ADR
  0001). It also lists only non-interactive commands. Rejected; MCP prompts
  are left out (decision 2).
- Codex 0.160.1 has no user-defined slash commands (its old `/prompts:` are
  gone from the binary); skills are mentioned with `$`, not `/`.

### 1.6 While the agent is busy

Claude runs an "immediate" command at once (`/status`, `/model`: their
dialog opens over the running turn, and Esc closes it without interrupting)
and **queues** the rest (`/context`: "ctrl+x ctrl+s to send now", run when
the turn ends — a `queue-operation` in the transcript). Codex opens `/model`
at once and **refuses** others in its own words ("■ '/compact' is disabled
while a task is in progress."). Both are reported as they are: `queued` for
Claude, Codex's line as the command's `output`.

### 1.7 Project-scoped commands

A repository's `.claude/commands/x.md` is text the repository supplies, and
Claude runs it when someone picks it. It can do two things a typed message
cannot: a `` !`cmd` `` line runs a shell command BEFORE the model sees the
prompt, permitted by the file's own `allowed-tools` front matter (measured:
`allowed-tools: Bash(date:*)` and `` !`date +%Y` `` ran with no question
asked); and `allowed-tools` widens what the model may do for that turn. So
any custom command — project, user or plugin — that has a `` !` `` line or
grants tools that act (`allowed-tools` beyond Read, Grep, Glob, LS) is in
the **sensitive** tier, whatever its source, and its row says why (§2.5 has
the rest of the rule, which fails closed). worktrees never runs one by itself: listing READS files, and a
person picks the command (ADR 0001 is about argv worktrees runs; this is a
person's choice, gated like one).

## 2. Design

### 2.1 Two rows

- **`agent_commands`** (`read`, `place`, optional `session`): the session's
  commands — `{harness, session, version, measured, commands: [{name,
  description, source, plugin?, tier, confirm?, why?, hint?, args}], note}`.
  `source` is `built-in`, `user`, `project` or `plugin`; `tier` is `normal`
  or `sensitive`; `confirm` is the desktop's two-click words (every
  sensitive command, and `/clear`, `/new`, `/exit`); `args` false where an
  argument is refused in the chat (`/model`, decision 1).
- **`agent_command`** (`instruct`, `place`, `session`; plus `admin`, checked
  in the body, for a `sensitive` command — as `agent_answer_text` checks
  `answer`): `{name, args?}` → `{delivered, queued, harness, session,
  command, opened?, output?, note}`.

**Why a new row, and why it is a third path that types.** The two that type
— `agent_send` and `agent_answer_text` — both run `harness::typed_line_ok`,
whose leading-`/` refusal is the ONE rule MCP `send` shares, so agents never
drive each other's commands; it stays as it is. `agent_answer_text` types
only into a field a prompt opened. A `command` argument on `agent_send`
would put two capabilities, two audit shapes and two validations behind one
row. So `agent_command` is its own row and, said plainly, a third path that
types into Claude. It is narrow: it types only `/<name>[ <args>]` where the
name is one the session lists, the arguments pass every rule of
`typed_line_ok` but the leading-`/` one for the whole line (they still may not
start with `/ @ ! #`), and it uses the same paste, the same place lock, the
same empty-composer and no-prompt checks, and the same submit proof as
`agent_send` — plus one proof of its own before Enter (§1.1). MCP `send` is
untouched and still refuses `/`.

### 2.2 The send

1. The name must be in `agent_commands`' list for that session; `!` and `#`
   lines are refused as what they are (shell, memory), not as unknown names.
2. `sensitive` needs `admin` (a phone: admin + step-up), and so does an
   argument naming a sensitive command of the session (`/loop 5m /login`
   would otherwise run `/login` through a normal command; `/loop` is itself
   sensitive, as it repeats work unattended); `/model <args>` is refused
   (decision 1).
3. Under the place's lock: a fresh capture with no prompt up and an empty
   composer.
4. Claude: paste the line; prove the composer holds exactly it and the menu,
   if up, highlights exactly `/<name>`; Enter; confirmed when the box is back
   (run, or queued behind the turn) or a prompt or dialog is up (`opened`).
   Codex: typed as `agent_send` types it, the same proof, then ONE Enter
   and a watch for the list it opened or its composer back empty (its usual
   submit waits for the composer, which a list hides); the history rows
   under the echoed command are read once (`output`, ≤ 40 rows): `/status`'s
   cell, or Codex's refusal.
5. Audit: the NAME, and the arguments' length and SHA-256, never their text;
   a device's command stamps `last_remote_send`.

### 2.3 Pickers and dialogs are prompts

Read by `worktrees_core::prompt` and answered with the existing
`agent_answer` (no new key path; each move proved before the key that
answers):

- `picker: "model"` — Claude's "Select model", Codex's first step. Claude's
  answer key is `s` (session only), pressed only while its footer shows `s to
  use this session only`; Codex's first step is Enter (it opens the effort
  step), and a model without several effort levels is host-only. What
  follows Claude's highlight — a row's `↓`/`↑` and the effort row, with a
  Max row's warning and the blank rows under it — is compared by its shape
  alone in the proof before `s`, as a permission's footer is; every other
  row (the head, the `/fast` row) is compared as drawn.
- `picker: "effort"` — Codex's second step, answered with `s`; read with
  its `enter select · esc back` hints only while `More reasoning…` (the
  terminal's) is lit.
- `picker: "view"` — Claude's Settings dialog (`/status`, `/usage`,
  `/config`, `/stats`) and `/help`: its visible rows and ONE choice, Close,
  which is Escape.
- Codex's `/new` picker (New worktree is host-only) and `/usage` picker
  (Redeem reset is host-only: it spends an account's reset).

Each is `kind: "question"`, so a client that knows none of this still shows
a question with its choices. Anything else a command opens (`/mcp`, `/hooks`,
`/permissions`, …) is a prompt the host cannot read: `agent_prompt` says
`readable: false` with its rows, as today, and it is closed at the host.

### 2.4 The app

Typing `/` in the composer opens a menu of `agent_commands`, filtered as you
type (name first, then description), each row with its description and
source. ↑/↓ and Enter pick; Esc closes (`useEscape`). A picked name fills the
field (`/compact `) and Enter sends it with `agent_command`. A `confirm`
command asks twice: the button turns into "Run /clear — starts a new
conversation" for four seconds. A picker or dialog shows as the prompt card;
output shows as a Command notice; a refusal says why on the composer's line.

### 2.5 Tiers fail closed

The front matter is read by a small reader, not a YAML parser, so whatever
it cannot read makes a command sensitive, never normal: a file past the 32
KiB the host reads, front matter that never closes (only a column-0 `---`
closes it), rows that are no `key: value`, a nested block, a key given
twice, an indented `---` inside, `allowed-tools` in any form but a list of
readers, and any key outside `name`, `description`, `argument-hint`,
`user-invocable`, `disable-model-invocation`, `when_to_use`, `license`,
`version` (`hooks`, `model`, `context`, `agent`, … alike). The shell scan
reads the whole file, `` !` `` and a fenced ` ```! ` block alike. A name two
sources give (the user's and the repository's, or a file and a built-in) is
listed once, with the MOST sensitive tier any of them has: which one the
harness runs is its rule, and a guess here must not lower the bar. A place's
folders are read with no link followed at any component (a planted
`.claude/commands/x.md -> ~/.ssh/id_ed25519` would otherwise send the key's
first line to a phone as a description).

## 3. Threat model: what a paired phone can now do

With `instruct` (and step-up per connection, and the DRIVE budget): run a
`normal` command of the session — compact, clear (its confirm is the
client's), switch the model for this session only, read status, context and
usage dialogs and close them, run a skill or a command whose file runs no
shell and grants no tools. It could already send any line; what is new is
reaching the composer's command parser, which is why the list, not the
client, decides what runs.

With `admin` too: the sensitive tier — account, login, permissions, sandbox,
config and settings writers, MCP, hooks, plugins, directories, memory files,
remote, cloud and background work, and commands that run shell or grant
tools. Most of these open dialogs the host cannot read, so a phone can open
them but not drive them; the confirm on the desktop and admin on the phone
are for the ones that act at once.

Never, from anywhere: `!` (shell mode) and `#` (memory), which stay refused;
an MCP `send` of `/`; a default model written from the chat; Codex's
"New worktree" or "Redeem reset" from a device.

## 4. Not done

- MCP prompts (decision 2).
- Codex inline output is read once from the pane, not from the rollout: a
  reload of the chat does not bring it back.
- Built-in tables are per measured version; a newer Claude's new built-ins
  are missing until measured, and a removed one is refused by Claude itself.
- Dialogs other than Claude's Settings and help (`/mcp`, `/hooks`,
  `/permissions`, `/agents`, …) are prompts this host cannot read: the chat
  shows their rows and they are closed at the host.

## 5. How it was verified

- Rust, each shown red first (a mutation, then restored): the tables and
  the name rule, the argument rule, the file enumeration and its link
  refusal, the fail-closed tiers, the readers against every fixture, the
  answer keys (`s`, Escape, Enter), the terminal-only choices, the rows'
  refusals before anything is typed, the admin split, the audit record,
  `agent_send` and MCP `send` still refusing `/`.
- End to end through the real rows on a private tmux server
  (`crates/worktrees-api/tests/agents_e2e.rs`): Claude 2.1.318 `/model`
  (picker → the model in effect with `s` → "for this session only",
  `~/.claude/settings.json` unchanged), `/context`, `/status` (opened, then
  Close), `/compact keep only the bee facts`; Codex 0.160.1 `/model` (model
  → effort with `s` → "for this session only", `~/.codex/config.toml`
  unchanged), `/status` (its output), `/compact`, and `/context` refused as
  not Codex's.
- After the owner's bug (2026-11-04), on `-L wtmp1` at 139×52 with the
  user's `xhigh`: `agent_answer 4` on Claude's list said "the prompt
  changed" before the fix and, after it, Down, Down, `s` → "Set model to
  Sonnet 5.5 for this session only"; then, after one exchange, `agent_answer
  2` → `s` → "Switch model?" (unread, answered at the host). Codex: model 2
  (GPT-6-Astra) → its effort list, left on `More reasoning…`, `agent_answer
  3` → Up, Up, `s` → "gpt-6-astra high for this session only". Both config
  files kept their SHA-256 and mtime.
- The app: `app/scripts/slashmenu-check.mjs` (pure), and
  `app/scripts/chat-slash-browser.mjs` against the mock in Chromium and
  WebKit (22 checks each).
