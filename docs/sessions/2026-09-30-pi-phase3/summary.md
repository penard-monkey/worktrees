---
title: "Session — pi phase 3"
---

# Session — pi phase 3: pi on the worktrees MCP bus

- **Date:** 2026-09-29 → 2026-09-30
- **Worktree:** `.worktrees/pi-phase3`
- **Branches:**
  - `pi-phase3-proposal`
  - `pi-phase3-core`
  - `pi-phase3-app`, stacked on the core branch
  - `pi-phase3-followups`
- **PRs:**
  - [#371](https://github.com/penard-monkey/worktrees/pull/371): research, and the rewrite of the proposal's §4. Docs only.
  - [#374](https://github.com/penard-monkey/worktrees/pull/374): core, CLI and MCP (squash `ce775f0`).
  - [#375](https://github.com/penard-monkey/worktrees/pull/375): app (squash `2ce2758`).
  - [#376](https://github.com/penard-monkey/worktrees/pull/376): review follow-ups (squash `3641c0a`).
- **Source of truth:** [pi-harness proposal](../../proposals/pi-harness.html) §4, §11 phase 3, and its Decisions. It was rewritten in #371.
- **Release tag:** none. A release is cut from main after this.
- **Planning files:** `planning.tar.gz` (`task_plan.md` and the lane's brief).

## What shipped

**Research (#371).**
- pi 0.99.1 ships MCP as a built-in extension, so the proposal's §4 was rewritten from probes.
- The user-scope `worktrees mcp` serves a pi lane unchanged:
  - the server's cwd is the place, so `report`'s `from` is correct;
  - `PI_CODING_AGENT=true` is in the server's environment.
- **Dropped:** the planned `worktrees msg` CLI verbs, the bus skill, and the `-e` extension for the bus.
- **Decided in review:**
  - `--exposure direct`;
  - install only through `pi mcp add`, at user scope, with no extension;
  - "warn and protect ours" for the allowance;
  - the upstream `--timeout` ask left open.

**Core, CLI and MCP (#374).**
- **`crates/worktrees-core/src/pimcp.rs`**, the `codexmcp.rs` shape:
  - Status READS `<agent dir>/mcp.json`. It never runs `pi mcp list`, which connects to every server.
  - Install is `pi mcp add worktrees --env WORKTREES_MCP_PROVIDER=pi --exposure direct -- <bin> mcp --mutations`, never with `-l`.
  - Uninstall is `pi mcp remove`.
  - Both are handed the same `PI_CODING_AGENT_DIR` that status read.
  - States: Codex's six, plus `disabled`, `pi-missing` and `unreadable`.
  - Exposed as `worktrees mcp --status|--install|--uninstall --ai pi`, and `doctor --pi` shows the state.
- **`trust.rs`: one precedence function for pi's trust flag** (`pi_launch_trust_for`), highest first:
  1. A place whose `.pi/mcp.json` defines `worktrees` never gets `--approve`.
  2. The worktrees `[trust] pi` allowance.
  3. pi's own `trust.json`. It is READ only, and resolved exactly as 0.99.1's `ProjectTrustStore` does: the nearest `true`/`false` from the realpath up, exact string keys, `null` walks on.
  4. `pi_project_trust`.
- **`mcp.rs`:**
  - stdin is read on a reader thread, which records `notifications/cancelled` for pending request ids.
  - `wait` pulses numeric `notifications/progress` every 15s to a request that carried a `progressToken`, and stops when cancelled.
  - A cancelled request gets no response.
  - `CLAUDE_PROJECT_DIR` is trusted only when `WORKTREES_MCP_PROVIDER` is unset or `claude`.
- **`pi.rs`: the current session.** It is the file in the pinned session dir whose header `cwd` is the place and whose newest entry is newest. This replaces "the file named by the derived id", which lost the dot after `/new` or a hand restart (a v0.33.0 bug). A resume passes that file's header id.
- **`pi.rs` and `harness.rs`: `send` to pi.**
  - Literal typing, with Enter only when the composer holds our text and has settled.
  - `Submitted` when the session file gains the user entry.
  - `Queued` (new) when pi's `Steering:` queue shows it.
  - Refused on the trust modal.
- **Fixtures:** `tests/fixtures/pi-send/0.99.1/` and `tests/fixtures/pi-session/restart/`.

**App (#375).**
- `PiMcpPanel.tsx`: Settings → pi → Worktrees tools.
- A "This repo:" line naming which trust rule applies, from `pi_status.launch`.
- The `pi-mcp` offer in `offers.ts`, pinned by `offers-check.mjs`.
- Settings → Commands lists pi's MCP state.
- The Plan tab pastes into pi, guarded by the trust modal (`pi::screen_of`).
- Mock support: `?pimcp=` and `?pitrust=`.

**Follow-ups (#376).**
- EOF on stdin counts as a cancel.
- `with_heartbeat`: progress for every non-`wait` tool call.
- A place's sessions must be no older than the place directory's birth time, and `session_present` needs a pi launch of ours. So a slug re-used after `rm` starts fresh with its brief.
- The `Steering:` match needs the sender's name through its closing quote.
- The mock gained `?pitrust=shadowed`.
- The birth-time tests skip where the filesystem records no birth time.

## Decisions

- **`--exposure direct`.** Both modes worked with the 480b coder, and direct measured no dearer in context (~17k against ~19k tokens on the first turn). Direct is also the one exposure the CLI can set without editing pi's file.
- **Detection reads `mcp.json` and never calls `pi mcp list`.**
  - It launches every server: 0 tools from a non-repo, 22 from a checkout.
  - It exits 1 if any server of the user's is down.
- **Protect ours at launch.** Under `--approve`, a repo's `.pi/mcp.json` `worktrees` entry REPLACED ours, and its stdio servers started with no prompt (observed with marker files). So a shadowing place never gets `--approve`, whatever else allows it.
- **pi's own trust counts** (David, mid-build; revises Q3).
  - The allowance still wins over pi's explicit distrust.
  - Shadowing wins over everything.
- **The server fixes `wait` for every client, rather than asking pi to change.**
  - pi re-arms its 60s timeout on progress.
  - `pi mcp add` has no `--timeout` flag, and we do not write pi's file.
- **A busy `send` confirms from the screen, not the file.** pi writes a steering message to the JSONL only when it DELIVERS it, at the end of the turn.
- **Birth time over generation for re-used slugs.** `rm` keeps the declared `pi_session_gen`, so the generation alone cannot tell a dead lane from a live one. `never_launched` became `!session_present` for the same reason.
- **A hand-run `pi` in a Claude lane is no longer resumed** by "Switch agent → pi". It starts fresh with the brief, which never ran under our launch. This narrows what #375 said, and the CHANGELOG says so.

## Dead ends / gotchas

- **A stale release binary answered for the branch.** The first live `send` returned the phase-2 refusal text, because the build predated the commit. AGENTS.md already warns about exactly this. Check `-nt` before believing a live run.
- **Still-frame fixtures passed a `send` that could never work live.**
  - The first version settled on the whole screen, which streams and spins mid-turn, so no Enter was ever pressed. It now settles on the composer, as Codex's loop does.
  - The second version matched an 80-character prefix against a `Steering:` line that pi cuts to the pane width (`…user] After...`), so every busy send read as unconfirmed.
  - Both were found only by the first live runs. Each now has a test built from a changing or truncated screen.
- **A history line quoting `Steering:` read as the queue.** The queue is now anchored by pi's `↳ … queued messages` hint line.
- **The model made answers up.** Asked to call `wait`, qwen replied `{"event": "timeout"}` without calling anything. Check the JSONL for the actual `toolCall` before believing a live tool result.
- **`$TMUX` is set in the agent's own shell,** because the session runs inside the user's tmux. Isolated runs need `unset TMUX` as well as `TMUX_TMPDIR`.
- **Two Ctrl-D killed both pi and the shell,** and with them the throwaway tmux server. One Ctrl-D on an empty prompt exits pi.
- **GNU `stat -f %B` is filesystem status and exits 0.** A btime guard that tried BSD syntax first would have skipped the test on every Linux runner. Measured in `ubuntu:24.04`.
- **The session files embed the whole system prompt,** which includes the user's skill list and paths. Fixtures elide the `system` entry and rewrite the cwd.
- **pnpm 12.8.1's corepack cache had no `pnpm.cjs`.** Running the cached pnpm 11.5.2 directly worked (`node ~/.cache/node/corepack/v1/pnpm/11.5.2/bin/pnpm.cjs install`).
- **The brief named `qwen3.6-27b`,** but the user's `models.json` now declares only `qwen/qwen3-coder-480b`. That one was used throughout.

## Verification

- **Gates** before every PR:
  - bats 394 → 395 ok / 0 not ok
  - lint
  - core 561 → 562
  - cli 34 → 37
  - app 163 (+1 ignored)
  - tsc
  - `cargo check -p app`
  - all 23 `app/scripts/*-check.mjs`, from the repo root
- **Every new test was shown red first,** against the old rule or with the fix removed.
- **CI:** 9/9 green on every code PR's final head.
- **`ls --json`** is byte-identical to shipped v0.33.0 across 27 places in two repos.
- **Live checks, pi 0.99.1.** Setup: `lm-studio/qwen/qwen3-coder-480b`, the release binary, a private `TMUX_TMPDIR`, and a throwaway `PI_CODING_AGENT_DIR` and XDG dirs.
  - A real `mcp --install --ai pi`.
  - `send` while idle → `Submitted` in 0.9s.
  - `send` while busy → `queued: true`.
  - pi's `wait` with `timeout_s: 90` → `timeout` at 90s. It had died at 60s before.
  - Manual checks §9 (pi's trust, distrust, allowance, and shadowing with a control marker) and §11 (`/new`, a bare `pi` by hand, `-r` by uuid keeping the conversation and the model) both pass.
- **Headless Chromium against the mock:**
  - the Settings → pi block, hit-tested;
  - the trust line for each source;
  - the `?whatsnew` offer deep-linking to pi/pi-mcp.
- **Nothing under `~/.pi` was written.** `~/.pi/agent/mcp.json` still does not exist.

## Follow-ups

These are in [ROADMAP](../../../ROADMAP.md):
- the live check still owed for Claude Code (and Codex) as a client of the new `wait` progress and cancel;
- the WKWebView pass;
- the 8 MiB whole-stream stdin bound;
- protect-ours being launch-time only;
- copied checkouts losing their pi sessions to the birth-time filter.
