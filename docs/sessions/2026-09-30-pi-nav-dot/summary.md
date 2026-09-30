---
title: "Session — pi lanes get their nav dot"
---

# Session — pi lanes get their nav dot

- **Date:** 2026-09-30
- **Worktree:** `.worktrees/pi-nav-dot`
- **Branch:** `pi-nav-dot-fix`
- **PR:** [#382](https://github.com/penard-monkey/worktrees/pull/382), squashed as `faac134`. It has three commits: the fix, a pointer from the harness checklist, and the review follow-ups.
- **Release tag:** v0.34.1 (#386)
- **Planning files:** `planning.tar.gz` (the lane's brief, `.planning/brief.md`; no task_plan/findings/progress).

## What shipped

**Root cause.** A pi lane never showed a nav dot in the app: no green, no amber, no afterglow.
- Core was right: MCP `place_status` answered `busy` for a working valleos pi lane.
- The app's dot poll (`app/src-tauri/src/lib.rs`, the 3s thread) built `sessions:busy` / `sessions:done` from only two sources:
  - Claude's probes (`claude_activity`);
  - Codex's rollouts (`codex_tick`).
- Phase 2 (#366/#367) wired pi into core's `activity` but never into this poll, and nothing tested the dots against pi.

**Fix.**
- **One dispatch for every harness** (`lib.rs`).
  - `harness_feed(id)` returns one of three things:
    - `Feed::Probes` for Claude;
    - `Feed::Lane(LaneTick)` for Codex and pi;
    - `Feed::Unpolled`, returned by a `_` arm on purpose.
  - The snapshot's Codex-only watch became `watch_lane(id, …)` for every session in `agent_sessions`.
  - `merge_activity` takes any number of lanes.
  - `codex_new_dones` became `new_dones`, with one seen-map per harness.
  - `CodexTick` was renamed `LaneTick`. Codex's behaviour is unchanged, and `edges` (the dwell counter's input) is still Claude-only.
- **`pi_tick` / `pi_tick_in`** (`lib.rs`).
  - Fed by `PI_WATCH`, which the snapshot sets while pi itself runs in the place's session.
  - It calls core's `pi::pi_state`, the same derivation `place_status` makes.
  - Its input is the lane's session file, via core's new `pi::lane_file` / `lane_file_in`.
  - The pane is captured only when the file cannot answer:
    - while the lane is starting (blank pane, or the trust modal → waiting);
    - while no turn is on file yet.
  - A lane is "settled" once its file has moved since the first tick, or once a capture showed pi's composer (`pi::at_composer`).
  - All captures of a tick share one `tmux` call. `activity::capture_chain` / `chain_blocks_in` were extracted from `codex_panes`, and Codex now maps its screens through them.
  - Afterglow is stamped from pi's own `Done { at }`, never at an Esc or a failed turn.
  - `models_moved` triggers a re-list.
- **Review follow-ups** (third commit).
  - `live_model("pi")` reads by the place's dir, as Codex does, not through the `~agent~pi` suffix.
  - The pi startup memory is keyed on the launch as well as the name. `PaneList::session_launch` returns the pane pid, read as an optional fourth `list-panes` field.
- **Mock** (`app/src/mock/`): a pi place `catalog-import` whose row cycles through busy, then afterglow, then amber.
- **Docs:** the "nav dot poll" item in `docs/adding-a-harness.md` now names `harness_feed`, `watch_lane` and the drift test.

## Decisions

- **Capture only while the file cannot answer, not on every tick.** `place_status` captures every time. The dot runs every 3s over every lane, and the brief asked for stat plus tail-on-growth.
  - Measured on pi 0.99.1 in a throwaway tmux server:
    - the pane is blank until the trust modal draws;
    - the modal is the first thing pi draws;
    - the composer appears only after the modal is answered.
  - So once the composer has been seen, the screen can say nothing more that the file will not.
  - `read_composer` alone would also match the modal (it is two rules as well). `at_composer` therefore requires `read_screen` to be `Working`/`Other` first, and a core test pins it.
- **Accepted gap, the same as Codex:** a pi killed mid-turn leaves a file ending busy. The dot clears at the next snapshot re-list: a tmux change, or at most 30s.
- **A drift test, not a comment.** `every_harness_feeds_the_dot_poll` loops over `provider::PROVIDERS` and fails if an id has no arm in `harness_feed` (`Unpolled`) or `watch_lane` (`false`). There is deliberately no catch-all `Lane` arm, so the next harness fails loudly.
- **Startup memory keyed on the pane pid, not only the session name** (review). A close + open recreates the same `~agent~pi` name, so a resumed lane in `ask` mode kept `settled` and its trust modal was never captured. The pid rides as an optional fourth `list-panes` field:
  - the bats shim prints three fields, and a missing pid degrades to `None`;
  - `ls --json` does not carry the pid, so its output stayed byte-identical.

## Dead ends / gotchas

- **A test fixture older than the place dir is invisible.** The first red run of the `live_model` test failed on the wrong assertion: the sidecar name read `None` too.
  - `current_session` drops sessions created before the place's directory. This is the rule that keeps a re-used slug from resuming a dead lane.
  - The test's temp dir was brand new, so the fixture's header was "older" than it.
  - Shifting the fixture's dates to 2099 made it fail for the real reason. This is the "a test can pass (or fail) because of its fixture" rule in AGENTS.md again.
- **A struct literal in a core test helper** (`tmux.rs` `pl()`) broke `cargo test -p worktrees-core` after `PaneList` gained a field. `cargo check -p app` and the app tests never compile core's `mod tests`, so only the core test run caught it. Run it before pushing, not after.
- **pnpm in a fresh worktree:** corepack's cached pnpm 12.8.1 has no `bin/pnpm.cjs`, and no Node 22.23.2 was installed. `node ~/.cache/node/corepack/v1/pnpm/11.5.2/bin/pnpm.cjs install --frozen-lockfile` under Node 22.19.0 worked.

## Verification

- **Tests shown red first:**
  - `every_harness_feeds_the_dot_poll`, with the pi arm removed from each of the two functions in turn;
  - `a_busy_pi_lane_lights_the_nav_dot_and_its_finish_is_stamped`, against a tick that ignores the file;
  - core `a_batched_capture_reads_each_pi_pane_and_knows_the_composer`, against a modal-blind `at_composer`;
  - `a_pi_relaunch_resets_the_startup_memory`, keyed on the name only;
  - `the_snapshots_pi_model_does_not_depend_on_the_session_name`.
- **Gates**, after a fresh release build (checked `-nt`), twice:
  - `make test` 395/395, `make lint` ok;
  - core 564, cli 37, app 170 tests;
  - `tsc` and `cargo check -p app`;
  - all 25 `app/scripts/*-check.mjs` from the repo root.
- **`ls --json`** was byte-identical to the shipped v0.34.0 in this repo and in valleos.
- **CI** passed on all jobs for both pushes (runs 36742916571 and 36745294582).
- **Live, read-only,** on valleos `charterpanel-write-spec~agent~pi`: the tick's derivation read the file as `Busy` on `lm-studio/qwen/qwen3-coder-480b` and the pane as `Working` at its composer, the same as `place_status`. No keys were sent, and the running app was not driven.
- **Mock harness** (port 1431), sampled every second over 22s: the pi row went `status-dot busy` → `done t1 unread` ("pi finished just now") → `status-dot waiting`.

## Follow-ups

- **The real-app check was not done** (ROADMAP). Watch a pi lane go busy → afterglow, then amber on a resumed `ask` lane.
- **A pi killed mid-turn and reopened reads busy until its next turn** (ROADMAP, core, pre-existing).
