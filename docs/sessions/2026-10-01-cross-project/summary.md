---
title: "Session — cross-project reach (P0–P4)"
---

# Session — cross-project reach: agents that see, message and act on other projects

- **Date:** 2026-10-01
- **Worktree:** `.worktrees/cross-project`
- **Branches:** `cross-project-proposal`, `cross-project-p0-registry`,
  `cross-project-p1a-resolver`, `cross-project-p1b-ui`,
  `cross-project-p2-messages`, `cross-project-p3-mutations`,
  `cross-project-p4-mentions`, `cross-project-close-out` (this archive)
- **PRs:** proposal [#395](https://github.com/penard-monkey/worktrees/pull/395);
  P0 [#397](https://github.com/penard-monkey/worktrees/pull/397);
  P1a [#402](https://github.com/penard-monkey/worktrees/pull/402);
  P1b [#404](https://github.com/penard-monkey/worktrees/pull/404);
  P2 [#405](https://github.com/penard-monkey/worktrees/pull/405);
  P3 [#406](https://github.com/penard-monkey/worktrees/pull/406);
  P4 [#407](https://github.com/penard-monkey/worktrees/pull/407). All
  squash-merged, each after a review by the orchestrator in `(main)` and its
  follow-up commit.
- **Release:** none yet. Everything is under `[Unreleased]`.
- **Planning files:** `task_plan.md`, `findings.md`, `progress.md` and the
  lane's brief, in `planning.tar.gz`.

## What shipped

The lane started as research: "let an agent reach places in other projects".
The proposal ([`docs/proposals/cross-project.md`](../../proposals/cross-project.html))
went through one review, recorded the user's four decisions, and was then built
phase by phase.

- **P0, the registry (#397).** The project list moved from the app's config dir
  into core: `crates/worktrees-core/src/registry.rs`, at
  `~/.config/worktrees/projects.json`.
  - Each entry has a NAME, seeded from the repo's OWN prefix sources and never
    from `WORKTREES_PREFIX` or the user prefix. Names are unique by
    construction (`-2`, `-3`).
  - Every write is a read-modify-write under an `flock` + mutex, then
    temp + rename.
  - The app union-merges its old `projects.json` once per process and never
    writes it.
  - New `worktrees projects [ls|add|rm|rename|private]`. `doctor` gains
    `nested-project`, and `cross_project` joins `USER_ONLY_KEYS`.
- **P1a, read-only reach (#402).** In `crates/worktrees-core/src/reach.rs`:
  - The level is `cross_project` (`off`/`read`/`full`, off by default, unknown
    means off), narrowed by a profile's `--cross-project`.
  - The caller's identity is the registry entry for its canonical root.
  - Addresses are `<project>:<slug>`.

  In the MCP server (`crates/worktrees-cli/src/mcp.rs`): `list_projects`, and
  foreign `place_status` / `wait idle`. A registered repo now counts as
  managed. `doctor` gains `prefix-collision`.
- **P1b, the app (#404).**
  - Settings → Agent guidance → Other projects (`app/src/CrossProjectPanel.tsx`),
    plus the `cross-project` after-update offer (`app/src/offers.ts`).
  - The skill's "Across projects" section.
  - Drag and drop into Claude, Codex and pi. Core's `reach::plan_drop` decides
    between the `@`-token and the plain address `place <project>:<slug>`;
    `dnd.ts::mentionPlan` mirrors it, guarded by `app/scripts/drop-check.mjs`.
  - A drop into a waiting Codex/pi pane is refused.
  - `nested-project` and `prefix-collision` get their own To do row.
- **P2, messages (#405).** `report` to `<project>:<slug>` files into the
  RECIPIENT repo's log, with `to` stored bare and `from` server-derived as
  `<name>:<slug>`. `messages::post_into` checks `reply_to` against the sender's
  log. The hub-copy refusal is applied to the target. A refused write keeps
  only the OS's reason.
- **P3, mutations (#406).** `Server::mutable_target` / `foreign_gate` require
  reach `full` AND the server's `--mutations`. That covers `send`,
  `close_session` and the setters, plus a new optional `project` on
  `create_worktree`. `remove_worktree` never acts across projects.
- **P4, `@`-mentions (#407).** Foreign places appear in `resources/list`
  (`mention::foreign_uri`, `place://<project>/<slug>`), `resources/read`
  answers for them, there is a template, and the watcher covers reachable
  projects. P3 follow-ups landed in the same PR:
  - brief provenance header;
  - consent re-read on EVERY foreign call;
  - hermetic tests;
  - connect cost cut from 11 to 5 git spawns.
- **Fixed on the way (#406):** `worktrees mcp` wrote git's success output
  ("HEAD is now at …") to stdout, which is the JSON-RPC stream, on every
  `create_worktree`. Fixed with `git::set_stdout_is_protocol()`, and red on
  v0.34.1.
- `docs/cross-project-manual-checks.md` lists everything owed in the real app.

## Decisions

- **Reach is OFF by default** and offered after the update (the user's call).
  A cross-project tool that a model could reach without the user's say-so is a
  permission granted by nobody.
- **The project handle is the registry name, never the session prefix.** The
  first draft used the prefix, but review found that `.worktree-prefix` and
  `[project] prefix` are REPO-supplied, so a cloned repo could claim another
  project's name and sign messages as it.
- **Core owns the registry; the app is a client.** The app's file was only
  readable by the app and its path depends on the bundle identifier. It is
  merged in by union, which never resurrects a removed project.
- **Messages are routed to the recipient's log, not a shared user-scope log.**
  The log lives in the git common dir so that a sandboxed agent with a foreign
  `$HOME` still reaches it; a shared log would lose that.
- **Levels follow the proposal's §5.1 table exactly.** `read` covers reading
  and `report`; `full` plus `--mutations` covers acting; `remove_worktree` is
  never allowed (Q6). No `confirm` was added, because the proposal requires
  none for these tools.
- **No paths below `full`.** Values are stripped by VALUE (`strip_paths`), so
  a path field added later is covered without anyone remembering it.
- **Consent is re-read on every foreign call, both sides**
  (`foreign_project` → `consent_now`). Marking a project private is consent
  withdrawn, and a running session must stop at its next call, not its next
  restart.
- **A foreign drag still types the address, not the token.** The app cannot
  see which server version a session holds, and a token typed into an older
  server resolves to nothing, silently (proposal §6.2).
- **Guidance VERSION was bumped once (3), not per phase.** The whole agent
  guidance feature is unreleased, so later skill edits ride the same version.

## Dead ends / gotchas

- **`drop_reference` looked up the RECEIVING session's server in the DRAGGED
  place's project.** It was right only while the app refused cross-project
  drops. It was found by reading the code, not by a test, and is now pinned by
  `a_drop_token_names_the_receiving_sessions_server`, which is red against the
  old lookup.
- **A Python slice with reversed bounds inserted code at the top of `lib.rs`.**
  `s[s.index(a):s.index(b)]`, where `b` came BEFORE `a`, is an empty string,
  and replacing `""` prepends. `cargo check` caught the duplicate fn. When
  slicing source to replace, assert the slice is non-empty.
- **A backup to the wrong path nearly left a red-first mutation in place.** Two
  `cp` fallbacks, one of which succeeded silently, and then the restore
  read the other. Use ONE backup path and grep for the mutation marker after
  the restore — every later red-first check here did that.
- **`have_git` cannot cache a NO.** The app may ask before `fixup_gui_path()`
  has run, and a cached false would outlive the PATH fix. So it caches yes
  only.
- **Tests on macOS temp dirs need canonical paths.** `/var` → `/private/var`
  made registry tests fail once keys were canonicalised.
- **The mock harness's nav starts collapsed**, and at 820 px tall the last row
  sits under the status bar. Both read as "drag does nothing". Open the nav
  first and use a 1000 px viewport.
- **Headless xterm throws `…dimensions` on a place switch.** It happens with
  no drag at all; it is reported, not counted.
- **Another lane's bare `tmux kill-server` killed the user's tmux server**
  mid-gates (exit 137), which led to #401's `tmux -L` rule. Re-run every gate
  after an interruption; do not trust a half-finished log.
- **The leaks found in review were all in ERROR and REFUSAL text:** a discover
  error carrying the root, an ambiguity listing every root, an io error
  leading with the recipient's git dir, and a hub-copy manifest's
  `local_root`. Check refusals for paths as hard as results.
- **Snapshot vs fresh.** P3 re-read `private` only on writes, and `report`
  (also a write) went through the read path and was missed. One resolution
  for every foreign path (`foreign_project`) is what made the rule hold.

## Verification

- Every phase passed, after a release build: `make test` (up to `1..447`),
  `make lint`, core/CLI/app unit tests, `tsc`, `make test-frontend`, and
  `ls --json` byte-identical to v0.34.1. CI was green on all 9 jobs for every
  PR.
- Red first throughout. Mutations restored each time with a grep check:
  - lock removed;
  - reply_to on the old log;
  - self-qualification removed;
  - paths not stripped;
  - root check off;
  - provenance off;
  - `foreign_index` empty;
  - snapshot consent.

  Bats files were also run against the shipped v0.34.1 binary.
- Two-repo bats over real stdio with a throwaway HOME (`test/crossproject.bats`),
  and the fake tmux for create/close/send.
- The mock harness was driven with Playwright in Chromium AND headless WebKit,
  hit-tested with `elementFromPoint`: the offer, Settings, and drops into
  Claude/Codex/pi.
- Connect cost was measured with a counting git shim.
- **Not run:** the real app, a live Codex (no tokens), and a live
  Claude↔pi exchange.

## Follow-ups

In `ROADMAP.md`:
- the real-app and token checks in `docs/cross-project-manual-checks.md`,
  including Q8 (whether Codex's sandbox binds the MCP server) and the Codex/pi
  composer drop that keeps the waiting refusal in place;
- the project-header collision badge and project-sheet rename/private
  controls;
- a per-sender share of a project's message log;
- switching the foreign drag to the `@`-token once the app can read a session's
  server version, plus the `slug` schema descriptions;
- the P0 first-launch merge of the app's old `projects.json`, never seen in a
  built app.
