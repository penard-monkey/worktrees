# Files tab: what it hides, and what it lies about

- **Date:** 2026-08-10
- **Worktree:** `.worktrees/ui-changes` (branch `ui-next`)
- **PRs:** [#104](https://github.com/penard-monkey/worktrees/pull/104) gitignored by default ·
  [#105](https://github.com/penard-monkey/worktrees/pull/105) symlinks ·
  [#106](https://github.com/penard-monkey/worktrees/pull/106) release
- **Release:** v0.11.0 (`1e8f43d`), workflow green — 4 CLI targets, 2 signed
  bundles, `latest.json`
- **Planning files:** `planning.tar.gz` beside this summary
- **Evidence:** `files-tab.png` / `terminal-ls.png` — the same place, same
  moment, dock switched between Files and Terminal

## The report, and why it was wrong

> "the files list still is missing files from the file system"

Two screenshots of `casa-del-valle-monorepo / audit-and-financials`: the FILES
dock beside a shell running `ls` in that worktree. Four entries were on disk and
not in the tab — `task_plan.md`, `findings.md`, `progress.md`, `node_modules/`.

Nothing was missing. `list_dir` does a real `read_dir` and then drops what
`git check-ignore` flags unless `files_show_ignored` is on — and that setting,
shipped in 0.10.0, defaulted to **off**. The four were exactly the four
gitignored entries. `ui-state.json` (mtime 05:03, screenshots 05:00) confirmed
`"files_show_ignored": false`.

**The premise I wrote into `findings.md` was itself wrong**, and that mattered
more than the bug. I recorded that the tab was keeping *some* ignored entries
(`_tmp`, `old-plans`, `patches`, `.redesign-shots`) while dropping others, and
built a theory on a filter that was "not all-or-nothing". Those four are all
**tracked** — `check-ignore` consults the index, so it never flagged them.
`_tmp` is a tracked symlink (mode `120000`); that monorepo's `.gitignore`
ignores only `_tmp/*` and `_tmp/**` under the comment "Track the symlink, never
the contents". Two fable agents, backend and frontend, independently landed on
the same answer; the thing that actually killed the false premise was running
`git check-ignore --stdin -z` by hand over the disputed paths.

So the defect was never the listing. It was that the tab withheld entries with
**nothing on screen admitting it**, behind an unlabelled `◌` that defaulted off.

## What shipped

### #104 — gitignored entries listed by default

- `app/src/settings.ts` — `files_show_ignored` default `false` → `true`. New
  `settings_rev` + `SETTINGS_REV` + `migrate()`: `loadSettings` merges
  `{...DEFAULTS, ...persisted}`, so a stored `false` outranks a new default
  **forever** — the flip would have been invisible to precisely the people
  already running the app. The migration re-applies changed defaults once and
  persists the rev, so a later toggle-off still sticks.
- `app/src/App.tsx` — the toggle's glyph now carries state, `◉` shown / `◌`
  hiding. The `on` tint alone was a weak signal for the state that misleads.
- Accepted consequence, called out in the PR: someone who *deliberately* turned
  ignored files off in 0.10.0 gets flipped back on once. Full-blob persistence
  makes a deliberate `false` indistinguishable from a defaulted one.

### #105 — symlinks render as symlinks

- `app/src-tauri/src/lib.rs` — `read_dir`'s `file_type()` is an lstat, so a
  symlink to a directory came back `is_dir: false`: file glyph, no caret, and a
  click that asked the viewer to read a directory as text. `_tmp`, which this
  repo's own convention puts in every worktree, was that row. Now stat'd
  **through** for shape, carrying the target as written plus `link_block` —
  `"outside"` / `"git"` / `"missing"`, or `None` when followable.
- `app/src/FilesPane.tsx`, `App.css` — blocked links render inert: no caret,
  click does nothing, tooltip gives the reason.
- `classify_symlink`, `project_roots`, `under_roots` extracted; unit test drives
  **real** symlinks in `$TMPDIR` (a fake would test nothing — the whole question
  is what the syscalls answer).

## Decisions

**Show ignored by default rather than annotate what is hidden.** Offered three
shapes (default-on / keep-off-plus-a-"4 hidden" affordance / both). Chosen:
default on, dimmed. The tab should mirror the filesystem out of the box.

**Migrate rather than just flip.** Non-optional given `{...DEFAULTS, ...raw}`,
and the alternative — renaming the key to invert its sense — leaves dead data
and a double negative in every call site.

**Mark symlinks; do not follow ones that leave the workspace.** Offered
follow-anywhere and follow-only-declared as alternatives. Following a
repo-supplied link would let a repo's own contents choose what the app opens —
the argument ADR 0001 makes about argv. The cost is accepted and visible:
`_tmp` still will not expand, but it now says why instead of pretending to be a
file.

**A reason code, not a bool.** All three blocked states render identically, but
"outside the workspace" is a false statement about a link that merely dangles,
and the tooltip is the only place a user learns why nothing happens.

## Dead ends / gotchas

**`ln -s .git g` would have made `.git` browsable.** Found by fable review of
#105, not by me. The listing drops `.git` by **name**, and `guard_under_projects`
has no opinion about it — it accepts any existing path under a root. So the fix,
by making links to directories expandable, opened a walk-around to the one
directory the tree deliberately hides. Blocked in the classifier, pinned by the
test. Note the shape of this: the fix's own mechanism created the hole, which is
why "verify the fixes" was worth a second review pass rather than a re-read.

**The claim that made it into release notes was false.** Both the CHANGELOG and
two comments said "every command behind the row canonicalizes first and would
refuse it". True for `outside` and `missing`; **false for `git`** — the guard
would list it happily, so classification is the whole defense there, not a
courtesy over a guard that would have caught it anyway. Caught on the re-review.
That was the sentence a reader would have used to reason about the security
property.

**A wrong premise survives longer when it is written down.** `findings.md` said
`_tmp` and `old-plans` were ignored-but-shown. Every subsequent step inherited
it, including the prompts I handed the subagents. Reproduction fixed it;
re-reading would not have.

**Remote `ui-next` needed a lease-force after the first squash-merge.** Squashing
#104 gave `main` a commit that is not the branch's, so the local reset to
`origin/main` diverged from the still-live remote branch. `--force-with-lease`,
not `--force`.

**`main` moved mid-stream** (#103 landed from another worktree). Merged it in
and re-ran the app gates before pushing — the merge touched `App.tsx`.

## Verification

- **Backend:** `cargo test -p app` — 8 passing incl. the new symlink test
  (relative in-workspace dir link, file link, absolute out-of-workspace link,
  dangling link, `ln -s .git`). Review additionally probed `.GIT`,
  `sub/../.GIT` and a two-hop uppercase link against `canonicalize` on APFS —
  all resolve to the on-disk `.git` component, so macOS case-insensitivity does
  not slip past.
- **Frontend, asserted through the DOM and `getComputedStyle`, not eyeballed:**
  fresh settings list ignored rows dimmed at `--txt-mute`; toggling hides them
  and the glyph hollows; an OFF choice persisted with `rev 1` survives reload
  without the migration re-running; a seeded 0.10.0-shaped blob migrates to
  `{rev: 1, show: true}`; `_tmp` renders `dir link inert` with no caret, cursor
  `default`, click does nothing, no error banner, no viewer; `shared → ../shared`
  keeps its caret and expands.
- **Gates, re-run after merging main:** `make test` 0 failures, `make lint`,
  `cargo test` across core/cli/app, `tsc --noEmit`, `cargo check -p app`.
  CI 9/9 on both OSes for every PR.
- **Release:** workflow green; assets confirmed via `gh release view v0.11.0`.

## Follow-ups

Roadmapped:
- The `.git` read-path gap — a markdown doc link to `/abs/path/.git/config`
  still opens in the viewer via `read_file`. Pre-existing on main, not a
  regression, and the same class of hole the classifier just closed for links.
- The containment predicate now exists twice (inline in the guard for its
  short-circuit, and `under_roots` for the slice) — drift hazard.

Noted, not worth an item:
- `classify_symlink` stats through every link, so a link to a hung network
  mount stalls that one listing. Async command, no UI freeze; inherent to
  statting for shape.
- An in-workspace cycle (`ln -s . self`) is expandable indefinitely. User-driven
  only; Finder allows it too.
- Children under a followed link carry canonical target-side paths, so the same
  file selected via `inside/` also highlights under `sub/`.
- Mock nit: a *direct* harness invoke of `list_dir` on a blocked path before its
  parent has been listed bypasses `fsBlocked`. Unreachable through the UI.
