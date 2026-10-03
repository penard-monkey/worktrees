---
title: "Session — the CLI stops opening a spare shell pane"
---

# Session — the CLI stops opening a spare shell pane

- **Date:** 2026-10-02
- **Worktree:** `.worktrees/cli-no-spare` (a lane briefed by `(main)`)
- **Branches:** `cli-no-spare`, `cli-no-spare-followups`
- **PRs:** [#421](https://github.com/penard-monkey/worktrees/pull/421), squashed as `95f6227` after a fable review said MERGE. [#424](https://github.com/penard-monkey/worktrees/pull/424), the review follow-ups, open at the time of writing.
- **Release tag:** none yet; the change is under `[Unreleased]`.
- **Planning files:** `planning.tar.gz` holds the lane's brief (`.planning/brief.md`). There were no task_plan, findings or progress files.

## What shipped

- **Single pane is the CLI default** (#421, `crates/worktrees-core/src/ops.rs`). `cmd_new` and `cmd_open` start with `spare_shell = false`, and the new `--spare` flag opts in. `--no-spare` is still accepted as the old opt-out, now the same as the default. If both flags are given, the last one wins.
- **The install command.** It only ever ran in pane 1 (`launch`'s contract). The default now prints it as `then: pnpm install   (not run — --spare runs it in a second pane)`, so someone who used to get auto-install is told how to get it back. `--no-install` suppresses both the run and the hint. `--no-tmux` was already printing the hint.
- **MCP `create_worktree`** (`crates/worktrees-cli/src/mcp.rs`). `spare: true` had pushed nothing and relied on the old default; it now pushes `--spare`. An absent or `false` value pushes `--no-spare` explicitly. The cross-project path reuses the same args. The schema is unchanged; in #424 only the description's wording changed.
- **The app** (`app/src-tauri/src/lib.rs`) still passes `--no-spare`, on purpose, so its layout never depends on the CLI default again. Only a comment changed.
- **Skill** (`crates/worktrees-core/src/guidance/SKILL.md`). The lane line (#421) and the "create a place" line (#424) both say `--no-attach`, because attaching takes over the calling agent's terminal.
- **`open --spare` on a live session** (#424) now says the flag applies only when a session is created. It does not split a running agent's pane.
- **`test/real-tmux.bats` isolation** (#424). See the dead ends below.
- README, CLI usage (`crates/worktrees-cli/src/main.rs`) and CHANGELOG.

## Decisions

- **Keep `--no-spare` as a no-op instead of an error.** The app, scripts, the pi manual checklist and older briefs pass it, and failing them would turn a layout change into breakage.
- **Hint, don't run, the install by default.** Running it needs a pane, and the pane is what was removed. A hint that names `--spare` is the honest middle ground.
- **No `guidance::VERSION` bump.** An edited skill goes `stale` on its own, because staleness is keyed on the hash of the shipped text (`read_edit_in`). That stale flag is what makes the "default changed" offer appear. `VERSION` re-shows the "agents now learn to work in places" offer to EVERY user and is documented as not moving for wording.
- **Spell out both MCP directions.** Pushing nothing for `false` would also have worked. Writing both down is what would have prevented the silent `spare: true` break.
- **No split on reuse.** Splitting a live agent's pane is not what a launch flag is for; saying so is enough.

## Dead ends / gotchas

- **`spare: true` would have broken silently.** Fail-first: with only the default flipped, `test/mcp.bats:182` (`tmux_pane1_cmd repo-agent-b` non-empty) went red. It went green after the `mcp.rs` change.
- **`test/real-tmux.bats` asserted the old default (2 panes) and is outside `make test`**, so it would have gone stale unnoticed. Its teardown was also a bare `tmux kill-server`, isolated only by `TMUX_TMPDIR`. That is the hazard in AGENTS.md: it was only safe because `common_setup` happened to `unset TMUX`. It now unsets `TMUX` itself, replaces the fake shim with a wrapper that pins every call to a per-test `-S` socket, and kills only that socket. worktrees shells out to plain `tmux` from PATH, so the binary under test lands on the same server as the assertions, and the test asserts this.
- **`-L` inside the bats harness fails with "File name too long".** `-L` names a socket inside `TMUX_TMPDIR`, which the harness points at the long `$BATS_TEST_TMPDIR`, past macOS's 104-byte limit. The fix is a short `mktemp -d /tmp/wtrt.XXXXXX` directory and `-S`.
- **`nvm use` printed "Now using node v22.23.3" while `node -v` still said v26** in this shell; something earlier on PATH wins. Prepending `$(dirname "$(nvm which 22.23)")` fixed it. The first frontend gate run was on the wrong Node and was redone.
- A user tmux session was renamed (`…` → `…~agent~codex`) between the before and after snapshots of the first E2E run. That was another project's lane relaunching, not this work. The follow-up proof compared names AND creation times.

## Verification

- #421 tip `c38bcb5` (after a rebase over #422; CHANGELOG conflict resolved as one `### Changed` and one `### Fixed`): `make test` 1..454 with 0 not ok, lint clean, core 667 passed, cli 70 passed, CI 9/9.
- Before the rebase, on the same change: app --lib 151 passed, tsc, `test-frontend`, and `ls --json` byte-identical to the installed v0.36.0.
- E2E with the release binary in a scratch repo, on its own `-S` server: `new` gives 1 pane plus the hint; `new --spare` gives 2 panes with `pnpm install` running in pane 1; `new --no-spare` gives 1; `open` gives 1; `open --spare` gives 2.
- #424 tip `7a7668b`: `make test` 1..456 with 0 not ok, lint, core 667, cli 70, app 151, tsc and frontend ok, `ls --json` identical, CI 9/9. The new reuse-message test was shown red against the pre-change binary.
- `make test-real-tmux` run INSIDE a lane, with `$TMUX` naming the real server: 4/4 passed. The real server's `list-sessions` (13 sessions, names and creation times, read-only) was identical before and after, and no `/tmp/wtrt.*` directories were left.

## Follow-ups

- None owed by this stream once #424 merges. Deliberately dropped: splitting on reuse; removing `--no-spare` from `docs/pi-manual-checks.md` (still valid, and that file is pinned to pi versions).
