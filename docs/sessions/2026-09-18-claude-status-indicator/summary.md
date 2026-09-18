# Session: say so when Claude itself is down

- **Date:** 2026-09-18
- **Worktree:** `claude-status` (idle base `claude-status-next`)
- **Branch:** `claude-status-indicator` off a freshly fetched `origin/main`
- **PR:** [#216](https://github.com/penard-monkey/worktrees/pull/216) → `cd72ae4` (one commit, squashed)
- **Planning files:** none — the design exploration was a canvas, not `task_plan.md`
- **Scratch:** `~/.cache/worktrees/worktrees/claude-status/` — gate logs, the
  generators for the design mockups, and a copy of the canvas source

## Context

David: "is there a way to check https://status.claude.com/? Like registering
for an event or rss?" — then, once the endpoints were on screen: "let's check
the status and add an indicator (only when down) to the different claude
widgets… I don't know if we should have different indicators or just an
overall issues/down indicator."

`status.claude.com` is an Atlassian Statuspage (page id `tymt9n04zgry`), so it
offers the whole standard toolkit: `history.rss` / `history.atom`, the
`/api/v2/*` JSON family, and email/SMS/Slack/webhook subscriptions from the
page's own modal. `api/v2/status.json` is 212 bytes and `summary.json` ~2KB,
both unauthenticated — cheap enough to poll, which is what shipped.

## What shipped

**Backend — `app/src-tauri/src/lib.rs`**

- `claude_status`, a twin of `claude_usage`: `curl` (no new deps, same
  reasoning as the release check), `STATUS_TTL_SECS` 240, and it never `Err`s —
  a probe whose job is "is Claude broken" must not raise a dialog when the
  network is down.
- `status_parse` is pure, so the mapping is tested against captured bodies
  rather than the live page. `status_rank` / `status_word` collapse Statuspage's
  vocabulary to `none | degraded | down`.
- `stale_or_silence` decides what a FAILED fetch serves: the expired cache
  while it is younger than `STATUS_STALE_MAX_SECS` (1800), silence after.

**Frontend — `app/src/App.tsx`, `App.css`, `mock/install.ts`**

- `useClaudeStatus` mirrors `useUsage` (visibility-gated poll at 300s, focus
  listener, `alive` flag). It returns `null` unless a watched component is
  unwell, which is the single gate every host is written against.
- `StatusDetail` is the reading — components, the incident's latest update, the
  link out — in one rendering with two hosts, the same shape `StatusSheet` uses
  for `StatusBody`.
- `StatusChip` for the line hosts; `.usage-badge` on the rail tile, with
  `StatusDetail` folded into the popover that tile already opens.
- `usePopPos` extracted out of `UsageMeter` so both popovers share one answer
  to "where does the panel go".
- The mock gained `claude_status` with `?status=degraded|down|both|off`,
  operational by default.

## Decisions

**Severity comes from two components, never from the page's own verdict.** The
page carries six; a worktree can feel `Claude Code` and `Claude API`. The
page's most recent incident before this landed was *"Issues with Google Play
subscriptions"* — `status.indicator` would have lit a badge in the nav for
something no worktree can feel. Matched by component ID (`yyzkbfz2thpt`,
`k8w3r06qmzrp`) with the name as fallback, because a rename is exactly the
change nobody here would notice; two tests cover a renamed component and a
re-issued one.

**Where it lands follows `usage_place`.** That setting already answers "where
does Claude's chrome live in this window", so the indicator inherits it rather
than inventing a second placement rule: chip in the strip/footer, corner dot on
the rail tile, and **nothing** on `off` — turning Claude's chrome off is not a
decision an outage may overrule. The strip is the fallback when the named host
is not on screen.

**The rail cannot be the only channel.** It has no Claude-owned icon of its
own; Places and Settings are the only permanent residents, and Settings already
wears the accent `.upd` dot. A rail-only indicator would have to add an icon
mid-incident — a rail that changes length exactly when you are confused about
why Claude stopped answering. Hence the strip fallback.

**The rail badge is always `--danger` red, never amber.** The tile's bars are
already `--warn` at `warning` severity, so an amber dot 4px from amber bars
would be two unrelated meanings on one 32px square. Degraded vs. down is a
distinction the popover makes in words.

**Only dots and tints carry hue; every word is a text token.** See the
measurement below.

**No escalation banner.** An earlier option (C) paired the chip with a
tmux-style banner at `major_outage`. David picked the per-host treatments
without it, so the banner was dropped rather than shipped unused.

## Dead ends / gotchas

**The severity tokens fail as text.** `--warn` on its own 16% tint measures
3.1:1 in tokyo-day and 2.2:1 in catppuccin-latte; `--danger` is 2.2:1 in nord
and 3.2:1 in gruvbox-dark. They are tuned as FILLS. The chip shipped its first
cut with `color: var(--warn)` and looked perfectly fine in tokyo-night (6.2:1),
which is the theme you develop in. Fixed by moving the hue to the dot and the
text to `--txt-hi` (5.4–11.1:1 in all six), and the same for `.status-band-head`
and `.status-row-state`.

The measurement itself has two traps. A translucent background must be
COMPOSITED before you can compute contrast — reading `getComputedStyle`'s
`backgroundColor` straight gives the tint's own colour and every ratio comes
back 1.0, which reads as "no contrast anywhere" rather than "your probe is
wrong". And a resolved `color-mix()` returns `color(srgb 0-1)` while plain
colours return `rgb(0-255)`, so the parser has to handle both (already in
CLAUDE.md from the diff-gutter work; it bit again here).

**Check a new number against the app's OWN tokens before calling it a defect.**
The first contrast sweep also flagged `.status-scope` (2.5–3.6:1) and
`.status-link` (3.1:1 in tokyo-day). Probing `--txt-mute` and `--accent` on
`--bg-panel` directly showed those are app-wide token characteristics — every
`.row-age` and `.usage-pop-head` in the app measures the same. Raising only
this panel would have made it inconsistent with every other one. Left alone.

**Making a panel interactive turned its own dismiss handler against it.**
`.usage-pop` is `pointer-events: none` (it is measured before it is placed), so
the status band's link needed `.usage-pop.pinned { pointer-events: auto }`. But
`UsageMeter`'s outside-click handler excluded only `trigRef` — pointerdown on
the new link unpinned the panel, React unmounted it, and the `click` that would
have opened the URL never landed on anything. A link that does nothing, in the
rail host only. Caught in review, not by the harness, because `.click()` does
not dispatch `pointerdown`; reproducing it needs a real `PointerEvent`.

**`sel` and `selected` are not interchangeable.** The chip's host gated on
`sel`; the footer renders under `selected && sel`, and `selected` is a *lookup*
into the workspace — `null` for the seconds between a restored selection and
the first `list_workspace`, and permanently for a place removed elsewhere. The
chip was assigned to a footer that did not exist and appeared nowhere. The fix
moves the declaration below `selected` (it cannot reference it any earlier).

**A failed fetch must not equal "all clear".** The first cut returned
`unavailable` on any failure, which the frontend renders as silence — so a pull
landing before the network was back (waking from a closed lid, and the
window-focus pull makes that likely) blinked a live incident off screen for up
to 5 minutes. Now it serves the expired answer, BOUNDED at 30 minutes: the
opposite failure is worse, a cached "Claude Code is down" surviving a long
disconnection and still being on screen hours after the outage ended. A body we
cannot PARSE still goes straight to silence — the shape moved, and no later
fetch would correct a stale entry.

**The full-bleed band read as a deliberate inset card.** `.status-band`'s
negative margins have to undo `.usage-pop`'s padding EXACTLY (`--s2 --s3`); the
first cut used `--s1 --s2` and rendered as a plausible box inside a box. Nothing
looked wrong — only `Math.round(bandW) >= Math.round(popW) - 2` said otherwise.

**HMR is dead inside `.worktrees/` (already in CLAUDE.md) and it bit twice.**
After the first CSS edit `curl … | grep` on the served file returned 0 while
the file on disk was correct. Restart with `--force` and content-check the
SERVED file before believing any harness result.

## Verification

- 12 backend unit tests (app crate 46 → 58). Each was shown to FAIL first by
  breaking the thing it guards: unknown component state reading as operational,
  adding claude.ai to the watched set, dropping the incident's
  watched-component filter, letting vanished components report an all-clear,
  taking the oldest incident update instead of the newest, and never serving a
  stale answer. Five went red in one run, the sixth in its own.
- Harness assertions via `getComputedStyle` / `elementFromPoint`, never by eye:
  operational renders nothing (0 chips, 0 badges); the chip is hit-testable and
  `margin-left: auto` in both line hosts; the popover shows both components, the
  incident and a reachable link; Escape closes it through `useEscape`; the rail
  badge is 6×6 at top-4/right-4 — `.rail-icon.upd::after`'s geometry — inside
  the rail with no chip anywhere; `usage_place: off` shows nothing even at
  `major_outage`; rail-with-no-usage-data falls back to a strip carrying only
  the chip; and after the review fix, a real dispatched `pointerdown` on the
  pinned panel's link leaves it open while an outside one still closes it.
- Contrast swept across all six themes, composited (see above).
- Gates: bats 335 ok / 0 not-ok, lint clean, core 315, cli 7, app 58, `tsc
  --noEmit` and `cargo check -p app` clean, no new clippy warnings. CI green on
  all 9 checks across both OSes.

## Follow-ups

- **Not verified in WebKit.** The chip is `display: block` with an inner flex
  span specifically because of the usage meter's WebKit shrink-wrap history
  (CLAUDE.md), but it was measured in Chrome. The headless-WebKit probe that
  found the meter's 201px → 321px gap has not been run against it.
- **`claude_status` has never been invoked against the live page from inside a
  running app** — `status_parse` is tested against captured bodies and the UI
  against the mock. Worth one `sandbox.sh --app` run, ideally during a real
  incident.
- **`under_maintenance` reads as "degraded"** in the chip's four words while the
  row beneath says "under maintenance". Deliberate: a third severity would
  ripple through the tones, chip, band and CSS for a cosmetic gain.
- **No drift check.** The frontend reimplements no core decision here (severity
  is computed in Rust and sent over), so nothing earns a `*-check.mjs` by the
  criterion in CLAUDE.md. The one coupling worth remembering is
  `STATUS_WATCHED` — if Anthropic splits or renames those components, the ID
  match goes stale silently and the name prefix is the only thing left.
