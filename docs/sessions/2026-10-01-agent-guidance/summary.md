---
title: "Session — teaching every agent to work in places"
---

# Session — teaching every agent to work in places

- **Date:** 2026-09-30 → 2026-10-01
- **Worktree:** `.worktrees/agent-guidance`
- **Branches:** `agent-guidance` (proposal), `agent-guidance-phase1`,
  `agent-guidance-phase2` (core/CLI), `agent-guidance-phase2-app` (app, stacked)
- **PRs:**
  - [#383](https://github.com/penard-monkey/worktrees/pull/383): proposal, squashed as `a2b6805`
  - [#385](https://github.com/penard-monkey/worktrees/pull/385): phase 1, `cb05a1f`
  - [#393](https://github.com/penard-monkey/worktrees/pull/393): phase 2 core/CLI, `477cbdc`
  - [#394](https://github.com/penard-monkey/worktrees/pull/394): phase 2 app, `17c240a`
- **Release tag:** the proposal rode along in v0.34.1. Phases 1 and 2 are
  under `[Unreleased]`.
- **Planning files:** `planning.tar.gz` (`task_plan.md` only, the phase-2
  checklist; the brief was `.planning/brief.md`).

## Why

An orchestrator Claude in `(main)` had the worktrees MCP tools connected the
whole time. It still did branch work with a raw `git worktree add` into a
scratch directory. The tools reached the agent; the paradigm did not.

The research lane's question was how every agent learns to do its own branch
work in a place. "Every agent" meant Claude, Codex and pi, orchestrator and
lane, in any worktrees-managed repo.

## What shipped

**Proposal** (`docs/proposals/agent-guidance.md`, #383). It covers:
- which guidance channel each harness actually honours;
- the tiered text;
- per-launch delivery through argv worktrees already builds;
- guards;
- a 16-run Claude eval.

The headline: with today's MCP `instructions`, an orchestrator asked to "fix
X and commit it on a branch for a PR" created a place in **0 of 5** runs. All
five ran `git checkout -b` in `(main)`. With a rule-first text in the same
channel, 5 of 5 created one.

**Phase 1** (#385, `crates/worktrees-cli/src/mcp.rs`). The MCP `initialize`
instructions now open with the rule, in 244 characters, inside Codex's
247-character visible budget, on one line:

> Managed by worktrees: every branch lives in its own PLACE … Never `git worktree add`; never switch branches in (main).

A role line follows for where the server runs:
- `(main)`;
- a lane;
- an automation run (`in_run`);
- a stray worktree;
- an unregistered directory under `.worktrees/`.

Only worktrees-managed repos get it: a `.worktrees.toml`, or a registered
place. Every other repo keeps the old text verbatim. `scripts/agent-guidance-eval.sh`
is the manual eval harness. Re-run with real binaries: shipped v0.34.0 made
0/5 places through the tooling, the build made 5/5.

**Phase 2, core/CLI** (#393, `crates/worktrees-core/src/guidance.rs`,
`guidance/SKILL.md`). Each launch in a managed repo gets the rule and a
repo-agnostic `worktrees` skill. The text ships in the binary and is
materialised to `$XDG_DATA_HOME/worktrees/agent/<FNV-1a of the content>/`.

| Harness | Per-launch flags |
|---|---|
| Claude | `--plugin-dir …/claude`, or `…/claude-guard` with the guard |
| pi | `--skill …/skills/worktrees --append-system-prompt …/rules.md` |
| Codex | `-c developer_instructions="<rule>"`, only when the user has none of their own |

The supporting pieces:
- **Seam.** `AiLaunch.guidance` is filled in `ops::ai_launch_for` and emitted
  by `launch_cmd` after every adapter's head words.
- **Settings.** `~/.config/worktrees/agent-guidance.json` (`enabled`, default
  on; `guard`, default off), plus `$WORKTREES_AGENT_GUIDANCE`.
- **Guard.** `worktrees guard pretooluse` refuses `git worktree add`
  anywhere, and `checkout -b` / `switch -c` only in `(main)`. It follows `cd`
  and `-C`, and any failure allows.
- **Commands.**
  - `worktrees guide [--status --json | --rules]`.
  - doctor's Info finding `guidance-skipped`.
- **Shared code.** `mcp.rs` now takes `HEAD`, `is_managed` and `where_is`
  from core.

**Phase 2, app** (#394). Settings → **Agent guidance** is its own category
(`app/src/GuidancePanel.tsx`). It shows:
- the rule and the skill;
- per-agent delivery, including Codex's skip reason or "decided the next time
  Worktrees launches it";
- the two toggles, through `set_agent_guidance`;
- a note when the guard is on but no `worktrees` CLI on PATH has it.

The `agent-guidance` offer in `offers.ts`:
- fires while delivery is on and an agent is installed;
- is fingerprinted on `guidance::VERSION`;
- deep-links to the section.

The Tauri commands are `agent_guidance_status` and `set_agent_guidance`; the
mock takes `?guidance=off|codex-own|unchecked|noguard`.

## Decisions

- **Q1. `(main)` on its default branch is this repo's habit, not a product
  rule.** So there is no `main-off-default` warning, and no `warnings` field
  on `LsJson`. The instructions still say "never switch branches in (main)".
  The stray-worktree warning stays.
- **Q2. The guard is off by default, behind a toggle.** The eval showed it
  works (3/3 recovered after one denial), but a false positive costs the
  user.
- **Q8. The settings get their own Settings category,** not part of AI
  profiles. Profiles are per profile and Claude-only.
- **The rule goes only to managed repos.** The server is installed at user
  scope and runs in every repo a session opens, so "Managed by worktrees"
  would be false elsewhere. The signals are the cheapest the CLI can see; the
  app's project list is not one of them.
- **Settings live in a worktrees-owned file, not app memory.**
  `create_worktree` launches from the CLI process an MCP client started,
  which an app-side override (the Codex-permissions pattern) never reaches.
- **Codex gets `developer_instructions` only when the user has none.** `-c`
  replaces the key; it does not merge. An unfamiliar first developer block
  reads as "do not override".
- **The materialised directory is content-addressed.** A running session's
  `--plugin-dir` never changes under it. Old directories are not swept.
- **No guidance is written into repos** (AGENTS.md sections or repo skills).
  It would be per repo and stale, and it would not reach untrusted pi. The
  paradigm belongs to the tool.

## Dead ends / gotchas

- **The channels are not equal.**
  - Claude puts MCP `instructions` in its system prompt.
  - Codex makes them the tool namespace description, cut to 250 characters
    including a 3-character `...`, first line only, when the tools are
    deferred.
  - pi shows them only in `codemode` / `tool_search`, so with the `direct`
    exposure worktrees installs, a pi lane never sees them. That is why
    phase 2 exists.
- **`codex debug prompt-input` STARTS every configured MCP server.** The
  proposal said it did not, because the prompt text showed none; nobody
  looked at the processes. The review measured a marker server launched, at
  about 1 s against 0.12 s. The first #393 ran the probe from status, from
  `guide --status` and from doctor, which the app sweeps every few minutes
  per project. The probe is now launch-only, with each configured server
  disabled (`-c mcp_servers.<name>.enabled=false`; `mcp_servers={}` does not
  work), and cached on the config files' and the binary's mtimes.
- **A remedy in a refusal is a claim.** The guard's message said "Settings →
  Agent guidance turns it off", but the hook lived in the launched plugin, so
  the toggle did nothing until a relaunch. The guard now reads the settings
  on every call.
- **An older `worktrees` on PATH would error on every Bash call** (measured:
  exit 1). The guard is delivered only when the binary answers
  `guide --rules`.
- **A rebase across a release lands a CHANGELOG entry in it, silently.**
  Phase 1's entry rebased cleanly into the published `## [0.34.1]`. It was
  found by `grep -n '^## \['` and moved into a fresh `[Unreleased]`.
- **The eval's template must itself be managed** after the managed-only
  decision, or arm B correctly gets the neutral text. With one place to
  copy, the shipped baseline stopped moving `(main)` in 3 of 5 runs and
  hand-made worktrees under `.worktrees/` with `git worktree add` instead.
  That is closer to the original miss.
- **The first draft said `list_places` omits `strays`.** Running it showed it
  includes them; the draft was corrected before review.
- **Pre-existing flakes these gates surfaced:**
  - `agentfiles`' private index was named per pid+second, so two parallel
    `fix` tests could share it and fail with "invalid object … error building
    trees". Fixed in #393, now per call.
  - `codex_usage::tests::fake_cli_malformed_eof_buffer_cap_and_deadline_are_bounded`,
    a 300 ms real-time deadline, failed once on Linux CI and passed on
    re-run. Recorded in ROADMAP.
- **Bats launch-line tests pin other flags exactly.** The suite runs with
  `WORKTREES_AGENT_GUIDANCE=off`, and `test/guidance.bats` turns it on.

## Verification

- **Eval:** 16 + 10 + 10 headless `claude -p` runs, Opus 5.5 on Claude Code
  2.1.285, in throwaway repos on an isolated tmux server, killed each time.
  It cost about $8.40 in total ($3.61, $2.25 and $2.50), plus one live
  check.
- **Live checks of phase 2 delivery:**
  - Claude: listed `worktrees:worktrees`, and the guard hook refused
    `git checkout -b` in `(main)` while `(main)` stayed on `main`;
  - pi 0.99.1: `rules.md` under `[Context]` and `worktrees` under `[Skills]`
    at startup, with no prompt sent;
  - Codex 0.159.0: no session, as the account was out of tokens; the emitted
    `-c` shows the rule as the first developer text in `prompt-input`.
- **Red-first:**
  - every new unit, bats and check-script test was shown failing before the
    fix;
  - the stray-inside-`(main)` case went red with the stray check removed;
  - the unmanaged-repo case went red with the managed check removed;
  - the launch bats tests went red with delivery stubbed out.
- **The app in headless WebKit against the mock:**
  - rail offers → offer → Settings → Agent guidance;
  - the toggles hit-tested with `elementFromPoint`;
  - silencing dropped the count from 3 to 2;
  - the `noguard` and `unchecked` states rendered;
  - two CSS fixes came out of the screenshots.
- **Gates on the final #393:**
  - bats 401/401, 0 `not ok`;
  - lint clean;
  - cargo core 577, cli 40, app 170 (1 ignored);
  - `cargo check -p app` clean;
  - `ls --json` byte-identical to the installed v0.34.1.

  #394 passed `tsc` and every `app/scripts/*-check.mjs` from the repo root.
  CI was 9/9 on every PR. One Linux flake was re-run; see above.

## Follow-ups

Recorded in `ROADMAP.md`:
- **The real-app pass, still owed:**
  - one lane per harness from the app;
  - the guard in a new Claude lane, then toggled off live;
  - a resumed lane and a profile lane;
  - `ps` showing no Codex MCP servers during app start, Settings open or a
    doctor sweep;
  - the offer after an actual update.
- **The 300 ms `codex_usage` timing test.**
- **Phase 3 of the proposal:**
  - opt-in user-scope skill links (`~/.claude/skills`, `~/.agents/skills`)
    for hand-started sessions;
  - the guidance version wired into the stale-binary warning;
  - sweeping old `agent/<hash>/` directories.
