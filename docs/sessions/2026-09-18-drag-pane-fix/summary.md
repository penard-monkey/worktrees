# Session: the drag never found Claude

- **Date:** 2026-09-18
- **Worktree:** `drag-drop-worktree-reference` (idle base
  `drag-drop-worktree-reference-next`)
- **Branch:** `drag-drop-worktree-reference-aipane` off a freshly fetched
  `origin/main`
- **PR:** [#224](https://github.com/penard-monkey/worktrees/pull/224) →
  `cb2318b`, shipped in **v0.25.1**
- **Planning files:** none, and no tarball — this was one reported bug worked
  straight through, not a work stream
- **Scratch:** `~/.cache/worktrees/worktrees/drag-drop-worktree-reference/` —
  the bats log and `live-drops.log`, the app.log lines proving the fix works
- **Predecessor:** [2026-09-18 drag-reference](../2026-09-18-drag-reference/summary.md)
  closed out before this bug was found, so its archive predates the fix.

## Context

v0.25.0 shipped "drag a worktree into a session". David tried it in the real app
and it failed: *"no Claude running in session cdv-(main)"* — while Claude was
plainly running in that pane, visible in the same screenshot.

## What shipped

`crates/worktrees-core/src/tmux.rs` — `is_version_like`, `pick_ai_pane`,
`pane_summary`, and a rewritten `ai_pane`. Plus comment corrections in `ops.rs`
and `profile.rs`.

## The bug

The AI pane was located by matching `pane_current_command` against the
configured AI word (`claude`) or `node`. A natively-installed Claude is a
symlink — `~/.local/bin/claude` → `~/.local/share/claude/versions/2.1.277` —
and XNU sets `p_comm` from the **resolved** last path component while `argv[0]`
stays `claude`. Measured:

```text
ps -o ucomm,comm -p 69362   ->   2.1.277   claude
pane_current_command        ->   2.1.277
```

Measured across the 21 live tmux sessions on this machine, the old rule found
the AI pane in **0 of 21**. Not an edge case: the feature could never have
worked for anyone with a native install.

## Decisions

**A bare `major.minor.patch` counts as the AI.** Nothing else in a worktree's
tmux session names itself that way, and the alternative — walking the process
tree under every pane — costs a `ps` per drop to learn what the pane's start
command already says.

**`pane_start_command` disambiguates, it never vouches.** This was the first
fix's bug, caught in review. `ops::launch` builds pane 0 as
`<ai_cmd>; exec "${SHELL}"`, so a pane whose Claude has exited keeps a start
command saying `claude` while running a shell — trusting it alone pasted the
reference onto that prompt and reported success, which is the precise outcome
`paste_to_ai` exists to refuse. A pane must have been launched as the AI AND
still be running it.

**No sole-pane fallback**, asked for explicitly and rejected on review's
recommendation. The app creates sessions single-pane, so "exactly one pane" is
equally the shape of a live session and of one whose Claude has exited — the
fallback would fire exactly where it is wrong. "One pane is overwhelmingly the
AI" is true of the pane's HISTORY; the foreground process is what receives the
paste. `pick_ai_pane` asserts `None` for that case, so adding it later goes red.
The resilience is bought instead with a refusal that NAMES what it saw
(`panes: %0=zsh`), so the next mismatch is one log line rather than a survey.

## Dead ends / gotchas

**"It renames its own process" was wrong**, and the correction matters. Nothing
is retitled, so a process-title search finds nothing; it is macOS-specific
(tmux's Linux backend reads `/proc/<pid>/cmdline` and reports `claude`); and an
npm or brew install reports `node`/`claude` and never reaches the version rule.
Two other comments in the tree asserted `pane_current_command` "stays `claude`"
and were corrected.

**The first fix reintroduced the bug it was fixing**, by a different route —
see the start-command decision above. Verified on an isolated tmux server: a
pane started as `exec sh -ic 'true claude; exec sh'` lists as
`cmd=[bash] start=["…claude…"]`.

**A `python3` replacement whose END anchor preceded its start anchor** silently
deleted `PaneId` and `ai_pane` and spliced the replacement into the middle of
`session_fingerprint`. The compiler caught it, but the repair attempt was worse
than starting over: the file was reset to `origin/main` (which already held the
shipped version) and the fix reapplied against unique anchors. Second time today
a non-unique anchor ate code — the earlier one removed 707 lines of `lib.rs`.

**The release moved under the session.** v0.25.0 was tagged mid-work, so the
CHANGELOG's `[Unreleased]` section had been consumed; the first edit landed a
`### Fixed` entry inside the released 0.25.0 block. Check the top of CHANGELOG
before editing it, not just before releasing.

## Verification

- `make test` exit 0, `1..338`, 338 ok / 0 not ok; lint; core 332; cli 13;
  app 58; `cargo check -p app`; `make test-frontend` 13/13. CI 9/9.
- Three mutations, each red: trusting the start command alone; dropping the
  version rule; adding the sole-pane fallback.
- Both tmux behaviours probed on an ISOLATED `-L` server, never the user's.
- The predicate re-run over all 21 live sessions: **21/21**, against 0/21.
- **Verified live, twice, from `app.log`** — the thing every prior session in
  this feature's history could not claim:

  ```text
  19:59:32Z drop_reference: @worktrees:place://bedrock -> valleos-(main)
  19:59:57Z drop_reference: @worktrees:place://drag-drop-worktree-reference -> worktrees-(main)
  ```

  Two different projects, both logged after `paste_to_ai` returned Ok.

## Follow-ups

The manual checks that remain genuinely unrun are in ROADMAP: dropping while
Claude is asking a permission question, and dropping into a session whose Claude
has exited (which this fix now refuses — worth confirming it refuses *visibly*).
