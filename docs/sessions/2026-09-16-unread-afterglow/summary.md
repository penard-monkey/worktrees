# Session: the afterglow dot waits to be seen (unread completions)

- **Date:** 2026-09-16
- **Worktree:** `ui-changes` (idle base `ui-changes-next`)
- **Branch:** `ui-changes-unread-afterglow` off a freshly fetched `origin/main`, rebased twice as main moved, deleted after merge
- **PR:** [#211](https://github.com/penard-monkey/worktrees/pull/211) → `9614a57`
- **Release:** unreleased at close-out (sits in `[Unreleased]`)
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress
- **Cost split:** fable researched and wrote the brief; an opus agent implemented and ran gates; fable reviewed the diff, rebased, re-ran gates, merged

## Context

David: "is there a way to mark 'unread' messages? Basically when the worktree
hasn't been clicked and the AI has responded. It may make sense that the pulse
in the purple doesn't start following the rule of fading away until it has been
opened the first time?" After the options were laid out he chose a **bigger dot
with a second ring** for the unread state, and rejected bolding the row name
("I don't know if bold will be clear enough").

## What shipped

- **`crates/worktrees-core/src/store.rs`**: `Declared.last_seen_epoch`
  (optional, `skip_serializing_if`), with a round-trip test that also proves
  an absent stamp is never serialised as `null`.
- **`app/src-tauri/src/lib.rs`**: `mark_seen(repo, slug, epoch)` — forward-only,
  read-before-write, touches nothing else. Registered beside `touch_place`.
- **`app/src/afterglow.ts`**: `seenEpoch(seen, opened)` (max, so
  `last_opened_epoch` is the upgrade fallback), `isUnread(worked, seen)`
  (`worked > 0 && worked > seen`; equality is SEEN so the ack is idempotent),
  `doneTierSeen` (unread ⇒ tier 1 past every bound, else `doneTier`).
- **`app/src/App.tsx`**: `seenPaths` optimistic map (same shape as
  `donePaths`), `seenAt`, `unreadOf` (gated on `activityOf` like `doneOf`),
  `ack` (stamps now locally, fire-and-forgets `mark_seen`), `doneOf` →
  `doneTierSeen`, `.unread` on `dotClass`, "— not seen yet" on `dotTitle` and
  the folder rollup title, `SEEN_DWELL_MS = 1000` effect keyed on primitives,
  and an ack with no dwell inside `enterPlace`.
- **`app/src/App.css`**: `.status-dot { position: relative }`,
  `.status-dot.done.unread` (10px dot, `margin: -1px` so the margin box, centre
  and 14px column stay put) and its `::after` ring at `inset: -6px`.
- **`app/src/SettingsSheet.tsx`**: one sentence in the Afterglow hint.
- **`app/src/mock/install.ts`**, **`fixtures.ts`**: `mark_seen` case;
  `search-index` is the UNREAD demo, new `perf-budget` is its SEEN twin; three
  other worked fixtures gained a seen stamp so their tiers keep meaning t2/t2/t3.
- **`app/scripts/afterglow-check.mjs`**: the unread block plus two static
  mirrors (the CSS class exists; App.tsx calls `isUnread` and carries no
  `worked > seen` of its own).
- **`CHANGELOG.md`** `[Unreleased]` → Added.

## Decisions

- **Unread holds tier 1 and outlives the horizon.** Mail semantics: the signal
  is spent by being read, not by time. Seeing a three-hour-old finish drops it
  to its real tier — no clock restart — so "how long ago" stays honest.
- **Geometry, not hue or motion.** Busy and waiting own the other two colours
  in the slot, and reduced-motion freezes any animation at frame 0. Tier 1 is
  already full brightness with the halo, so the only room left was size and a
  second ring.
- **Seen = selected in a VISIBLE window for a 1s dwell, or entered.** The
  dwell stops arrow-keying through the nav from acking every row; visibility
  stops a place left selected behind a hidden window from counting. A finish
  that lands while the place is selected acks itself the same way — the "I
  watched it finish" case.
- **`opened < worked ⇒ unread` was rejected.** The common case is sitting IN
  the place while Claude finishes: the open predates the completion even
  though the user watched it. The ack has to be about presentation, not entry.
- **The stamp lives in the declared store, not `ui-state.json`.** Survives
  restarts and the startup backfill, mirrors `last_opened_epoch`, and the
  frontend supplies the epoch so the optimistic map and the file agree.
- **`last_opened_epoch` is the fallback when the seen stamp is absent**, so the
  upgrade lights only places worked after the last entry, once, instead of
  every place ever worked in.
- **Ring is a `::after`, not a second box-shadow.** A shadow cannot leave a gap
  without painting the row background, and that differs on hover and `.sel`.
- **Effect deps are primitives** (`sel?.repo`, `sel?.slug`, `pageVisible`,
  `selUnread`), not `selected`: that object is rebuilt on every
  `list_workspace` sweep, and an identity-keyed dwell would restart on each and
  never fire on a workspace that refreshes faster than 1s. Agent's call, agreed
  in review.

## Dead ends / gotchas

- **`gh pr merge` refuses a PR whose CI is still pending** (`Pull Request is
  not mergeable`, `mergeStateStatus: UNSTABLE`) even with no required checks.
  Right after a force-push, mergeability also reads `UNKNOWN` for a few
  seconds. Watch the run (`gh run watch <id> --exit-status`), then merge.
- **The merge then "failed" with `'main' is already used by worktree`** — the
  known post-merge local checkout step; `gh pr view --json state,mergeCommit`
  showed MERGED. Do not retry.
- **Main moved twice during the session** (#208/#209 while the agent worked,
  #205 before the merge). The only conflict was CHANGELOG ordering under
  `### Added`; kept both, ours first.
- **The agent rewrote a plan row** (phase 7) into its own wording while
  updating `task_plan.md`; fable's later `sed` on the original text silently
  matched nothing. Check the row after an agent has touched the plan.
- **Fixtures had to change beyond the brief**: three existing fixtures had
  `last_worked_epoch` newer than `last_opened_epoch`, so the new rule would
  have pinned them all to tier 1 and destroyed the t2/t2/t3 the harness and
  the check script's default-curve comment rely on. Any new "seen" input
  re-asks every fixture whether its tier still means what it did.

## Verification

- `afterglow-check.mjs` shown RED first: flipped `isUnread` → 23 failures;
  renamed CSS class + inlined comparison → 3; dropped `skip_serializing_if` →
  the Rust test fails. All restored → green.
- Mock harness (chromium via playwright, vite on :1477 as a real background
  task, served CSS grepped for the new class first): unread and seen twin rows
  both 43px; dot column 14px on both; dot centres identical (44px from row
  left); `.nav-scroll` scrollWidth 299 === clientWidth 299; ring 22×22, 2px
  clear gap from the halo, clipped by nothing; ring contrast Δmax 67.8–130.2
  RGB units against both `--bg-tree` and `--bg-elev` in all six themes.
  Ack: 300ms select-and-move-on → 0 `mark_seen`; 1.5s dwell → exactly one;
  two already-read places for 1.5s each → 0 more. Rollup title reached
  "a session finished — not seen yet".
- Gates on the final rebased tree: bats 335 ok / 0 not ok, lint clean,
  core 313, cli 7, app lib 48, tsc + `cargo check -p app` clean, afterglow /
  zoom / race / dnd / remote / ctxmenu checks green. CI: 9/9 jobs passed
  before the squash-merge.

## Follow-ups

- **The live busy → done → seen hand-off has not run in the real app.** No
  fake claude; the mock resolves in a microtask. Run a task in a place you are
  not looking at, confirm the ringed dot, select the row for a second, confirm
  the plain halo, and check `last_seen_epoch` landed in that repo's
  `.worktrees.places.json`.
- Reading a session from a separate terminal attached to the same tmux is
  invisible to the app; the place stays unread until its row is selected here.
  Accepted; `tmux list-clients` polling was judged overkill.
- Not built, by scope: "mark as unread" / "mark all read" gestures, a Dock
  badge count of unread places, a macOS notification when the window is
  hidden, and a CLI / `place_status` surface for the seen stamp.
