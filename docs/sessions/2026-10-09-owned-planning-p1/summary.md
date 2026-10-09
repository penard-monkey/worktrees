---
title: "2026-10-09 — owned planning, phase 1 (the cdv pilot build)"
---

# Owned planning, phase 1 — build close-out

- **Date:** 2026-10-09
- **Worktree:** `owned-planning-p1`
- **Branches:** `owned-planning-p1` (A), `owned-planning-p1-b` (B),
  `owned-planning-p1-close-out` (archive)
- **PRs:** [#471](https://github.com/penard-monkey/worktrees/pull/471) core + CLI,
  squash-merged as `fe3665a`;
  [#475](https://github.com/penard-monkey/worktrees/pull/475) Claude plugin + app,
  squash-merged as `9eb2ccc`. Both passed a fable review; CI 9/9 on each.
- **Release:** none yet (both under `## [Unreleased]`).
- **Spec:** [`docs/proposals/owned-planning.md`](../../proposals/owned-planning.md) §9 phase 1;
  design session [2026-10-09-owned-planning](../2026-10-09-owned-planning/summary.md).
- **Planning files:** `planning.tar.gz` beside this file (`task_plan.md`,
  `progress.md` — the red-first log).

## What shipped

**Core + CLI (#471)**
- `crates/worktrees-core/src/planning.rs`: user tier only.
  `~/.config/worktrees/planning.json` holds `{default: unset|full|off,
  version}`. The registry `Entry` (`registry.rs`) gains `planning` / `plan_path`
  / `plan_scope`. `planning::effective(main_root)` is the single reader, and it
  re-reads both files every call. `set_project` refuses full over a tracked
  `.planning/` (one `git ls-files`, at adoption only) and validates a show path
  against main. `ensure_planning_excluded` appends `/.planning/` to
  `$GIT_COMMON_DIR/info/exclude`.
- `crates/worktrees-core/src/plan.rs`: `pub fn resolve(root, &Mode)` with
  `Legacy | Owned | Show { rel, scope, main_root }` and a `Resolution`.
  - Legacy is the old code path, unchanged.
  - Owned is §3.2's case table: no newest-dir guess, `pending`, and
    `invalid_pointer`, which falls back to the root plan and never echoes the
    pointer.
  - `PlanSummary` gains `level` / `topic` / `plan_scope` / `reason`.
  - `summarize_place` picks the mode for the tab and MCP.
- `crates/worktrees-core/src/safepath.rs`: `safe_under`, moved from the
  docserver (which re-exports it), plus `safe_dir_under` and a reason classifier.
- `crates/worktrees-core/src/plancmd.rs`: `worktrees plan resolve [--json]`
  (JSON stamps the version) and `plan hook session|prompt` (3k/1.5k budgets,
  framed as data, prompt only on change via
  `~/.cache/worktrees/plan-hook/<hash(session_id)>`, silent unless full, always
  exit 0, ahead of the git guard), plus `plan default` and `plan level`.
- `ops.rs` `cmd_new`: at full, `.active_plan` = slug, an empty topic dir and the
  exclude line. Every `new` warns about a tracked `.planning/`.
- Tests:
  - `crates/worktrees-cli/tests/plan_parity.rs`: one binary, the tab's side vs
    the hook's side, across cdv's scenarios plus the symlink, `../` and evil
    cases;
  - `test/plan.bats`;
  - core unit tests.

**Claude plugin + app (#475)**
- `guidance::materialize_plan_in` writes a separate content-hashed
  `claude-plan/` plugin (SessionStart `startup|resume|clear|compact` +
  UserPromptSubmit). `ops::planning_for` puts it in a new `AiLaunch.planning`
  field, **outside** `guidance_for`. `plancmd::probe_cli` (`plan hook
  --version`) finds the hook's binary and its version.
- App:
  - `app/src/PlanningPanel.tsx`: Settings → Planning, `PlanningChoice` and
    `cliSkew`;
  - the `planning` offer in `offers.ts`;
  - in `App.tsx`: `AddProjectDialog` (Add existing now confirms, pre-set from
    `[plan]`), the planning row in New project / Clone, and Clone's post-clone
    line;
  - `PlanPane.tsx`: a widened union, `resolutionLine`, and the skew warning at
    full;
  - `lib.rs` commands `planning_status` / `set_planning_default` /
    `set_project_planning` / `planning_suggested`, and `probe_dir.plan_suggested`;
  - mock entries;
  - `app/scripts/planresolve-check.mjs`, a new drift guard: the TS union vs
    `plan::Resolved`, and every value with its own words.
- #471's review nits:
  - exclude before the pointer's early return;
  - normalize a show path at entry only;
  - a `.git` FILE is a readable main;
  - help text;
  - one `phase_sections` walk;
  - the fix-it wording printed once.

## Decisions

- **Two PRs, B based on A.** B needed `planning.rs`, the summary fields and the
  hook verb. After A was squash-merged, B was moved with `git rebase --onto
  origin/main <A tip>` so the pre-squash commit dropped out.
- **The planning plugin has its own `AiLaunch` field and its own hash dir.**
  Riding `guidance` would drop it whenever guidance is off. Sharing the hash dir
  would move the guidance plugin under running sessions the moment planning is
  turned on. There is a bats test for each, and the guidance one was shown red.
- **The global choice is two buttons, not three.** "Choose per project" and
  "Off" both store `off` (the proposal says so), so a third button could never
  show as selected after a reload.
- **Add existing gained a confirm dialog.** It is the only flow with the repo
  on disk, so it is the only one that can pre-set from `[plan]`. It costs one
  click per add, which was flagged in the PR.
- **Build notes added to the proposal** (§3.2) for cases the table left open:
  - a control character in a pointer is invalid;
  - a symlinked `task_plan.md` under a valid topic is `invalid_pointer`, never
    `pending`;
  - a symlinked `.planning` in owned mode is `invalid_pointer`;
  - `show_path` covers success and failure.

  The two final-review clarifications are in §2.5.2: main scope reads MAIN's
  working tree, never `HEAD:`, and "main unreadable" is a reason.
- **`clear` is in the SessionStart matcher.** Without it, a `/clear`ed session
  has no plan until the plan changes. The proposal is updated in this archive PR.

## Dead ends / gotchas

- **`mv file.bak file` keeps the OLDER mtime, and cargo then keeps the mutated
  build.** The parity test stayed red after the mutation was "restored". This is
  now an AGENTS.md line (#471). Restore by editing or `cat >`, or `touch`
  afterwards.
- **The model's own account of its context is not evidence.** In the two-turn
  stream-json probe, haiku said turn 3 carried plan context. Tee-ing the hook's
  stdout showed turn 3 printed nothing; the dedupe was correct. Probe hooks with
  a log on the command itself, and use the model only to show the text arrived.
- **`claude -p --resume` fires SessionStart:resume.** That carried the change
  and recorded the fingerprint, so it could not isolate UserPromptSubmit. One
  live `--input-format stream-json` session with the file edited between turns
  can. The script is `twoturn.py` in the scratch dir.
- **The app's float stack mounts only for `err || notice || undo`.** A new
  floating line (the post-clone hint) rendered nowhere until it joined that
  gate. The mock showed it; reading the code did not.
- **The mock answered an impossible combination** (show level with
  `active_plan`). `place_plan` now post-processes `place_plan_raw` so show
  always reports `show_path`, which is what core does.
- **A refused select kept displaying the refused value.** The control must say
  what is stored. `ProjectRow` reverts on refusal except for a show path, which
  stays editable.
- **The machine was at load ~50–100 during the gate runs** (other lanes), and
  bats looked hung at ~10 s per test. It was not; the run just took a while.
  Check `uptime` before suspecting a hang.
- **`cwd` drift in the Bash tool.** A `cd app && …` without a subshell moved the
  shell's cwd for later calls. Use `make -C` or `(cd …)`, as AGENTS.md says.

## Verification

- #471 gates: `make test` 490/490, lint, core 756, cli 70 + parity, app --lib
  162, tsc, `cargo check -p app`, all `*-check.mjs` from the root.
  `ls --json` is byte-identical to v0.40.1, and MCP `place_status.plan` is
  additive only.
- #475 gates: `make test` 494/494, lint, core 758, cli 70 + 1, app 162, tsc,
  check, `make test-frontend`. CI 9/9 on both PRs.
- Red-first, each restored to green:
  - owned tests with Owned→Legacy;
  - pointer validation disabled;
  - the parity test with the hook on legacy, which reproduced the §1.3 hijack:
    the hook named `.planning/orchestrator/task_plan.md`;
  - bats rows 1–4;
  - the plugin with guidance gating;
  - offers-check with a projects>0 precondition;
  - planresolve-check with `show_path` dropped;
  - nits 1–3 against the old code.
- Real Claude (haiku, `--plugin-dir`, scratch repo, worktrees config isolated
  by `XDG_CONFIG_HOME`):
  - SessionStart text was quoted back verbatim (`Plan: \`.planning/lane-x/task_plan.md\``
    and a marked open item);
  - UserPromptSubmit carried an edited progress line on the next turn of one
    live session;
  - the hook log showed nothing / update / nothing across three turns.
- Mock harness (Chrome) drove every surface:
  - the offer deep link with no project selected;
  - the global choice retiring the offer (7 → 6);
  - a tracked refusal in the row;
  - show path + main scope stored;
  - Add existing pre-set from `[plan]` (Add hit-tested), and a refused full
    becoming a notice;
  - Clone → post-clone line → stored full;
  - New project → stored full;
  - Plan tab: pending / invalid / showfail / main's copy / active + skew, and
    off showing nothing;
  - no console errors.
- **Not done:** the WKWebView/real-app pass. The sandbox takes no scripted
  input, so it is the pilot below.

## Pilot steps (cdv) — David's, after the release that ships #471 + #475

1. Update **both** the CLI and the app. Settings → Planning should show no
   version warning.
2. **Restart cdv `(main)`'s Claude session.** Its `worktrees mcp` server is the
   old image and would create lanes without a pointer, which reads exactly like
   adoption failing.
3. Sparkles → "Let Worktrees keep your agents' plans" → Review… → set
   casa-del-valle-monorepo to **Full**. No refusal is expected, because nothing
   under cdv's `.planning/` is tracked. Nothing in cdv changes: `.planning/` is
   already ignored.
4. Main's Plan tab shows **"active plan: orchestrator"**.
5. Create one lane. On disk: `.planning/.active_plan` = `<slug>` and an empty
   `.planning/<slug>/`.
6. The lane's tab shows **"no plan yet — the session writes it in
   .planning/<slug>/"**. Ask its Claude to "quote the plan context you were
   given at session start" to see the Pending line. `ps` shows `--plugin-dir
   …/claude-plan` on it.
7. Once the agent writes its plan, the tab shows **"active plan: <slug>"**. The
   next prompt's `[worktrees planning v…]` block names the same path, and
   `worktrees plan resolve --json` in the lane agrees.
8. Existing root-layout lanes show "root task_plan.md (the legacy location)".
9. To back out, set Off. Running sessions go quiet on their next prompt, and
   nothing is deleted.

## Follow-ups

In `ROADMAP.md`:
- the git-init route skips the planning row;
- `confirmAdd` drops the pick silently when the project was already
  registered;
- `probe_cli` spawns on every status read and full-level launch;
- `PlanningChoice` says "your default: off" when the default is unset;
- the pilot itself.

Phase 2 and 3 are unchanged.
