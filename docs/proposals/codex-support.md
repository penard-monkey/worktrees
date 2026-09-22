---
title: "Proposal — Codex support"
---

# Codex support in Worktrees — implementation plan

**Status:** implemented on the feature branch 2026-09-22; awaiting review.
**Branch:** `feat/codex-support`, based on `origin/main` at `e429ffe`.

## Goal and current behavior

A user can run Claude and Codex simultaneously in the same worktree, see both
terminals side by side, and open, resume, brief, or close either session without
disturbing the other. Worktrees' MCP tools can be connected to each provider
independently. The app's Settings default selects which agent starts first;
the CLI's `ai_cmd` remains its default. Neither restricts which agents may run
there.

Today `--ai codex` starts a Codex process in pane 0, and tests cover that basic
launch. Each place has only one canonical tmux session and one main terminal in
the app. Session adoption chooses one pane by worktree path, so a second agent
could be mistaken for the first. The default resume argument is Claude's `-r`;
`--brief` supplies an initial prompt only to Claude; the MCP setup command and
Settings panel are Claude-specific. AI profiles are Claude config bundles.
Activity and usage read Claude-owned data. The worktree must become the shared
container for two independent agent sessions.

## Proposed implementation

### 1. Represent two agent sessions per place

- Add an explicit provider identity (`claude` or `codex`) to session lookup,
  launch, close, and place status. Use distinct, collision-safe tmux session
  names for each provider, with a migration path that recognizes an existing
  canonical session. Match both worktree path and actual process/provider when
  adopting a session; never satisfy a Claude open with a Codex pane or vice
  versa. Preserve the current generic `--ai <cmd>` path for other tools.
- Add a Claude/Codex default provider selector in Settings. It determines the
  initial agent when the app creates or enters a place; users can start the
  other provider at any time. Keep explicit CLI `--ai` and the CLI's existing
  config precedence. Changing the default never stops or replaces live sessions.
  Add an
  explicit action in the app and CLI to start or reopen the other provider in
  that same place. Reopening either provider reuses its live session; a closed
  provider resumes its own conversation. A second provider does not create a
  second git worktree or replace the first session.
- Resolve resume per provider: Claude keeps `-r`; Codex uses `codex resume
  --last` scoped to the place directory. Preserve explicit resume overrides
  where they apply, and start a fresh Codex session if no saved session exists.
  Verify the CLI argument form before implementation.
- Deliver the fixed `.planning/brief.md` opener to a newly launched Claude or
  Codex session. The brief file is shared by the place; launching the second
  agent reads its current contents. Do not silently resend a changed brief to
  an already running session or put user-authored brief text in argv.
- Make `open --ai <provider>` and `close --ai <provider>` target one provider.
  Define the existing unqualified `close <place>` as closing all Worktrees-owned
  agent sessions for that place, listing them before confirmation. Removing a
  place must account for both live sessions and preserve the existing adopted
  session safety checks.

### 2. Connect Worktrees MCP to Codex

- Extend MCP status and setup with an explicit provider parameter, independent
  of `ai_cmd`. For Codex, detect the
  `worktrees` entry in Codex's user config and report absent, installed,
  read-only, stale, and foreign states with the same safety rules as Claude.
- Register through Codex's own `codex mcp add worktrees -- <worktrees-bin> mcp
  --mutations` command; use `codex mcp remove` for a Worktrees-owned entry when
  repairing it. Never write `~/.codex/config.toml` directly or overwrite a
  foreign entry. Keep a read-only install option.
- Make `worktrees mcp --status|--install|--uninstall --ai codex` and separate
  Settings sections operate on Codex, while the current unqualified CLI command
  retains its Claude behavior for compatibility. Show setup state for both
  providers at once in Settings. Keep project-local config
  detection separate from user-wide setup, since Codex loads project config
  only for trusted projects.
- Audit MCP server project discovery: it currently checks
  `CLAUDE_PROJECT_DIR` before cwd. For Codex, rely on the process cwd and
  confirm that a user-wide server serves the intended worktree in a real
  session. Review provider-specific wording in tool descriptions and errors.

### 3. Show both sessions in the app

- Change the main terminal area to show Claude and Codex side by side when both
  are open, with independent attach, focus, sizing, scrollback, and lifecycle.
  When only one is open, it gets the available width. Provide a clear start or
  resume action for the absent provider. Reattaching the app must restore both
  terminals without restarting either process.
- Report presence and activity per provider instead of collapsing both into
  one `tmux_session` or one agent dot. Keep Claude's existing busy/waiting
  probe and usage meter associated only with Claude. For Codex, derive running
  presence from its tmux pane; add richer activity only if a reliable Codex
  signal is established. Never infer busy from log modification times.
- Show provider-specific MCP setup alongside Claude-only AI profiles, usage,
  and status controls. Label those controls accurately when both providers are
  active. Route Plan prompts and terminal paste to the user's selected pane;
  keep Claude-specific cross-session messaging and `@worktrees:` mentions out
  of Codex panes.

### 4. Documentation and verification

- Update README configuration and examples, CLI help, Settings copy, and manual
  checks for simultaneous sessions, provider-targeted actions, resume, and both
  MCP connections.
- Add Rust and bats coverage for distinct session names, adoption, close and
  remove safety, resume, brief delivery, independent MCP setup, and existing
  Claude behavior. Add app checks for side-by-side rendering, provider-specific
  controls, and restoring both attached terminals.
- Run the repository's normal CLI and app gates, then manually run Claude and
  Codex together in one real worktree. Close and resume each independently,
  verify neither steals the other's pane, and connect MCP to both. Check a
  Claude-only place for compatibility.

## Decisions and limits for this change

- The app's default first agent is configurable in Settings and starts as
  `claude` for existing installations. The CLI keeps `ai_cmd` and `--ai`;
  either provider can be added later.
- Worktrees-launched Codex sessions require ChatGPT browser sign-in through
  Codex's own OAuth flow. Worktrees neither requests nor stores OpenAI API keys.
  Existing API key sign-ins must be replaced with `codex logout` followed by
  `codex login` before using Codex in Worktrees.
- The first implementation supports one Worktrees-managed session per provider
  per place. It does not restrict sessions a user launches manually.
- Claude AI profiles are not converted into Codex profiles. Codex has its own
  `config.toml`, `AGENTS.md`, and skills model; a separate profile design would
  need its own requirements and migration rules.
- A Codex usage meter and Claude-style busy/waiting indicators are outside this
  change unless a reliable supported data source is found during implementation.
  The UI should show no fabricated value for either.
- No repository-provided file may name a command to execute. All MCP setup and
  AI command choices remain user-controlled.

## Acceptance criteria

1. In one worktree, the user can start Claude and Codex and interact with both
   terminals side by side. Neither session changes the other's process, cwd,
   transcript, or tmux target.
2. The user can close and reopen either provider independently. Codex resumes
   the most recent conversation for that place, or starts fresh when none exists.
   An unqualified close accounts for both sessions before acting.
3. Either provider started with `--brief` receives the same
   `.planning/brief.md` task without placing its text in argv.
4. CLI and app can detect and install Worktrees MCP for each provider without
   changing the other's configuration or an existing foreign server.
5. The app reports each provider's session state accurately, restores both
   attached terminals after restart, and identifies Claude-only controls.
6. Existing Claude launch, resume, profiles, MCP, and activity behavior passes
   its current tests and manual checks.
7. Changing the default provider in Settings affects future app launches only;
   live Claude and Codex sessions continue running unchanged.

## Official Codex references

- [CLI command reference](https://developers.openai.com/codex/cli/reference/) —
  `codex resume --last` and `codex mcp add|get|list|remove`.
- [MCP guide](https://developers.openai.com/codex/mcp/) — Codex's MCP config and
  supported local stdio registration.
- [AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md/) —
  Codex's project instruction discovery.
