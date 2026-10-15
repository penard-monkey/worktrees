# resource-guard — keep many places from melting the machine

- **Date:** 2026-10-14 → 2026-10-15
- **Worktree:** `.worktrees/resource-guard`
- **Branches:** `resource-guard` (P1), `resource-guard-p2` (P2), both squash-merged; `resource-guard-close-out` (this archive)
- **PRs:** [#485](https://github.com/penard-monkey/worktrees/pull/485) (P1, merged as `4baa15b`); [#486](https://github.com/penard-monkey/worktrees/pull/486) (P2, merged as `469e1f6`)
- **Release:** none. Both entries are under `## [Unreleased]` → `### Added`
- **Planning files:** none. `planning.tar.gz` holds the brief (`.planning/brief.md`), and `proposal.md` is the Phase A design David approved

## The incident

On 2026-10-14 at about 21:51 David's Mac (14 cores, 36 GB) locked up and needed a reboot. valleos (main) nudged eleven lanes, and within 22 s seven of them ran `pnpm gate`: an uncached `turbo … --force` over the whole monorepo, using every core. Five casa-del-valle-monorepo lanes were working at the same time, and about 35 Claude sessions had been live across both repos. Each session costs ~1.2 GB before it does anything.

The transcript showed something the brief did not say: **the nudge was a raw `tmux send-keys` loop, not MCP `send`.** That reordered the plan. A throttle on `send` would never have seen it, and spacing eleven nudges 3 s apart still overlaps seven multi-minute gates. So the phases went 1 → 3 → 2, with slots first.

## What shipped

**P1, #485: machine-wide slots** (`crates/worktrees-core/src/slots.rs`)
- `worktrees run --slot <name> [--slots N] [--timeout S] -- <cmd>`:
  - Takes one of N flock'd files under `$XDG_STATE_HOME/worktrees/slots/<name>/` (per machine, never per repo), runs the command, and passes back its exit code (128+n on a signal). `--timeout` exits 75 and runs nothing.
  - Prints `waiting for slot gate (2/2 held by valleos:photo-order 3m12s, …)` while it waits.
- `worktrees slots [--json]` lists holders and waiters. Both commands work outside a repo (`main.rs` dispatches them ahead of the git guard).
- `[slots] gate = 2 / default = 2` in the user's config. `slots` is in `USER_ONLY_KEYS`.
- MCP `place_status` gains `slots: [{state, name, since_ms, cmd, held_by}]`.
- Guidance:
  - a "Heavy work" section in `guidance/SKILL.md`;
  - `guidance::HEAVY` after the place rule in `rules_text()` and the managed MCP instructions;
  - guidance `VERSION` 4 → 5.

**P2, #486: live-agent cap + memory floor at launch** (`crates/worktrees-core/src/guard.rs`)
- `guard::admit` sits in `ops::launch` on the quota gate's Soft-refusal path:
  - the place and brief are created, the launch is refused with exit 5;
  - one `force` overrides it (`--force`, "Launch anyway", MCP `force: true`).
- Counting:
  - tmux sessions with an agent pane (`PaneList::agent_sessions`), plus Claude probes outside tmux;
  - a bare `node` session counts only when a probe names it;
  - the launching place's own sessions are excluded, and a launch that only adopts a session already in the place is not checked at all.
- Memory is read through `sysctlbyname` (`kern.memorystatus_level`, `vm.swapusage`, `hw.memsize`) or `/proc/meminfo`. If it can't be read, the launch is allowed.
- Defaults: cap = RAM GiB / 2 (18 on this Mac), refuse below 20% free memory.
- `[limits]` in the user's config, in `USER_ONLY_KEYS`.
- Order in `ops::launch`:
  1. an unlocked `admit` (cheap early refusal);
  2. `Adapter::prepare` (quota, pi's host checks), with no machine lock held;
  3. `guard::launch_lock()` (`$XDG_STATE_HOME/worktrees/launch.lock`), then the deciding `admit`, held across `new-session` only.
- bats harness:
  - `WORKTREES_GUARD_PROBE=off` is pinned in `common_setup`, so a busy machine cannot fail the suite;
  - the fake tmux gains opt-in `FAKE_TMUX_NEW_CMD` / `FAKE_TMUX_NEW_DELAY`.

## Decisions

- **Slots first, stagger last or never.** See above. The slot is the only mechanism that bounds simultaneous heavy work whoever sends the nudge. P3 is in ROADMAP as hygiene, waiting on David.
- **The hold and the queue are both kernel locks.** A dead holder or waiter is gone because its fd closed. Pid liveness is not used anywhere: it was used for waiters at first, and review B1 showed why it is wrong (see below).
- **The lock fd is CLOEXEC, and the child stays in the caller's process group.** CLOEXEC means a daemon the gate leaves behind (turbo, metro) never holds a slot. The process group is a change from the proposal, which suggested a group of its own: a group-wide kill (a terminal's ^C) then reaches the gate too. The price is that SIGKILLing `run` alone frees the slot while its child runs on.
- **Signals.** TERM and HUP are forwarded, and `run` waits for the child, so the slot frees when the WORK stops. The first INT is not forwarded (the terminal already sent it to the group, and a second one makes pnpm and turbo skip their cleanup). A second INT becomes a TERM. A signal that lands before the child exists is stored in `PENDING` and passed on once it is spawned.
- **FIFO is bounded-overtake, deliberately.** A waiter takes a free slot only while fewer live waiters are older than it than there are free slots. Two newcomers in one poll can delay an elder by one round; a strict queue would need a ticket server.
- **The guard sits outside the adapters.** Memory is spent per session, so a harness added tomorrow is capped by default. That is the opposite of `usage()`'s opt-in, and the module doc says why.
- **Refuse, don't queue, a launch.** A queued launch needs a process that outlives the caller (the CLI exits, an MCP server dies with its session, the app isn't always open). Queueing lives in slots, where the caller is already blocking.
- **Cap default RAM/2.** David's call. The orchestrator suggested 12 (18 × 1.2 GB leaves little for gates); it's a one-line constant if he changes it.
- **`HEAD` stays under 247 chars.** Codex shows only the first 247 chars of the MCP instructions when tools are deferred, so the slot rule rides after the place rule instead of in it.
- **App clicks skip the launch gap** (David, for P3, if it is built).

## Dead ends / gotchas

- **`WT_BIN=<a raw binary>` makes every `run_wt` test red for the wrong reason.** `run_wt` runs `bash "$WT_BIN"`, so a binary fails with "cannot execute binary file". My first "red on v0.41.0" runs for both phases were invalid that way. To RED-check against the shipped release, use a one-line shim: `exec ~/.local/bin/worktrees "$@"`. Redone properly, everything was red as claimed; there's a correction comment on #485.
- **Waiter liveness by pid (review B1 on #485).** A waiter killed while waiting left `wait/<pid>.json`. Once that pid was reused by a long-lived process, the file read as a live, OLDER waiter, and every newcomer refused a FREE slot: "0/1 held by nobody yet; 1 waiting ahead" → exit 75. That is exactly the line that tempts an agent to drop the wrapper. The fix:
  - the wait file is flock'd from before its rename;
  - a file nobody holds is treated as dead and deleted, but only if the path still names the inode that was probed (a new waiter reusing the pid may have renamed its own file over it).
- **The live count over-reported a bundler as claude (review B1 on #486).** `provider::for_pane`'s wrapper rule (`node` → claude) is safe only inside a worktree path. Machine-wide, `cdv-metro` (metro) counted as an agent.
- **The machine lock was held across network probes (review B2 on #486).** `prepare`'s quota curl can wait up to 15 s, and every launch on the machine would have queued behind it. Hence the unlocked → prepare → locked order.
- **Claude Code's Bash tool does NOT kill on timeout; it moves the command to the background**, where it keeps running and keeps its slot. Stopping that background task killed the wrapper and its child, and the slot freed. This was measured, not assumed. The first doc draft asserted the opposite.
- **A flock flake in a multi-threaded test process.** `a_held_slot_is_seen_and_a_dropped_one_is_free` failed about 1 in 7 full core runs. A sibling test's fork shares the open file description, and with it the lock, until its exec closes the CLOEXEC fd. Production is unaffected (every spawn execs at once). The test now waits out the window. slots.rs now says: never add a fork that does not exec while a lock `File` may be alive on another thread.
- **`projcfg.rs` contains a NUL byte**, so plain `grep` treats it as binary and prints nothing. Use `grep -a`. `USER_ONLY_KEYS` looked missing until I did.
- **A CHANGELOG rebase across two PRs needs its headers counted.** The P2 rebase conflicted in CHANGELOG and `USER_ONLY_KEYS`; both resolve to "keep both". Afterwards I checked for one `[Unreleased]` and one `### Added`.

## Verification

- **Every new test was shown RED first:**
  - against v0.41.0, through the shim;
  - or by a mutation targeted at its own rule: ignore the flock result; clear CLOEXEC; treat file content as liveness; drop the FIFO check; pid-only waiter liveness; `>=` → `>` in the cap; `launch_lock()` → `None`; count every pane as an agent; `0` → `Some(0)`; disable the adoption skip; drop the problem warnings; drop the unknown-provider check.
  - Each was restored by edit + `touch`.
- **Real processes:** 5 parallel `run --slot t --slots 2` jobs logged a maximum overlap of exactly 2; a `kill -9`'d holder freed its slot within 100 ms; a forked `sleep 30` daemon did not hold the slot.
- **Final gates on #486's last commit (`2f4276f`):**
  - release build first, binary confirmed fresh;
  - `make test` 511/511;
  - `make lint` 0;
  - core 764 passed (2 ignored);
  - cli 67+1;
  - app 162;
  - `tsc` 0, `cargo check -p app` 0;
  - all `app/scripts/*-check.mjs` from the root.
- **CI:** #485 green on both OSes. The fable reviewer re-ran both PRs in isolation, macOS plus Linux in docker.

## Follow-ups (in ROADMAP)

- **P3:** launch and send gaps. Waiting on David.
- **P4:** Settings → "Limits", the "waiting for slot" chip on a place's status row, and `slots` in `ls --json`.
- **A cap on BUSY lanes** (the quota-burn signal), and budgets that span hubs and machines.
- **Stale Claude probes whose pid was reused** over-count by one. Visible in the refusal text; fix only if it bites.
- **Not filed:** the project-rules lane wasn't running, so its note is only in #485's body. The rules pack should name each repo's heavy commands, wrapped in `worktrees run --slot gate -- …` (valleos: `pnpm gate`, `turbo run … --force`, `pnpm install`; this repo: `make test`, `cargo build --release`).
