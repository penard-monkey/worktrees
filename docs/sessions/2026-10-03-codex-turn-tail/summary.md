# codex-turn-tail — a long Codex turn read as idle

- **Date:** 2026-10-03
- **Place:** `codex-turn-tail` (branch `codex-turn-tail`)
- **PR:** [#434](https://github.com/penard-monkey/worktrees/pull/434), squash-merged as `0175bd5`
- **Release:** none (under `## [Unreleased]`)
- **Planning files:** none. The lane worked from `.planning/brief.md`, a handoff from a pi lane.

## What shipped

- **`codex::rollout_turn_since(path, end, floor)`** (`crates/worktrees-core/src/codex.rs`):
  - Finds the newest turn boundary in the first `end` bytes, among the lines not yet complete at byte `floor`.
  - Reads backwards in `TURN_SCAN_CHUNK` (64 KiB) steps and stops at the first boundary or at the floor.
  - A line split by a step is carried into the next one whole before it is decoded, so a marker or a multi-byte character is never cut.
  - `turn_of_line` does the cheap reject on raw bytes, so megabytes of tool output are almost never decoded.
- **`activity::codex_tail`** (`crates/worktrees-core/src/activity.rs`):
  - The turn now comes from that scan, with `floor` set to the cached length. The cached turn stands only when the new bytes hold no boundary.
  - A file that shrank is scanned from scratch.
  - A failed scan caches nothing, so the next poll retries the same bytes.
  - The model still comes from the 256 KiB tail. `ROLLOUT_TAIL_BYTES`'s doc says so.
- **Tests (5)**, each shown red on the old derivation or by a mutation:
  - beyond the tail
  - a cached Done does not outlive a new turn
  - a marker split by a scan seam (through `ü`)
  - growth without a marker keeps the cached turn (red with `.or(pt)` deleted)
  - `rollout_turn_since` stops at its floor
- **CHANGELOG:** one `### Fixed` entry. It also merges a duplicate `### Fixed` header that main already carried under `[Unreleased]` (#432).
- **AGENTS.md / `app/src/mock/install.ts`:** the stale `codex_waiting_panes` became `activity::codex_panes`.

## The bug

`mac-utilities:release-path`'s pane said "Working (7m 16s)". MCP `place_status` said `idle` and the nav had no dot. That turn's only `task_started` was 1,248,675 bytes from the end of a 1,271,681-byte rollout. `codex_tail` read only the last 256 KiB, found no boundary, and fell back to no answer.

## Decisions

- **The floor is the cache's length, not a second, bigger window.** A cached turn is the newest boundary in the file up to exactly its length. So a scan of only the bytes appended since that length is both complete and cheap: one scan per growth step, covering only the growth. Any fixed window can be outrun by a long enough turn.
- **The scan is its own read; it does not reuse the 256 KiB tail.** The tail stays for the model. The scan's first 64 KiB step normally reaches the floor (a poll's growth is a few KiB). That costs one extra small read, against changing the shape of `tail_lines_checked`, which the app also uses.
- **A line still unterminated at the floor is weighed again.** The floor test compares the line's TERMINATOR offset with the floor. A marker half-written at the last poll is not lost.
- **pi's draft was replaced, not patched.**
  - Its fixtures (2000 × 116 B ≈ 236 KB) were under the 262,144-byte window, so they passed on main.
  - It consulted the cached turn BEFORE scanning, which is the stale-Done bug.
  - It re-read each chunk to EOF (`read_to_end`), which is quadratic.
  - It mapped `turn_aborted` to `Done`.
  - It was kept as a WIP commit first, not discarded, and squashed away at the end.
- **ONE derivation was confirmed.** The app's `codex_tick` / `codex_label` and MCP `place_status` both call core's `activity::codex_tail`; `lib.rs` reads no rollout itself.

## Dead ends / gotchas

- **The first seam test asserted `6 * 64 KiB > 256 KiB + 64 KiB` as `5 *`**, which is equal, not greater. The fixture's own guard caught it. Size fixtures from the constants AND assert the distance, or a fixture quietly lands inside the window.
- **A green suite was not enough.** The review found that deleting `.or(pt)` kept every test green while bringing back the PR's own symptom: a marker-free growth step read idle mid-turn. A cache-correctness fix needs both directions pinned, the cache must yield (stale Done) and the cache must stand (no marker since). Same lesson as AGENTS.md's watch-signal note.
- **`git checkout -- crates/` to drop pi's diff was refused by the auto-mode classifier** as irreversible. Committing it as WIP was the right move anyway.
- **GitHub refused the first push**: "push declined due to email privacy restrictions". Main's newest commits already used the noreply address. The PR's commits were re-authored as `198289462+penard-monkey@users.noreply.github.com`, without touching git config. David then fixed the setting. This close-out commit uses the default author to check that fix.
- **pi wrote its edits into the (main) checkout as well as this place.** The orchestrator reverted them there before the handoff. A copy is at `~/.cache/worktrees/worktrees/codex-turn-tail/pi-root-edits.patch`.

## Verification

- Fail-first:
  - `codex_tail` was swapped back to `rollout_turn(&lines).or(pt)`. Three tests went red: `None` vs `Busy`; `None` vs `Done{7,"tür-1"}`; `Done{100,"t1"}` vs `Busy`.
  - With the seam carry dropped, the seam test went red.
  - With `.or(pt)` dropped, the marker-free-growth test went red (`None` vs `Busy`).
- Gates:
  - release build: fresh (`-nt` checked)
  - `make test`: 1..463, 0 not ok
  - `make lint`: ok
  - core: 687 passed, 1 ignored
  - cli: 70 passed
  - app `--lib`: 154 passed
- CI: 9/9 on both pushes (runs 37167967623 and 37168583687).
- Review (from (main)): 5 real rollouts were replayed at 173–185 growth points each, and every answer was correct. Main was wrong on all 146/140/133/125/140 prefixes past 256 KiB.
- Not done: watching the running app's nav dot through a real long Codex turn.

## Follow-ups

These are in ROADMAP.
- The quadratic carry on one huge line.
- A rollout replaced by a longer file at the same path.
- pi lanes writing into (main).
