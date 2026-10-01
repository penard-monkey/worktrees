---
title: "Adding an agent harness — checklist"
---

# Adding an agent harness — checklist

Claude, Codex and pi are the harnesses today; opencode is proposed
([proposal](proposals/opencode-harness.html)). A harness touches the core, the
CLI, the MCP server, the app's backend poll and the app's frontend, and **a
harness that works in one of those can be invisible in another**. pi shipped in
v0.33.0 with a correct core reading — `place_status` said `busy` — and no nav
dot at all, because the app builds its dots in its own poll and pi was never
added to it. Three reviews passed it, because every check exercised the core
and nothing looked at what the app draws.

So this list is organised by **surface**, and every item says how to see it
working — in the running app against a live session where the surface is the
app. A box is ticked when that has been seen, not when the code exists.

Build on the shared shape in [pi-harness §2.3](proposals/pi-harness.html#shared-shape);
read that first.

## 1. Registry and adapter (core)

- [ ] **Registry row** in `crates/worktrees-core/src/provider.rs` `PROVIDERS`:
      `id`, `match_word`, `sidecar_suffix` (`~agent~<id>`), `canonical_default:
      false`, `name_arg`, `model_arg`. Existing session names stay
      byte-identical.
- [ ] **Adapter** in `harness.rs` (`trait Adapter`): `installed`,
      `place_flags`, `launch_args`, `launch_env` (secrets go in env, never
      argv), `resume_arg` **and** `resume_display` (the Settings default must
      not derive a session id from an empty cwd), `prepare` (refusals: missing
      binary, runtime floor, unreachable model host), `running_model`,
      `session_present` / `may_resume`, `activity`, `agents`, `send`.
- [ ] **Pane identity.** What does tmux report as `pane_current_command`?
      A `node` or version-string pane was once read as Claude; `for_pane` keys
      on the sidecar suffix first. Test a pane of this harness in its own
      sidecar AND a stray `~agent~<unknown>` session.
- [ ] **Resume is keyed by the place directory, never the repo** (opencode's
      `-c` resumed a sibling worktree's session). Survive a hand restart: the
      session the place actually has, not the id we launched with (pi #376).
- [ ] **A re-used slug starts fresh** after `rm` + `new` (declared state
      outlives `rm`).
- [ ] **Model:** `model_arg`, the catalog source (`ModelOption` with
      `ready`/`reason`), never on resume, validated as data.

## 2. Activity — every consumer, not just core

The same derivation must reach **all four** readers. Keep them on one reader
(AGENTS.md: the nav dots and MCP `place_status` read one truth).

- [ ] **MCP `place_status`** reports `busy` / `waiting` / idle + `last_done`
      and the model — check with a live session.
- [ ] **MCP `wait`** returns on the harness's turn end, and survives the
      harness's own MCP request timeout (pi cancels at 60s — progress
      notifications) and a cancel.
- [ ] **The app's nav dot poll** (`app/src-tauri/src/lib.rs`: an arm in
      `harness_feed` returning a `LaneTick` — `codex_tick` / `pi_tick` are the
      precedents — and one in `watch_lane` so the snapshot hands the tick its
      live sessions; `merge_activity`, `completion_edges` and `new_dones` then
      emit `sessions:busy` / `sessions:done`). **This is the one pi missed.**
      `every_harness_feeds_the_dot_poll` fails until both arms exist. Check
      in the RUNNING APP: a busy lane shows the busy dot, a lane waiting on you
      shows amber, a finished turn leaves the afterglow ring.
- [ ] **Re-list triggers.** Anything the snapshot shows from agent state (the
      model label, the session) needs its own trigger — neither a finished
      turn nor a model switch moves tmux (`codex_models_moved` is the
      precedent).
- [ ] "Which write set this field?" for every file the harness owns: mtime is
      never activity; a probe/transcript can be stale by design.

## 3. Talking to it

- [ ] **MCP setup**: install/uninstall by shelling out to the harness's own
      `mcp add` (never write its config), detection by READING its config file
      (`… mcp list` launches every server — the `claude mcp list` trap), the
      right exposure mode, `WORKTREES_MCP_PROVIDER` set so the server knows who
      it serves. Check `report`'s `from` is the place.
- [ ] **`send`**: typed inline or folded? Confirmation from the harness's own
      record when idle, from the screen when busy; modals (approval, trust)
      mean refuse — Enter selects the default. Scrolled-back (copy-mode) panes
      must leave the mode first. Capture real screens; fixtures miss streaming
      and width.
- [ ] **Plan tab paste** and **drop into a session** reach it, with the same
      modal guard.
- [ ] Claude↔Claude uses Claude's own messaging; everything else goes over the
      MCP message log — say which applies.
- [ ] **Agent guidance** (`guidance::delivery`): how the harness takes the
      `worktrees` skill and the rule per launch — a plugin, a skill flag, a
      prompt-append flag, or a config key that must not clobber the user's own
      value. `delivery_knows_every_registered_harness` fails until it has
      one. Check whether the harness shows MCP server `instructions` at all
      (pi with `direct` exposure does not). SEE the skill loaded in a live
      launch, and check what any detection step STARTS: `codex debug
      prompt-input` starts every MCP server.

## 4. Security (ADR 0001)

- [ ] What can a cloned repo make this harness EXECUTE at launch (project
      config, plugins, MCP servers, skills)? Worktrees must never be the
      channel. `USER_ONLY_KEYS` for any new config key.
- [ ] Trust defaults and the user-scoped allowance; a repo that could replace
      the worktrees MCP server is never approved (protect-ours).
- [ ] Never write the harness's credentials or config; never read its auth
      file.

## 5. The app (frontend)

- [ ] `app/src/harness.ts`: `HARNESSES`, `HARNESS_LABEL`, `NEEDS_MODEL`.
- [ ] `App.tsx`: `HARNESS_MARK` (the row icon — a missing entry once rendered
      the OpenAI mark), `USAGE_READ` (usage dispatch; `None` is legitimate).
- [ ] New-worktree dialog segment + model picker, "Switch agent…" sheet,
      Settings → Agents default, a Settings category for the harness (binary,
      version, runtime, readiness, MCP setup), the after-update offer
      (`offers.ts`, `offers-check.mjs`).
- [ ] The terminal: alternate screen or not, mouse or not — does the wheel
      scroll it (`term_wheel`), does scrollback exist?
- [ ] **Mock harness** (`app/src/mock/install.ts`) tracks every new command and
      can show this harness's dot, mark and states.
- [ ] **WKWebView pass** in the real app (`app/scripts/sandbox.sh --app`) —
      the mock runs in Chromium.

## 6. Usage and availability

- [ ] What limit, if any, can be shown honestly (see
      [codex-usage](proposals/codex-usage.html)); a placeholder, never a
      fabricated zero.
- [ ] Unreachable model host / missing runtime: refuse with "launch anyway".

## 7. Proof and ship

- [ ] Unit tests + a drift check guarding every place that enumerates
      harnesses (so the next one fails loudly instead of silently).
- [ ] `docs/<harness>-manual-checks.md`, run live on the version you pin, and
      re-run on every upgrade of the harness.
- [ ] `worktrees ls --json` byte-identical against the shipped binary for
      existing places.
- [ ] **End-to-end in the running app, one lane:** create it from the dialog,
      watch the dot go busy → afterglow, `send` to it from another lane, switch
      agent, resume after an app relaunch, remove it.
- [ ] CHANGELOG, and a line in AGENTS.md for anything that surprised you.
