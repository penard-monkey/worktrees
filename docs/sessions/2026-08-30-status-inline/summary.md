# Session: the status check moves into the session-less pane

- **Date:** 2026-08-29 → 2026-08-30 (design, implementation, merge; closed out
  2026-09-01)
- **Worktree:** `ui-tweaks` (idle base `ui-next`)
- **Branch:** `ui-status-inline`, off `c956724` (v0.19.0) — deleted after merge
- **PR:** [#179](https://github.com/penard-monkey/worktrees/pull/179), squash-merged as `ecf7b96`
- **Release:** [v0.19.1](https://github.com/penard-monkey/worktrees/pull/180) —
  cut immediately after, `c73f067`
- **Planning files:** `planning.tar.gz` here — task_plan, findings, progress

## What shipped

One change with three faces, all in the app's frontend. Nothing in Rust moved:
`status_prompt`, `place_health` and `ai_status_report` are untouched, which is
why reads cached before this change still render.

1. **A session-less place opens onto its own status check**
   (`app/src/App.tsx`, the `.term-empty` branch). That pane used to hold one
   line — "No live session for X" — and an Enter button, which is the least
   useful thing to show at the moment you are deciding whether the place is
   still worth your time. It now hosts the whole check under the Enter hero:
   verdict, reasons, facts, commits-not-on-base, Claude's read, and the
   act-on-it row. `place_health` runs on selection (local git, sub-second) and
   seeds from a module-level cache, so a revisit paints the last verdict while
   the new one is fetched behind it.
2. **`StatusBody` extracted from `StatusSheet`** (`app/src/StatusSheet.tsx`).
   Both hosts mount the same component, so the verdict, the facts table and the
   actions cannot drift into two answers to one question. `hideEnter` is the
   only difference either host is allowed — the inline host already has Enter as
   its hero. `StatusSheet` is now a scrim, a slide-over and an Escape key around
   it, and stays exactly as it was for places whose terminal owns the window.
3. **Claude's read renders as markdown** (same file, plus `.hs-read-md` in
   `app/src/App.css`). The backend prompt asks three numbered questions and a
   recommendation, so the answer comes back markdown-shaped — and it was being
   printed into a `<pre>`, asterisks and all. It now goes through `Markdown`,
   the lexer-only renderer, in both hosts. The mock's canned text
   (`app/src/mock/install.ts`) was reshaped to match, because flat prose would
   have exercised only the degenerate one-paragraph case.

The menus follow: **"Status check…" drops out only when the check is already on
screen for that exact place.**

## Decisions (each with the why)

- **`place_health` auto-runs on selection; "Ask Claude" never does.** The health
  check is local git and is over before you have read the title, so selection is
  a good enough gesture for it. The claude spawn costs tokens and 30–90s, so it
  keeps its button — the ⚠ comments in `StatusSheet.tsx` and `lib.rs` both say
  so, and the harness proved it with an IPC spy: zero `ai_status_report` calls
  across a mount plus a 6s idle window.
- **The menu gate is "already on screen for THIS place", not "session-less".**
  The obvious gate is wrong, and the difference is the whole point: right-click
  does not select (`placeCtx`), and the same ctx menu backs the Home briefing's
  resume rows, where there is no inline panel at all. A blanket gate would have
  removed the check at exactly the "should I resume this?" moment it exists for.
  The topbar `⋯` gate *is* blanket, because that call site is always the
  selected place.
- **One `StatusBody` on screen at a time, and the sheet wins.** `statusSheet` is
  tied to neither the selection nor session liveness, so it can be open over the
  inline host (its place lost its session, or it was opened from another place's
  menu). Two mounted bodies mean every `status-*` testid twice, two concurrent
  `place_health` fan-outs and two Ask Claude buttons for one place. Suppressing
  the inline one is the cheap half — it is behind a scrim anyway, remounts from
  `healthCache` when the sheet closes, and an in-flight ask is not lost because
  `ai_status_report` writes the read to the store.
- **Session death under an open sheet is deliberately NOT handled.** Yanking a
  check someone is mid-read is worse than the duplicate it would avoid.
- **A `report: null` deletes the cache entry; a thrown invoke does not.** One
  shape disproves the cache, the other only fails to confirm it. See below.
- **No Rust changes.** The prompt already produces markdown; changing it would
  have invalidated every cached read for no gain.

## Dead ends / gotchas

The load-bearing section. Four of these were caught by review or the harness
*after* the implementation passed every unit gate, and each was measured with a
counterfactual rather than argued.

- **`.update-log`'s recessed background is `--bg-abyss` — and so is `.main`'s.**
  The inline host sits directly on `.main`, so all three boxes the check borrows
  that class for (the error `<pre>`, the commits list, Claude's read) had
  **zero** background contrast, leaving only the 1px `--line` border to define
  them. Measured in every theme: 0 in all six. In tokyo-day that border is ~5
  RGB units off the surface — inside the band this repo has already recorded as
  invisible — so the prose floated with no box at all. The sheet never showed it
  because `.settings-sheet` is a different surface. Fixed with
  `.term-status .update-log { background: var(--bg-panel) }`, which clears the
  surface by 16–32 units everywhere. **`--bg-input` was the tempting wrong
  answer**: it is identical to `--bg-abyss` in catppuccin-mocha and under 7 units
  off in five of six themes. Same family as the sticky-cell rule already in
  CLAUDE.md — composite a recessed box over a token that cannot equal the
  surface it is drawn on.
- **`.term-empty` is shared with the dock's two shell empty-states.** Rewriting
  it into a scroller silently restyled "process exited" and "No shells",
  stretching their cards to full pane height. The base rule is now byte-identical
  to before and the scrolling lives in a `.term-empty-scroll` modifier that only
  the main window carries. Both dock cards measured dead-centred on both axes
  afterwards. Nothing in the diff pointed at the dock — the class name was the
  only evidence.
- **`place-items: center` cannot host a scroller.** A grid item taller than its
  track centres by overflowing **both** edges, which puts the top of the panel
  above the scroll origin where it cannot be reached. Centring has to be
  horizontal only.
- **`align-items: flex-start` is load-bearing, and looks like decoration.** The
  default `stretch` clamps the column to the container's height, so once the
  content overflowed, the column's bottom padding sat at the scroll *origin*
  instead of after the last section: scrolled to the end, "Act on it" finished
  flush against the statusbar. Measured as exactly 32px (`--s6`) missing, and
  0px with the fix. Nothing moves when the content is short, which is why it
  reads as ornamental.
- **A fence inside Claude's read painted larger than the prose around it.**
  `.hs-read-md .md { font-size: var(--fs-meta) }` rescales every `em`-based size
  in the block, but `.md-fence .code` sizes off `--term-size`, an **absolute**
  length, so it does not reach: 13px code inside 11.25px prose — the bigger the
  smaller. The inverse of the `--md-zoom` lesson (that conversion made sizes
  relative to the zoom knob, not to the container), so this host has to say so
  itself. The dock's document view, where 13px code *is* the intent, is scoped
  out.
- **Two `place_health` calls for one place can be in flight.** The mount fetch,
  and the unconditional re-check the Abandon/Archive buttons fire (they are not
  disabled while one runs). The mock resolves in a microtask so they cannot
  cross *there*; the real one is a git fan-out and they can — and the older
  answer landing last would both paint and **seed** pre-lifecycle facts. A `seq`
  ticket per call, only the newest may write. App's `refreshSeq` rule at one
  component's scale.
- **A cache needs to be able to say "I no longer know".** `report: null` is the
  backend affirming the check did *not* run (a guard exit — the directory was
  deleted, the repo moved), which is proof any cached report is wrong. Without a
  delete, every later mount of either host re-paints the disproved verdict and
  blanks it again a fan-out later, forever, on exactly the places whose facts are
  most wrong. The `catch` path deliberately keeps the seed: a thrown invoke is a
  transport failure that says nothing about the report's truth.
- **`checking` starting at `false` cost a wrong-copy frame.** An uncached mount
  took the `rep === null && !checking` branch and painted "No report — the check
  did not run." for one frame — measured at 43ms, replaced at 58ms — before the
  effect flipped it. Behind a menu item that was a blink; the inline host
  cold-mounts front and centre on every first selection of a session-less place.
  Initialising to `true` is honest, because the mount effect refetches
  unconditionally. Under the real git fan-out the bad frame would be *longer*,
  not shorter.
- **The mock's instant resolve hid the whole timing class, as CLAUDE.md warns.**
  Everything above about ordering and races was reasoned from the source and
  confirmed structurally, not observed. The `patchDeclared`-then-`refresh`
  ordering in particular is **not** genuinely exercised by any test here: the
  bug it prevents is a `list_workspace` sweep already in flight when the backend
  writes the store, and the mock's `list_workspace` resolves instantly while its
  `ai_status_report` writes declared state synchronously before resolving.
  Reading the source confirms the order; nothing here distinguishes it from the
  wrong one.
- **A synthetic selection change is the only route to the sheet's close-on-select
  effect.** With the sheet open, `elementFromPoint` on a nav row returns
  `.scrim`, so a real mouse click cannot move the selection that way — it closes
  the sheet instead. The effect is reachable via ⌘K or the sheet's own Enter.
  The logic is verified; the mouse path to it does not exist.
- **The xterm `'dimensions'` console error is still pre-existing.** Reproduced
  here on a clean reload by selecting a session-up place, in `TerminalPane.tsx`,
  which this diff does not touch. Independent corroboration of the roadmap item
  from the #172 session. The entire new code path produced zero console output.
- **`gh pr merge` failed the way CLAUDE.md says it does.** `fatal: 'main' is
  already used by worktree at …` — `gh`'s local checkout step, *after* the merge
  landed. Verified with `gh pr view --json state,mergeCommit` instead of
  retrying. `--delete-branch` died with it, so the remote branch needed a
  separate `git push origin --delete`.

## Verification

- **Full gate suite**, all green: bats 325 ok / 0 not ok, `worktrees-core` 267,
  `worktrees-cli` 7, `app --lib` 45, `make lint`, `tsc --noEmit`,
  `cargo check -p app`, and all six frontend check scripts (`race-check`,
  `ctxmenu`, `termfit`, `termresize`, `zoom`, `dnd`).
- **CI**: all nine checks green on both OSes. `test (macos-latest)` was still
  running at merge time and was confirmed `success` afterwards.
- **Two live-harness passes** (mock, port 1435/1436), asserting with
  `getComputedStyle` / `getBoundingClientRect` / `elementFromPoint` and frame
  sampling rather than screenshots. Highlights: effect keying held under 10
  forced `list_workspace` sweeps with `place_health` pinned; 0 duplicate
  `status-*` testids across 1385 sampled frames; cold mount showed 0
  wrong-copy frames, **falsified** by forcing a reject, which produced 732; the
  contrast, dock-centring, padding and fence-size fixes each measured against
  their own pre-fix counterfactual.
- **Not run: the real app.** See Follow-ups.
- Process: implementation and fixes by opus from a written brief; four review
  lenses (React/state, CSS/layout, repo conventions, spec compliance) over the
  working-tree diff; every finding through three adversarial refuters, majority
  refutation killing it. 19 findings survived into 6 distinct defects, all
  fixed. Two of the six — the dock regression and the invisible box — were
  invisible to every unit gate and to the implementer's own report.

## Follow-ups

- **v0.19.1 shipped without a real-app pass.** `app/scripts/sandbox.sh --app`
  is still owed for this change specifically: select a session-less place and
  watch the panel arrive under a real git fan-out (the mock's microtask hides
  the whole gap), re-select to confirm the cache seed beats a real refetch, and
  run one real Ask Claude so `patchDeclared`-then-`refresh` meets a real
  in-flight sweep. Folded into the existing status-check roadmap item rather
  than added as a second one.
- The commits-not-on-base section was never rendered inside the *inline* host
  during verification — no session-less fixture had commits ahead. It shares the
  `.update-log` class, so the contrast fix covers it by construction, but it has
  not been seen there.
- `place_health` fires twice on mount under React StrictMode's dev double-effect.
  Confirmed as dev-only by inspection, not by building a production bundle.
