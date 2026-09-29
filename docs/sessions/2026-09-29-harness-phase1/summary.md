# Harness phase 1: "two agents" becomes "N agents" — 2026-09-29

- **Date:** 2026-09-29
- **Worktree:** `harness-phase1`
- **Branches:** `harness-phase1-core`, `harness-phase1-app` (stacked), `harness-phase1-close-out`
- **PRs:**
  - [#363](https://github.com/penard-monkey/worktrees/pull/363): refactor(core), one adapter per harness, and `for_pane` reads the sidecar name. Squash-merged as `38a9bb0`.
  - [#364](https://github.com/penard-monkey/worktrees/pull/364): refactor(app), agent sessions per harness and one shared `Harness` type. Squash-merged as `7199678`.
- **Release:** none. The `for_pane` fix sits under `## [Unreleased]` → `### Fixed`.
- **Planning files:** none. The lane worked from `.planning/brief.md` and did not keep `task_plan.md`.
- **Source of truth:** `docs/proposals/pi-harness.md` §2.3 (`shared-shape`), §10 and §11, plus opencode-harness §3.4 (the trait must allow a per-launch env and runtime handle).

## What shipped

**Core (#363)**
- **`crates/worktrees-core/src/harness.rs`:** an `Adapter` trait with `Claude` and `Codex` implementations. `ALL` lists them in registry order, and a test pins that order to `provider::PROVIDERS`. The trait has these methods:
  - `place_flags`: Codex's permission mode.
  - `launch_args`: returns `head`/`tail` shell words, so Codex's `-c` flags still land before `resume --last`.
  - `launch_env`: empty for both harnesses.
  - `resume_arg(cwd)`: now a method. The `Provider.resume_arg` field was removed.
  - `session_present` and `may_resume`: these absorb the `!= "codex" || session_present` gate that both `ops` launch paths had.
  - `activity`.
  - `agents`: the `place_status` rows.
  - `send → Delivery { Typed | Refused | Elsewhere }`.
- **Codex's send-confirm loop moved into core** (`submit_codex`, `SendOutcome`, `may_type`, the `SEND_*` bounds), together with its 7 tests. They came from `crates/worktrees-cli/src/mcp.rs`.
- **Readers take a `Scan { probes, panes }`** instead of a bare pane list, so a harness whose activity is an API call (opencode) has somewhere to carry its handle.
- **`activity::most_active` takes any number of readings.** `harness::place_activities` returns one reading per harness that is present.
- **MCP:** `place_status` and `send` loop over the adapters. Claude returns `Delivery::Elsewhere`, which is only used if no other harness took the message. That keeps the old rule of checking Codex first.
- **`provider::for_pane(session, command)` fix:** a `node` or version-like pane in a `~agent~<id>` session now belongs to that harness, and to none when the id is unknown. Before, it was always Claude. `provider::SIDECAR_MARKER` is new.

**App (#364)**
- **`lib.rs::agent_sessions_for`** builds one entry per adapter. The IPC shape is unchanged, because it was already an id-keyed object. A test pins it against the old two-provider formula, which the test keeps verbatim as its reference.
- **Provider checks go through the registry** (`known_harness`). `open_place`'s resume gate is now `Adapter::session_present`.
- **Dead helpers removed from `tmux.rs`:** `session_is_codex`, `codex_session_name`, `claude_session_name` and the two marker constants. `PaneList::from_rows` is new, added for tests.
- **`app/src/harness.ts`** holds `Harness`, `HARNESSES` and `HARNESS_LABEL`. It replaced every `"claude" | "codex"` union in `App.tsx`, `settings.ts`, `SettingsSheet.tsx`, `planUsage.ts`, `PlanPane.tsx`, `TerminalPane.tsx` and the mock fixture type.
- **"Switch to the other one" is now "switch to any other harness"**, both in the confirm dialog and in the ⋯ menu. The new-worktree segment lists the installed harnesses.
- **`app/scripts/zoom-check.mjs`** now inlines `harness.ts` next to `afterglow.ts`.

## Decisions

- **`provider::Provider` stays the registry, and behaviour lives in `harness.rs`.** A static table cannot express `--session-id <derived>` or "read the rollout, then maybe the pane". The shared proposal also says no rename.
- **Only methods with a caller today went on the trait.** `session_for` stayed off, because its only caller turned out to be Codex's own `send`. `running_model` stayed off, because Claude's reader caches tails in the app and logs a failed read through `applog`, and core cannot log. Moving it would have swallowed that error (AGENTS.md: never swallow errors).
- **The `for_pane` fix keys on the session name, not `pane_start_command`.** Reading `pane_start_command` would have changed the `list-panes -a` format that the bats fake-tmux shim emits. A canonical (pre-sidecar) session keeps defaulting to Claude, which is all it has ever had to go on.
- **`Delivery::Elsewhere` instead of registry order for `send`.** Iterating Claude first would have changed which harness answers when two are live at once. `Elsewhere` means "I have my own bus": it answers only if nothing else in the place took the message.
- **Frontend modules loaded by check scripts stay free of runtime imports.** `planUsage.ts` imports only a type, because `plan-usage-check` loads it as a `data:` URL, and `providerName` was dropped for `HARNESS_LABEL`. `settings.ts` does import `harness.ts` at runtime, so `zoom-check` inlines the real `harness.ts` source instead of stubbing it, following the AGENTS.md rule.

## Dead ends / gotchas

- **Moving code between crates can silently delete a neighbour.** Cutting the `SendOutcome … submit_codex` span out of `mcp.rs` by start and end markers also took `record_send`, which sat between them. The build caught it. Cut by named item, not by span.
- **The Chrome DevTools MCP profile is shared across sessions.** `new_page` failed with "browser is already running". It was not forced, because another session owned that browser. No Playwright is installed in a fresh worktree either, so #364 got no visual pass (ROADMAP).
- **pnpm is not on PATH here, and `nvm use 22.23.2` fails because that version isn't installed.** `npx -y pnpm@10 install --frozen-lockfile` worked, under Homebrew node 26.
- **Bats takes ~10 minutes** at 382 tests. Run it in the background and check the `1..N` plan line and the `not ok` count; do not tail it.

## Verification

- **Fail-first:** both new `for_pane` tests, `provider::tests::a_wrapper_pane_in_a_sidecar_is_that_sidecars_harness_never_claude` and `tmux::tests::agents_in_reads_a_node_pane_by_its_sidecar`, failed against the old body and passed against the new one. The `agent_sessions` equivalence test failed with the "another harness claims the primary" guard removed.
- **Both branches, release binary rebuilt first and checked newer than every crate file:**
  - bats `1..382`, 382 ok, 0 `not ok`, exit 0
  - `make lint`: exit 0
  - core: 517 passed
  - CLI: 26 passed
  - app lib: 162 passed, 1 ignored (pre-existing)
  - `tsc --noEmit`: exit 0
  - `cargo check -p app`: ok
  - `pnpm build`: ok
  - all 22 `app/scripts/*-check.mjs`: ok
- **Byte-identical against the shipped v0.32.1 (`~/.local/bin/worktrees`), with `cmp`:**
  - `worktrees ls --json` across 15 places.
  - MCP `initialize` + `tools/list` + 15× `place_status`, including `create_worktree`'s provider enum.
  - No Codex session was live, so the Codex branches are covered by unit tests only.

## Follow-ups

- **Phase 2 (pi)** carries most of the phase-1 review's follow-ups in its brief (`pi-phase2` worktree).
- **ROADMAP picks up what phase 2 does not cover:**
  - a drift check between `harness.ts` and `PROVIDERS`
  - `running_model` onto the trait once core can log
  - `launch_env` riding in tmux argv (wrong for a secret)
  - a per-harness `data-testid` on the Switch item
  - the missing mock visual pass
