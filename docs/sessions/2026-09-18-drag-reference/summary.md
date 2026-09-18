# Session: drag a worktree into a session to talk about it

- **Date:** 2026-09-18
- **Worktree:** `drag-drop-worktree-reference` (idle base
  `drag-drop-worktree-reference-next`)
- **Branch:** `drag-drop-worktree-reference-drag` off a freshly fetched
  `origin/main`
- **PR:** [#221](https://github.com/penard-monkey/worktrees/pull/221) →
  `10eacdb` (three commits + a merge of `origin/main`, squashed)
- **Planning files:** `planning.tar.gz` beside this summary
- **Scratch:** `~/.cache/worktrees/worktrees/drag-drop-worktree-reference/` —
  the final bats log, and `shadow.py`, the throwaway probe that caught the uri
  shadowing bug in the sibling session
- **Sibling:** [#215](https://github.com/penard-monkey/worktrees/pull/215)
  shipped the typed `@` autocomplete earlier the same day
  ([summary](../2026-09-18-mcp-resources/summary.md)); this is the drag half it
  deliberately left unbuilt.

## Context

The original ask had two halves: drag a worktree into a session, and an
autocomplete while typing. #215 shipped the second. This is the first.

## What shipped

Dragging a place row onto the terminal types `@worktrees:place://<slug>` into
the Claude session running there — the same token the `@` menu completes, so it
expands into that place's status.

- `crates/worktrees-core/src/mention.rs` — the uri rule, MOVED out of
  `worktrees-cli` so both producers share one implementation, plus
  `server_name_for` and `uri_for`.
- `crates/worktrees-core/src/tmux.rs` — `ai_pane`, `is_ai_command`, `PaneId`,
  `paste_to_ai`, `paste_commands`.
- `app/src-tauri/src/lib.rs` — the `drop_reference` command.
- `app/src/{TerminalPane,App}.tsx`, `App.css`, `navdrag.ts`, `mock/install.ts`.
- `app/scripts/dropzone-check.mjs`, and `make test-frontend`.

## Decisions

**The payload is the resource uri, not a path — a user decision that reversed
the archived design.** ROADMAP recorded "insert a quoted absolute path". David
chose the uri once the alternatives were costed: a path mention expands to a
~27-entry directory LISTING, the uri expands to branch/dirty/tmux/agent state.
The objections that drove the earlier recommendation had also weakened —
staleness is handled by #215's watcher, escaping by `safe_uri_part`, and the
note-injection surface turned out not to exist.

**The frontend never builds the token.** The client resolves a mention by exact
string equality against its cached resource list, so a second implementation off
by one character would reference the wrong place SILENTLY. The uri rule moved to
core and the app asks the backend for the finished string — deliberately the
opposite of `dnd.ts::predictTier`, which mirrors a core rule and needs a drift
check *because* it mirrors.

**`make test-frontend`.** The twelve existing `app/scripts/*-check.mjs` guards
ran nowhere — not in CI, not in any target. Adding a thirteenth without fixing
that would have been adding dead weight.

## Dead ends / gotchas

**A drop could land in a DIFFERENT worktree's Claude.** tmux targets
prefix-match. Probed on a private socket: with only `api-fix` alive,
`paste-buffer -t api:0.0` returns 0 and the token lands in `api-fix`.
`session_exists` already documents this trap for `has-session`; this walked into
it anyway. Anchor with `=`, or address a pane id.

**"Pane 0 is the AI" is false three ways.** `set -g base-index 1` in a user's
tmux.conf → `can't find window: 0`, so EVERY drop fails (nothing in the repo
pins base-index). Closing a pane renumbers the survivors, so index 0 becomes the
shell. An adopted session was never laid out by `new_session` at all. The pane
is found by what it RUNS, using the same rule adoption uses.

**Two mutations passed that should not have, both the same shape.** The value
under test was chosen in the CALLER while the test called the argv builder with
a literal — so reverting the pane target to `{session}:0.0` passed the entire
suite. First occurrence was fixed with an extracted `pane_zero()`; the second,
after the rewrite, with a `PaneId` newtype whose constructor is private. The
wrong target now fails to COMPILE. A test asserting "the target starts with `%`"
is not enough when the target is chosen elsewhere.

**`--strict-mcp-config` is CONDITIONAL** on `!inherit_global_mcp`
(`profile.rs`), not unconditional as the first comment claimed — and
`worktrees_mcp: false` registers no stanza at all. `server_name_for` returns a
`Result` and names the case rather than guessing `worktrees` and producing a
token that expands to nothing under a success notice.

**A safety claim that was a guess.** The first comment said `paste-buffer -p`
is safe during a permission prompt because a prompt does not request mode 2004.
It is the same program with the mode already set, so that is almost certainly
wrong. The argument that holds is paste-vs-keystrokes; the behaviour during a
prompt is flagged as the least-verified thing in the manual checks.

**GitHub will not run CI on a PR it cannot merge.** Zero runs registered, which
first looked like a broken `ci.yml` edit (the YAML was fine). The cause was
`mergeable: CONFLICTING` after main moved. "No checks reported" is a symptom of
the conflict, not a CI fault.

**A merge surfaced a duplication no test could see.** #218 landed `mcpsetup`,
which already judges "is this stanza ours" and owns the `worktrees` constant;
`mention` had grown its own copy hours apart. `server_name_in` now delegates.

**A `python3` replace anchored on non-unique text deleted 707 lines** of
`lib.rs` — `s.index()` found an earlier `std::env::var("HOME")`. Caught by the
compiler, restored from HEAD. Anchor edits on text that is unique.

**The check script failed on its own bug first** — the body slicer stopped at
`} & TermFindProps) {`, cutting the JSX off before the thing it checked for.
And the ShellPane assertion passed vacuously when the component was renamed,
because `!/drop=/.test("")` is true.

## Verification

- `make test` exit 0, `1..338`, 338 ok / 0 not ok; lint; core 330; cli 13;
  app 58; `cargo check -p app`; tsc; `make test-mcp`; `make test-frontend`
  13/13. CI 9/9 on both OSes.
- Mutations shown red: hardcoded `data-drop` in the shared component; the
  mention branch after the tier early-return; the cross-project guard removed;
  `-p` dropped; the AI word hardcoded; `ShellPane` renamed. Plus the two that
  did NOT fail at first, described above.
- The two tmux behaviours were PROBED on a private socket (`-L`), never the
  user's server.
- Real claude 2.1.276 re-verified after the core move — the first attempt used a
  stale `/tmp/wt-test-bin`, the "a stale binary can make it PASS" trap; redone
  against a binary proven newer than `mention.rs`.
- Four fable reviews across the two sessions.

## Follow-ups

**The drag has never been performed.** Every part is tested except the gesture
itself: the mock records the invoke but has no tmux, and driving the real app is
off-limits. `docs/ai-profiles-manual-checks.md` has the steps, including the two
that matter — dropping during a permission prompt, and dropping into a session
whose Claude has exited.

In ROADMAP: the debug logging still comes out at v0.26.0; the uri edges; the
`## [Unreleased]` guard.
