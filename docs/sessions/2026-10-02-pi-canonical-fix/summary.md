---
title: "Session — pi typed into a place's own session"
---

# Session — pi typed into a place's own session, and `unknown` instead of `none`

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/pi-canonical-fix`
- **Branches:** `pi-canonical-fix` (a pi agent's first cut, then this
  session's rework on top), `pi-canonical-fix-ps-ttys`,
  `pi-canonical-fix-close-out` (this archive)
- **PRs:** [#409](https://github.com/penard-monkey/worktrees/pull/409) and its
  follow-up [#412](https://github.com/penard-monkey/worktrees/pull/412), both
  squash-merged by the orchestrator in `(main)` after a fable review. #409's
  first commit was written by a pi agent (qwen3-coder-next). The review
  verdict was REWORK, and this lane took over the same branch and PR,
  adding commits on top with no force-push.
- **Release:** none. The entries sit under `[Unreleased]`.
- **Planning files:** only the handover `.planning/brief.md`, in
  `planning.tar.gz` here. It holds the fable review verbatim and the original
  investigation of the valleos `pi-bench-log-redact` lane.

## Why

An orchestrator hand-launched pi into a Claude lane: it ran `/exit` on Claude
and typed `pi …` into the place's own tmux session. tmux names only the
interpreter, `node`, and the `node` heuristic hands a canonical-session pane to
Claude. So the lane had:

- no nav dot;
- `place_status` answering `agent_state: none` mid-turn;
- `wait until: idle` returning at once;
- `send` refusing;
- and `open --ai pi` would have started a second pi on the place.

## What shipped

**#409 (squash `3bdc200`)**
- `crates/worktrees-core/src/tmux.rs`: `PaneList` holds one `Pane` row per
  pane, `{session, path, cmd, pid, tty, fg}`, replacing the parallel vectors.
  `fetch` adds `#{pane_tty}`. Only when a `node` pane outside a provider
  sidecar or dock shell needs naming does it run one `ps` and record each
  tty's foreground process-group leader (`pid == tpgid`). The ps parser
  (`parse_foreground`) and the pane-row parser (`parse_pane_rows`) are pure.
  `normalize_tty` strips `/dev/`. `Pane::program()` uses the leader only
  when it names a harness on a wrapper pane.
- `provider::for_pane` and `canonical_provider` are pure again over the
  snapshot.
- `activity::pi_session_for`: pi's activity (`pi.rs`) and `send`
  (`harness.rs`) route through `PI.session_name(canonical, owner)`, as Codex
  does.
- `activity::place_answer` is the one derivation behind `place_status`
  (`mcp.rs` `add_agent_status`) and `wait` (`place_activity`). With no
  harness reporting and a non-shell program in the canonical session, the
  answer is `State::Unknown` with an `Activity::reason`. Rank order is Busy <
  Waiting < Idle < Unknown < None. `settled` never settles on it, and
  `may_type` has its own text for it.
- `guidance/SKILL.md`: start an agent only through a launch, with the real
  costs of a hand-typed one, and what `unknown` means. Guidance `VERSION` 4.
- Tests: core (ps over macOS and Linux fixtures, tty normalisation, 3/4/5-field
  rows, the ps count, attribution, `place_answer`), cli (`settled`), app
  (`agent_sessions_for`), and bats. The bats shim's `list-panes -a` now
  prints 5 fields with `/dev/`-spelled ttys, and an always-on fake `ps` logs
  to `ps.log`.

**#412 (squash `64028ab`)**
- `ps_foreground(ttys)` runs `ps -t <the wanted ttys>`, which took under 10ms
  against 120–130ms for `-A`. It falls back to `ps -A` only when `-t` fails.
- CHANGELOG spells out the behaviour change: any unclaimed program (`vim`,
  `pnpm dev`) answers `unknown`, and `wait` runs to its timeout.
- ROADMAP: the hand-test item gains the real-app dot for a hand-typed pi and
  the Linux ps path.

## Decisions

- **Resolve once in `fetch`; every question after that is pure.** The first
  cut spawned tmux and ps lazily inside `for_pane`, `canonical_provider` and
  `agents_in` loops, which also made an existing unit test hit the real tmux.
- **Skip ps for sidecars and dock shells.** A sidecar's name already says
  whose it is, and the app fetches a snapshot every 3s. In the common case
  no ps runs at all.
- **A leader counts only when it names a harness.** An `npx`-started Claude
  leads with `npm`, and reading that would have taken it away from the
  `node` → Claude heuristic. This is why `ls --json` stayed byte-identical to
  v0.35.0.
- **`unknown` covers ANY unclaimed program, not only agents.** An
  orchestrator told "nobody is there" acts on it. The reason text names the
  program and admits the other case, an agent that has not started reporting
  yet.
- **Guidance VERSION 4 stays.** VERSION 3 shipped in v0.35.0, so the bump
  re-shows a real change. The SKILL's first-cut reason ("worktrees cannot
  tell") became false with this fix and was replaced by the real costs: no
  record, no places rule, and a resume onto the file pi is still writing.
- **`ps -t` with a `ps -A` fallback** (#412). The peer asked for `-t` only.

## Dead ends / gotchas

- **The CHANGELOG entry merged CLEANLY into the released `[0.35.0]`
  section.** The merge of origin/main put the pi PR's `[Unreleased]` entry
  inside the release, exactly as AGENTS.md warns. Found by diffing
  CHANGELOG against origin/main after the merge, not from the exit code.
- **`ps -t` fails the whole call if any listed tty is gone.** On macOS it
  exits 1 with no rows and prints "No such file or directory". On
  procps-ng 4.0.4 it exits 1 with "TTY could not be found". A container
  without `-t` has NO ptys, so every `-t` failed there and looked like
  "procps can't take a list"; `docker run -t` under `script` gave it a real
  `pts/0`, and the list form works.
- **An MCP stdio probe that closes stdin cancels `wait`.** `wait` answered
  `timeout` with `waited_s: 2` against `timeout_s: 10`. That came from the
  probe (EOF reads as a cancel), not the product. Keep stdin open with
  `(printf …; sleep N) | worktrees mcp`.
- **A tmux socket under the scratchpad path is "File name too long".** The
  throwaway server needed a short `mktemp -d /tmp/…` instead.
- **The shell-sidecar marker is `~term`, not `-term`.** The first draft of
  a test used the wrong one.
- **macOS `ps` `comm` is argv[0] or the process title.** It names `pi` and
  `claude` even where tmux and `ucomm` say `node` or `2.1.287`.

## Verification

- Every new test was shown red against a deliberately broken line
  (normalisation dropped, fg ignored, any leader counting, ps for sidecars or
  always, the old `agent_state` line, `settled` ignoring unknown, the old
  rank, no dedup, no fallback, `-A` only), then green.
- Gates for both PRs, release build first: `make test` 450/450, `make lint`,
  core 655/656, cli 70, app 151, `tsc`, `make test-frontend`. CI was 9/9 on
  both PRs.
- `ls --json` was byte-identical to the installed v0.35.0 for both.
- Live, against real tmux and real ps on a throwaway server (`TMUX` unset,
  own socket dir) with a scratch place, since removed. A
  `node -e 'process.title="pi"…'` typed into the place's pane read `none` on
  v0.35.0 and `idle pi @<canonical>` on the branch, and `wait` settled after
  pi's two samples. `sleep 300` read `none` (0s) on v0.35.0 and `unknown`
  plus reason on the branch, where `wait` returned `timeout`.
- Linux `ps -t` list and vanished-tty behaviour were measured in a Debian
  container on procps-ng 4.0.4.

## Follow-ups

- In ROADMAP, under "Hand-test the pi nav dot in the real app": the running
  app's dot for a hand-typed pi was never watched (only
  `agent_sessions_for` is unit-tested), and the Linux ps path has never run
  on a real desktop.
- For the valleos memory, from the original investigation: a pi-lanes recipe
  (`worktrees new … --ai pi --model lm-studio/qwen3-coder-next`). It is not
  this repo's, and it was not touched.
