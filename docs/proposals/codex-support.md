---
title: "Proposal — Codex support"
---

# Codex support in Worktrees — implementation plan

**Status:** implemented on `feat/codex-support`.

## Goal

A worktree can use Claude or Codex, with both providers configured on the same
machine. It has **one active provider session at a time**. The New worktree
dialog offers a provider choice when both CLIs are available. An explicit
provider change closes the current Worktrees-managed tmux session before
opening the other provider. Claude and Codex retain separate saved
conversations, so each can resume when selected again.

The Settings default preselects the provider for a new worktree and applies
when entering a place with no running agent. Entering a place that already has
one running provider keeps that provider. Changing the default never closes a
live session by itself.

## Session lifecycle

- Resolve each provider's session by worktree path and actual process. Keep
  collision-safe provider names and recognize older sessions under the
  canonical name. A Codex process must never be treated as a Claude session,
  or vice versa.
- `new --ai` and `open --ai` start the selected provider. Before launch, close
  any other Worktrees-managed provider session in that place. If closing it
  fails, stop without starting another agent. Reopening the same provider
  reuses its live session. A Git-directory lock serializes concurrent opens
  from the app, CLI, and MCP so two processes cannot start different agents
  after each observes an empty place.
- Do not silently kill a provider session adopted from a personal tmux name
  or an older prefix. Refuse the switch and name the session for explicit
  closure. A launch must not adopt a running session of the other provider.
- A legacy place that already has both sessions is rendered as one terminal.
  When selected in the app, it is reconciled through the locked launch path,
  keeping the Settings default and closing the other managed session. An
  explicit provider selection can also reconcile it. Closing or removing the
  place still accounts for every live session, including legacy sidecars.
- Claude resumes with its configured resume argument. Codex uses `codex
  resume --last` in the place directory when a saved conversation exists.
  A fresh action bypasses resume. A newly written `.planning/brief.md` is
  opened through a fixed prompt without putting its text in process arguments.
- Generic CLI `--ai <cmd>` behavior remains available for other tools.

## Configuration and authentication

- Settings keeps independent Claude and Codex MCP setup controls and shows
  both states together. Connecting Worktrees tools to either provider is
  optional for launching its CLI. The CLI offers provider-targeted MCP setup:
  `worktrees mcp --status|--install|--uninstall --ai codex`; the unqualified
  command retains Claude behavior.
- Codex MCP registration uses `codex mcp add` and `codex mcp remove`. Never
  edit Codex's config directly or replace an entry owned by another tool.
  Project-local configuration is reported separately from user-wide setup.
- Worktrees-launched Codex requires ChatGPT browser sign-in through Codex's
  OAuth flow. Worktrees neither requests nor stores OpenAI API keys. If the
  Codex CLI is missing, show installation instructions before launch.
- The MCP `create_worktree` tool accepts an optional `provider: "claude" |
  "codex"`. Without one, it uses the project's configured AI command. Its
  brief and provider values are validated as data before reaching the CLI.

## App behavior

- The main area shows one provider terminal. When no agent runs, **Open**
  buttons offer both providers. With an active session, **Switch to** the other
  provider appears in the place's three-dot menu and warns that the current
  session will close before proceeding. Terminal focus, scrollback, and Plan
  prompts follow the selected provider.
- Claude activity and usage remain Claude-specific. Codex presence comes
  from its tmux session; no busy or usage value is invented for Codex.
  Claude-only messaging and `@worktrees:` mentions stay out of Codex panes.
- The worktree, branch, brief, files, notes, and plan are shared. The agents'
  transcripts and tmux sessions remain separate, although only one provider
  session runs at a time.

## Handoff feasibility

A manual handoff works now: ask the current agent to write
`.planning/handoff.md`, switch providers, and ask the receiving agent to read
it and inspect the current files and git diff. The document should record the
goal, completed work, decisions, uncommitted changes, blockers, and next
steps. It is a snapshot, not a transcript conversion.

A future one-click handoff needs a reliable acknowledgement that the outgoing
agent finished writing the document, timeout and error handling, then a fixed
read prompt for the incoming agent. It must close the outgoing session before
starting the incoming one. This automation is outside the current provider
switch implementation.

## Acceptance criteria

1. A worktree never gains a second Worktrees-managed provider session from
   `new`, `open`, the app, or MCP. Switching Claude → Codex → Claude leaves one
   live session after each step and preserves both saved conversations.
2. Entering a place with one live provider keeps it, regardless of the
   Settings default. A failed provider switch does not open a second session.
3. An adopted session is not killed without explicit closure. A legacy place
   with both managed sessions displays one terminal and is reconciled on
   selection in the app.
4. When both CLIs are available, New worktree offers both providers. The
   choice reaches the launch command and survives a failed create.
5. Both MCP connections can coexist. Setup for one provider does not alter
   the other or a foreign entry. Codex uses ChatGPT sign-in.
6. Brief delivery, resume, close/remove safety, Claude profiles and activity,
   and existing generic AI command behavior pass their tests.
7. The app build, Rust tests, bats suite, and shell lint pass. Live sessions
   are checked manually without replacing either provider's saved transcript.

## Official Codex references

- [CLI command reference](https://developers.openai.com/codex/cli/reference/) —
  `codex resume --last` and `codex mcp add|get|list|remove`.
- [MCP guide](https://developers.openai.com/codex/mcp/) — Codex's MCP config
  and local stdio registration.
- [AGENTS.md guide](https://developers.openai.com/codex/guides/agents-md/) —
  project instruction discovery.
