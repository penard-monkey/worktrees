# Session: agents — a place can be briefed, named, and asked who is working in it

- **Date:** 2026-09-02
- **Worktree:** `bug-fixes` (idle base `bug-fixes-next`)
- **Branches:** `bug-fixes-agent-brief`, `bug-fixes-strays` (stacked, rebased
  onto the squash) — both deleted after merge
- **PRs:** [#183](https://github.com/penard-monkey/worktrees/pull/183) → `8196e7c`,
  [#184](https://github.com/penard-monkey/worktrees/pull/184) → `802a367`
- **Release:** none yet — both land in `[Unreleased]`; v0.20.0 is the
  follow-up
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress

## Context

David's valleos repo runs an orchestrator Claude in `(main)` (its `.mcp.json`
registers `worktrees mcp --mutations`) that creates worktrees, each with its
own Claude in pane 0. He liked the shape and wanted the tool to grow toward
it without over-building. Two complaints started it: agent-created worktrees
came up with two panes, and the orchestrator had no way to hand a worker its
task.

## Decisions

- **Vocabulary (settled).** Project = repo. **Place = a workstream** — a kind
  of work, durable, parked on `<place>-next` (already how David works).
  **Agent = one Claude session with a brief**, 1..n per place. **Orchestrator
  = `(main)`'s Claude**, driven by David; the app does not host one.
- **Parallelism stance: one agent per place at a time.** More parallelism =
  more places, or the agent's own Claude-native helpers (agent teams,
  `Agent(isolation: worktree)`), which we SHOW but do not model. The door to
  sub-places (`communications/<task>` worktrees) is kept open, not built.
- **No bus of our own.** Claude Code's cross-session messaging (v2.1.224+,
  `ListAgents`/`SendMessage`, `notify_when_idle`, unix socket, no servers) is
  the channel. Every orchestrator in the prior-art survey ships its own; we
  do addressability + observability only. Verified from this session:
  `ListAgents` lists every local session with its tmux pane, and the probe
  file `~/.claude/sessions/<pid>.json` carries `name`, `tmux`, `sessionId`.
- **The one hard rule: an agent is a Claude session launched against a brief
  file** (`.planning/brief.md`), never a prompt in argv. Survives restart/`-r`,
  human-readable, archived at close-out with the rest of the planning trio,
  and there is no `safe_arg` surface to defend.
- **Agent name = the FULL tmux session name** (`valleos-communications`), not
  the slug: several projects run at once, and claude suffixes a colliding
  name into something nobody predicted.
- **MCP registration is not a doctor item.** `worktrees mcp` discovers the
  project from cwd, so one user-scope `claude mcp add -s user worktrees --
  worktrees mcp --mutations` covers every repo. Per-repo `.mcp.json` was the
  wrong layer.
- **Strays are reported from the snapshot, never moved.** `snapshot()` already
  runs `git worktree list`; keeping the entries it used to drop costs nothing
  and lets the app flag a project at load without breaking doctor's
  on-demand rule. Moving is the user's (tmux sessions and editors hold cwds).

## What shipped

**Slice 1 (#183)**

- `worktrees new … --brief <text>` → `.planning/brief.md`, claude launched on
  `Read .planning/brief.md and begin.`; warns via real `git check-ignore`
  when `.planning/` is not ignored. `crates/worktrees-core/src/ops.rs`
  (`BRIEF_PATH`, `BRIEF_OPENER`, `write_brief`).
- `AiLaunch.opener` + `launch_cmd`/`pane0_body_for`: `--name <session>` and
  the opener, claude only (by `ai_word_of(cmd)`, which also excludes the
  fail-closed printf). `--name` sits between `-r` and the opener because `-r`
  takes an OPTIONAL session id. `crates/worktrees-core/src/profile.rs`.
- MCP `create_worktree {branch, base?, brief?, spare?=false}`; `place_status`
  gains `agent_state` + `agents[]`. `crates/worktrees-cli/src/mcp.rs`.
- New `crates/worktrees-core/src/agent.rs`: the probe reader moved out of the
  app (`ClaudeProbe`, `busy_is_delegated`, `pid_alive`, `live_probes`,
  `agents_at`), so the nav dots and the MCP answer read one truth.

**Slice 2 (#184)**

- `model::Stray`, `LsJson.strays` (additive, `#[serde(default)]`),
  `project.rs` `parse_worktree_list`/`strays_from` — one spawn shared with
  `registrations`.
- `diag::Code::StrayWorktree` + `ops::stray_findings`: whole-project runs
  only, never `--config-only`, never beside hub-copy, never `--strict`-promoted.
- App: ⊟ on the project row (`App.tsx`), a Project-sheet section with the
  exact `git worktree move` per stray (`ProjectSheet.tsx`), mock fixture (cdv
  carries the dmux stray).
- Profile `worktrees_mcp_mutations` → injected server args
  `["mcp", "--mutations"]`; checkbox nested under the existing toggle.

**Outside the repo:** casa-del-valle's `.dmux/worktrees/dmux-1781998195357`
(`feature/api-default-deny-auth`) moved to `.worktrees/api-default-deny-auth`
by hand; `worktrees ls` there sees it now.

## Dead ends / gotchas

- **The first four bats failures were all test-side.** (1) `p["agents"][0]
  ["tmux"]` → `KeyError`: `Agent` uses `skip_serializing_if`, so absent fields
  are absent, not null. (2) The stray slug rule is `cmd_new`'s (`feature/x` →
  `feature-x`); the prettier `api-default-deny-auth` was a name chosen by
  hand for CdV. (3) git registers the PHYSICAL path (`/private/var/…`);
  `$BATS_TEST_TMPDIR` is logical — `pwd -P` before comparing. (4) `doctor
  --strict` exits 2 on a config whose file was never materialized into the
  worktree — `relink --all` first, or write the config before `new`.
- **A markdown brief starts with `- `.** The generic "expected value starts
  with `-`" guard in `cmd_new`'s parser would have refused the most natural
  brief there is; `--brief`'s value is exempt because it is consumed whole
  and never read as a flag — which is also why a `--ai=…`-shaped brief
  becomes a FILE (bats pins it, the `base` guard's twin).
- **`gh pr merge --delete-branch` deletes the LOCAL branch too** — the
  rebase of the stacked branch then needs the sha (`dc5bcfe`), not the name.
  It cannot delete the checked-out one, so the second merge left the tree on
  `bug-fixes-strays`.
- **A profile's injected worktrees MCP server was read-only.** Found while
  answering "should doctor check `.mcp.json`": `--strict-mcp-config` drops the
  user-scope server, so an orchestrator under a profile could never create a
  place. Nothing in the suite could have said so.
- **Cross-session inbound default.** A bypass-permissions receiver HOLDS
  messages unless the sender also bypasses; `-p` workers need
  `--settings '{"crossSessionInbound":"accept"}'`. Not hit yet — recorded
  before it is.

## Verification

- Gates on both PRs: release build confirmed fresh (`-nt`), `make test` full
  suite, `make lint`, core 272 → 274, cli 7, app `--lib` 43, `tsc --noEmit`;
  CI green ×9 on each before squash-merge.
- New tests: `launch_cmd` ordering/quoting/exclusions; `agents_at` ordering
  + real-probe parse; `parse_worktree_list`/`strays_from` incl. the
  sibling-dir prefix trap (`/w/repo-design-skills` vs `/w/repo`); profile
  `--mutations` args; bats for `new --brief`/`--name`, MCP `create_worktree`
  shapes, `place_status` with live/dead/elsewhere probes, `ls --json` strays,
  doctor stray finding through adoption.
- NOT run: the mock harness for the ⊟/sheet section (tsc only); the real
  loop on valleos (needs a release); `SendMessage` to a BUSY worker.

## Follow-ups

- Cut **v0.20.0** (CHANGELOG `[Unreleased]` → section, bump, `make release`).
- valleos: `claude mcp add -s user worktrees -- worktrees mcp --mutations`,
  drop `.mcp.json`, brief a place from `(main)`, test `SendMessage` while
  busy + `notify_when_idle`. Document the orchestrator loop in its CLAUDE.md.
- Harness pass for the stray flag + sheet; a `worktrees adopt <path>` verb
  if the move gets typed by hand more than twice.
- CdV leftovers: sibling `…-design-skills` worktree, two unregistered dirs in
  `.dmux/worktrees/`, modified `infra/terraform/environments/prod/terraform.tfvars`.
- Sub-places, brief-progress as status, a `worktrees agents` view — parked.
