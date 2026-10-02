---
title: "Session — finishing brethash's quota gate (#358)"
---

# Session — finishing brethash's quota gate (#358)

- **Date:** 2026-10-01
- **Worktree:** `.worktrees/fix-358`
- **Branches:** `feat/agent-load-balance` (brethash's, commits added on top,
  never rewritten or force-pushed), `fix-358-closeout` (this archive)
- **PR:** [#358](https://github.com/penard-monkey/worktrees/pull/358),
  squash-merged as `5c1a010`, credited to brethash
- **Planning files:** none. The lane's brief (`.planning/brief.md`) is archived
  as `planning.tar.gz`

## What shipped

#358 refuses a Claude or Codex launch onto a nearly spent plan window, as a
`Refusal::Soft` in the adapter's `prepare` (`quota::gate`). It also moves both
usage readers into `worktrees-core`. Two reviews had already re-seated the gate
on the adapter. This lane added four commits on top of brethash's:

1. **Merge of `origin/main`.** The CHANGELOG `[Unreleased]` section was left
   with one `### Added` and one `### Changed`, keeping both sides' items.
2. **`aa55482`: the brief opener survives a quota refusal.** `cmd_open` sends
   `BRIEF_OPENER` only when `Adapter::never_launched` is true, and only pi
   implemented it. So the retry the refusal prints (`open <slug> --force`), and
   a plain `open` once the window had headroom, started Claude or Codex BLANK.
   Claude and Codex now answer it the way pi does,
   `!self.session_present(p, &p.place_dir(slug))`, in
   `crates/worktrees-core/src/harness.rs`. New `test/quota.bats` cases cover:
   - the opener after `open --force`, for Claude and for Codex;
   - the opener after a plain `open` with headroom;
   - NO opener once claude has a transcript in the place;
   - MCP `create_worktree` refused, then `force: true` launching with the
     opener.
3. **`a958ec0`: text.**
   - `LaunchRefusedDialog` (`app/src/App.tsx`) had pi's "waits on its model
     until the host answers" sentence. It now has a generic body.
   - `refusalLine` strips core's agent-facing "Ask the user before
     overriding", because in the dialog the button is the asking.
     `app/scripts/harness-check.mjs` pins this.
   - The `App.tsx` comments, the `claude_usage.rs` header, README and
     CHANGELOG were corrected. `WORKTREES_USAGE_PROBE=off` switches the GATE
     off, not the meter.
   - New doc notes on `Window::model` (a lane with no model is fail-open by
     design) and on `other_harnesses` (why Claude's default `installed()` is
     trusted).
4. **Second merge of `origin/main`.** #396 landed mid-lane and made the PR
   DIRTY. The ROADMAP conflict kept both sides.

## Decisions

- **The opener check follows pi's shape exactly.** It is not a new
  "brief unread" flag. One question, asked the same way for every harness,
  keeps `cmd_open` harness-agnostic.
- **No shorter gate deadline.** The readers' cache and backoff are shared with
  the app's meter. A gate-side timeout would either feed failures into that
  shared state, or run the reader on a thread the CLI abandons at exit, which
  can orphan a `codex app-server`. The README states the worst case instead.
- **`other_harnesses` still trusts Claude's default `installed()`.** Claude is
  routinely reached through things a PATH lookup cannot see: the installer's
  alias under `~/.claude/local`, profile wrappers, `WORKTREES_CLAUDE_CMD`.

## Dead ends / gotchas

- **CI did not start after the first push, and the cause was a conflict.**
  Main moved (#396) between the merge and the push. The PR went `DIRTY`, and
  `gh run list --commit <short sha>` returned nothing, because `--commit` wants
  the full SHA. `gh run list --branch` found the run once the PR was clean.
- **`nvm use` did not take in the Bash tool.** A later PATH entry kept Node 26
  in front. Prepending `$NVM_BIN` explicitly fixed it.
- **One test passes on the old code by construction.** The "no opener once
  claude has run" test checks the opposite case from the fix. It was proven
  red by forcing `never_launched` to `true`, not against brethash's code.

## Verification

- The three opener tests went red on brethash's code before the fix and green
  after it. The guard tests were each mutated red.
- `make test`: exit 0, `1..415`, 0 `not ok`.
- Lint passed.
- Unit tests: core 614 passed + 1 ignored, cli 40 passed, app `--lib` 150
  passed.
- `tsc` and `cargo check -p app` passed. The one warning is `viewer::claim`
  dead code, which is also on main.
- Every `app/scripts/*-check.mjs` passed from the repo root on Node 22.23.
- `ls --json` was byte-identical to v0.34.1.
- CI run 36939863954 was green on all 9 jobs.

## Follow-ups (in ROADMAP)

- The refusal has never fired live against a real spent window.
- A slug re-used after `rm` counts as launched for Claude and Codex.
- A first launch in a place with an old brief starts on that brief.
- The gate's offline stall.
