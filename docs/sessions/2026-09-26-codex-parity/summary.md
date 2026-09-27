---
title: "2026-09-26 — Codex parity"
---

# Codex parity: instruction files, skills, MCP, activity, usage, repair, messaging

- **Dates:** 2026-09-23 (planning) → 2026-09-26 (build)
- **Worktree:** `.worktrees/codex-skills-mcps` (orchestrator) plus three lane places, now removed:
  `codex-usage-meter` and `codex-mcp-migrate` (Codex agents), and `codex-activity-nav` (a Claude agent)
- **PRs:**
  - [#341](https://github.com/penard-monkey/worktrees/pull/341) agent setup, the CLAUDE.md fallback and auto-review
  - [#340](https://github.com/penard-monkey/worktrees/pull/340) Codex activity dots and logos
  - [#342](https://github.com/penard-monkey/worktrees/pull/342) Codex usage meter
  - [#343](https://github.com/penard-monkey/worktrees/pull/343) copying MCP servers into Codex
  - [#344](https://github.com/penard-monkey/worktrees/pull/344) Repair / upgrade
  - [#348](https://github.com/penard-monkey/worktrees/pull/348) place↔place messaging
  - #346 (David): the Fix run on this repo, which made AGENTS.md canonical
- **Releases:** v0.31.0 (#341–#344); v0.32.0 (#348), in progress as #349 at close-out
- **Planning files:** `planning.tar.gz` (task_plan, findings, progress)

## What shipped

- **Codex reads CLAUDE.md with no repo change**
  (`crates/worktrees-core/src/profile.rs` `launch_cmd`):
  - `-c project_doc_fallback_filenames=["CLAUDE.md"]` goes on every Codex launch.
  - Measured with `codex debug prompt-input`, which never calls the model: the
    nested CLAUDE.md files load too, and AGENTS.md still wins where both exist.
- **Codex permissions: ask | auto-review (default) | full**
  (`crates/worktrees-core/src/codex.rs`, Settings → Codex):
  - Auto-review is `--approve-for-me` plus two sandbox holes measured in a
    linked worktree: the git common dir as a writable root, and
    `network_access=true`.
  - The mode resolves from the app override, then `$WORKTREES_CODEX_PERMISSIONS`,
    then `codex_permissions` in config, then auto-review.
- **`worktrees agent-setup status|fix|link-skills`**
  (`crates/worktrees-core/src/agentfiles.rs`):
  - It classifies each dir on the DEFAULT-BRANCH REF.
  - The Fix makes AGENTS.md canonical, turns CLAUDE.md into an `@AGENTS.md`
    stub, and symlinks repo `.claude/skills/*` into `.agents/skills/`.
  - The Fix is built with plumbing: a private GIT_INDEX_FILE, then
    commit-tree, then a create-only update-ref on `agent-instructions`, then a
    push and `gh pr create`. It never touches or creates a working tree.
  - `pending` / `pending_on_origin` tell a waiting PR apart from a branch that
    was never pushed.
  - `link-skills` symlinks `~/.claude/skills` into `~/.agents/skills`, the
    folder Codex reads.
- **Codex activity dots and a logo per nav row** (#340):
  - Busy / needs-input / afterglow come from the rollout's `task_started`,
    `task_complete` and `turn_aborted`, plus a screen check for the approval
    footer.
  - A dead Codex clears within a tick, because the watch is gated on a live
    non-shell pane.
  - The marks are the Claude spark and OpenAI blossom from Simple Icons (CC0),
    on `--txt-dim`.
- **Codex plan usage in the meter** (#342, `app/src-tauri/src/codex_usage.rs`,
  `app/src/planUsage.ts`):
  - A short-lived `codex app-server --listen stdio://` runs
    `account/rateLimits/read`, with single-flight, TTL, backoff and a process
    group kill.
  - The Codex slot is hidden when the CLI is missing.
- **Copy Claude MCP servers into Codex** (#343,
  `crates/worktrees-core/src/mcpmigrate.rs`):
  - `worktrees mcp --migrate --ai codex` and a section in Settings → Codex.
  - Every write goes through `codex mcp add`.
  - Literal env values are flagged, never preselected, and never serialised.
  - A lock file guards add-vs-add.
- **Projects say what they need** (#344, `app/src/projectTodos.ts`):
  - A header badge, and right-click → **Repair / upgrade… (N)** → a To do list
    with one action per row. Each count is only what that row's button can
    clear (relink / re-seed / provision / agent-setup Fix).
  - After-update offers: "Let Codex drive your worktrees" and "Let Codex use
    your Claude skills".
- **Claude ↔ Codex messaging over the worktrees MCP** (#348,
  `crates/worktrees-core/{messages,activity}.rs`,
  `crates/worktrees-cli/src/mcp.rs`):
  - New tools: `report`, `messages` and `wait`, plus `send` (with `--mutations`).
  - The log lives in the git common dir, and `from` is derived from the
    caller's own place.
  - There is now ONE activity derivation, shared by the nav, `place_status`
    and `wait`.
  - `send` labels each message, refuses a leading `/ @ !`, and refuses while
    Codex is waiting on an approval.
- **Docs, in this close-out:**
  - `docs/codex-manual-checks.md`;
  - the Codex traps in AGENTS.md;
  - `ci.yml` paths-ignore gains AGENTS.md (the rules moved there in #346);
  - `.claude/close-out.md` now points at AGENTS.md.

## Decisions

- **AGENTS.md canonical, CLAUDE.md an `@AGENTS.md` stub, never a symlink.**
  Claude follows `@` imports and Codex does not, and a tracked symlink is
  branch-dependent.
- **The Fix goes on its own branch and PR** (David), so it can be reverted on
  its own. It is built with plumbing because a temporary worktree would flash
  up as a stray in the nav.
- **Never write Codex's config.** All MCP changes go through `codex mcp add`.
  Codex's own Claude importer (`external_migration`) is REMOVED in 0.157.1, so
  the migration is ours.
- **Auto-review is the default** (David). Without the writable common dir,
  `git add` fails with exit 128 on `index.lock` in any linked worktree.
- **Activity is read from rollout content,** not `notify` (which carries no
  approval or abort events) and not hooks (`-c` hooks stop on "Hooks need
  review").
- **Real logos, not glyphs** (David); monochrome `currentColor`, with hue kept
  in the activity dot.
- **Repair is a badge plus a right-click menu; machine-wide items are
  after-update offers.** The per-project nav banner from #341 was dropped as
  noise.
- **Messaging is a general place↔place channel with no human UI** (David).
  Unblocking stalls is its first use, not the design target.
- **Every merge needs David's explicit OK.** The auto-mode classifier blocks
  `gh pr merge` without it. It was never worked around.

## Dead ends / gotchas

- **A bats test pushed a real branch.** The new `agent-setup fix` test ran
  from the suite's cwd (this checkout), not `$REPO`, and pushed
  `agent-instructions` to the real origin. No PR was opened and nothing else
  moved, and the branch was deleted at David's request. The tests now
  `cd "$REPO"` and refuse unless origin is `$ORIGIN`. The #348 harness also
  unsets `CLAUDE_PROJECT_DIR`, `WORKTREES_MCP_PROVIDER` and `CODEX_HOME`.
- **A CHANGELOG entry rebased CLEANLY into the published `[0.31.0]`** (the
  CLAUDE.md trap). It was moved to a new `[Unreleased]` by hand, and the
  `[0.31.0]` section was diffed byte-for-byte against main.
- **#341 CI: "Author identity unknown" on Linux only.** macOS invents an
  identity for git. Reproduced with `user.useConfigOnly=true`.
- **The mock hid the offer that never retired.** After a successful Fix, the
  default branch does not move until the PR merges, so `fixable` stayed true.
  The mock had rewritten the report instead of mirroring the backend.
- **Review caught `send` answering approvals.** Enter on Codex's approval list
  is "Yes, proceed", and a leading `/` runs builtins (`/logout`, `/clear`).
  Codex's own policy calls typed text user intent "even if high-risk". Both
  were verified live against a real Codex parked on a network-escalation
  approval.
- **An unexplained "You approved" was David at the keyboard.** The rollout
  timestamps had already cleared `send`: its Enter came 8 seconds before the
  modal existed.
- **A careless ROADMAP slice deleted three unrelated items** lying between the
  two I edited. They were restored after reading the diff's removed lines.
- **Codex lanes cannot report back.** Two sat idle for about 12 minutes. The
  stop-gap was a printed marker line plus a background watcher; #348 is the
  real fix.
- **The first Codex launch in a repo stops on "Trust this folder?"** It happens
  silently in an app-launched pane (see ROADMAP). Probe repos left trust
  entries in `~/.codex/config.toml` for David to remove.
- **Two claims a subagent made could not be reproduced.** It reported
  `plan-usage-check.mjs` failing on main, but it passes 3/3 here. The lesson:
  re-run a delegated agent's failures before believing them.

## Verification

Gates were run on every head before merge: release build first, then
`make test` (361 → 379), lint, and `cargo test` for core (449 → 498), cli
(15 → 23) and app (136 → 161), then tsc, `make test-frontend` and CI (9 jobs).

Live checks:
- **Real Codex 0.157.1:**
  - the launch argv, driven through the release binary;
  - `debug prompt-input` for the instructions and skill roots;
  - auto-review commit/push in a scratch linked worktree;
  - `send` → a real TUI answered;
  - `send` refused while an approval was pending;
  - `wait` / `place_status` reported idle with a real `last_done`.
- **Harness:** Chromium, plus WebKit for the new buttons and usage meter,
  across the six themes.

## Follow-ups

In ROADMAP:
- detecting the trust prompt;
- two interactive Codex sessions in one cwd are ambiguous;
- per-launch hooks as a future activity source;
- one-click provider handoff (now unblocked by `report`/`wait`);
- the existing `session_present` and `CODEX_WATCH` items.

Also: run `docs/codex-manual-checks.md` on the next `codex` upgrade. After
0.32, David installs the Codex MCP (Settings → Codex) so Codex agents get the
new tools.
