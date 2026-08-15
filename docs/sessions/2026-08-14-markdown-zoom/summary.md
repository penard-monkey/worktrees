---
title: Markdown docs read at whatever size you need
---

# Markdown docs read at whatever size you need

- **Date:** 2026-08-14
- **Worktree:** `.worktrees/ui-changes`
- **Branches:** `ui-changes-md-zoom` → merged; archive on
  `ui-changes-close-out-md-zoom`; tree parked on `ui-changes-next`
- **PRs:** [#129](https://github.com/penard-monkey/worktrees/pull/129)
  `feat(app): markdown docs read at whatever size you need` (squashed as
  `6333633`)
- **Release tag:** none — sits in `[Unreleased]` after v0.13.0
- **Planning files:** `planning.tar.gz` beside this summary
- **Screenshot:** `reader-125.png` — the reader (⌘⇧E) at 125%

## What shipped

The Files viewer rendered markdown at one fixed size. The only nearby knob was
Settings → UI font size, which scales the **whole app** — enlarging a README
moved every column boundary with it, so nobody used it for reading.

The viewer header now carries a stepper — `A− 100% A+` — whenever a document is
being READ, plus ⌘+ / ⌘− / ⌘0 from the keyboard, in the dock and in the reader
(⌘⇧E) alike. The readout doubles as the reset. Steps are 70 / 80 / 90 / 100 /
110 / 125 / 150 / 175 / 200 (`MD_ZOOM_STEPS`), widening as they climb because
the difference between 175 and 180 is invisible and the one between 90 and 100
is not.

Source view has no stepper and ignores the chord: that is code, and it keeps
following the terminal font like every other source file in the viewer.

### One multiplier, and a rem→em sweep (`app/src/App.css`)

`.md` defines `--md-z: var(--md-zoom, 1)` plus `--md-s1..s6` (the `--s*` spacing
tokens times the zoom). The knob itself is written **inline on the `.scroll`
box** by `FilesPane.tsx`, and read through a `, 1` fallback so an unset value
simply means "normal".

A bare `font-size` on `.md` would have scaled the paragraphs and left everything
else behind, because nothing inside `.md` was expressed against `.md`:

| was | why it would not have moved |
| --- | --- |
| `.md-h1/h2/h3` in `rem` | root-relative — immune to a `font-size` on `.md` |
| `.md-table`, `.md-fence-lang`, `.md-rawhtml-block` via `--fs-*` | same |
| `.md-fence .code` via `--term-size` | px |
| `--s1..s6` margins/padding | px — fixed gaps around 200% text read as no gap |

Headings became exact em ratios of their old rem values (1.714 / 1.371 / 1.2 em
= the old 1.5 / 1.2 / 1.05 rem against `--fs-body: 0.875rem`), so **100% renders
as before to within 0.01px** — measured, not assumed.

### Per place, not global (`app/src/settings.ts`, `app/src/App.tsx`)

The size lives in the existing `place_panels[repo|slug]` record — a repo of wide
reference tables wants small, the one you are writing docs in wants large.

`files_md_zoom` is the **only optional field** in `PlacePanels`, and that is
load-bearing: absent has to keep meaning "still inheriting". `panelsFor` returns
`p.files_md_zoom ?? s.files_md_zoom`, so a place with no size of its own tracks
the flat global (which keeps meaning "the last size you actually chose"), and an
entry written by an older build inherits instead of reading as "no size".

Unlike `dock_open`, the zoom **does** seed from the global. `dock_open` stopped
seeding because arriving somewhere new should not find the furniture rearranged;
an inherited reading size changes nothing until you are actually reading, so the
same objection does not apply.

Files touched: `app/src/settings.ts` (`files_md_zoom`, `MD_ZOOM_STEPS`,
`clampMdZoom`, `stepMdZoom`, `PlacePanels`, `panelsFor`), `app/src/App.css`
(`.md` block + `.zoomseg`), `app/src/FilesPane.tsx` (props, header control,
inline `--md-zoom`), `app/src/App.tsx` (`ZOOM_KEYS`, the chord, `updatePanels`,
both `FileView` mounts), `CHANGELOG.md`.

## Decisions

- **Markdown preview only** — not the source view, and not a second zoom for it.
  Source is code; it already has a size (`--term-size`, Settings → Terminal).
  A second stepper would have made the header state ambiguous.
- **Discrete steps, not a slider or a free number.** A reading size is chosen by
  pressing a key until it looks right; 1%-at-a-time is a dozen presses, and a
  slider needs a surface the header does not have.
- **The percentage readout IS the reset button.** A percentage you cannot click
  back to 100 leaves counting steps as the only way home.
- **Per place, seeded globally** — chosen over per-file and over per-file-on-top-
  of-per-place. Per-file answers "some files need different sizes" more literally
  but is an unbounded map keyed on paths that outlive places, needs its own
  pruning rule, and reopens a doc at a size with no visible cause. Per-place
  reuses `place_panels`, whose seeding and pruning (`dropPanels` + the
  once-per-session dead-place sweep) already exist.
- **⌘+ / ⌘− / ⌘0 handled ABOVE the meta-only gate** in the keydown handler: on a
  US layout ⌘+ arrives as ⌘⇧= (`key === "+"`), and that gate drops every shifted
  chord. Key repeat is allowed here, unlike the toggles — holding the key to walk
  up the steps is the point.
- **`files_md_zoom` belongs in `place_panels`; `term_tab_active` does not.**
  Every key in that record must also exist as a global, which doubles as the
  seed. A reading size transfers meaningfully between places; tab 3 in one
  worktree says nothing about the next.

## Dead ends / gotchas

- **A per-place field with a global twin freezes the seed unless it is
  optional.** The first implementation wrote `files_md_zoom: p.files_md_zoom ??
  cur.files_md_zoom` in `updatePanels`, mirroring the other three fields. But
  `cur` comes from `panelsFor`, which falls back to the global — so ANY panels
  write (⌘J, a divider drag, a tab switch) pinned whatever the seed happened to
  be into a place where no size had ever been chosen, and then, because the
  record is spread back over the globals, handed that stale number to the next
  new place. The other three fields are safe from this only because each is
  written by an act you can SEE. Caught in review, not by any gate: every test
  passed, because the value written was always *correct at that moment*.
- **`width: 1em` does not scale a native checkbox.** Form controls do not
  inherit font, so `.md-check`'s `1em` resolved against WebKit's own 13.33px
  control font — measured 13.33px beside 150% prose, and the pre-existing
  `top: 0.35em` had been silently ignoring the zoom for the same reason.
  `font-size: inherit` is what makes an em inside a form control mean the
  document's em. The first fix looked right in the CSS and did nothing.
- **A stash-pop across a release puts your CHANGELOG entry in the WRONG
  section.** `origin/main` had moved four commits (v0.13.0 shipped) while this
  work sat uncommitted. `git stash` → branch off `origin/main` → `git stash pop`
  auto-merged cleanly — and dropped an `[Unreleased]` entry INSIDE the released
  `## [0.13.0]` section, where it reads as "this shipped in 0.13.0". Nothing
  flags it; the merge was clean. After any rebase across a release, look at
  which heading your entry ended up under.
- **Two vite instances held port 5199, and the survivor served the STASHED
  file.** CLAUDE.md's rule (`lsof -ti:PORT -sTCP:LISTEN`, then grep the served
  file) earned itself again with a new twist: the survivor had started during
  the stash window, so it was serving `FilesPane.tsx` *without the feature* —
  `curl … | grep -c zoomseg` returned 0 while the same grep on disk returned 1.
  Read as "the change vanished", was "a zombie is serving the pre-work file".
- **`.md-fence .code-text` kept its 8/12px inset** through the whole first pass:
  the sweep covered every `font-size` and every margin, and missed the padding
  belonging to a shared class (`.code-text`) that the fence merely borrows.
  200% code in a 100% box.
- **Three `.click()`s in one `browser_evaluate` advanced the stepper by one
  step** — the documented React-batching rule, confirmed again. Reads exactly
  like a control that ignores clicks.

## Verification

Mock harness (`VITE_MOCK=1 vite --port 5199 --force`), Playwright, every claim
via `getComputedStyle` — and the served file grepped for the edit before each
run, per the zombie above.

| measured | 100% | 110% | ratio |
| --- | --- | --- | --- |
| `.md` font-size | 13.125px | 14.4375px | 1.10 |
| `.md-h1` | 22.496px (was 22.5 = 1.5rem) | 24.746px | 1.10 |
| `.md-h2` | 17.994px (was 18) | 19.794px | 1.10 |
| `.md .code` (fence) | 13px (= `--term-size`) | 14.3px | 1.10 |
| `.md-table` | 12.193px (was 12.1875) | 13.412px | 1.10 |
| `.md` padding | 16/24/32px | 17.6/26.4/35.2px | 1.10 |
| `.reading .md` padding | 24/24/32px | 26.4/26.4/35.2px | 1.10 |

At 150%: fence inset 12/18px (was fixed 8/12), checkbox 19.6875px = 1em of
prose (was 13.33px), `top` offset 6.89px.

Behaviour: ⌘+ → 125%, ⌘− → 110%, ⌘0 → 100% and the readout disables; Source view
shows no stepper and `.code` stays 13px through a ⌘+; the reader overlay carries
the same control; `get_settings` round-trips the value.

Per-place, the exact scenario the review predicted: opening the dock in a place
with no size of its own writes **no** `files_md_zoom`; setting 150% elsewhere
leaves that place reading 150% live; a ⌘J there neither pins it nor moves the
global. Place A 125% → B inherits 125 → B set to 90 → A still 125.
`panelsFor` on a legacy 3-field entry inherits the global;
`clampMdZoom(133 / NaN / 999)` → 125 / 100 / 200.

Gates green on the merged tree: `make test` (288 ok, 0 not ok), `make lint`,
core 208, cli 6, `cargo test -p app --lib` 29, `tsc --noEmit`,
`cargo check -p app`. CI 9/9 on the merged SHA.

Review: fable on the full diff before merge — three findings, all fixed in
`0093d7d` (the `updatePanels` seed freeze, the fence inset, the checkbox). It
also settled the WKWebView question statically: no native menu and no
`zoomHotkeysEnabled` in `tauri.conf.json`, so ⌘+/⌘− reach JS in a real build.

## Follow-ups

- **Nobody has pressed the keys in a built app.** The harness is Chromium and
  the events were synthetic `KeyboardEvent`s. Static evidence says the chord
  reaches JS; 30 seconds inside `app/scripts/sandbox.sh --app` would close it.
- Disabled stepper buttons do not show their `title`, so the ⌘−/⌘+ hints vanish
  at the ends of the range.
- The zoom chord fires behind the What's-new scrim and the ProjectSheet (only
  `switchOpen`/`settingsOpen` are guarded) — the existing behaviour of every
  chord in that handler, not a regression, and worth fixing for all of them at
  once if it ever bites.
