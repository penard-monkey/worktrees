# plan-ask-main — "Ask (main)" in the Plan tab

- **Date:** 2026-10-15
- **Worktree:** `.worktrees/plan-ask-main`
- **Branches:** `plan-ask-main` (squash-merged); `plan-ask-main-closeout` (this archive)
- **PR:** [#489](https://github.com/penard-monkey/worktrees/pull/489), merged as `9c080b4`
- **Release:** none. The entry is under `## [Unreleased]` → `### Added`
- **Planning files:** none. `planning.tar.gz` holds the brief (`.planning/brief.md`). `proposal.md` is the Phase A proposal (in this directory) that the orchestrator approved

## What shipped

- **A dock-header button, "Ask (main)", on the Plan tab** (`app/src/App.tsx`, beside "open"). It shows when the place has a plan or a brief. It is hidden on `(main)` itself, and disabled with an honest tooltip when (main) has no session, or has a session but no agent. On a click the app pastes the prompt into (main), then switches to (main) and shows "Prompt pasted into (main) — press Enter there".
- **`ops::PLAN_UPDATE_PROMPT` + `ops::plan_update_prompt(slug)`** (`crates/worktrees-core/src/ops.rs`).
  - One line with one `{slug}` hole. It tells (main) to read the place's progress and update its own plan.
  - It also tells (main) to rewrite the lane's `.planning/brief.md` if stale, to touch nothing else in the lane's tree, and then to message the lane.
  - The slug filter accepts only `[A-Za-z0-9._-]`, with no leading `.`, at most 128 bytes.
- **Tauri `plan_ask_main(root, session, provider)`** (`app/src-tauri/src/lib.rs`).
  - The slug comes from the guarded `root`, which must be a direct child of the project's `.worktrees/`. The frontend never supplies any text.
  - It resolves the AI pane once, captures it, and runs the pure `ask_main_refusal` on the capture.
  - It pastes into that same pane with the new `tmux::paste_into`, and never presses Enter.
- **`ask_main_refusal` refuses the paste when:**
  - (main) is waiting on a modal: Claude's probe for that pane/name says `waiting`, or Codex's `waiting_on_screen` matches;
  - the input box cannot be recognised (`agent::prompt_box_on_screen`, new; `codex::composer_on_screen` returning `None`);
  - the box holds a draft (`draft_from_screen` on the `-e` capture, so Claude's own DIM suggestion is not taken for typing);
  - for pi: the trust modal is up, pi is gone, or the screen is unreadable.
- **`tmux::paste_into(&PaneId, text)`**: the paste half of `paste_to_ai`, which now delegates to it.
- **Mock:** `plan_ask_main` is recorded in `__mock.askMain()` (`app/src/mock/install.ts`).

## Decisions

- **Paste, not send.** The MCP `report` log was rejected because (main) reads it only when it calls `messages`/`wait`, so the request would sit there unread. Send-keys + Enter was rejected because "no draft found" is not proof the box is empty, and the Generate-plan rule is that the user reads the prompt before sending it. The switch to (main) is what puts the composer in front of the user.
- **(main) may write the lane's brief directly** (orchestrator's call). The brief is (main)'s document and untracked; the lane's `task_plan.md`/`progress.md` are the lane session's and stay read-only to (main). Re-issuing through `create_worktree` and message-only were both ruled out.
- **Not in the empty state.** A place with no brief was never steered by (main), so Generate plan is the right action there.
- **The label stays "Ask (main)"; the harness is named only in the tooltip.** The proposal had "Ask (main)'s Codex", but the header is narrow.
- **The slug filter stays narrow** (review nit 5). Legal branch names with `+#@` produce a refusal, which is honest. Widening it is a ROADMAP item, not a fix.

## Dead ends / gotchas

- **The brief's slot runner was stale.** `<root>/target/release/worktrees` was v0.40.1, which has no `run` verb, so `run --slot gate -- …` printed usage and exited 1. That reads like a failing gate. The fix was the branch's own fresh release build (`target/release/worktrees` in the worktree). A fresh worktree has its own `target/`, because the workspace does NOT share the root's.
- **The first cut read "no draft" as "empty".** Fable's review caught that `draft_from_screen` returns `None` both for an empty box and for no recognisable box at all: a `/model` selector, the trust prompt, or a permission prompt with no probe. The paste would have landed in the selector. The fix was a separate box-presence predicate, `prompt_box_on_screen`, sharing `prompt_start` with `draft_from_screen`.
- **The first test passed identical text as `plain` and `styled`,** so swapping the `-e` capture for the plain one stayed green: the AGENTS.md "test that cannot fail" shape. It now carries a dim-suggestion pair that differs on purpose. It was shown red by the swap, and red again with the no-box check removed.
- **The first `waiting` check keyed on cwd == repo root.** That would let a waiting claude in some other terminal refuse the paste. It now keys on the probe's pane `%id` or peer name.

## Verification

- Every new test was shown red with its guard removed, then restored by editing back (with `touch`): the slug filter; draft styled→plain; the no-box check.
- Gates, run twice (initial PR, then review fixes), all green:
  - fresh release build;
  - `make test` (1..511, 0 `not ok`);
  - `make lint`;
  - `cargo test` for core (767), cli (67) and app --lib (163);
  - `tsc`, `cargo check -p app`;
  - every `app/scripts/*-check.mjs` from the repo root.
- CI: both runs were green on all 9 jobs (the second was 38757559311 on `1755e79`).
- Mock harness (vite on :1487, Chrome):
  - the button hit-tests in the header;
  - a click recorded `{slug, session, provider}`, switched to (main) and showed the notice;
  - the button is absent on (main).
- **Not done: a real paste into a live (main) session.** David's running app was not driven.

## Follow-ups

See ROADMAP: the real-app pass, a pi composer reader, and widening the slug filter.
