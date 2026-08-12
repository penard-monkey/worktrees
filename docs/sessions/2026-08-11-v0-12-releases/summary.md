# Session — the sandbox found what the mock harness could not

- **Date**: 2026-08-11
- **Worktree**: `.worktrees/ui-tweaks`
- **Branches**: `ui-dock-default-closed`, `release-0-12-0`, `fix-title-refresh`,
  `release-0-12-1` (all squash-merged), `close-out-v0-12-releases` (this archive)
- **PRs**: [#114](https://github.com/penard-monkey/worktrees/pull/114) dock starts closed ·
  [#117](https://github.com/penard-monkey/worktrees/pull/117) v0.12.0 ·
  [#118](https://github.com/penard-monkey/worktrees/pull/118) nav refresh ·
  [#119](https://github.com/penard-monkey/worktrees/pull/119) v0.12.1
- **Releases**: **v0.12.0** (`327e10c`) and **v0.12.1** (`e2414ad`) — both workflows
  green, 4 CLI targets + 2 signed bundles + `latest.json` each
- **Planning files**: none (this stream continued the
  [space-workbench session](../2026-08-11-space-workbench/summary.md), whose
  planning tarball is archived there)

## The shape of this session

Three bugs shipped in code that had passed gates, adversarial review, and
harness verification. **Every one was found by running the real app**, and every
one was invisible to the mock harness *by construction*. That is the story worth
carrying forward; the fixes themselves are small.

## What shipped

### #114 — a place you have never opened the dock in starts closed

`app/src/settings.ts` (`panelsFor`).

The per-place panel memory from #111 used the global `dock_open` as a **seed**
for places with no entry, on the theory that arriving somewhere new should not
rearrange the furniture. In practice it made the dock **spread**: open it in one
worktree and every worktree clicked afterwards was already open before being
asked — the same "it changed what I left it in" complaint pointed the other way.

"No entry" now means *not set up yet*. `dock_tab`/`dock_width` still seed, since
neither is visible until the dock is open.

### #118 — renamed places reach the nav

`app/src/App.tsx`, `app/src/mock/install.ts`, `app/scripts/race-check.mjs` (new).

Two independent causes of one symptom:

1. **Latency.** A declared edit reached the nav only via a full `list_workspace`
   sweep of every registered project — 0.28s measured for one project with nine
   worktrees, seconds across a real workspace. `patchDeclared` now applies
   title/pin/note to the workspace in hand; the sweep still confirms.
2. **A race.** `refresh()` had no ordering guard. Eight call sites fire it
   without awaiting each other, so several sweeps are in flight and the last to
   **resolve** won — a sweep that read the store *before* a write could land
   *after* it and restore the old value, pinned there by the byte-compare dedupe
   until the next `places:changed` (realistically the 30s safety emit). Reads now
   carry a start-order ticket.

## Decisions

**Optimistic update, not a spinner.** The user's own framing was "handle that
better so the user knows it's working". A spinner communicates a wait; removing
the wait is better, and for declared edits the result is known before the call
returns.

**`patchDeclared` covers title, pin and note — NOT lifecycle.**
`lifecycle_effective` is reconciled server-side from the declared label *and*
live tmux state, so patching the label alone would render a row disagreeing with
its own badge. Waiting beats lying.

**The guard is "older than what LANDED", not "not the newest ISSUED".** Both fix
the race; only one stays live. Gating on the newest *issued* ticket discards
every read that completes while another is in flight — during a burst that is
all of them, and the tree freezes until the burst ends (measured: first update
1606 ms into a 1200 ms burst, vs 602 ms). Exactly the large-workspace case the
bug was reported from.

**Ship the race harness in-repo.** `app/scripts/race-check.mjs` slices the real
`refresh`/`commitWs`/`patchDeclared`/`mutate` source out of `App.tsx` and drives
it under controlled promise-resolution orders. A test nobody else can run is
worthless, and this class had by then shipped twice.

**Hold the tag for a human.** 0.12.1 was prepped, then paused before tagging —
signed bundles reaching users via the updater is the one hard-to-reverse step,
and the immediately preceding fix had made things worse. Tagged on explicit
instruction.

## Dead ends / gotchas

**⚠ The three bugs share one root cause, and it is the harness.** The mock's
`list_workspace` answers in a microtask. So: two sweeps never overlap (the race
cannot exist), there is no gap between "write done" and "refresh returned" (the
latency cannot exist), and every panel-memory check used places already set up
(the seed bug's failing state — the *second* place visited — was never
exercised). Each bug was verified as fixed against a harness structurally
incapable of showing it.

Mitigations shipped: `?slowlist=<ms>` in the mock (matching `?slowcreate`), and
`race-check.mjs`. Neither existed before; both fail on the shipped code and pass
on the fix.

**⚠ I fixed a bug by introducing a worse one.** #118's first commit added
optimistic updates and left `lastSnap` untouched — deliberately, with a comment
explaining why. But when the confirming refresh returns a workspace
byte-identical to what refresh last saw (**a write that FAILED**, or a name
retyped identically), the dedupe bails and `setWs` never runs. The optimistic
value then stands with nothing left to correct it, and an idle workspace
serializes identically for a long time, so it survives until restart. A
*permanent* stale state, replacing a transient one. The comment claimed "the
refresh corrects anything the optimism got wrong"; it did not.

**⚠ Same failure mode, three times: a stated invariant the code did not honour.**
`preHydration` (guarded the map, missed the scalars it had just widened),
`patchDeclared` (documented the correction that never happened), and the seed
decision (written into the CHANGELOG as deliberate, wrong in practice). In each
case the comment was right and the code below it was not. Confidence in an
intent is not evidence the code implements it.

**⚠ Gate output that lies.** Three distinct traps hit in one session, all now in
CLAUDE.md: `cargo test | tail -4` prints the *Doc-tests* block and reads as an
empty suite; `grep -c` exits non-zero on zero matches and silently breaks an
`&&` chain of gates; and a pipeline's exit status is the last stage's, so
`make test | tail` is green whatever bats said.

**⚠ A stale `target/release` made a full gate pass meaningless.** Built at
bootstrap, `store.rs` edited an hour later; `make test` and the `ls --json` diff
both read that binary. CLAUDE.md warned a stale binary makes bats *fail* — here
it made them *pass*. Also: the first freshness check compared the CLI binary
against an *app* source file it is not built from, reporting a false "stale".
The honest check is `find crates -newer target/release/worktrees`.

**A red CI check that was not a code problem.** #110's `test (macos-latest)`
failed inside `actions/checkout@v7` — runner infrastructure. Re-running the
failed job alone turned it green. Read the failing *step* before diagnosing.

**Refuted, worth recording so it is not re-litigated.** Moving `<header>` out of
`<main>` was flagged as orphaning landmarks; the opposite is true — inside
`<main>` a `<header>` maps to `sectionheader`, so there was no banner landmark to
lose. Established by reading Chromium's AX tree over CDP, not from the spec.

## Verification

- **v0.12.0** and **v0.12.1** release workflows both completed green; `gh release
  view v0.12.1` confirms 4 CLI targets, 2 signed `.app.tar.gz` + `.sig`,
  `latest.json`, `checksums.txt`.
- `race-check.mjs` by exit code — v0.12.0 exits **1** (stale read wins, commitWs
  hole), #118's first commit exits **1** (plus the failed-write stickiness),
  merged main exits **0**.
- Battery dedupe proven intact: 0 re-renders across 10 idle polls, across two
  overlapping polls resolving out of order, and across 10 idle polls after a
  rename. That dedupe exists for battery and was the explicit risk of touching
  `refresh()`.
- Optimistic path timed against a 2 s simulated backend: nav updates in **52 ms**
  with the fix, **2056 ms** without (fix removed, vite restarted `--force`,
  served bundle content-checked, re-measured).
- Gates on every PR, and on the combined state before each release: `make test`
  288 bats / 0 failing · `make lint` · core **208** · cli 6 · `tsc --noEmit` ·
  `cargo check -p app`. Release binary rebuilt and confirmed newer than all
  `crates/` sources before believing bats.

## Follow-ups

See `ROADMAP.md`. The load-bearing one: **the mock harness cannot express real
backend timing**, and three shipped bugs came out of that gap in a single
session. `?slowlist` and `race-check.mjs` cover the two shapes hit here; the
general problem is untouched.
