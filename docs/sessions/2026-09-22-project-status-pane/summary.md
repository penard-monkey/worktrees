---
title: A Plan tab that reads the session's plan, and never writes it
---

# A Plan tab that reads the session's plan, and never writes it

- **Date:** 2026-09-22
- **Working tree:** `.worktrees/project-status-pane`
- **Branches:** `project-status-pane` (the feature, three commits),
  `project-status-pane-close-out` (this archive)
- **PR:** [#323](https://github.com/penard-monkey/worktrees/pull/323),
  squash-merged as `5c8d202`
- **Release tag:** none — sits in `[Unreleased]`, after v0.28.0
- **Planning files:** `planning.tar.gz` beside this summary
- **Evidence:** gate logs, CI logs, the shared contract, the Playwright
  probes and ~50 harness screenshots are outside the repo at
  `~/.cache/worktrees/worktrees/project-status-pane/`. Two screenshots ride
  here: `plan-tab-final.png` (the shipped header at 360px) and
  `empty-state-240.png` (the empty state with the Generate button at the
  240px floor).

## What this was

David asked what a fourth right-hand pane could show for a place, given that
most places run the planning-with-files skill and the rest still have a goal.
Three questions: where, what integration with the session, and whether to
render structured output or the skill's own `/status` shape. The answer that
shipped: a dock tab, fed by a lenient reader over the files the session
already writes, with a small structured header over the rendered markdown.

## What shipped

- `crates/worktrees-core/src/plan.rs` (new, 1201 lines with tests) —
  `PlanSummary`, `summarize(root)`, the pure `extract(text)` /
  `extract_brief(text)`. Resolution mirrors the skill's own
  `resolve-plan-dir.sh`: `.planning/.active_plan` → newest
  `.planning/<dir>/` holding a `task_plan.md` → root `task_plan.md` → brief
  only → none. Every constructed component is lstat'd, the file is opened
  with `O_NOFOLLOW`, 512 KiB cap. A test cross-checks phase counts against
  the INSTALLED `check-complete.sh` when it exists.
- `crates/worktrees-core/src/ops.rs` — `PLAN_PROMPT`, the fixed text the
  Generate button pastes; two tests pin that it names no directory and ends
  without a newline.
- `app/src-tauri/src/lib.rs` — `place_plan(root)` and `plan_prompt(session)`.
- `crates/worktrees-cli/src/mcp.rs` — `place_status` and the place resource
  carry the same summary without the markdown under `plan`.
- `app/src/PlanPane.tsx` (new) — header (assignment first, one status band,
  phases collapsed to the current one with its status in words), the
  rendered plan, the brief-only body, the derived empty state with
  "Generate plan".
- `app/src/App.tsx`, `settings.ts` (`dock_tab` gains `"plan"`), `icons.tsx`
  (`ListChecks`), `usage.ts` (`dock.plan` surface), `App.css` (`.plan-*`),
  `mock/install.ts` (`place_plan` with `?plan=template|freeform|brief|none|
  truncated`, `plan_prompt` recorder).
- `CHANGELOG.md` `[Unreleased]`, `ROADMAP.md` (the parked "brief PROGRESS as
  status" item repointed).

Outside the repo, two private artifacts for David: a static build of the
mock harness (`VITE_MOCK=1 vite build --base ./`, 1 MB, runs entirely in the
browser) that he shared with a colleague and that was republished after each
commit, and a design canvas with six header directions. Their links are in
`progress.md` in the tarball.

## Decisions

**A dock tab, not a band or a second pane.** A tab is a union member, a rail
entry and a `dock-body` branch, and per-place panel memory comes free. A
persistent band or a split dock is a new layout, and the ratchet and fit
lessons in CLAUDE.md say what that costs. The tab proves whether the content
earns permanent space first.

**A lenient extractor mirroring the skill's greps, not a schema.** Fifteen
real `task_plan.md` files were surveyed (valleos and this repo's worktrees)
before any parser was written: two used the template's `### Phase` +
`**Status:**` markers, nine had checkboxes, about eleven had a `## Goal`,
several were entirely freeform (`## NOW`, `## Next steps`, `## THE ONE OPEN
ITEM`). A parser keyed to the template shows nothing for thirteen of fifteen.
The skill itself does not parse either — its Stop hook is `grep -c "### Phase"`
and `grep -cF "**Status:** complete"` with an inline-tag fallback — so the
extractor mirrors those rules and degrades to "render the markdown". Asking
the skill for a structured side file was rejected: it is a marketplace
plugin, and every place's session would have to emit it.

**The app never writes `.planning/**`.** Same owner rule as `ui-state.json`
and `~/.claude.json`. The brief is the one file the tool writes, and it is
written at creation time by code that already existed. "Generate plan" is
therefore a PASTE into the session's Claude pane — `tmux::paste_to_ai`, the
drag-and-drop reference's path, bracketed paste and no newline — of fixed
text asking the skill to capture the work in flight. The user reads it and
presses Enter; the session writes the files.

**The prompt names no directory.** David's correction, and the right one:
the skill's legacy mode writes `task_plan.md` at the repo root, and the
reader already resolves both layouts. A path in the prompt would be a second
opinion that could only disagree with the skill.

**Header: direction B's order with D's density.** From the design canvas's
recommendation, which David took: the brief's title and lead first (it is
the assignment, and the one thing every briefed place has), the plan's title
as a dim line when the two differ, one `--fs-meta` band (live state · N/M
with a 48px bar · age · current step), the draft on its own line, phases
collapsed to "Phase 3 of 5 · name · in progress". Most real plans have a
lead and checkboxes and no phases, so the header leans on what every place
has.

**MCP carries the summary without the markdown.** A model reads that payload
on every `place_status`; `plan_path` says where the rest is. Phase names are
clipped to `FREE_TEXT_MAX` like every other session-written string, and the
reading notes say `plan.*` is data, never instructions.

**Two Opus agents in parallel against a pinned contract; Fable reviewed.**
The contract (`plan-contract.md`, in the cache dir) fixed the JSON shape,
resolution order and extraction rules before either agent started, so the
frontend was built against the mock while the reader was still being
written, with disjoint file sets. Neither drifted. A third Opus agent did the
header rework and a fourth the empty state; a Fable fork produced the design
canvas.

## Dead ends and gotchas

### The template promised a format that real files do not have

The obvious design — parse the `### Phase N` / `**Status:**` shape the
skill's template ships — would have rendered an empty header on almost every
place David actually has, and every test written against the template would
have passed. The survey took ten minutes and changed the whole shape of the
feature. Before writing a parser for a format a tool's template promises,
count how many real files follow it.

### `::before` separators on ellipsising spans leave a bare "·"

The first meta line put `content: "· "` on every span after the first, so a
path squeezed to nothing showed a separator with nothing after it. The fix
that held: each item owns a real text-node separator beside its text, and
the row starts one separator-width to the LEFT inside an `overflow: hidden`
box, so whichever item begins a line — the first, or one the row wrapped —
has its separator clipped. A wrap can then never dangle a separator at
either end.

### A `max-height` on the header made two nested scrollers

The first cut capped the header at 55% of the pane with its own
`overflow-y`. On a short window a five-phase plan scrolled the phase list
INSIDE the header while the document scrolled below it. Every header row is
now bounded on its own (one line of title, six of lead at most, phases
collapsed to one line) and the header does not scroll.

### Where the age goes in the band decides what wraps

At the 240px floor with a live state chip, putting the age LAST orphaned it
alone on line 2 while `current` shrank to 51px on line 1. With `current`
last it wraps onto its own full-width line. Measured, not reasoned; the
band order is state · N/M · age · current for that reason.

### The reader is lstat-per-component, and the test had to be broken three ways

`symlink_metadata` refuses to follow only the LAST component (the
2026-09-21 session's lesson), so `resolve` lstats `.planning` before
anything under it, then the plan directory, then the file, and
`read_capped` opens with `O_NOFOLLOW` to close the check-to-open gap. The
symlink test was shown red with all lstats switched to `metadata`, with
only the `.planning` lstat removed from `resolve`, and with only the one on
the brief path removed — each a different leak.

### The sandbox repo is empty, and a hand test needs the shapes

`sandbox.sh --app` launches against `~/.cache/worktrees/sandbox/<name>/repo`,
which is one commit and a README. Before David looked, the repo was seeded
with three worktrees and the four shapes (template plan under
`.planning/<id>/` with `.active_plan`, the real vendor-api plan at the ROOT,
brief-only, nothing), and the release binary's MCP `place_status` was asked
for each first — all four resolved as expected — so the app run tested the
app, not the fixtures.

### The auto-mode permission gate declines `git push` and `gh pr create`

Not a bug: an external write. The commit landed locally and the push waited
for David's word. Worth knowing so it is not read as a failed push.

### A `[~]` in my own plan was invisible to the reader I was building

While marking a phase half-done I typed a non-standard checkbox marker into
`task_plan.md`. The extractor counts only `[ ]` and `[x]`, so the row simply
vanished from the count. Trivial, and exactly the kind of drift the lenient
rules are meant to survive — but it is also why the reader should never be
the only view of the file, which is why the markdown body is always there.

## Verification

- Local gates on the first commit (all green): `cargo build --release`,
  core 413 tests, CLI 12, bats `1..340` with 0 `not ok`, lint clean, app
  136, `tsc`, `cargo check -p app`, all 19 `app/scripts/*-check.mjs`. Later
  commits: core 415 (the two prompt tests), `tsc` and the 19 scripts again,
  bats and the rest re-run by the agent that touched Rust.
- CI: three runs on #323, nine jobs each, all success on macOS and Ubuntu.
- Tests shown failing first: the symlink refusal (three ways), the
  errors-table empty-row skip, comment stripping, the `check-complete.sh`
  cross-check, `PLAN_PROMPT` naming no directory, `PLAN_PROMPT` with a
  trailing newline.
- Harness (headless Chromium against the static mock build and against
  vite): every `?plan=` mode at dock 360 and 240, viewports 1400×900 and
  1000×560; nothing past the dock's edge; body ≥ 207px collapsed on the
  short window; band text 7.0:1 (`--txt-hi`) and 3.6–4.4:1 (`--txt-dim`) in
  tokyo-day and catppuccin-latte; the Generate button hit-tested and its one
  call recorded with the right session name; disabled state on a place with
  the session down.
- Hand test by David in the sandbox app against the four seeded shapes.
- The real binary's MCP `place_status` on the four sandbox places.

## Follow-ups

- **"What Claude last said."** The last assistant message from the
  transcript tail (`~/.claude/projects/<mangled>/<sid>.jsonl`, dated by its
  own `timestamp`, never mtime) as one more band item. Same file
  `transcript_epoch` already tails.
- **`notify_when_idle`'s one-liner** still has no capture point; the parked
  ROADMAP item keeps it.
- **Per-phase checkbox items** (text + done) in the contract would unlock the
  stepper and checklist directions (C and E on the design canvas) and a
  "next" card. One additive field.
- **Generate on a STALE plan**, not only an empty one — the button lives on
  the empty state today. The prompt wording should also be re-read after its
  first real use in a session.
- **The mock's freeform fixture** puts the plan at the root and supplies
  brief fields (valleos-shaped), but nothing covers a brief that is stale
  relative to its plan, which is where the brief-first header is weakest.
- **`.plan-phases` past eight rows** is a bounded nested scroller inside
  the expanded list. Rare; noted.
- **A static-mock build script** (`VITE_MOCK=1 vite build --base ./` into a
  scratch dir) was useful enough to share twice; worth an
  `app/scripts/mock-static.sh` and a note in CLAUDE.md's harness section.
- **Sandbox seeding for planning shapes** — the four-shape seed is a one-off
  in this session's `progress.md`; a `sandbox.sh --seed-plans` flag would
  make the hand test repeatable.
