---
title: Clone from URL… — 2026-10-03
---

# Clone from URL… — 2026-10-03

- **Date:** 2026-10-03
- **Worktree:** `.worktrees/clone-project` (lane started by (main) from `.planning/brief.md`)
- **Branches:** `clone-project` (#428), `clone-project-review-fixes` (#432), `clone-project-close-out` (this archive)
- **PRs:** [#428](https://github.com/penard-monkey/worktrees/pull/428), squash-merged as `f953ce7` after a fable review said MERGE. [#432](https://github.com/penard-monkey/worktrees/pull/432) holds the review nice-to-haves; it is open and CI is green.
- **Release:** none. Both changes are under `## [Unreleased]`.
- **Planning files:** `planning.tar.gz` beside this summary.

## What shipped

- **`crates/worktrees-core/src/clone.rs`** is the one implementation.
  - `parse_source` accepts any git URL. `owner/repo` is short for GitHub https. `NAME_CASES` is the shared table of URLs and the names they produce.
  - `plan_target` refuses an existing target.
  - `clone_env` sets `GIT_TERMINAL_PROMPT=0` and runs the user's own ssh command with `-o BatchMode=yes -o ConnectTimeout=20`. #432 adds `GIT_ALLOW_PROTOCOL`.
  - `clone_repo` creates missing parents and claims the target with `create_dir`. It runs `git clone --progress -- <url> <target>` in its own process group and streams throttled `CloneProgress`. On cancel or any failure it removes the claimed folder.
  - `classify` and `failure_message` turn git's output into one of auth, not_found, host_key, network or other, with a headline plus git's last `fatal:` line.
  - `cmd_clone` serves the CLI.
- **CLI:** `worktrees clone <url> [--into <dir>] [--name <folder>]` runs ahead of the git guard and registers the result. Ctrl-C becomes a cancel through a SIGINT handler that sets a flag. Tests: `test/clone.bats`.
- **App backend** (`app/src-tauri/src/lib.rs`):
  - `clone_project(id, url, parent, name, on_progress: Channel)` returns `{workspace, root, dir, has_submodules}`. An error is core's `CloneError {kind, message}`.
  - `clone_cancel(id)` cancels a running clone.
  - `CloneJobs` is the cancel registry; it is a struct so the tests own their map.
  - The result goes through `add_project`, the single door.
  - #432 adds: exit cancels in-flight clones, and log lines carry the redacted URL.
- **Frontend:**
  - `app/src/clone.ts` mirrors core's parser for the live preview.
  - `CloneDialog` (App.tsx, module scope) has the URL field, Clone into with Change…, an optional folder name, a path preview, a progress bar, and Cancel.
  - The add menu has a new "Clone from URL…" item.
  - `Settings.projects_parent` is one remembered folder, shared with New project….
  - The mock implements `clone_project` and `clone_cancel`. The URL picks the failure (`private`, `missing`, `hostkey`, `offline`), and `?clonedelay=<ms>` slows the clone down.
  - `app/scripts/clone-check.mjs` checks the mirror and the mock against core.

## Decisions

- **Any git URL, with only the shorthand tied to GitHub.** Refused:
  - helper URLs (`helper::address`, `ext::` included);
  - a leading `-`. git also gets `--` before the URL;
  - local paths, which belong to Add existing….
- **An existing target is refused, never suffixed.** A silent `repo-2` would hide that you already have the repo. An optional Folder name field is the way out of a collision.
- **"Only the folder this clone created" is guaranteed by construction.** `create_dir` fails if the folder exists, so a successful claim proves ownership; `create_dir_all` would not.
- **git gets its own process group.** Killing git alone leaves ssh and git-remote-https holding the stderr pipe, and the cancel hangs. Side effect: the terminal's Ctrl-C no longer reaches git, which is why the CLI sets a cancel flag from a SIGINT handler.
- **Submodules are not recursed.** Each one is a URL the repo supplies, and one more place a prompt can hide. The result says when they exist.
- **One remembered folder for both dialogs.** There was none before; New project's default was derived from the existing projects. Whichever dialog last succeeds writes `projects_parent`, and both read it.
- **A CLI command was added.** It was cheap, and it gives bats and the real-network probes a driver with no GUI.
- **`GIT_ALLOW_PROTOCOL` rather than `GIT_PROTOCOL_FROM_USER=0`** (#432). The latter also blocks `file://`, whose default policy is `user`.

## Dead ends / gotchas

- **Keeping the first 64 KB of stderr kept the wrong end.** git's `fatal:` line comes last, after the `\r` progress segments. A long clone that failed late was classified `Other`, with a progress line quoted as the evidence. The review caught it, and a fake ssh flooding about 100 KB proved it red. The fix keeps only lines that are not progress, newest last.
- **`fd::0` as a "not allowed" witness hangs git** when the allowlist is missing: git waits on stdin. A witness must fail, not hang, so it was replaced with `nosuchhelper::x`, which fails at once.
- **The first scheme-case mutation silently did not apply.** A perl one-liner with `&s[i..]` matched nothing, and the run read like "the test passes anyway". Re-done in python with an `assert` that the pattern exists, it went red. A mutation that prints nothing proves nothing.
- **git's scheme matching is case-sensitive.** `FILE://` gives "git: 'remote-FILE' is not a git command".
- **This Mac's osxkeychain has no github.com https entry.** David uses ssh, so a private https clone "fails" identically to plain `git`. With gh's helper injected by env (`GIT_CONFIG_COUNT`), the same clone succeeds, which proves helpers work under the no-prompt env.
- **`nvm use` does not win the PATH here.** Homebrew's node comes first in the profile, so gates were run with `$HOME/.nvm/versions/node/v22.23.3/bin` prepended by hand.
- **A CLI probe registers into the real `~/.config/worktrees/projects.json`.** Probes used `XDG_CONFIG_HOME=<scratch>`; the real registry was checked to contain nothing from them.
- **A late-target race test caught data loss.** With `create_dir_all` instead of `create_dir`, the failure cleanup deleted another party's file.

## Verification

- Every new test was shown to FAIL first, by mutation or against the pre-fix code; the evidence is in both PR bodies.
- Core tests: 10 in #428, 15 in #432.
- `test/clone.bats`: 6 tests, 7 with #432.
- App `--lib` tests: 2, 3 with #432.
- `clone-check.mjs`: 37 assertions, 38 with #432.
- Real-network probes ran through the release binary:
  - public https and the shorthand cloned;
  - private ssh cloned;
  - git:// cloned against a local `git daemon`, file:// cloned;
  - a missing repo gave auth, DNS failure gave network, an empty known_hosts gave host_key;
  - Ctrl-C against a hanging ssh exited 130 with the folder and the grandchild gone.
- Mock harness under Playwright, in Chromium and WebKit:
  - the URL field takes focus without scrolling;
  - `elementFromPoint` hits every control;
  - the preview updates; each of the four failure kinds shows;
  - progress, Cancel, and Escape and the scrim being inert while running;
  - success selects `(main)`; the remembered folder is shared; an existing target is refused;
  - all six themes.
- Full gates ran for both PRs:
  - #432: bats `1..463` with 0 not ok;
  - cargo: core 682, cli 70, app 154;
  - lint, tsc, `cargo check`, test-frontend 31/31;
  - `ls --json` byte-identical;
  - CI 9/9 green on both.

## Follow-ups

- Hand tests in the real app are owed (ROADMAP): Cancel during a large clone, and a private https clone.
- #432 awaits review and merge.
