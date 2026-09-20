# Session: the unread ring came back on every restart

- **Date:** 2026-09-20
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branch:** `ui-afterglow-truth` off a freshly fetched `origin/main`, rebased twice as main moved, deleted after merge
- **PR:** [#310](https://github.com/penard-monkey/worktrees/pull/310) → `1f6b981`
- **Release:** unreleased at close-out (sits in `[Unreleased]`)
- **Planning files:** none — the investigation ran off live measurement, not a plan
- **Cost split:** one opus thread investigated, implemented and ran the gates; no separate review pass — David asked for the merge on green CI

## Context

David: "there is a situation that when the app is restarted (tmux stays up as
it should). However the outer purple cicle in the left nav of the places come
back. So it is like the flag is triggered but doesn't save correctly?"

The outer circle is `.status-dot.done.unread::after` — the UNREAD ring from
[2026-09-16](../2026-09-16-unread-afterglow/summary.html), shown when
`last_worked_epoch > last_seen_epoch`. The reading in the question ("the flag
doesn't save") was the natural one and was wrong: the ack saved perfectly. What
moved was the other side of the comparison.

## What shipped

Two independent causes, both of which re-open a signal the user has already
spent. Neither is visible to any suite in this repo.

### 1. The backfill dated work by a file's mtime

**`app/src-tauri/src/lib.rs`** — `backfill_worked` refines a prompt's time
upward to when the work LANDED, and took that from the mtime of the session's
transcript (`mtime_epoch`, now deleted). **Claude Code keeps rewriting a live
session's `.jsonl` long after its last turn.** Measured across every transcript
touched in a day on this machine: **24 of 28 had an mtime running 12 minutes to
34 HOURS ahead of the newest entry inside the file.**

The stamp is forward-only and the backfill runs on every launch, so each
restart re-dated every place whose session was still up in tmux to "just
finished", pushing `last_worked_epoch` back past the `last_seen_epoch` the
visit had just written. `last_worked_epoch` is also THE clock (`activityAt`)
behind row age, nav sort order, ⌘K ranking and the Home Resume list — so a
restart was silently reshuffling and misdating the tree as well as lighting
rings.

`transcript_epoch(path)` replaces it: the max `timestamp` over a 256K tail of
the file (`TRANSCRIPT_TAIL_BYTES`), parsed by `entry_epoch` (ISO-8601 first, so
a date string is never mistaken for `history.jsonl`'s epoch-millis). Memoised
per session file in `backfill_worked` — a busy afternoon leaves dozens of
history lines naming the same transcript, and each read is a 256K tail.
Unreadable, missing, or a tail landing inside one enormous line all degrade to
`None`, and the stamp falls back to the prompt time exactly as before.

### 2. The ack was guarded on the DISPLAY predicate

**`app/src/App.tsx`** — `unreadOf` subtracts live activity (`!activityOf(p)`),
which is right for a slot that can show only one glyph and fatal as a write
guard. A visit to a place whose session sits at `waiting` — the state it is in
precisely BECAUSE it wants you — spent nothing: the ring was hidden behind the
amber dot while you read, the store kept the finish unread, and the ring
returned the moment that session went quiet.

Split in two: `unseenWork(p)` is the FACT (`isUnread(workedAt, seenAt)`, what
the ack guards on, at both call sites and in the dwell effect's `selPending`),
`unreadOf(p)` is the fact minus live activity (what the dot, the rollup and the
title say).

**`app/scripts/afterglow-check.mjs`** — five new static mirrors: `unseenWork`
exists, `unreadOf` still subtracts activity, no `ack` is guarded on `unreadOf`,
one is guarded on `unseenWork`, and the dwell predicate is the fact.

**`CHANGELOG.md`** `[Unreleased]` → Fixed, two entries.

## Decisions

- **Date a transcript by its content, never by its metadata.** The file's own
  `timestamp` fields are a fact about the work; the mtime is a fact about
  Claude Code's bookkeeping. Reading the tail costs one 256K read per session
  file at startup, on the poll thread, against a stat — worth it.
- **Max over the tail, not the last line carrying a timestamp.** A transcript
  is not strictly ordered: a rewritten summary lands at the end with an older
  stamp. The caller bounds the answer by `now` anyway.
- **The existing inflated stamps were left alone.** The store is forward-only
  and nothing in this app writes `last_worked_epoch` backwards; walking them
  back is a migration with its own risks, and they age out as real work
  overtakes them. Flagged to David rather than done silently.
- **The fact/display split lives in App.tsx, not afterglow.ts.** `unreadOf`
  was already there and `afterglow.ts` owns the arithmetic, not the render
  gating. Moving it would have widened the diff without changing behaviour;
  the static mirrors cover the drift.
- **Ack-while-busy is intended.** A selected place that finishes again becomes
  unseen again and re-acks after the dwell — that is the "I watched it finish"
  case the 2026-09-16 session already designed for, now reachable from the
  busy and waiting states too.

## Dead ends / gotchas

- **The first four hypotheses were all wrong**, and each looked right until it
  was measured: a lost `mark_seen` write (the store showed the stamps landing,
  one second apart, matching `nav.row` clicks in `ui-events.jsonl`), a
  read-modify-write race in `store::edit` (it reads under two locks), the
  missing mount-time pull on `sessions:busy` (real, but the first poll tick is
  3s after setup and the webview mounts well inside that), and a false
  completion edge in the poll thread. What actually found it was matching
  stored stamps against file timestamps: **five places had
  `last_worked_epoch` exactly equal to a transcript's mtime**, which a live
  exit edge — stamped from `now_epoch()` at the tick — cannot produce.
- **`updated_epoch` in the store dates the WRITE, the fields date the FACT.**
  `valleos`'s store read `updated_epoch = 1789877336`, the second the app
  started, while no field in it was less than 29 minutes old. That gap is what
  proved the backfill had written it, and it reads at first like a file that
  changed for no reason.
- **A conflicted PR gets NO CI at all.** #310 sat at `mergeStateStatus: DIRTY`
  from a CHANGELOG conflict, and `gh run list` showed zero runs for the branch
  while other branches ran normally. GitHub cannot build `refs/pull/N/merge`
  when the merge commit cannot be created, so `on: pull_request` never fires —
  which reads exactly like a hung or mis-configured workflow. Rebase first,
  then look for the run.
- **`gh pr merge` reported the documented false failure again** (`fatal:
  'main' is already used by worktree`) — its local checkout step, after the
  merge landed. `gh pr view --json state,mergeCommit` said MERGED. It also did
  NOT delete the remote branch despite `--delete-branch`, because it died
  before that step; `git push origin --delete` finished the job.
- **`--delete-branch` aside, `pkill` on a running bats suite writes a `not ok`
  into the log.** Killing the redundant local re-run produced `not ok 270 rm:
  multiple names with -y removes both` failing inside `common_setup`, plus
  `bats warning: Executed 270 instead of expected 340 tests`. A future grep of
  that log would read as a real regression; it is a killed harness.
- **Main moved twice mid-session** (#308 the docs viewer, #309 the usage
  meter), both conflicting on the `[Unreleased]` CHANGELOG block. #308 also
  added a `mermaid` dependency, so `pnpm install` was needed before `tsc`
  would run on the rebased tree.

## Verification

- **The shipped Rust against the real data.** A throwaway `#[test]` ran the
  real `transcript_epoch` over this machine's `~/.claude/projects` (removed
  before commit): 24 of 28 transcripts inflated, and the four places being
  stamped hours early now date to their last real turn —
  `mark-all-as-read` 35h ago not 44m, `created-markdown-and-planning` 11.7h ago
  not 21m, `valleos/bedrock` 8.2h ago not 2m, `docs-render` 2.6h ago not 45m.
- **Fail-first, both halves.** The two new `lib.rs` tests were shown RED
  against the mtime read (body reverted, watched fail, restored); the three new
  ack mirrors were shown RED against the old `unreadOf`-guarded acks, and a
  fourth against a renamed `unseenWork`.
- **Gates** on the final rebased tree: bats 340 ok / 0 not ok, lint clean,
  core 385, cli 14, app lib 112, `make test-mcp`, tsc + `cargo check -p app`
  clean, all 17 `app/scripts/*-check.mjs` green.
- **CI:** 9/9 jobs green on both ubuntu and macos before the squash-merge.
- Scratch (the probe scripts and their output) in
  `~/.cache/worktrees/worktrees/ui-tweaks/`: `backfill_sim.py`, `unread.py`,
  `unread-state-after-fix.txt`, `backfill-mtime-vs-prompt.txt`.

## Follow-ups

- **The busy → done → seen hand-off STILL has not run in the real app.** Same
  follow-up the 2026-09-16 session left, now with a restart on the end: let a
  place you are not watching finish, confirm the ringed dot, select it for a
  second, confirm the plain halo, then restart and confirm it stays plain.
  There is no fake claude and the mock resolves in a microtask, so no suite can
  answer this.
- **Places carrying an inflated `last_worked_epoch` keep it** until real work
  overtakes it — a one-shot repair would have to write that stamp backwards,
  which nothing in the app does today.
- **`sessions:busy` has no mount-time pull** while its backend emit is
  change-gated, so a frontend that mounts after the first poll tick holds empty
  busy/waiting sets until some session changes state — no dots, and unread
  rings unmasked on places that should be showing live state. `list_drafts` got
  a mount-time pull for exactly this reason (comment in App.tsx). Not hit at
  startup today because the first tick is 3s out, but a dev reload or a window
  recreation lands inside that window.
