# Session — a space owns its whole workbench, not just its tmux session

- **Date**: 2026-08-11
- **Worktree**: `.worktrees/ui-tweaks`
- **Branches**: `ui-space-scoped-panels`, `ui-space-panel-memory`, `ui-place-title`
  (all squash-merged), `close-out-space-workbench` (this archive)
- **PRs**: [#110](https://github.com/penard-monkey/worktrees/pull/110) space header ·
  [#111](https://github.com/penard-monkey/worktrees/pull/111) per-place panels ·
  [#112](https://github.com/penard-monkey/worktrees/pull/112) place title
- **Release**: unreleased (three `[Unreleased]` CHANGELOG entries)
- **Planning files**: `planning.tar.gz` beside this summary

## The report

> "right now our terminal window isn't really considered part of the space? The
> banner across the top only holds the agent/tmux session. We need to move the
> files and terminal to be part of a 'space/worktree'. We also need each
> space/worktree to remember which state those panels are in open/closed
> files/terminal. Switching spaces/worktree with different combinations is a bit
> jarring when it changes what you left it in. We also need to be able to rename
> the place/worktree in the UI."

The first complaint was **structurally true, not a matter of styling**.
`.topbar` was a child of `<main>`, and the dock was a separate grid column
sitting *beside* it:

```
rail | nav | main[ topbar · note · TerminalPane · statusbar ] | dock | rail
```

So the banner was a header *of the tmux terminal column*, and Files/Terminal sat
outside its scope under a bare `Files`/`Terminal` title. DESIGN.md's layout
picture predates the dock entirely — it was added beside the model rather than
into it, and the UI had been telling the truth about that ever since.

## What shipped

### #110 — the space header crowns the terminal and the dock

`app/src/App.tsx`, `app/src/App.css`, `app/src/settings.ts`

- `.app` grid 5 columns → 4: `rail | nav | .space | rail`. **The dock is no
  longer a grid track.** `.space` is a flex column (TmuxBanner, topbar,
  note-strip, `.space-body`); `.space-body` is a flex row holding `<main>`, the
  dock (`flex: 0 0 {fit.dockW}px`) and the reading overlay.
- A grid-column span **cannot** express this: `gridCols` builds its template with
  `.filter(Boolean)`, so hidden columns are *removed*, not zeroed, and every line
  index shifts when the nav collapses.
- Reading mode got **simpler**: it carried an inline
  `right: dockShown ? -fit.dockW : 0` to escape `main` and reach across the dock,
  and now just fills `.space-body`, which is already that wide → plain `inset: 0`.
- `fitLayout`'s `tight` moved from `mainW` to `mainW + dockW` — the header spans
  both, so a wide dock gives it room the centre pane alone does not have.
- `TmuxBanner` deliberately stays **above** the header: tmux missing is an
  app-level condition, not a per-place one.

### #111 — each space remembers its own dock panels

`app/src/settings.ts`, `app/src/App.tsx` — **frontend only**
(`get_settings`/`set_settings` shuttle `serde_json::Value`, so the backend is
schema-blind: no Rust, no new command, no mock change).

- `place_panels: Record<'repo|slug', PlacePanels>` in `ui-state.json`, copying
  the one existing per-place precedent, `term_tab_names`.
- The flat `dock_*` fields keep their meaning as the **last-used seed**: a place
  with no entry inherits them, so arriving somewhere new looks like where you
  came from rather than snapping to a default — the same complaint in reverse.
- `panelsFor()` merges the two; `eff` is what the UI reads. **9 read sites**
  moved, including two that don't render the dock at all but gate reading mode
  (`filesDockShown`, `keyRef.filesTabOpen` behind ⌘⇧E).
- Pruning: once-per-session sweep + explicit drops on `remove_place` (both call
  sites) and `remove_project`. The sweep is guarded **per project** — a repo
  whose snapshot failed is skipped, because its places are *unknown*, not absent.

### #112 — name a place without renaming the worktree

`crates/worktrees-core/src/store.rs`, `app/src-tauri/src/lib.rs`,
`app/src/App.tsx`, `app/src/App.css`, `app/src/mock/{install,fixtures}.ts`

- `title: Option<String>` on `Declared` + `set_title` command + mock case.
- `nameOf(p) = declared.title?.trim() || p.slug` at the header, nav rows, ⌘K
  rows, Home resume rows and alpha sort; ⌘K's matcher and the nav filter also
  **match** on title.
- Module-scope `TitleEditor` (uncontrolled; Enter commits, Escape cancels).
- The `⋯` menu is no longer gated on `!is_main` — only Remove is.

## Decisions

**The dock becomes a flex sibling, not a grid column.** Offered as a header
spanning two grid columns first; that is unimplementable against a template that
*removes* hidden columns. The flex `.space` wrapper has no line-index math and
gives the reading overlay a container that is already exactly terminal+dock wide.

**Panel memory is three fields, and the globals stay as the seed.**
`dock_open`/`dock_tab`/`dock_width` only. The Files viewer's own preferences
(split, wrap, gitignored) stay global — they are how you like to *read a file*,
not which panel a place has open — and so does the nav, which is how you *leave*
a space. Globals-as-seed is deliberate: a fixed default would make first arrival
jarring in exactly the way the report describes.

**The open file is NOT remembered.** Asked and confirmed with the user. A
remembered path can be deleted, renamed or gitignored between visits, turning
"restore what I left" into an error banner on arrival.

**Rename is a LABEL; identity stays put.** The slug is not a name the tool
stores — `project.rs` derives it as `basename(worktree_dir)` on every read — so
renaming the *place* means renaming the directory, and that is six systems at
once: git worktree registration; the store key (`edit()` has no delete); the
tmux session `{prefix}-{slug}` plus every `~term` sidecar; the recorded
`COMPOSE_PROJECT_NAME` (which wins forever once written); the in-app slug-keyed
maps; and **the Claude history directory, which is keyed on the absolute
worktree path** — a rename silently orphans the conversation and breaks
auto-resume. That last one is disqualifying for something that looks like
editing a caption. The codebase already had the right pattern one layer up: AI
profiles split an immutable `id` from a freely renameable `name` (`profile.rs`).
A true rename wants a `worktrees rename` CLI verb owning all six with a recovery
path → roadmap.

**The slug stays visible next to a title.** The directory, the tmux session and
"Copy path" are all still slug-derived; hiding it would make them look wrong.

## Dead ends / gotchas

**⚠ A stale `target/release` binary made a whole gate pass meaningless.** The
release binary was built during bootstrap at 15:37; `store.rs` was edited at
16:47. `make test` and the `ls --json`-vs-shipped diff both read that binary, so
"288 bats passing" and "identical to shipped" **validated pre-change code**.
CLAUDE.md warns a stale binary makes bats *fail* mysteriously; here it did
something worse — it passed. Rebuild, then check `binary -nt source` before
believing either result.

**⚠ `cargo test … | tail -4` reads as an empty suite.** It printed
`running 0 tests / ok. 0 passed`, which is the **Doc-tests** block; the real
`running 207 tests` was above the cut. Report cargo with
`grep -E "^running|^test result"`, never a tail.

**⚠ `grep -c` exits non-zero on a count of 0**, which silently broke an `&&`
chain and skipped the bats re-run entirely. A gate that never ran looks exactly
like a gate with no output.

**⚠ Guarding the obvious instance of a hazard is not guarding the hazard.**
The one real bug this session (caught in review, see below): `updatePanels`
stashed the full `{dock_open, dock_tab, dock_width}` triple into `preHydration`.
Pre-hydration `settings` is `DEFAULTS`, so a ⌘J that meant to toggle `dock_open`
was widened with default `dock_tab`/`dock_width`; hydration shallow-merges
`preHydration` over the settings read from disk **and then saves**, so the user's
real values were overwritten and persisted. The comment on that very line
correctly explained why `place_panels` must stay out of `preHydration` — the
same shallow-merge reasoning — and then the code below it committed the
identical error one level down with the scalars. Fix: stash the resolved patch
`p`, not the widened `triple`. The two invariants are exact opposites and both
are load-bearing: `place_panels` needs the FULL triple (a partial entry falls
through to the globals, which is how another place's tab bleeds in), while
`preHydration` needs the NARROW patch.

**Two console error kinds in the mock harness are pre-existing**, and proving it
required a baseline harness rather than reasoning: `dimensions` (xterm
`Viewport.syncScrollArea`) and `unregisterListener` (the Tauri event shim). Both
reproduce identically on a content-checked pre-change harness running the same
click sequence (18 and 12 occurrences). Neither is from this session's work.

**A CI failure that was not a test failure.** #110's `test (macos-latest)` job
failed in `actions/checkout@v7` — runner infrastructure. Re-running the failed
job alone turned it green. Worth reading the failing *step* before treating a
red check as a code problem.

**Landmark semantics: the intuitive read was backwards.** A review finding
claimed moving `<header>` out of `<main>` orphaned content from landmarks. The
opposite: inside `<main>` a `<header>` maps to `sectionheader`, so there was **no
banner landmark to lose**; outside it maps to `banner`, a net gain. Established
by a verifier building a probe page and reading Chromium's AX tree over CDP, not
by citing the spec. A planned `.space-header` wrapper was dropped as a result.

## Verification

Every layout claim was measured with `getBoundingClientRect` /
`getComputedStyle` against the mock harness, never eyeballed — and every harness
was **content-checked** (`curl` the served file and grep for the edit) because
HMR is dead inside `.worktrees/`.

| Claim | Evidence |
| --- | --- |
| header spans terminal + dock, all 4 nav×dock combos | 1082 = 722+360; 1382 = 1382; right edges all 1426; no h-overflow |
| reading overlay covers the body incl. dock, header above it | 344→1426; topbar 0–50, note 50–83, overlay 83–815 |
| no cross-place bleed | A=Files → B=Terminal → back to A stays **Files** |
| per-place width/closed survive a round trip | Terminal @490px returns 490px; a closed dock stays closed |
| unvisited place inherits the seed | opens as Terminal after B set Terminal |
| **rename editor survives the 3s poll** | same DOM node, focus, value and caret after 7s (>2 cycles) |
| ⌘K / nav filter find a renamed place | by title **and** by slug |
| `(main)` can be named, Remove still absent there | "◆ Monorepo root" + `(main)` alias |
| `ls --json` unchanged | byte-identical to `~/.local/bin/worktrees`, **after** a release rebuild |

Gates (final, on a rebuilt binary): `tsc --noEmit` · `cargo check -p app` ·
`make test` 288 bats / 0 failing · `make lint` · `cargo test -p worktrees-core`
**208** (the new `title` test) · `cargo test -p worktrees-cli` 6.

**Both new tests were checked against the unfixed code first**, so neither is a
test that passes either way:

- `title_round_trips_beside_note_and_unknown_keys` — removing
  `skip_serializing_if` makes it panic on "cleared title must not be serialized".
- the pre-hydration fix — `stash=triple` clobbers `terminal/490` to `files/360`,
  `stash=p` preserves it.

This matters because ROADMAP records a zombie-children regression test that
passed identically with and without its fix.

### Review record

Each PR got a four-lens adversarial review (find → independently refute):

| PR | filed | survived | outcome |
| --- | --- | --- | --- |
| #110 | 5 | **0** | CSS + React lenses found nothing; a11y findings refuted by AX-tree measurement |
| #111 | 6 | **1** | the `preHydration` bug above — found by 3 lenses, reproduced by 2 verifiers |
| #112 | 14 | **0** | mostly pre-existing CSS, disproved by measurement (a 300-char string overflows ⌘K identically whether it is a slug or a title) |

A useful confirmation from #112's review: a scratch crate proved an older binary
parks `title` in its flattened `extra` and re-emits it verbatim, so the
no-version-bump claim holds in **both** directions.

## Follow-ups

See `ROADMAP.md` — the xterm/unlisten harness noise, the true `worktrees rename`
verb, and the deferred `--nav-w` retire.
