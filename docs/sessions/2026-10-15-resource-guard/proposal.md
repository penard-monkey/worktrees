# resource-guard — proposal (Phase A)

## 0. What the transcript actually shows (it changes the priorities)

- The nudge was **raw `tmux send-keys`** in a Bash loop from valleos (main)
  (`for l in dev-env demo-recorder catalogue-photos …; do tmux send-keys -t valleos-$l -l …`),
  **not** MCP `send`. A throttle on `send` would not have seen it at all.
- Even a 3 s stagger spreads 11 nudges over ~30 s. A `pnpm gate --force` runs for minutes, so all
  seven would still have overlapped. **Stagger is hygiene. The fix is #1, slots.** Only a cap on
  concurrent heavy work stops this shape, whoever sends the nudge.
- 35 sessions × ~1.2 GB idle is ~42 GB on a 36 GB machine. The machine was already in swap before
  any gate ran, which is the case for #3.
- Swap is still 7.7 / 9.2 GB used right now (`vm.swapusage`); `kern.memorystatus_level` = 85.

Recommended order: **1 → 3 → 2**.

## 1. Machine-wide slots: `worktrees run --slot`

**Surface**
```
worktrees run --slot gate [--slots N] [--timeout SECS] -- pnpm gate
worktrees slots [--json]          # name, capacity, holders, waiters
```
- Prints to stderr at once, then waits:
  `waiting for slot gate (2/2 held by valleos:photo-order 3m12s, worktrees:resource-guard 40s)`.
- Re-prints when a holder changes, then `got slot gate (1/2) after 4m03s`.
- Exit code = the child's. A child killed by a signal gives 128+n. `--timeout` expiry → exit 75
  (EX_TEMPFAIL) with "slot gate still busy after Ns; nothing ran".
- `--slot` with no name is an error. The name is `[a-z0-9-]+`.

**Config** (user `config.toml` only; add `slots` to `projcfg::USER_ONLY_KEYS` per ADR 0001):
```toml
[slots]
default = 2      # any name not listed
gate = 2
```
- `--slots N` overrides for that call only.
- Capacity is whatever each caller believes. Two callers with different N don't agree, so the docs
  will say to put it in config. `slots` warns when live holders disagree about N.

**Locking**
- Directory: `$XDG_STATE_HOME/worktrees/slots/<name>/` (`init::state_dir`). It is per machine and
  per user, never per repo.
- Slot k is the file `k.lock`, k < N. To acquire, try `flock(LOCK_EX|LOCK_NB)` on each in turn. The
  first one that succeeds is ours. We then truncate and write holder JSON into it: pid, project:place
  from cwd discovery (fallback: the path), argv, start epoch, N.
- Waiting polls every 500 ms (LOCK_NB over all k). A blocking flock on one file would pick a slot
  arbitrarily and miss the others freeing.
- A waiter registers `wait/<pid>.json` (place, since) so `slots`, MCP and the app can see it. It
  removes the file on acquire or exit.
- **Fairness:** a waiter takes a slot only if no older live waiter exists (FIFO by `since`, ties by
  pid). Without this, seven pollers race and one lane can starve.

**Robustness**
- The kernel drops a flock when its holder dies by any route, SIGKILL and crash included. A held
  slot therefore cannot outlive its process, and lock files never need cleanup.
- Pid liveness is needed only for the **metadata**:
  - a `wait/*.json` whose pid is dead is ignored and deleted;
  - holder JSON in a file we can LOCK_NB is stale and is shown as free.
- **The lock fd is CLOEXEC** (Rust's default; asserted in a test). If the child inherited it, a
  daemon the gate spawns (turbo daemon, metro, gradle) would hold the slot forever.
  - The cost: SIGKILL `worktrees run` itself and its child keeps running unslotted.
  - Mitigation: run the child in its own process group, and forward INT/TERM/HUP to that group. A
    Claude Bash-tool kill signals the whole group anyway.
- macOS and Linux both have BSD `flock` on a local fs. `$XDG_STATE_HOME` on NFS is out of scope and
  documented.
- **Agent tool timeouts are a real failure mode.** A Bash call that blocks 6 min waiting for a slot
  hits Claude's 2–10 min tool timeout. That is harmless, since no slot is held while waiting, but
  the agent may then retry WITHOUT the wrapper. The guidance says: run slotted gates in the
  background, or give them a long timeout. The immediate "waiting for slot" line tells the agent
  why it is blocked.

**Visibility**
- MCP `place_status` and `list_places`: `waiting_for_slot: {name, since, held_by:[…]}` and
  `holding_slot: {name, since}`, derived from the slot dir by place.
- App: the place status row/chip shows "waiting for slot gate · 2m". It reads the same core
  function in its existing poll; no new event.
- The CLI's `worktrees ls` gains the same in `--json`.

**Optional, David's call:** valleos's own `package.json` could make `gate`
`command -v worktrees >/dev/null && exec worktrees run --slot gate -- turbo … || turbo …`. It's the
repo's own script, so ADR 0001 is untouched. Unlike guidance, it can't be forgotten by an agent.

## 2. Staggered fan-out

The throttle lives in **core, behind a shared file**, because three kinds of process launch and send:
the long-lived MCP servers (one per session, and there are many), CLI processes, and the app
in-process. Serializing inside one MCP server would cover one orchestrator only. (One server already
answers requests one at a time.)

**One mechanism, two stamps:** `$STATE/fanout/{launch,send}.stamp`.
1. Take a flock on the stamp file.
2. Read the last epoch-ms. If less than `gap` has passed, sleep the difference while holding the lock.
3. Do the action (for launch: up to tmux `new-session` returning), write now, release.

N parallel callers therefore come out exactly `gap` apart, in whatever order the lock grants.

```toml
[fanout]
launch_gap_secs = 5   # create_worktree / new / open that START an agent
send_gap_secs = 3     # MCP send, CLI send
```
- 0 turns either off.
- The app's human clicks skip the gap; the cap and memory check from #3 still apply.
- The launch lock is the same lock #3 holds while it counts, so the cap is race-free. See §3.
- `max_in_flight` is **not proposed**. For a send, "in flight" only means "the lane is busy now",
  and that is #3's signal. An optional `send.max_busy` (refuse a `send` when ≥K lanes are busy) is
  listed as a later, deferred item.
- Raw `tmux send-keys` bypasses all of this, and nothing can stop it. The guidance says to use
  `send`.

## 3. Live-agent cap + memory check at launch

**Where:** one call, `guard::admit(provider, force)`, in `ops::launch` immediately before
`adapter.prepare(...)`, returning `Refusal::Soft`.
- It shares the same path, the same "place + brief created, only the launch refused" property and
  the same override: `--force`, "Launch anyway", `open --force`, MCP `force: true`.
- It is deliberately NOT inside each adapter. Memory cost is per session, not per provider, so a
  harness added tomorrow must be capped by default. That is the opposite of `usage()`'s opt-in, and
  the module doc will say why.

**Config:**
```toml
[limits]
max_live_agents = 16          # default: floor(RAM_GB / 2); 0 = off
max_live = { claude = 12 }    # optional per provider
min_free_memory_pct = 20      # macOS kern.memorystatus_level / Linux MemAvailable%; 0 = off
max_swap_used_pct = 0         # off by default; macOS swap grows, so % is noisy
```
- RAM / 2 here gives 18. That alone refuses the 19th through 35th sessions of the incident.
- Settings → a "Limits" group, same writer as `[quota]` (`USER_CONFIG_LOCK`).

**Counting:**
- Distinct tmux sessions with an agent pane, over all panes on the server (`PaneList::fetch` +
  `provider::for_pane`, the same test `agents_in` uses, just without the path filter). The tool owns
  that server, so this spans every project.
- Plus, for Claude: `~/.claude/sessions/<pid>.json` with a live pid and no tmux match. Those are
  claude sessions started outside tmux: David's terminal, the IDE. They cost the same RAM.
- Both are already read by `agent`.

**Race:**
- Count → `new-session` runs under the `launch.stamp` flock from #2. Eight parallel
  `create_worktree` calls then see 0, 1, 2, … in turn.
- This is the "cross-process launch reservation" ROADMAP asks for, with no expiry to reconcile. The
  reservation IS the lock, and the session exists before the lock is released.
- Lock order: repo agent-switch lock first, then the machine lock. That is consistent everywhere,
  so they can't deadlock.

**Memory reading:**
- macOS: `sysctlbyname("kern.memorystatus_level")`, and `vm.swapusage` (`xsw_usage`). These go
  through libc with no subprocess, since `memory_pressure` takes ~1 s.
- Linux: `/proc/meminfo` MemAvailable/MemTotal. Optionally also `/proc/pressure/memory` `some
  avg10` > 25.
- Any read error means allow (fail open, same as quota). The refusal names the number and the
  setting: "memory 14% free (limit 20%) — 22 agents live. Close some, or launch anyway: …".

**Refuse vs queue: recommend REFUSE.**
- A queued launch needs a process that outlives the caller to start it later. The CLI exits, the MCP
  server dies with its session, and the app isn't always running.
- "It started an hour later, unattended" is a surprise.
- The orchestrator already has `wait`, and the refusal text tells it to retry.
- Queueing belongs where the caller is already blocking, and that is #1. A lane waiting for a slot
  costs nothing. A queued launch costs a daemon.

**Interaction with quota:**
- Order: cap → memory → quota. They are local and cheap, then a keychain/network probe.
- The first refusal wins, and one `force` skips all three.
- The quota module's "not a burst limiter" doc and the ROADMAP item get updated to point here; the
  ROADMAP item is closed by this.

## 4. Reboot / tmux-restart stampede: answer, no build

The app does **not** restart sessions in bulk.
- Sessions are created only by explicit gestures (`enterPlace`, App.tsx:5511: double-click, Enter,
  ▸, quick-switch).
- A plain nav click only selects.
- The one automatic `open_place` (App.tsx:4948) reconciles two live providers in the SELECTED place
  only.
- No tmux-resurrect/continuum is configured.

After a reboot nothing relaunches until someone asks. The remaining risk is an orchestrator told
"reopen all my lanes" looping `open`/`create_worktree`.
- #2's launch gap spaces those out.
- #3's cap stops them at the limit.
- #1 keeps the resumed lanes' first gates two at a time.

## 5. Guidance vs rules pack

**`guidance/SKILL.md` + `HEAD` (this lane), portable:**
- "Wrap gates, builds and full test runs in `worktrees run --slot gate -- <cmd>`. If it prints
  `waiting for slot`, that is expected: run it in the background or with a long timeout, and never
  drop the wrapper to get past it."
- "Nudge or launch lanes in batches of ≤3 and let their heavy work finish before the next batch. Use
  `send`, never `tmux send-keys`, to talk to a lane."
- "A launch refused for live agents or memory: close finished lanes (`close_session`) before
  `force`."

**Rules pack (project-rules lane, untouched by me):**
- The per-repo list of WHICH commands are heavy, e.g. valleos: `pnpm gate`,
  `turbo run … --force`, `pnpm install`; this repo: `make test`, `cargo build --release`.
- The same slot line, worded for that repo's commands.
- A message to that lane once this is agreed.

## 6. Tests (real processes, no mocks)

- **Core unit tests:**
  - slot pick and FIFO logic over a temp dir;
  - CLOEXEC asserted with `fcntl(F_GETFD)`;
  - config parsing;
  - cap count over a `PaneList` built from fixture lines;
  - memory parsers on captured `/proc/meminfo` text and fail-open.
- **bats, real processes, temp `XDG_STATE_HOME`:**
  - Concurrency: start 4× `worktrees run --slot t --slots 2 -- sh -c 'echo start >>log; sleep 1; echo end >>log'` in parallel, then
    assert the log never shows more than 2 open intervals and that all 4 finished.
  - Kill: `kill -9` a holder's `worktrees run` → a waiter acquires within 1 s.
  - Crash: a child exiting 3 → exit 3, slot freed.
  - Fd leak: the child forks a `sleep 30` daemon and exits → the slot is free while the daemon lives.
  - Waiting line: the text names the holder's place.
  - `--timeout` gives 75 and the command never runs.
  - `slots --json` shape.
- **bats, fake tmux shim:**
  - the cap refuses the (N+1)th launch with exit `EXIT_LAUNCH_REFUSED`, place + brief created,
    `--force` launches;
  - the launch gap: two parallel `new`s → `new-session` timestamps ≥ gap apart.
- **MCP unit tests:** `place_status` includes `waiting_for_slot`; `send` honours the gap.
- Every new test shown RED first.
- Nothing spawns real agents. The heaviest thing run is `sleep`.

## 7. Size and phases (one PR each)

| Phase | What | Size |
|---|---|---|
| **P1** | `worktrees run --slot`, `slots`, `[slots]`, guidance lines, MCP/ls fields | ~600 lines + tests |
| **P2** | `guard::admit`: cap + memory, `[limits]`, the machine launch lock, quota docs/ROADMAP | ~500 + tests |
| **P3** | fanout gaps (launch + send), `[fanout]` | ~250 + tests |
| **P4** | app: slot chip, Settings → Limits (mock harness + `install.ts`) | ~300 |

P1 alone would have contained the incident's CPU half, and P2 its memory half.

## Questions for David

1. **Cap default.** On by default as RAM/2 (18 here), or off until set?
2. **valleos's own `gate` script.** Do you want me to propose the self-wrapping line to that repo's
   lane, as a belt to the guidance's braces?
3. **Fanout gap for app clicks.** Skip it (as proposed), or apply it everywhere?
