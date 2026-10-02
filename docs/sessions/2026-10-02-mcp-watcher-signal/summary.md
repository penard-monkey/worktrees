# MCP resource watcher: notify on what the list shows

- **Dates:** 2026-09-20 (measurement) → 2026-09-23 (fix merged); closed out 2026-10-02
- **Worktree:** `bug-fixes`
- **Branches:** `bug-fixes-mcp-watcher-signal` (merged, deleted), `bug-fixes-close-out-mcp-watcher` (this archive)
- **PRs:** [#332](https://github.com/penard-monkey/worktrees/pull/332) — merged `c054e19`, shipped in v0.30.0
- **Release tag:** none cut by this stream
- **Planning files:** none

## What shipped

**`membership()` now digests what the resource list RENDERS, not the sidecar's
bytes** (`crates/worktrees-cli/src/mcp.rs`). The signal behind
`notifications/resources/list_changed` hashed `read_dir` names plus
`.worktrees.places.json`'s `mtime:len`. Those bytes move on every write, and the
sidecar is rewritten constantly for fields the `@` picker never renders —
`last_worked_epoch`, stamped as you work, above all. It now digests each place's
`(slug, lifecycle, title)` via `store::read_lenient`.

The watcher takes the **repo root** instead of a hand-built sidecar path, so the
store derives and canonicalizes it the way every other reader does
(`spawn_list_watcher`, and the `cmd_mcp` call site that used to
`format!("{}/.worktrees.places.json", …)`).

**`mcp::tests::the_watch_signal_moves_on_membership_and_not_on_work`** became a
pair of directions instead of a list. **`scripts/mcp-resources-check.py`** gained
the same pair over the real protocol.

ROADMAP's chattiness entry was **replaced** rather than deleted — see Follow-ups.

## The measurement that drove it

Read out of `~/.cache/worktrees/mcp-debug.log` — the temporary `dlog` apparatus —
before the `(main)` session deleted it at v0.27.0. 47 hours, 20 sessions, 5 repos:

| | |
| --- | --- |
| `list_changed` notifications whose reason was `sidecar` | **1,651 / 1,730 (95%)** |
| `resources/list` responses identical to that session's previous one | **1,634 / 1,733 (94%)** |
| `resources/read` | **2** |
| `resources/read MISS` | **0** |
| cost per re-fetch | p50 17ms · p95 38ms · max 264ms |

Field frequency across 10 sidecars / 75 places — the fields that *can* reach the
list barely move:

```
last_worked_epoch  75     <- woke the watcher, invisible in the list
last_opened_epoch  50
last_seen_epoch    47
pinned             19
title               2     <- actually rendered
lifecycle           1     <- actually rendered
```

The `worktrees` sidecar held *only* clock fields, so nothing in it could change
the list, yet every stamp woke every live session in the repo — ~35 spurious
wakes an hour, per session, multiplied by every session in that repo.

## Decisions

**Digest the rendered fields, don't debounce.** A debounce or a longer poll would
have cut the rate while keeping the wrong question. Hashing `(slug, lifecycle,
title)` makes the signal mean what its name says, and a title edit still reaches
the picker within one poll.

**Parse the sidecar rather than stat it.** The old code stat-ed to stay cheap.
These files are 128 B to 4.9 KB and `resources()` already calls `read_lenient` on
every list, so the parse costs nothing worth counting — and it is what makes the
precise signal possible at all.

**Replace the ROADMAP entry, don't delete it.** The chattiness is fixed, but the
log's "0 read errors" was never evidence of correctness: with **2** reads in 47
hours the cache-divergence risk the watcher exists to cover was never exercised.
What survives is that open question. Deleting the whole entry would have retired
a measured fact along with the fixed one.

**Report the two facts separately.** "1,817 lists against 2 reads" invites "nobody
uses resources, so who cares". It is really 95% watcher noise *over* a
barely-used surface — two independent facts, and conflating them would have got
the fix filed as not worth doing.

## Dead ends / gotchas

**The covering test encoded the bug as correct.** Its last assertion wrote
`{}` → `{"places":{}}` — a content-free change — and asserted the signal moved.
It passed *only because the byte length moved*, so it would have passed
identically for a clock stamp. This is why the suite stayed green for the life of
the bug. Root cause: the assertion described the mechanism it happened to have
(the file changed) rather than the rule it meant (what the list shows changed).

The red was unusually legible once written:

```
assertion `left == right` failed: a clock-only write must NOT notify
  left: "alpha|1790188487367:89"
 right: "alpha|1790188487367:58"
```

Identical mtime — same millisecond — only the length differs.

**A unit test on `membership()` cannot see the call site.** A wrong path handed to
`spawn_list_watcher` leaves it green, which is exactly the half this change
touched. Hence the protocol-level pair in `mcp-resources-check.py`; it was
confirmed to fail (exit 2) against the restored `mtime:len` signal.

**Both directions had to be pinned, and only one of them was obvious.** Too broad
is what shipped. Too narrow — dropping the sidecar half — would silently stop new
titles reaching the picker. Mutating the fix each way and watching a *different*
assertion go red is what proved the pair.

**`~/.cache/worktrees/mcp-debug.log` kept GROWING for two days after the
apparatus was removed, and that is not a bug.** It reached 2.8 MB with a last
write of Sep 22, days after #317 merged on Sep 20 — which reads exactly like a
failed deletion. It was not: the log contains no `start` line above `v0.26.0`,
and its last writers (pids 69604, 78069) were MCP servers launched *before* the
upgrade, still running their old binary image. The installed CLI was already
v0.29.0. **A long-lived MCP server outlives a CLI upgrade**, so "the new binary
cannot be doing this" does not mean the new binary is not installed. Incidentally
a second confirmation of the finding: two zombie servers wrote ~1.85 MB in two
days from pure sidecar churn.

**`gh pr merge` from this side worktree reported a failure it did not cause** —
the known `fatal: 'main' is already used by worktree at …` from gh's *local
checkout* step, after the merge had landed. Verified with
`gh pr view 332 --json state,mergeCommit` instead of retrying.

**A stale claim of mine, corrected during this close-out.** I reported
`bug-fixes-codesign-local-installs` as a merged squash leftover, safe to delete.
It is not merged — PR #124, closed not merged, deliberately parked and documented
at `ROADMAP.md`. I had matched its subject against
`chore: close out codesign-privacy-prompts session (#128)`, a different thing.
A subject match is not a merge check; the branch survives because this close-out
verified content presence (`git cat-file -e origin/main:<path>`) instead.

## Verification

Gates on the final tree, re-run against `origin/main` at v0.36.0 (84 commits
after the fix merged, to confirm it survived): `cargo build --release -p
worktrees-cli` · `make test` **451/451** (plan `1..451`, zero `not ok`) ·
`make lint` · `make test-mcp` · core **665 passed / 1 ignored** · cli **70** ·
`cargo test -p app --lib` **151** · `tsc --noEmit` · `cargo check -p app`.

The fix is intact on current main (`store::read_lenient(repo)` at `mcp.rs:3289`;
both protocol checks and the unit-test pair present).

PR #332's own CI was green on all 10 jobs across both OSes — read from raw job
conclusions, not from the `gh run watch` exit code.

CHANGELOG placement was checked after the merge rather than assumed: main moved
twice (#329, #330) while #332 was open, and the entry landed under
`## [Unreleased]` as intended.

## Follow-ups

- **The MCP resource surface is barely used and its correctness is unproven** —
  `ROADMAP.md`. 2 `resources/read` in 47 hours; the `-32002` divergence path the
  watcher exists to cover has never been exercised. Worth deciding before more is
  built on the surface.
- **`~/.cache/worktrees/mcp-debug.log`** — 2.8 MB, inert since Sep 22, nothing
  reads or writes it. Deletable; left in place because it is the raw evidence
  behind the numbers above.
- **`bug-fixes-codesign-local-installs` stays parked** (PR #124, closed not
  merged) — see `ROADMAP.md` for resurrection instructions. Not a leftover.
