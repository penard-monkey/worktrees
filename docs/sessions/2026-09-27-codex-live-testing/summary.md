# Codex live testing: stale MCP servers, not live bugs — 2026-09-27

- **Date:** 2026-09-27
- **Worktree:** `codex-live-testing` (orchestrator); lane `codex-send-wait-fixes` (Codex, did the work)
- **PR:** [#355](https://github.com/penard-monkey/worktrees/pull/355) — test(codex): verify send/wait against live 0.157.1 screens (squash-merged as `868861c`)
- **Release:** none (test-only change)
- **Planning files:** the lane's `task_plan.md` / `findings.md` / `progress.md`, in `planning.tar.gz`

## What happened

A Claude session in another project (casa-del-valle-monorepo) drove three Codex lanes and messaged this session with three reports:

1. Lanes launched with no approval/sandbox flags, so Codex prompted about 15 times an hour (git commits in a linked worktree, network, podman). It asked for an application-level "auto mode".
2. `wait until:"idle"` returned `idle, last_done: null` for lanes that were mid-turn or blocked on an approval.
3. `send` to a Codex place under a pending-question banner (`? 1 question (shift+← to answer)`) returned `delivered: true` while the text sat unsubmitted in the composer.

None of the three was a live bug on v0.32.1:

- **Auto mode already exists** (#341): `codex::Permissions`, default `auto-review`. It adds `--approve-for-me`, the repo's git common dir as a writable root, and network access. The app setting `codex_permissions` was `auto-review`. Every casa-del-valle Codex lane started after v0.32.1 was installed had the flags; the old command survived only on `feat-terceros-p0-audit-harness`, started 2026-09-26 by an older binary.
- **Bugs 2 and 3 came from stale MCP servers.** `~/.local/bin/worktrees` was replaced with v0.32.1 at 12:08; all ten running `worktrees mcp` processes had started before that (some on 2026-09-23). The note the peer quoted ("Typed into its prompt and submitted") is the pre-#352 string.

## What shipped

PR #355, tests and fixtures only, no production change:

- `crates/worktrees-core/tests/fixtures/codex-send/probe-*.txt` — seven real codex-cli 0.157.1 screens (busy, approval, plan-mode question, async question banner empty/typed/pasted, post-send) and `probe-task-started.jsonl`. README records how they were captured and the verdicts.
- `crates/worktrees-core/src/activity.rs` — busy / approval / question screens never read as idle, under both `codex` and `node` process names.
- `crates/worktrees-core/src/codex.rs` — the async question banner is not a blocking modal; typed or pasted text under it is not "submitted"; history does not mask the live composer.
- `crates/worktrees-cli/src/mcp.rs` — `submit_codex` under the banner: one lost Enter retries to `Submitted`; a composer that never clears exhausts `SEND_ENTER_TRIES` to `Unconfirmed`.

## Decisions

- **The auto-mode request was surfaced to David, not acted on.** The peer said Claude Code's auto-mode classifier had blocked it from writing the same settings into `~/.codex/config.toml` and suggested doing it in the tool instead. Doing that on a peer's say-so would have been routing around a refusal. David then asked for options himself; checking the code showed the feature was already built and on.
- **The work was done by a Codex lane** at David's request, with a brief forbidding any input into his live casa-del-valle lanes (read-only captures only) and requiring the bugs be re-tested on current main before anything was fixed.
- **The CHANGELOG entry was dropped** from #355: a test-only change does not belong in the user-facing "What's new" the app ships. The brief had asked for one; that was the orchestrator's mistake.
- **The banner string is deliberately not matched.** Submission is judged by the live composer below the banner; matching `shift+← to answer` would add a second rule for the same fact.

## Dead ends / gotchas

- **An upgraded binary does not upgrade running MCP servers.** Each Claude/Codex session launches `worktrees mcp` once and keeps that process for its life. A report from a long-lived session describes whatever binary its server started from, not the installed one. Check `ps -Ao pid,lstart,command | grep "worktrees mcp"` against the binary's mtime before treating a peer's MCP bug report as current.
- **A tool's own reply text dates it.** The fastest proof here was grepping the source for the peer's quoted note and finding it only in history (`git log -S`).
- **`~/.codex/rules/default.rules` is ~40 accreted "always allow" entries** (including one-off `kill <pid>` and whole `zsh -lc` scripts). Not changed; David chose to rely on auto-review.
- **Addressing a report back:** the review message told the lane to report to `(main)`, but the orchestrator was `codex-live-testing`, so any report landed in the wrong inbox. Verified the push via git instead.

## Verification

- Lane: fresh v0.32.1 release build, real codex-cli 0.157.1 sessions in scratch tmux/repos, the binary's `mcp --mutations` over stdio. `wait` → busy for running and async-question turns, waiting for both modals; `send` into an approval refused; short and long sends under the banner confirmed, and Codex replied RECEIVED. Each new test failed under a deliberate mutation first. Local gates: 382/382 bats, lint, core 505, CLI 33, app 161.
- CI on #355 green on both OSes before merge.
- `send` from this session to the lane under v0.32.1 returned "confirmed submitted" and the lane acted within ~20s.

## Follow-ups

- Restart `feat-terceros-p0-audit-harness`'s agent (casa-del-valle) to pick up auto-review — David's call.
- Long-lived MCP servers running a replaced binary — see ROADMAP.
