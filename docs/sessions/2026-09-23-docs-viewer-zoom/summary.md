---
title: "2026-09-23 — The browser docs page zooms itself"
---

# The document in the browser has its own reading size

- **Date:** 2026-09-21 → 2026-09-23 (closed out 2026-09-26)
- **Worktree:** `.worktrees/ui-tweaks`
- **Branch:** `ui-docs-zoom` (1 commit, squash-merged)
- **PR:** [#330](https://github.com/penard-monkey/worktrees/pull/330), merge `8eaeaf7`
- **Release tag:** none yet. The CHANGELOG entry is under `## [Unreleased]`
- **Planning files:** none. The session kept no `task_plan.md`/`findings.md`/`progress.md`

## What shipped

The docs page in the browser had no zoom of its own, so the only way to make
the text bigger was the browser's own. The reported symptom was Safari zooming
and then snapping back; the reported wish was to use more of the window.

- **`app/viewer/zoom.ts`** (new) — the step table, the chord table, the
  persistence and the `useDocView` hook. Steps are
  `70 80 90 100 110 125 150 175 200 250 300`, two stops past the dock's 200%
  because this surface is a whole browser window rather than a pane.
- **`app/viewer/Viewer.tsx`** — `ViewControls` at module scope (`A− 100% A+`
  plus a `Wide` toggle), rendered into the sticky header's nav row on the
  document route only, and `view` threaded to `DocBody`.
- **`app/viewer/blocks.tsx`** — `--md-zoom` and `data-wide` written onto
  `.doc`, plus `anchorDelta` (below).
- **`app/viewer/viewer.css`** — `.chrome-view` placement, `.nav-path` made
  shrinkable, and `.doc[data-wide="1"] { max-width: none }`.
- **`app/scripts/docsviewer-check.mjs`** — a new section that builds the real
  `zoom.ts` and drives it through the mounted page.

Measured against the running app's own data on a 1672px window:

| | prose column | body text |
|---|---|---|
| before | 645px of a 1393px page | 13.1px |
| 175% | 1100px | 23.0px |
| 175% + `Wide` | 1393px — all of it, no h-scroll | 23.0px |

## Decisions

**The page owns the zoom, not the browser.** Page zoom is wrong here three
times over. It is not ours to bind — `⌘+` is a menu item, and nothing in this
bundle can cause or fix Safari resetting it. It would not survive anyway: the
docs server binds an ephemeral port (`docserver.rs`,
`SocketAddr::from((LOCALHOST, 0))`), so every app launch is a new origin, and
per-site zoom is keyed by origin. And it scales the whole page, which here is
mostly not the document — a 264px nav column and a staleness header that has
nothing to gain from being larger.

**`--md-zoom` already existed and the viewer simply never set it.** App.css's
`.md` block ("ONE knob scales the rendered document") expresses every size
against it: headings in `em`, tables in `--fs-*`, fences off `--term-size`, the
`--md-s*` spacing scale, and the 78ch measure, which tracks font-size. So the
column *widens* as it grows. Nothing new had to be designed; the work was
wiring and the two things below.

**`Wide` is a separate control, not the last zoom step.** Because `ch` is
font-relative, zoom alone can never reach the edge of a wide window — it grows
the measure along with the text. Dropping the cap is the one thing the stepper
cannot express. Offered as an option and chosen explicitly; the alternative
(zoom only) would have left ~300px of gutter at every size.

**Persistence is `localStorage`, deliberately shallow.** Three options were put
up: a cookie (not port-scoped, so it would survive the port changing — but
`docserver.rs` says "never a cookie" about the token and the distinction would
need explaining), a server-side prefs route (durable, but a mutating route on a
deliberately read-only server), and `localStorage`. David chose `localStorage`
knowing the setting resets when the app restarts on a fresh port. That cost is
written into `zoom.ts` and into the CHANGELOG rather than left to be found.

**The step table is NOT imported from `app/src/settings.ts`.** That module's
first line is `import { invoke } from "@tauri-apps/api/core"`, and a browser
bundle with no Tauri under it must not pull the IPC client in to borrow an
array. This is duplication the repo tolerates: a list of stops is a
*preference*, so drift shows up as a button that steps differently, never as a
wrong answer — unlike `dnd.ts::predictTier`, which mirrors a rule in `store.rs`
and therefore needs `dnd-check.mjs`.

**The chord is bound only on the document route.** The handler calls
`preventDefault` to stop the browser zooming underneath it. On the index screen
there is no `.md` block for `--md-zoom` to reach, so binding it there would
take the browser's own zoom away and give nothing back. A chord that is
prevented and then does nothing is worse than an unbound one.

**The controls reuse App.css's `.seg` / `.zoomseg` / `.zoom-val` / `.ctrl.sm`.**
The viewer already loads App.css whole, and the dock's Files pane paints
`A− 100% A+` from those rules for exactly this job. Reuse of the same control,
not a borrowed name — the failure CLAUDE.md records for `.seg` was reusing it
for something that was not a segmented control.

**`??`, never `||`, in the chord table.** `0` is a legal direction (reset) and
falls straight through `||`.

## The scroll anchor needed the other half of the fix

`useBlockScrollAnchor` held the anchored block's **top**. That is exactly right
for a live edit landing underneath you (docs-transport §3.3, the requirement the
whole file exists for) and wrong for a size change, where the block *itself*
rescales: pinning its top pushes the line you were reading down by however far
into the block you already were.

Measured on a real document — a block 951px tall with 702px above the fold —
one 10% step drifted the line **102px**, and 100%→175% would have been ~520px,
most of a screen. `anchorDelta` now preserves the *fraction* above the fold, and
only on a size change (`sizing`), leaving the update path byte-identical.
Re-measured after: **9px** at one step, **85px** over 100%→175%, exactly
reversible. The residue is real and inherent — reflow is not a scale, so line
wrapping redistributes content inside the block — but it holds the reader within
a screen instead of losing them.

## Dead ends / gotchas

**Driving the real bundle needed a proxy, and the reason is worth knowing.** The
running docs server serves the *installed* app's `viewer.js` and there is no way
to swap it under a running app. Fetching `:62347` from a page on another origin
is cross-origin and the docs server sends no CORS headers — correctly. So the
page has to be served from the same origin as its data: a ~40-line Node proxy
(`~/.cache/worktrees/worktrees/ui-tweaks/viewer-proxy.mjs`) serves the local
bundle and the shell and forwards everything else to `:62347` **with the `Host`
header rewritten**, because `host_ok` refuses a request that does not name its
own port. That is the cheapest way to run this bundle against live data.

**`docsviewer-check.mjs`'s `load()` destructures a FIXED binding list.** A name
missing from it is not a missing import anyone would recognise — it is an
undefined free variable at render time, reported as a `ReferenceError` under a
multi-kilobyte base64 `data:` URL stack trace. Cost two round trips. Adding
anything to a module the check loads means adding its exports to the *consumer's*
bindings too.

**The check's JSX recorder does not call function components.** `h` stores
`{tag: ViewControls, props}` without invoking it, so `.chrome-view` never
appears in the recorded tree and `findClass` returns null forever. The probe has
to find the mounted node by `typeof tag === "function"` — which is what proves
the route gate — and then invoke it to read the labels.

**Two guards passed at first and had to be made able to fail.**
- The `??`→`||` mistake is *invisible* in the obvious test: `BY_KEY["0"]` is `0`
  and `BY_CODE["Digit0"]` is also `0`, so both spellings answer correctly. It
  only fails where the tables disagree — `{key: "0", code: ""}`.
- `clampZoom` on the way *in* was never exercised, because the store only ever
  held values the page itself had written. It needed a test that seeds the store
  with `{"zoom":9999,"wide":"yes"}` — which is the real case, since that key is
  the origin's and can be hand-edited.

Both were found by breaking the source and watching for red, not by reading. Six
mistakes were injected in total and each one failed its own assertion.

**A failing new assertion is not always the code.** The first `anchorDelta`
expectation had the wrong *sign* — the implementation was right and the constant
was wrong. Check the arithmetic before the change.

**The Chrome extension cannot send `⌘+` at all** ("Page zoom shortcuts … are not
supported"). So the chord was driven by a dispatched `KeyboardEvent`, which
proves our handler runs and calls `preventDefault` — not that a browser's own
zoom is suppressed by it. That gap is still open (Follow-ups).

**`resize_window` reported success and `innerWidth` never changed.** Rather than
retry it, the narrow layout was exercised by constraining `.page` to the widths
the drawer breakpoint produces (900 → 400px) and measuring the header row
directly. Controls stayed inside the row, never shrank below their content, and
stayed hit-testable at every width; at 400px the row wraps, which is what
`flex-wrap` is there for.

**Main moved 16 commits mid-session** (v0.29.0 shipped), so the branch was cut
from a freshly fetched `origin/main` — and then **#329 landed about a minute
after that**, which put the PR at `DIRTY`/`CONFLICTING` with **zero** CI runs.
That is the documented conflicted-PR signature (`on: pull_request` builds
`refs/pull/N/merge`, which GitHub cannot create while the branch conflicts), not
a broken workflow. Both `[Unreleased]` entries belonged, so the resolution kept
theirs, then mine, then their `### Fixed`.

**`gh pr merge` failed after succeeding, and took `--delete-branch` with it.**
The documented half is that `fatal: 'main' is already used by worktree at …` is
`gh`'s *local* checkout step, after the merge landed. The new half: because it
dies there, `--delete-branch` never runs — the remote branch survives a
successful merge and has to be deleted by hand. Added to CLAUDE.md.

## Verification

- Gates, twice (once on the original base, once after the rebase onto
  `origin/main` + #329): release build, `make test` **361/361**, `make lint`,
  `cargo test -p worktrees-core` **440**, `-p worktrees-cli` **15**,
  `-p app --lib` **137**, `tsc --noEmit`, `cargo check -p app`, and
  `make test-frontend` — all 18 `*-check.mjs`, including
  `viewer-boundary-check` (mermaid stays out of `app/src`).
- CI on #330: nine jobs, ubuntu + macOS, all `success`, watched with
  `gh run watch <run-id> --exit-status` rather than `gh pr checks --watch`.
- The built bundle driven against the **running app's own server data** through
  the proxy above: controls present and hit-tested with `elementFromPoint`;
  A+/A−/reset/Wide measured; the chord table exercised including the composed
  `⌥-` en dash; persistence read back from `localStorage`; the anchor drift
  measured before and after the fix.
- Incidentally observed: the app's docs server was up and serving a live place
  (`app-review`) from David's installed app, which only happens once
  `open_docs_viewer` has run. That is strong evidence for the app-side start
  path in the roadmap item "The docs viewer has never been opened in a real
  browser from a real app", though not a clean observation of WKWebView's
  `openUrl` itself — nobody watched the hand-off.

## Follow-ups

1. **Safari is unverified**, and it is the browser the complaint came from.
   Whether `preventDefault` suppresses a menu-item shortcut there is untested,
   and the original "zooms and then resets" was never explained. If Safari still
   page-zooms, the buttons work regardless and the binding can move to a chord
   Safari does not own. → ROADMAP.
2. **Mermaid diagrams do not scale with the reading size.** `Mermaid.tsx` sets
   no `fontSize`, so mermaid lays the SVG out at its own default in px, and
   `.mermaid-host svg { max-width: 100%; height: auto }` only ever scales it
   *down*. At 175% the prose is 23px and a diagram's labels are still 16px.
   Reasoned from the config and the sheet, **not measured** — the docs server
   was down by close-out. → ROADMAP.
3. **The setting resets when the app restarts**, by design (ephemeral port,
   origin-keyed store). If that grates in daily use, the two ways out are a
   cookie or a prefs route on the server; both were costed during the session
   and are recorded in `zoom.ts`.
