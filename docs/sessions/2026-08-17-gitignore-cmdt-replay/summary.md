# 2026-08-17 — clean first status, ⌘T, files menu, and the shell that came back wrong

- **Date:** 2026-08-17 (evening, ran past midnight UTC)
- **Worktree:** `.worktrees/bug-fixes`
- **Branches:** `bug-fixes-gitignore-seed` (PR [#150](https://github.com/penard-monkey/worktrees/pull/150)), `bug-fixes-cmdt-files-menu` (PR [#151](https://github.com/penard-monkey/worktrees/pull/151)), `bug-fixes-shell-replay` (PR [#153](https://github.com/penard-monkey/worktrees/pull/153))
- **Release:** none (all three ship with the next one)
- **Planning files:** `planning.tar.gz` beside this summary
- **Workflow:** fable planned/briefed/reviewed; three opus agents implemented
  and ran gates, one per PR

## What shipped

1. **A project the app creates starts with a clean `git status`** (#150).
   `init_repo` (`app/src-tauri/src/lib.rs`) seeds a `.gitignore` — app state
   (`/.worktrees.places.json`, `/.worktrees/`) + planning docs — on the
   bootstrap-commit path only, staged so the existing `--allow-empty` commit
   carries it; create-or-nothing, an existing `.gitignore` is never touched.
   For repos that already have history, `store::edit`
   (`crates/worktrees-core/src/store.rs`) appends the two app-state lines to
   `.git/info/exclude` the first time it creates the store — per-checkout,
   pure `std::fs`, silently skipped when `.git` isn't a real directory.
2. **⌘T opens a new shell terminal tab from anywhere** (#151). Terminal tab
   visible → adds a shell; dock closed or on Files → brings the tab up and
   stops (the mount's restore already produces the tabs). ⌘⇧T kept as silent
   alias. `app/src/App.tsx`, `app/src/SettingsSheet.tsx`.
3. **Files tab right-click menu** (#151): Reveal in Finder / Open / Copy path /
   Copy relative path; dir rows lead with Open in Finder; inert rows get the
   copies only. Menu shell extracted to `app/src/CtxMenu.tsx` (shared with the
   nav's menus — same component, byte-identical rendering). `relPath` in
   `app/src/filekind.ts` + `app/scripts/relpath-check.mjs`.
4. **A dock shell tab comes back the way you left it** (#153). Attach waits
   for a measured host (rAF loop, 1s bound); the ResizeObserver ignores an
   unmeasured host; `shell_resize` is gated on the attach having answered;
   the backend remembers the pty's size and drops same-size requests
   (`next_pty_size`, `Shell::resize` in lib.rs). `app/src/TerminalPane.tsx`.
5. Hand fix outside the repo: `~/workspace/does-it-end` got the same
   `.gitignore` by hand (it was created before #150 existed).

## Decisions

- **`.git/info/exclude`, not `.gitignore`, for existing repos** — the choice
  is this checkout's, not the project's: never committed, and it cannot hide
  a *tracked* file, so a user who deliberately committed the store is
  unaffected. Writing into a stranger's `.gitignore` uninvited was rejected.
- **Seeding only on the bootstrap-commit path.** `first_commit` is shared
  with `create_initial_commit` (a repo the user owns, unborn HEAD) — seeding
  there would write files into a repo nobody asked us to touch.
- **⌘T never adds a tab in the same act that mounts the Terminal tab** — the
  token consumer's skip-initial guard makes the bump a no-op during mount,
  and the fix leans on that instead of fighting it.
- **No-op pty resizes are suppressed backend-side even though macOS/Linux
  TIOCSWINSZ already compares** — belt and braces; the real value is the pty
  size being first-class state and a tab flip costing zero ioctls.
- **Replay-then-resize order kept in `shell_open` re-attach**: ring and sink
  are taken in the reader's order; hoisting the resize above the snapshot
  would let the redraw it provokes land in neither.
- **No migration for pre-existing stores** (#150) and **no terminal-state
  serialization** (#153) — both deliberate scope cuts, parked in ROADMAP.

## Dead ends / gotchas

- **The first shell-replay hypothesis was half wrong, and only measurement
  caught it.** Assumed: SIGWINCH redraws accumulate in the replay ring and
  replay shows them all. Measured (real login zsh, real pty): a ring holding
  eight redraw copies replays as ONE clean prompt at its recording width —
  the `ESC[J` erases do their job. The entire visible bug is *width mismatch
  at replay*: one column off already leaves residue, and a 174-column ring
  replayed at 80 columns stacks the pasted line four deep with the cursor at
  row 19 — the user's screenshot exactly. A fix built on the original theory
  would have "worked" without touching the actual mechanism.
- **`fit()` does not throw on an unmeasured host — it returns early.**
  `safeFit()`'s try/catch never sees the failure, so the pane attached at
  xterm's default 80×24 and the backend obediently sized the pty to it,
  walking the shell real→80×24→real on every affected attach. Proven in the
  harness by holding `.term-host` at `display:none`: `shell_open` at 80×24
  plus three redundant resizes, corrected only when layout arrived.
- **`opener:allow-open-path` bare allows NOTHING.** The plugin's
  `is_path_allowed` ANDs the fs scope with "some allowed entry names a
  path"; without a scope entry the invoke rejects exactly as if the
  permission were missing — silently. And `**` still rejects every path with
  a dot component (`glob`'s `require_literal_leading_dot`, true by default on
  unix), which is every `.worktrees/…` path this app opens. Two fixes:
  scoped permission object + `plugins.opener.requireLiteralLeadingDot:
  false`. Both found by reading plugin source and probing glob standalone —
  the mock stubs `plugin:opener|*`, so no harness would ever have caught it.
- **`openPath` (plugin fn) collides with the `openPath` prop in
  FilesPane** — imported aliased as `openInDefaultApp`; tsc's symptom is the
  baffling "String has no call signatures".
- **TIOCSWINSZ compares before signalling on macOS/Linux**, so a suppression
  bug cannot show up as a spurious SIGWINCH in a test — the kernel hides it.
  The pty test therefore proves the opposite, catchable failure: a genuine
  size change still reaches the shell.
- **`grep -c` exits non-zero on a count of 0** and a pipeline's exit status
  is its last stage's — both already in CLAUDE.md, both encoded into every
  agent brief's gate instructions this session; no gate was miscounted.

## Verification

- Full gates per PR (three times, each by its implementing agent): release
  CLI build + freshness check, bats 314 ok / 0 not-ok, `make lint`, core 251,
  cli 7, app 35→37, `tsc --noEmit` + `cargo check -p app`. CI green on both
  OSes for all three PRs.
- Every new test shown red first (CLAUDE.md rule): gitignore-seed and
  exclude tests via targeted breaks; `relpath-check.mjs` via prefix/root
  breaks; `next_pty_size` bookkeeping and the SIGWINCH-trap pty test via
  three distinct breaks.
- #150 extra: both flows replayed against real git in a scratch dir —
  seeded new repo and exclude-only existing repo both answer
  `git status --porcelain` empty with app state present.
- #151: mock harness (:5199) — five ⌘T cases, both menu variants, dismissal
  paths, real clipboard round-trips, 27-property computed-style diff of the
  two menus. David exercised the features in the sandbox.
- #153: ring captures from a real zsh (2 copies per attach cycle, 0 for
  same-size resizes) + replay counts across widths in real `@xterm/xterm`
  (174→1 copy, 80→4 copies); harness before/after showing zero invokes while
  unmeasured, then one open at the real size.

## Follow-ups

- ROADMAP (added this session): one-time `.git/info/exclude` sweep for repos
  whose store predates #150; one real-app click on Files → Open for a
  dot-component path (the only unverified edge of #151).
- ROADMAP (added at close-out): a genuine layout change while a shell tab is
  detached still replays its older ring bytes at the wrong width — honest
  residual of #153; the full fix is terminal-state serialization, parked.
- David's sandbox pass after #153 not yet done (fix landed after his last
  run); the replay fix wants one real flip-and-close with a half-typed line.
