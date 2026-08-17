---
title: A parked job pinned the green dot on
---

# A parked job pinned the green dot on

- **Date:** 2026-08-16 (work) / 2026-08-17 (close-out)
- **Worktree:** `~/workspace/worktrees/.worktrees/ui-changes`
- **Branches:** `ui-changes-parked-busy-dot` → `ui-changes-closeout-parked-dot`
- **PRs:** [#140](https://github.com/penard-monkey/worktrees/pull/140) (fix)
- **Released in:** v0.15.0
- **Upstream:** [anthropics/claude-code#87131](https://github.com/anthropics/claude-code/issues/87131)
- **Planning files:** none — the stream was one bug, diagnosed and fixed inline
- **Scratch:** `~/.cache/worktrees/worktrees/ui-changes/` (`probes.py`, `probe-corpus-2026-08-16.txt`)

The nav's pulsing green "Claude working" dot stayed lit on two places whose
sessions had been sitting at an idle prompt for 22h and 32h. The report arrived
as a question — *is there a running agent in that session?* — and the answer was
no: the dot was faithfully reporting a probe file that Claude Code had stopped
maintaining.

## What shipped

`app/src-tauri/src/lib.rs` — `claude_activity()` no longer lights the dot for a
probe whose last write did not set the status it carries:

- `ClaudeProbe` gained three optional fields — `parkedJobId`, `updatedAt`,
  `statusUpdatedAt` (`Option`, so a probe written before the park feature still
  parses; a hard parse error there would drop that session from the scan and
  take its dot with it).
- `busy_is_delegated()` — `parkedJobId.is_some() && updatedAt > statusUpdatedAt`.
- The `busy` arm of the match is guarded by it; `waiting` deliberately is not
  (see Follow-ups).
- Two tests built from the captured probes verbatim, plus a CHANGELOG entry and
  a ROADMAP item.

## The bug

Claude Code writes one probe per live session at `~/.claude/sessions/<pid>.json`
and rewrites it on status **transitions**. Parking a turn — handing it to a
background job — is not a transition but it *does* rewrite the file, setting
`parkedJobId` and nulling `bridgeSessionId`. If the turn was **mid-flight**, that
rewrite carries the `busy` status forward and nothing ever writes the file again:

```
16304.json  interactive  status=busy  parkedJobId=d8d56f6f  statusUpdatedAt 09:40:45  updatedAt 09:41:01
56821.json  bg           status=idle  jobId=d8d56f6f        went idle 21:38, ~12h later
```

Both probes carry the same `cwd` (the bg session is a fork of the parent), so the
place kept a `busy` vote from a session that had been idle since the morning. The
app's only liveness guard is pid-alive — deliberately no age expiry, because the
probe is transition-written and `updatedAt` can legitimately be minutes old on a
genuinely working session — so a live-but-idle process pinned the dot forever.

The split is clean, 4 out of 4 on this machine: parking **while busy** leaves the
stale status, parking **while idle** preserves idle and is fine.

## Decisions

**Fix the app even though the bug is upstream.** The probe contract does not
promise a status that is always current, and the app is the thing showing a wrong
light. Filed upstream as well (#87131) rather than only working around it.

**Key on the stamps, not on `parkedJobId` alone.** The obvious rule — *an
interactive probe with `parkedJobId` set never contributes busy* — was the first
recommendation and was rejected: nothing proves Claude clears that field on a
later turn, so a session that parked a job once could go permanently dark. A
permanently dark place is the worse failure of the two, because a stale green dot
at least says *look here*. `updatedAt > statusUpdatedAt` reads as "the last write
did not set this status", which is exactly the residue, and it lets a new turn in
the same session light up normally.

**No cross-probe join with the background session.** The alternative was matching
`parkedJobId → jobId` and reading the child's status. Not needed: while the parked
job is genuinely running, its own probe carries the same `cwd` and lights the same
place. Staying a pure per-probe predicate also covers the case the join cannot —
a bg probe that is gone. Cost: a parked job running with no local session at all
(a purely remote bridge) shows no dot, which is the honest answer for a pane that
is sitting idle.

**Precedence between the two probes sharing a `cwd` stays busy-wins.** A blanket
"the bg probe overrides the interactive one for a shared cwd" would break a new
interactive turn running next to an old idle parked job in the same directory.

## Dead ends / gotchas

**The "8 hours untouched" in the UI was a red herring.** That is the place's
activity age, derived from git/tmux, and it has nothing to do with the dot. Two
independent signals disagreeing looked like one bug and were two facts.

**`#[serde(default)]` on an `Option` field is redundant.** Removing it to prove
the "old-shape probe still parses" test could fail did nothing — serde already
maps a missing field on an `Option` to `None`. The test still earns its place (it
pins the behaviour against someone changing the type), but the comment claiming
the attribute is what saves us was wrong and was corrected. The *first* test was
confirmed red the proper way, by gutting the predicate.

**The tests pin the predicate, not the wiring.** Reverting the guard in
`claude_activity` to an unconditional `busy.push()` leaves both new tests green —
`claude_activity` scans hardcoded config roots and cannot be pointed at a temp
dir. Inherent, and the guard is two visible lines, but the coverage boundary is
`busy_is_delegated`, not the scan.

**`gh pr merge` fails in a side worktree**: *fatal: 'main' is already used by
worktree at …*. The merge itself had already succeeded on GitHub — only `gh`'s
local checkout step failed. Check `gh pr view --json state` before believing the
error; the same shared-branch rule CLAUDE.md warns about, from a new direction.

**`node_modules` was 8 commits stale** and `tsc --noEmit` failed on a missing
`@xterm/addon-search` that `package.json` had declared. Nothing to do with the
change; `pnpm install` fixed it and left the lockfile untouched. A gate failing
in a file you never touched is worth one `git status` before debugging it.

## Verification

- Gates on the fix branch: `make test` (309 ok, 0 not ok, checked by redirecting
  and grepping — not `| tail`), `make lint`, `cargo test -p worktrees-core` (227),
  `-p worktrees-cli` (7), `cargo test -p app --lib` (32), `tsc --noEmit`,
  `cargo check -p app`. Release CLI rebuilt first.
- The new predicate test was confirmed **red** with `busy_is_delegated` gutted to
  `false`, then restored.
- Ground truth on the two stuck sessions: `ps` showed `S+`, 0.0% CPU, children
  only the configured MCP servers; the panes sat at an empty prompt.
- Independent sweep of all 35 live probes (fable): `updatedAt - statusUpdatedAt`
  was exactly 0 on 27 of the 30 stamped probes and non-zero **only** on the four
  park writes plus one +95 ms bridge registration on a bg probe. No counterexample
  in either direction — no genuinely busy probe with skewed stamps, no stale-busy
  probe with equal stamps. And no heartbeat write exists: the session's own probe
  went untouched through 15+ minutes of continuous busy work.
- The afterglow was checked and needs no suppression: `busy_ticks` is
  process-local and starts empty, so on the fixed build's first tick the stuck
  places are filtered before `completion_edges` ever sees them — no spurious
  `sessions:done` ember.
- **Not done:** the fix has never been observed in a real build. The stuck probes
  were deleted as an unstick before a sandbox run, so reproducing now means
  synthesising a probe pair against live pids.

## Follow-ups

- **Eyeball the fix in a real build** — write two probe files against live pids
  (one `busy` + `parkedJobId` + skewed stamps, which must stay dark; one `busy`
  with equal stamps, which must light) and run `app/scripts/sandbox.sh --app`.
  Folds into the existing "Eyeball busy dot in the real app" roadmap item.
- **Does a parked `waiting` probe pin the amber dot too?** Same residue family,
  one word to fix, no evidence it is reachable. Already on ROADMAP.
- `pid_alive` uses only `kill(pid, 0)` while the probe carries `procStart` — pid
  reuse could keep a dead session's dot alive. Pre-existing, not touched.
