---
title: "Proposal — Codex support"
---

# Codex support in Worktrees — implementation plan

**Status:** proposed for review, 2026-09-22. No product implementation has started.
**Branch:** `feat/codex-support`, based on `origin/main` at `e429ffe`.

## Goal and current behavior

A user who chooses `codex` as the AI command should be able to create, open,
resume, and brief a place, connect Worktrees' MCP tools to Codex, and understand
which Worktrees features apply to that choice. Existing Claude behavior must
continue to work.

Today `--ai codex` starts a Codex process in pane 0, and tests cover that basic
launch. The default resume argument is Claude's `-r`; `--brief` supplies an
initial prompt only to Claude; the MCP setup command, Settings panel, and home
prompt are Claude-specific. AI profiles are Claude config bundles. Agent
activity and the usage meter read Claude-owned data. These paths cannot simply
be relabeled.

## Proposed implementation

### 1. Make launch and resume provider-aware

- Centralize recognition of the supported `claude` and `codex` commands, including
  absolute executable paths and command flags. Keep other AI commands on the
  existing generic path.
- Resolve the resume invocation per provider: Claude keeps its current `-r`;
  Codex uses `codex resume --last` in the place directory. Preserve the existing
  `WORKTREES_AI_RESUME_ARG` and `ai_resume_arg` override behavior for users who
  set it explicitly. Ensure a place with no saved Codex session starts normally
  rather than opening an empty picker or failing silently.
- Pass the fixed `.planning/brief.md` opener as an initial Codex prompt on new
  places, after verifying the actual CLI argument form. Continue writing only
  the fixed file path into argv, never the user-authored brief text.
- Keep tmux adoption and auto-resume based on the executable word so both
  providers are recognized after a pane or app restart.

### 2. Connect Worktrees MCP to Codex

- Extend MCP status and setup with a provider field. For Codex, detect the
  `worktrees` entry in Codex's user config and report absent, installed,
  read-only, stale, and foreign states with the same safety rules as Claude.
- Register through Codex's own `codex mcp add worktrees -- <worktrees-bin> mcp
  --mutations` command; use `codex mcp remove` for a Worktrees-owned entry when
  repairing it. Never write `~/.codex/config.toml` directly or overwrite a
  foreign entry. Keep a read-only install option.
- Make `worktrees mcp --status|--install|--uninstall` and the Settings panel
  explain and operate on the selected provider. Keep project-local config
  detection separate from user-wide setup, since Codex loads project config
  only for trusted projects.
- Audit MCP server project discovery: it currently checks
  `CLAUDE_PROJECT_DIR` before cwd. For Codex, rely on the process cwd and
  confirm that a user-wide server serves the intended worktree in a real
  session. Review provider-specific wording in tool descriptions and errors.

### 3. Make the app report Codex state honestly

- Show the configured provider in Settings and use Codex-specific MCP text and
  setup commands. Keep Claude-only AI profiles, usage, and status integrations
  labeled as such. Do not imply that a Claude profile applies to Codex.
- For Codex places, derive the basic running state from the tmux pane. Keep
  richer busy/waiting dots on Claude until a documented or verified Codex
  session signal can support the same semantics; do not infer busy from log
  modification times.
- Gate Claude-specific cross-session messaging and `@worktrees:` mention
  insertion in Codex panes. Keep provider-neutral terminal paste and Plan
  prompts where they work with Codex; use clear feedback for unsupported actions.

### 4. Documentation and verification

- Update README configuration and examples, CLI help, Settings copy, and manual
  checks. Document how to set `ai_cmd = codex`, how resume behaves, and how
  Codex connects to Worktrees MCP.
- Add focused Rust and bats coverage for provider recognition, resume command
  construction, brief delivery, MCP detection/install safety, and unchanged
  Claude behavior. Add app checks for provider-specific setup copy and states.
- Run the repository's normal CLI and app gates, then manually create, close,
  reopen, brief, and MCP-connect a Codex place with a real Codex CLI. Test a
  Claude place again to catch regressions.

## Decisions and limits for this change

- The default AI command remains `claude`. Choosing Codex is explicit through
  `--ai codex` or the existing user config.
- Claude AI profiles are not converted into Codex profiles. Codex has its own
  `config.toml`, `AGENTS.md`, and skills model; a separate profile design would
  need its own requirements and migration rules.
- A Codex usage meter and Claude-style busy/waiting indicators are outside this
  change unless a reliable supported data source is found during implementation.
  The UI should show no fabricated value for either.
- No repository-provided file may name a command to execute. All MCP setup and
  AI command choices remain user-controlled.

## Acceptance criteria

1. `ai_cmd = codex` creates a place whose pane runs Codex and receives a
   `--brief` task from `.planning/brief.md`.
2. Reopening a closed Codex place resumes the most recent conversation for
   that place, or starts a fresh session when none exists.
3. CLI and app can detect and install Worktrees MCP for Codex without modifying
   unrelated Codex configuration or an existing foreign server.
4. Codex users see accurate Settings and place state; Claude-only controls are
   identified or withheld.
5. Existing Claude launch, resume, profiles, MCP, and activity behavior passes
   its current tests and manual checks.

## Official Codex references

- [CLI command reference](https://developers.openai.com/codex/cli/reference/) —
  `codex resume --last` and `codex mcp add|get|list|remove`.
- [MCP guide](https://developers.openai.com/codex/mcp/) — Codex's MCP config and
  supported local stdio registration.
- [AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md/) —
  Codex's project instruction discovery.
