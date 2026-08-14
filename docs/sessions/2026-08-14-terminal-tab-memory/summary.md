---
title: Terminals come back as you left them
---

# Terminals come back as you left them

- **Date:** 2026-08-11 → 2026-08-14 (released 2026-08-14)
- **Worktree:** `.worktrees/ui-tweaks`
- **Branches:** `ui-terminal-cwd`, `ui-terminal-active-tab`,
  `ui-release-0-13-0` → all merged; archive on
  `close-out-terminal-tab-memory`; tree parked on `ui-next`
- **PRs:** [#122](https://github.com/penard-monkey/worktrees/pull/122)
  `feat(app): dock terminal tabs reopen where you left them` (squashed as
  `756e567`) · [#123](https://github.com/penard-monkey/worktrees/pull/123)
  `feat(app): the terminal tab strip comes back as you left it` (`1081926`) ·
  [#126](https://github.com/penard-monkey/worktrees/pull/126)
  `release: v0.13.0` (`959595b`)
- **Release tag:** `v0.13.0`
- **Planning files:** `planning.tar.gz` beside this summary

## What shipped

A dock shell is a PTY the app owns, so it dies when the app quits. Three
separate facts about a tab used to evaporate with it, and the tab came back as
a stranger: its **directory**, the **strip** it belonged to, and which one was
**in front**. All three are now remembered, and only those three — no history,
no scrollback, no environment.

### The directory (#122, backend)

`app/src-tauri/src/lib.rs`:

- `proc_cwd(pid)` — reads a live process's cwd from the OS. macOS via libproc
  (`libc::proc_pidinfo` / `PROC_PIDVNODEPATHINFO`; `libc` was already a
  dependency), Linux via `/proc/<pid>/cwd`, `None` elsewhere.
- `live_pid(child)` — `try_wait` first, so only a RUNNING shell is ever sampled.
- `shell-cwds.json` in the app config dir: `CwdMap = "repo|slug" → index → path`,
  with `read_cwds_at` / `edit_cwds_at` (lock, read-modify-write, skip the write
  when nothing changed, tmp+rename), `merge_cwds`, `remembered_dir`,
  `forget_tab`, `forget_vanished_places`.
- Capture: every 5th tick (~15s) of the existing 3s poll thread, plus once in
  `RunEvent::Exit` before the shell sweep.
- Restore: `shell_open`'s spawn branch only, via `pick_start_dir`.
- `close_shell_session` gained `keep_cwd` — a restart keeps the directory, a
  close drops it.

### The strip and the front tab (#123, frontend)

`app/src/settings.ts` — `term_tabs: Record<string, number[]>` and
`term_tab_active: Record<string, number>`, keyed `repo|slug` like
`term_tab_names`.

`app/src/App.tsx` — `TerminalTabs` gained `commitIds(shown, remembered)`,
`withRemembered`, `pick`, and an `upToken` that re-runs the restore when a
session comes back up. `dropPanels` gained a `fields` parameter.

### Infrastructure

`.github/workflows/ci.yml` + `CLAUDE.md` — **CI never ran the app crate's
tests.** `cargo build`/`check` do not compile `mod tests`, so ~230 lines of new
tests only ever ran when someone typed the command. Running them on ubuntu also
executes the Linux half of `proc_cwd`, which no machine here had ever run.

## Decisions

**A separate backend-owned file, not a key in `ui-state.json`.** That file is
written WHOLE-BLOB by the frontend (`set_settings` takes the entire settings
object), so anything the backend wrote into it would be erased by the next
settings save. This is now recorded in `CLAUDE.md` and `DESIGN.md`.

**Read the cwd from the OS, not from the shell.** OSC 7 was the obvious
alternative and is a non-starter: Apple's `/etc/zshrc` only emits it when
`TERM_PROGRAM` is `Apple_Terminal`, so it would mean lying about `TERM_PROGRAM`
or editing the user's rc files. `lsof` was rejected as a subprocess per sample
in a repo that counts spawns deliberately.

**Restore any still-existing directory, with no subtree restriction.** If you
`cd`'d out of the worktree, out of the worktree is where you were. A path that
no longer exists falls back to the place root.

**The main tmux terminal stayed out of scope.** tmux already holds its pane cwd
for as long as its server lives, and feeding a recorded cwd into
`new-session -c` would mean changing `launch()` in core, which the CLI shares.

**`term_tab_active` is a sibling of `term_tab_names`, not a `place_panels`
field.** `panelsFor` returns `{...s, ...p}`, so every key in `PlacePanels` must
also exist as a global in `Settings` — and a global would let one place's tab
seed another's.

**`keep_cwd` (absent ⇒ false), not `forget` (absent ⇒ true).** An optional flag
whose absence means its own name is true is a trap for the next caller. The
mock asserts the key, exactly as it already does for `remove_place`'s
`delBranch`, because `Option<bool>` turns a mistyped key into a silent `None`.

**Untracking a project keeps its tabs' identity.** Widening `dropPanels` to
sweep every per-place map was right where a place is genuinely gone, and wrong
for `removeProject`: untracking is REVERSIBLE, and the backend deliberately
keeps each tab's directory. Sweeping there destroyed one half of a tab's
identity and kept the other.

## Dead ends / gotchas

**`portable-pty`'s `Child::kill()` sends SIGHUP, not SIGKILL**
(`portable-pty-0.9.0/src/lib.rs:347`). An interactive `/bin/sh` on a pty whose
master is still open survives it. The app gets away with this only because
dropping the `Shell` closes the master and the EOF finishes the job.

**A pty test MUST drain the master.** Without a reader the shell's output fills
the pty buffer and the child wedges MID-EXIT — `ps` shows state `E`, it is never
reaped, and even SIGKILL + `wait()` blocks forever (observed: `wait4` stuck
>10 minutes, twice). Production never hits this because `shell_open` always
spawns a reader thread. Tests now call a `drain()` helper.

**`Child::process_id()` keeps returning a reaped pid.** portable-pty's impl for
`std::process::Child` is `Some(self.id())`, unconditional — and
`list_shell_sessions` calls `try_wait()` (which reaps) on every dock mount. Left
alone, the sampler would eventually record an unrelated process's directory
after pid wraparound. Hence `live_pid`.

**The mock harness cannot express half of this, and hid two real bugs.** It
models neither real shells nor real directories, and its invokes resolve in a
microtask. But it CAN model `close_place`/`open_place`, and simply never being
driven there is what let the session-down defects through — a harness gap, not
a harness limitation.

**Playwright: the project-remove button is a two-click arm with a 4-second
timeout, and its `title` CHANGES when armed.** Selecting on the unarmed title
clicked a *different* project, and the round-trip between two
`browser_evaluate` calls exceeded the timeout — so a run "passed" three times
having done nothing. Both clicks must go in ONE evaluate with a delay between
them; read state in a later call. (This is the one legitimate exception to the
repo's one-click-per-evaluate rule, which exists for reading state, not timing.)

**The Bash tool's cwd persists between calls, and it silently invalidated a
test run.** An earlier `cd app` meant a mutation script wrote nothing
(`FileNotFoundError` scrolled past) while the test suite still ran and reported
green — the mutation-testing equivalent of a gate that never ran. Always `cd` to
the repo root explicitly in a scripted step.

**A squash-merge with the base branch KEPT does not retarget a stacked PR.**
#123's history still carried the pre-squash commits, so its diff would have
re-proposed #122 on top of the squashed version. Fixed with
`git rebase --onto origin/main <old-base-tip> <branch>`, `--force-with-lease`,
then `gh pr edit --base main`.

**Two fixes introduced two of the six defects.** The shown/remembered split
fixed a destructive clobber and created the index-impersonation bug; widening
`dropPanels` fixed a leak and broke untracking. Neither was caught by gates or
by re-reading the diff.

## Verification

Four review rounds found **six defects, none of them caught by gates, by
re-reading the diff, or by the harness runs that had already passed**:

| # | Defect | Found by |
|---|---|---|
| 1 | Sampling a reaped pid could record an unrelated process's directory | 5-lens adversarial review |
| 2 | CI never compiled the app crate's tests | fable, on #122 |
| 3 | A deliberate edit clobbered an un-restored strip, destroying unnamed tabs | fable, on #123 |
| 4 | The restore never re-ran when a session came back up | fable, on #123 |
| 5 | A new tab inherited a remembered index's name and directory | fable, re-review |
| 6 | Untracking a project destroyed half a tab's identity | 4-lens workflow |

Nine tests added to the app crate, **each shown FAILING first** under a
deliberate mutation. One spawns a real `/bin/sh` on a real pty, tells it to
`cd`, and reads the directory back by pid — the other `proc_cwd` test would pass
even if it could not follow a child.

Harness runs asserted the DOM *and* the persisted settings at every step:
add / select / leave-and-return / reload / close-active / close-place /
re-enter / "+"-while-down / untrack.

Gates green at every merge: bats (0 `not ok`), lint, core 208, cli 6, app 29,
`cargo check -p app`, `tsc --noEmit`.

Real-app sandbox run: capture confirmed from the app-instance/file timestamps
mid-session; the full restore path and the two features together confirmed by
the user before the release was tagged.

Release: `v0.13.0`, run `31765080192`, all 7 jobs green — CLI ×4 targets,
signed app bundles ×2, `latest.json`.

## Follow-ups

- **A place removed and recreated with the app closed** keeps its old
  directories (same `repo|slug` key). Accepted and documented at
  `forget_vanished_places`: what survives is a real directory inside the new
  worktree, so the shell opens somewhere that exists.
- **A stale `term_tab_active`** can point at a tab that does not exist yet and
  become live again if that index is later recreated. Self-corrects on the next
  deliberate tab change.
- **"+" while a session is down** shows a strip that deliberately differs from
  the record until the session comes back up.
- The locally installed CLI is still `0.12.1` — `install.sh` copies rather than
  symlinking, so it does not follow the repo.
