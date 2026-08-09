---
title: "Session: the app stopped burning power in the background"
---

# Session: the app stopped burning power in the background

- **Date:** 2026-08-07 (archived 2026-08-09, session paused by a power outage)
- **Worktree:** power-consumption
- **Branches:** `power-consumption` (feature), `release-0.9.1` (version bump), `close-out-power-consumption` (this archive)
- **PRs:** [#92](https://github.com/penard-monkey/worktrees/pull/92) (squash-merged → `f49f2b9`), [#93](https://github.com/penard-monkey/worktrees/pull/93) (release → `b429265`)
- **Release:** **v0.9.1**, tagged and published (10 assets, signed bundles)
- **Planning files:** planning.tar.gz alongside this summary (task_plan / findings / progress)

## Where this started

> worktrees app on mac I noticed always shows as consuming significant power,
> how do we make that less without impacting and in fact improving performance
> would be ideal. Let's put a fable agents to research our options.

Four fable agents researched in parallel: frontend/webview, Rust backend, macOS
platform + Energy Impact literature, and a measurement harness.

## What shipped (v0.9.1)

Measured, release vs release, same workspace:

| scenario | v0.9.0 | v0.9.1 |
|---|---|---|
| **backgrounded idle** CPU | 3.19 % | **0.10 %** |
| **backgrounded idle** energy impact | 3.22 | **0.10** |
| **backgrounded idle** child spawns / 120 s | 159 | **1** |
| visible idle CPU | 3.19 % | 1.80 % |
| visible idle energy impact | 3.22 | 1.91 |
| subprocesses / snapshot (cold) | 91 | 55 |
| subprocesses / snapshot (repeat-poll) | 91 | **32** |

**Visibility gating** (`app/src/App.tsx`) — new `useWindowAwake()` hook; the
`places:changed` listener defers when hidden and catches up on the visible edge;
the usage tick/poll and the 5-minute doctor sweep follow the same rule;
`refresh()` bails on a byte-identical workspace.

**Subprocess diet** (`crates/worktrees-core/src/project.rs`, `sysclock.rs`) —
`status_v2()` folds three git calls into one `status --porcelain=v2 --branch`;
`last_commit()` folds two `git log` into one; the canonical session check reads
the already-prefetched `PaneList`; the BSD/GNU probe is `OnceLock`'d and birth
dates are memoized on `(path, dev, ino)`.

**Cosmetics that never idled** (`app/src/TerminalPane.tsx`, `App.css`) — cursor
blink follows the window; `breathe` is stepped instead of interpolated.

## Decisions

- **Frontend, not backend, is the throttle point for the git sweep.** The
  backend's `places:changed` emit is nearly free; `refresh()` is what invokes
  `list_workspace` and fans out a git subprocess per place per project. Gating
  the listener is what cancels the storm.
- **Kept git/tmux shelled out**, per CLAUDE.md. Every win came from not paying
  twice for answers already in hand, never from swapping in a library.
- **`OnceLock` on `have_git`/`have_tmux` was rejected**, though it saves 2
  spawns/snapshot: `lib.rs` `tmux_check(refresh)` calls `have_tmux()` before and
  after specifically to detect tmux being installed mid-session. Caching breaks a
  real feature for a trivial win.
- **The nav hoist was deliberately not batched in.** `PlaceRow`/`GroupHeader`/
  `ProjectNode`/`FlatLens` live inside `App()` and rebuild the whole nav per
  render, but all four must move together (~30 closed-over values), it is
  invisible to bats, and landing it in the same measurement window would make any
  regression unattributable. Still open.
- **No `objc2`/NSWindow occlusion observer.** Settled empirically instead of on
  the literature — see below.

## Dead ends / gotchas

**The initial ranking was wrong, twice.** Hand recon put subprocess spawns first.
Platform research corrected it — macOS Energy Impact weights GPU time up to ×3,
and Ghostty had this exact symptom from cursor blink alone (−85 % when disabled)
— so the GPU/animation path became the prime suspect. Then measurement showed the
spawn diet and the visibility gate did essentially all of the work, and **the
GPU hypothesis was never validated**. Blink and `breathe` shipped, but their
contribution is unseparated from the rest. Research narrowed the search; it did
not pick the winner.

**Two agents contradicted each other on `visibilitychange`.** One cited
tauri#6864/#10592 as proof it doesn't fire on occlusion (so we'd need an NSWindow
observer via objc2); the other noted those are Windows/WebView2 reports. Settled
by instrumenting the transition through `log_event` and reading `app.log` on a
real build: **WKWebView does fire it on macOS.** Cheap empiricism beat two
confident literature reviews.

**`ps`-sampling undercounts, badly.** The same app read 158 spawns/120 s at 1 Hz
and 255/60 s at 5 Hz; sub-millisecond `tmux list-sessions` calls are missed almost
entirely. Exact counts need a PATH shim of counting wrappers
(`~/.cache/worktrees/worktrees/power-consumption/spawn-count.sh`).

**That shim cannot measure the app.** `fixup_gui_path()` prepends the login-shell
PATH at startup and keeps the inherited one only as a trailing fallback, so the
real git wins. Measure the engine through the CLI — `ls --json` runs the identical
`snapshot()` path.

**`app.log` is UTC; `measure.sh` labels are local.** Cross-referencing them is the
only way to know what window state a measurement actually ran under. The first
"hidden" run appeared to show MORE work than visible (257 spawns, 2.88 % CPU) —
the window had flapped visible→hidden five times and the app was being used
mid-window. **Discarded, not reported.** The re-run greps `app.log` for any
`window visible` transition inside its own window and fails itself if it finds one.

**Running the bundle's binary to probe it LAUNCHES the app.**
`.../worktrees.app/Contents/MacOS/app --version` is the GUI entry point; it
started a second instance rather than printing a version.

**porcelain v2 nearly changed `ls --json` silently.** `# branch.upstream` reports
the *configured* upstream even when its remote-tracking ref is gone; the
`rev-parse @{u}` it replaced reported only one that *resolves*. That turned
`"upstream": null` into `"upstream": "origin/<branch>"` for one worktree.
`# branch.ab` is the free discriminator — git emits it only when the upstream
resolves. **Neither bats nor the 205 unit tests covered this**; only diffing
`ls --json` against the shipped binary caught it. Diff output against the last
release whenever a git invocation is consolidated.

**A review found three cases of the change undoing its own goal.** The
visibility guard sat *after* the immediate `sweep()`/`pull()`, so both effects
spent git and a fetch on the transition into going quiet; `winFocused` was dead
state re-rendering the whole tree on every ⌘-Tab; and `steps(4, jump-both)` never
reaches either keyframe endpoint, collapsing the busy pulse to a 0.2 opacity
swing (`jump-none` gives `{0, ⅓, ⅔, 1}`). Also caught `stat_birth` memoizing a
failed probe as `0` forever — one unlucky fork would pin `created: "-"` until
restart.

**`git checkout -B main` in a worktree moves the shared ref out from under the
root worktree.** Doing that mid-release left the repo root on `main` at the new
commit with a stale working tree and 8 files showing as phantom modifications.
Verified the staged diff was exactly the release inverted, and that no untracked
files existed, before resetting. Tag from the worktree that already owns `main`.

**CI flake:** `skills add installs from a local git repo and pins the commit`
(`test/skills.bats:175`) failed on macOS only, passed locally and on Linux, and
passed clean on re-run. A `git clone file://` test, untouched by this work.

**`/tmp` is pruned.** The session paused two days (power outage); every scratch
file under `/tmp/claude-501` was gone on resume. Scratch belongs in
`~/.cache/worktrees/<project>/<worktree>/`, as CLAUDE.md says.

## The zombie leak: investigated, NOT fixed, deliberately dropped

Worth its own section because the obvious diagnosis is wrong and someone will
otherwise redo it.

`term_close`/`kill_shell` call `child.kill()` with no `wait()`, which for a plain
`std::process::Child` provably leaks (verified: kill + drop → `Z`). **But these
are `portable_pty` children**, and its `ChildKiller::kill`
(`portable-pty-0.9.0/src/lib.rs:340-373`) is not a plain kill: it sends
**SIGHUP**, then polls `try_wait()` five times over ~200 ms — and `try_wait`
reaps. The common path already cleans up.

The real hole: a child outliving that grace period falls through to `SIGKILL` and
returns **with no final wait**. Narrow, but real.

**Could not reproduce the actual leak** in four attempts (plain `sleep` via PTY;
`sh -c 'trap "" HUP; sleep 30'`, which should hit the SIGKILL path; enumerating
every child rather than probing one pid; and standalone outside the test
harness). A fix was written and reverted rather than shipped under a claim it
hadn't earned.

⚠ **A regression test was written that passes IDENTICALLY with and without the
fix.** Only checking whether it failed on the *unfixed* code caught it. Removed —
a test that cannot fail is worse than none. Verify the guard fails on the old
behaviour FIRST.

Observed: a 14-hour v0.9.0 instance had 41 zombies and 15 accumulated dock
shells; a fresh v0.9.1 had 0 at ~9 minutes. Trigger correlates with open/close
cycles that can't be driven without the GUI. Live check:

```sh
pgrep -f "worktrees.app/Contents/MacOS/app" | head -1 \
  | xargs -I{} sh -c 'ps -axo ppid,pid,stat | awk "\$1=={} && \$3 ~ /Z/" | wc -l'
```

## Verification

- **Gates:** `make test` 286 → 288 after merging main (all ok) · `cargo test -p
  worktrees-core` 205 · `make lint` · `tsc --noEmit` · `cargo check -p app` ·
  release CLI + app bundle build clean.
- **CI:** 9/9 green on both PRs (after one re-run for the flake above).
- **Correctness:** `ls --json` byte-identical to v0.9.0 on the real workspace,
  verified by diff — the check that caught the upstream regression. One
  documented exception: an unborn HEAD whose upstream resolves now reports no
  upstream (git omits `branch.ab` because the branch is initial). `Place.upstream`
  has no consumer.
- **Energy:** measured with `~/.cache/worktrees/worktrees/power-consumption/`
  (`measure.sh`, `compare.sh`, `spawn-count.sh`, `runs.csv`, `logs/`), 120 s
  windows, hidden run self-validating against `app.log`.
- **Steady-state spawn count** measured by calling `ls()` three times in one
  process (50 → 32 → 32), since a one-shot CLI run cannot show the caches.

## Follow-ups

- **Backend 3 s poll loop is now the largest remaining background cost**
  (`lib.rs` setup thread: `session_fingerprint()` + `claude_activity()` every 3 s,
  ungated, plus an ungated `git fetch` per root). Needs a `set_poll_mode` push,
  same shape as the existing `set_fetch_interval`.
- **Nav hoist** — all four components to module scope with `memo`. Unproven
  value; measure render churn first.
- **Zombie leak** — open, unreproduced. See above.
- **PR3 (FSEvents)** — replace the 30 s blind emit and the claude probe scan with
  a `notify` watcher; would make updates sub-second instead of up-to-30 s stale.
- **PR4 (terminal)** — PTY→IPC coalescing. Tauri's `Channel::send` JSON-array-
  encodes raw payloads < 1024 B into a webview JS eval, once per read syscall.
  Optionally `@xterm/addon-webgl` (needs a manual-check doc entry; invisible to
  bats).
- **Flaky test** — `skills.bats:175` on macOS CI.
