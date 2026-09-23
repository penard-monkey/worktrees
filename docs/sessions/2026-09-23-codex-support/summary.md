---
title: "2026-09-23 — Codex support"
---

# Codex support: one active provider per worktree

- **Date:** 2026-09-23
- **Worktree:** `.worktrees/feat-codex-support`
- **Branch:** `feat/codex-support` (11 commits, squash-merged)
- **PR:** [#328](https://github.com/penard-monkey/worktrees/pull/328), merge `1ca9884`
- **Release tag:** [v0.29.0](https://github.com/penard-monkey/worktrees/releases/tag/v0.29.0) (#327), whose CHANGELOG section documents Codex
- **Planning files:** none. The session kept no `task_plan.md`/`findings.md`/`progress.md`. The design record is
  [`docs/proposals/codex-support.md`](../../proposals/codex-support.md)

> This summary was rebuilt at close-out from the commit series, the merged diff,
> the PR body and the proposal. The close-out thread did not have the working
> conversation, so the dead ends below are the ones the commits record, not a
> full account.

## What shipped

A worktree can now run **Codex** as well as Claude. Both CLIs can be configured
on the same machine, but a place has **one active provider session at a time**.

- **Core launch path:** `crates/worktrees-core/src/ops.rs`, `tmux.rs` and `profile.rs`. `new --ai` / `open --ai` start
  the selected provider. First they close any other Worktrees-managed provider
  session in the place. If that close fails, no agent starts. Reopening the
  same provider reuses its live session. A git-dir lock serialises concurrent
  opens from the app, the CLI and MCP.
- **Codex resume:** `crates/worktrees-core/src/codex.rs`. `codex resume --last` runs only when a
  saved conversation exists for this exact cwd. The check reads the first JSONL
  record (`session_meta`) of each rollout under `$CODEX_HOME/sessions/Y/M/D/`
  and never reads transcript content.
- **Codex MCP setup:** `crates/worktrees-core/src/codexmcp.rs`. It registers
  through `codex mcp add|remove` and never edits Codex's config itself. A
  project-local entry is reported separately.
  `worktrees mcp --status|--install|--uninstall --ai codex` is the CLI surface
  (`crates/worktrees-cli/src/main.rs`, `mcp.rs`). Without `--ai`, the command
  keeps its Claude behaviour.
- **MCP `create_worktree`:** it accepts an optional
  `provider: "claude" | "codex"`, which is validated as data before it reaches
  the CLI (`test/mcp.bats`).
- **App:**
  - The New worktree dialog offers a provider choice when both CLIs are
    present.
  - Settings has a default provider and both MCP panels side by side
    (`app/src/CodexMcpPanel.tsx`, `SettingsSheet.tsx`).
  - When no agent is running, **Open** offers both providers. With an active
    session, **Switch to** the other provider lives in the place's three-dot
    menu and asks for confirmation before it closes the current session
    (`App.tsx`).
  - Terminal focus, scrollback and Plan prompts follow the active provider
    (`TerminalPane.tsx`, `PlanPane.tsx`).
  - The mock harness tracks the new commands (`mock/install.ts`).
- **Codex sign-in:** Codex requires ChatGPT browser sign-in through Codex's own
  OAuth flow. Worktrees never asks for or stores an OpenAI API key. If the CLI
  is missing, the app shows install instructions before it launches.
- **Sandbox:** `app/scripts/sandbox.sh --app` exports `WORKTREES_CLI_BIN`, so MCP
  setup from a sandbox registers the branch under test rather than
  `~/.local/bin/worktrees`.
- **Docs:** `README.md` and the proposal. The proposal includes a manual
  handoff recipe (`.planning/handoff.md`).

## Decisions

- **One live provider per place, not side by side.** The second planning commit
  (`fded3c2`) explored running Claude and Codex together. The shipped design
  keeps one terminal and one live agent per place, because two agents writing
  one working tree is a conflict generator. Each provider keeps its own saved
  conversation, so switching back and forth loses nothing.
- **A switch is explicit and confirmed.** Changing the Settings default never
  closes a live session. Entering a place keeps whichever provider is running.
  Only a Switch action closes a session (`46d716b`). Switch moved out of the
  main area into the three-dot menu (`ed30dae`) so it is not a one-click
  accident beside **Open**.
- **Adopted sessions are never killed silently.** A provider session found
  under a personal tmux name or an older prefix makes the switch refuse and
  name the session, so the user closes it explicitly. A launch never adopts
  the *other* provider's session.
- **Go through Codex's own CLI and never write its config.** This follows the
  same rule as `~/.claude.json` in CLAUDE.md: the file belongs to another tool.
- **No invented Codex telemetry.** Busy/usage dots stay Claude-only, because
  they come from `~/.claude/sessions`. Codex presence is only "its tmux session
  exists". Claude-only messaging and `@worktrees:` mentions stay out of Codex
  panes.
- **Handoff stays manual for now.** Automating it needs a reliable "the outgoing
  agent finished writing" signal, which neither CLI provides.

## Dead ends / gotchas

- **The sandbox registered the wrong binary.** MCP setup from `sandbox.sh --app`
  found the installed `~/.local/bin/worktrees` rather than the branch's build,
  so the Codex MCP entry pointed at code without the feature (`0694a5f`). The
  fix is `WORKTREES_CLI_BIN`. This is the same family as CLAUDE.md's
  "sandbox.sh does not isolate `$HOME`" note.
- **Launching Codex with no CLI failed opaquely.** It now prompts with install
  instructions (`f0c0cfd`).

## Verification

From the PR body, run on the branch before merge:

- `make test`: 360 bats tests passed
- `make lint`: passed
- `cargo test -p worktrees-core`: 440 passed
- `cargo test -p worktrees-cli`: 15 passed
- `cargo test -p app --lib`: 136 passed
- `cargo check -p app`: passed
- `npm run build`: passed

At close-out, `feat/codex-support` and `origin/main` had an empty diff, so the
gates were not re-run.

**Not verified in this record:**

- Live Claude → Codex → Claude switching against real CLIs is acceptance
  criterion 7 in the proposal. It has no written manual checklist the way AI
  profiles do (`docs/ai-profiles-manual-checks.md`), and bats has no fake
  `codex` for the live-agent side.

## Follow-ups

- **Manual check for Codex.** Add a `docs/`-level checklist in the shape of
  `ai-profiles-manual-checks.md`: switch round-trip, resume after a switch,
  adopted-session refusal, legacy both-sessions reconcile, and sign-in or
  missing-CLI prompts. Re-run it on a `codex` upgrade.
- **`codex::session_present` scans every rollout.** It walks all of
  `~/.codex/sessions` and opens every `.jsonl` on each launch, so cost grows
  with Codex history. Walk newest-first and stop at the first match, or bound
  the walk by date.
- **One-click handoff.** Blocked on a completion acknowledgement from the
  outgoing agent (see the proposal's "Handoff feasibility").
- **CLAUDE.md's Architecture → Agents paragraph** still describes agents as
  Claude-only. Update it when the next code change touches that area.
