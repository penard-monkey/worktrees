---
title: "Session — the MCP heartbeat test on a virtual clock"
---

# Session — the MCP heartbeat test on a virtual clock

- **Date:** 2026-10-01
- **Worktree:** `.worktrees/flaky-heartbeat`
- **Branch:** `flaky-heartbeat-virtual-clock`
- **PR:** [#389](https://github.com/penard-monkey/worktrees/pull/389), squashed as `a2e1540`
- **Release tag:** none. The change is test-only, so there is no CHANGELOG entry.
- **Planning files:** `planning.tar.gz` (the lane's brief, `.planning/brief.md`; no task_plan/findings/progress).

## What shipped

**The flake.** CI run 36793011965 (`rust (macos-latest)`, on #359's `.gitignore`-only change) failed `mcp::tests::a_heartbeat_pulses_while_a_call_runs_and_never_after` with `n >= 3 pulses in 60ms at 5ms`. A re-run passed. The test ran `with_heartbeat` on real time: a 5ms beat thread during a real 60ms sleep. A loaded runner starves that thread.

**Fix** (`crates/worktrees-cli/src/mcp.rs`):
- `with_heartbeat(token, wait: Beat, notify, f)`. The beat thread's wait is injected the way `poll_until` takes its sleep. `Beat` is `Box<dyn FnMut(&Receiver<()>) -> Option<u64> + Send>`: `Some(secs)` for a beat, `None` once the stop channel disconnects.
- `real_beat(every_ms)` is production's wait: the same `recv_timeout` loop, with `t0` taken at the thread's first look as before. The cadence, the `secs.max(last + 1)` progress rule, join-before-reply and no-token-no-thread are unchanged.
- `Inflight::beat: fn(u64) -> Beat` (default `real_beat`) is the seam the wired call site uses.
- Heartbeat test:
  - Beats are driven by hand and every pulse is acknowledged.
  - It asserts the exact progress `[1, 2, 20, 21, 40]` and the token on every pulse.
  - One extra beat fires only after the stop channel drops, then sleeps 20ms before landing, so only the join can keep it ahead of the return.
  - After the return, `Arc::strong_count(&notify) == 1` shows the beat thread is gone.
  - With no token, a clock that would beat produces zero pulses.
- `a_long_tool_call_carries_the_heartbeat` had the same flaw: it asserted ≥1 pulse at 1ms during a real `list_places`. It now uses a one-beat clock and asserts exactly one pulse carrying `"lp"`.

## Decisions

- **Inject the wait, not a clock value.** The beat thread is the thing being starved. Only an injected *wait* lets a test decide when each beat happens and still exercise the real join and stop channel.
- **`Inflight::beat` is a `fn` pointer, not an `Arc<dyn Fn>`.** A non-capturing closure coerces to it, which is all the wired test needs, and the field stays `Copy`-cheap beside `progress_every_ms`.
- **Sibling tests left alone.** `stdin_closing_stops_a_wait` and `wait_pulses_the_callers_token_and_stops_silently_on_cancel` pulse after a real ≥1000ms sleep against a 1ms threshold. A monotonic clock guarantees that, not the scheduler. Their only real-time bound is an upper `< 5s`. The same holds for `run_automation`'s "must not block" `< 5s` (~mcp.rs:3185). `poll_until_stops_at_the_step_after_a_cancel` was already virtual. No real-time lower bounds remain in `mcp.rs` tests.

## Dead ends / gotchas

- **The first rewrite of the test could not fail when the join was removed.** The racing last beat was sent before `f` returned, and the beat thread simply won the race every time, so "join removed" stayed green. The fix: the late beat waits for the stop channel to disconnect and then lands slowly. It now models production's real race, a `recv_timeout` that times out just as the stop drops. The general lesson: a "nothing after return" assertion needs an event that can only come late.
- **The local stress loop cannot reproduce the CI flake.** The OLD test passed 50/50 on this 14-core M-series Mac with `yes` on 28 processes. Macs don't starve a runnable thread the way a 3-vCPU CI VM does. The evidence for the fix is that no real-time dependency is left, not the loop count.
- **Local `pnpm` is broken** here: corepack can't find `pnpm/12.8.1/bin/pnpm.cjs`, and `nvm use 22.23.2` is not installed (22.19.0 is). So `tsc --noEmit` did not run locally. The diff had no TypeScript and CI's `app` jobs passed.

## Verification

- **Red first.** Each mutation went red on exactly the test that guards it:
  - no join → `[1, 2, 20, 21]` vs `[…, 40]`
  - no-token spawns a thread → "no token, no pulses"
  - `last = secs` → `[0, 0, 20, 20, 40]`
  - call site never passes the token → 0 vs 1
- **Stress.** Both heartbeat tests plus three siblings ran 50× with `yes` on all 14 cores: 50/50 pass.
- **Gates.** These passed:
  - release build (checked fresher than `mcp.rs`)
  - `make test` (395, 0 `not ok`) and `make lint`
  - core (564), cli (40), app `--lib` (170 + 1 ignored), `cargo check -p app`
- **CI.** PR run 36887350510 passed 9/9.

## Follow-ups

- None from this lane. The older macOS-only `skills.bats` flake is already on the ROADMAP.
