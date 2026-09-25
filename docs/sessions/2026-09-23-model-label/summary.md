---
title: "2026-09-23 — Model in the agent label"
---

# The agent label names the model

- **Date:** 2026-09-23
- **Worktree:** `.worktrees/feat-model-label`
- **Branch:** `feat-model-label` (2 commits, squash-merged)
- **PR:** [#335](https://github.com/penard-monkey/worktrees/pull/335), merge `4f86339`
- **Release tag:** none yet. The CHANGELOG entry is under `## [Unreleased]`
- **Planning files:** none. The session kept no `task_plan.md`/`findings.md`/`progress.md`

## What shipped

The strip above the agent's terminal used to say only "Claude" or "Codex".
Now it also names the model: "Claude Opus 5.5" or "Codex gpt-6-astra". If you
switch model with `/model` partway through a session, the label follows within
one poll tick (about 3s). That works for both providers and does not wait for
your next message.

- **Where Claude's model comes from:** `crates/worktrees-core/src/agent.rs`
  (`transcript_model`, `model_display`). The probe file
  (`~/.claude/sessions/<pid>.json`) does not record a model, but the
  session's transcript does, in two places:
  - every assistant reply's `message.model`;
  - the confirmation a `/model` switch writes immediately, a `user` entry
    whose content is ``<local-command-stdout>Set model to `Fable 5.1` …``.

  Whichever of the two is newer wins. Ids are shortened for display, so
  `claude-opus-5-5[1m]` becomes `Opus 5.5` and `claude-haiku-4-5-20251001`
  becomes `Haiku 4.5`. An id in any other shape is shown exactly as written.
- **Finding the right Claude transcript:** `app/src-tauri/src/lib.rs`
  (`claude_model`, `probe_model`). The code takes the probe whose `tmux` field
  names the same tmux session the label sits over, not merely any probe with
  the same cwd. The transcript path is built from that probe's own config root
  (`ClaudeProbe::root`, a `#[serde(skip)]` field that `live_probes` fills in).
  That way a profiled session reads its own dir instead of `~/.claude`.
- **Where Codex's model comes from:** `crates/worktrees-core/src/codex.rs`
  (`latest_rollout`, `rollout_model`, `user_thread_cwd`). The code looks back
  14 day dirs for the newest rollout from a *user* thread whose cwd is the
  worktree, and reads the newest `turn_context.model` or
  `thread_settings_applied.thread_settings.model` in it.
- **Keeping the label current:** `app/src-tauri/src/lib.rs`
  (`claude_activity`, `codex_models_moved`, `CODEX_WATCH`, `MODEL_CACHE`).
  - Each poll tick checks the current model of every live agent and fires
    `places:changed` when one differs.
  - Transcripts and rollouts are cached by file length, so a file is tail-read
    only after it has grown.
  - If a grown tail now holds no model at all (one huge tool result filling
    it), the previous answer is kept.
- **Frontend:** `App.tsx` / `App.css` gain `.agent-name` / `.agent-model`,
  and `AgentSession` gains an optional `model`. The model is shown in the same
  colour as the label, so "Claude Opus 5.5" reads as one name.
- **Close button removed from the label.** The label strip's own Close button
  is gone, because the ⋯ menu's "Close session" already does the job. It was
  the only caller that passed `provider` to `close_place`, so `doClose` no
  longer takes that argument. The Tauri command still accepts it, and the
  CLI's `--ai` is unchanged.

## Decisions

- **Read the transcript rather than the settings.** The label shows the model
  the session is actually on. A default in `settings.json`, or the model
  another session saved as default, would be wrong for any session that has
  switched. A session that has neither replied nor switched shows the provider
  alone rather than a guessed default.
- **Match the session by its tmux name, not its cwd.** A second `claude` started
  by hand in the dock shell has the same cwd, and it is not the agent the
  label is about.
- **Refresh when the model changes, not on a timer or on status.** The first
  fix fingerprinted each probe's `statusUpdatedAt`. That caught turns but not a
  `/model` switch, which changes neither tmux nor the status. Comparing the
  models themselves catches all three cases: the first reply, a Claude switch
  and a Codex switch. It also fires only when the label would actually change.
- **Leave the mock's `close_place` provider branch in place.** The mock mirrors
  the backend command, which still takes `provider`.

## Dead ends / gotchas

- **"It doesn't show" was a refresh delay, not a parsing bug.** The snapshot
  already returned `"model":"Opus 5.5"`. Places only re-list on a tmux
  fingerprint change or the 30s safety tick, and a finished turn is neither.
  The busy-set edge doesn't help either: a 2s reply never spans the two ticks
  that edge needs. Calling the real `snapshot()` from a throwaway test against
  the live sandbox repo (`WORKTREES_PREFIX=sbx-…`) settled it in one run, and
  it is the fastest way to split a backend fault from a frontend one here.
- **Codex's own sub-agents share the cwd.** The auto-review "guardian" writes
  its own rollout with the same cwd, and it starts *after* the session that
  spawned it. So "newest rollout for this cwd" was the reviewer, and the label
  read `codex-auto-review`. A user thread has no `parent_thread_id` and no
  `source.subagent`.
- **Codex creates the rollout file seconds before writing its first line.**
  That line (`session_meta`, over 20KB because it carries the whole base
  prompt) arrives a few seconds later. A lookup in that gap was cached as "no
  rollout" for 30s. Now a first line is cached per file only once it is
  complete, and the 30s rollout TTL is gone.
- **I wrongly concluded Codex records nothing until the next turn.** In the one
  rollout I had, `thread_settings_applied` fell in the same second as a
  `task_started`. That session was from VS Code, which applies settings when
  you send, so I read it as "Codex writes the model when the turn starts". The
  CLI writes the event *at the switch*: 42s before the next message in the
  sampled rollout, and again on the switch back with no message after it.
  **Sample the client you are building for; one coincidence is not a rule.**
  Codex's `state_5.sqlite` `threads.model` column is no better: it updated
  only after the next turn finished.
- **A throwaway `codex` run is not free.** On a new folder it asks to save a
  trust decision into `~/.codex/config.toml`. The probe was abandoned at that
  prompt and the config left untouched. Next time, use the user's own session
  or ask first.

## Verification

- **Gates run on the final code:**
  - release build;
  - `make test` (0 `not ok`);
  - `make lint`;
  - `cargo test -p worktrees-core` (447);
  - `cargo test -p worktrees-cli` (15);
  - `cargo test -p app --lib` (144);
  - `tsc --noEmit`;
  - `cargo check -p app`;
  - every `app/scripts/*-check.mjs` (after the Close-button removal).
- **Tests shown to FAIL first:** `a_subagent_rollout_is_not_the_users_thread`
  (with the filter removed) and `a_half_written_first_line_is_asked_again`
  (with the completeness check removed). The two `/model` switch tests fail
  against the reply-only parsers by construction.
- **Checked against live data**, through temporary tests that were then
  removed:
  - Claude: `Opus 5.5`, then `Fable 5.1` from the switch alone, with no reply
    after it;
  - Codex: `gpt-6-astra`, `gpt-6-sol`, then back to `gpt-6-astra` after a
    switch with no message sent.
- **Confirmed by the user in the sandbox app:** the Claude label, and the
  Claude mid-session switch.
- **Not verified:**
  - the Codex mid-session switch in the running app after the final fix
    (verified only at the snapshot level);
  - the label strip's height now that the button that set it is gone;
  - anything in the mock harness.

## Follow-ups

- The label strip may be a few pixels shorter without the Close button. Pin
  its height if it looks cramped.
- `CODEX_WATCH` drops a worktree only when a snapshot of its repo sees Codex
  down. A project removed while its Codex session is live stays watched until
  the app restarts, which costs a few `read_dir` calls per tick.
